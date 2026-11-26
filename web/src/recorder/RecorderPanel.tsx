import { useCallback, useEffect, useRef, useState } from "react";
import { Banner } from "../ui/Banner";
import { DisplayFrame } from "../display/DisplayFrame";
import type { ShellDisplay } from "../shell/session";
import type { Transport } from "../transport/types";
import {
  bytesToBase64,
  encodeWav,
  formatRecordingClock,
  recordingFileName,
} from "./recording";
import "./recorder.css";

export interface RecorderPanelProps {
  display: ShellDisplay;
  transport: Transport | null;
  /** Hand the saved recording to the shell, which opens it in a Spectrogram. */
  onRecorded: (path: string) => void;
}

type Phase = "idle" | "requesting" | "recording" | "saving" | "error";

interface ActiveRecording {
  context: AudioContext;
  source: MediaStreamAudioSourceNode;
  processor: ScriptProcessorNode;
  sink: GainNode;
  stream: MediaStream;
  chunks: Float32Array[];
  startedAt: number;
}

function recordingErrorText(reason: unknown): string {
  const name = reason instanceof DOMException ? reason.name : "";
  if (name === "NotAllowedError" || name === "SecurityError") {
    return "Microphone access was denied. Allow microphone access for this app and try again.";
  }
  if (name === "NotFoundError" || name === "OverconstrainedError") {
    return "No microphone was found.";
  }
  if (name === "NotReadableError") {
    return "The microphone is busy or unavailable.";
  }
  return reason instanceof Error ? reason.message : String(reason);
}

/**
 * Microphone take recorder. Audio is captured as PCM through Web Audio and
 * saved as a lossless WAV, then opened in a Spectrogram: the Recorder tab is
 * replaced by that Spectrogram.
 */
export function RecorderPanel({
  display,
  transport,
  onRecorded,
}: RecorderPanelProps) {
  const [phase, setPhase] = useState<Phase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [elapsed, setElapsed] = useState(0);
  const [device, setDevice] = useState<string | null>(null);
  const active = useRef<ActiveRecording | null>(null);
  const mounted = useRef(true);
  const saveRef = useRef<(chunks: Float32Array[], sampleRate: number) => void>(
    () => {},
  );

  const save = useCallback(
    async (chunks: Float32Array[], sampleRate: number) => {
      if (!transport) {
        return;
      }
      setPhase("saving");
      try {
        const wav = encodeWav(chunks, sampleRate);
        if (wav.length <= 44) {
          throw new Error("The recording is empty.");
        }
        const filename = recordingFileName();
        const saved = await transport.invoke({
          name: "recording.save",
          args: { dataBase64: bytesToBase64(wav), filename },
        });
        if (!mounted.current) {
          return;
        }
        setElapsed(0);
        setPhase("idle");
        onRecorded(saved.path);
      } catch (reason) {
        if (!mounted.current) {
          return;
        }
        setError(reason instanceof Error ? reason.message : String(reason));
        setPhase("error");
      }
    },
    [onRecorded, transport],
  );
  saveRef.current = (chunks, sampleRate) => {
    void save(chunks, sampleRate);
  };

  useEffect(() => {
    if (phase !== "recording") {
      return;
    }
    const timer = window.setInterval(() => {
      const current = active.current;
      if (current) {
        setElapsed((performance.now() - current.startedAt) / 1000);
      }
    }, 200);
    return () => window.clearInterval(timer);
  }, [phase]);

  const teardown = useCallback((current: ActiveRecording) => {
    current.processor.onaudioprocess = null;
    current.source.disconnect();
    current.processor.disconnect();
    current.sink.disconnect();
    current.stream.getTracks().forEach((track) => track.stop());
    void current.context.close().catch(() => {});
  }, []);

  useEffect(
    () => () => {
      mounted.current = false;
      const current = active.current;
      active.current = null;
      if (current) {
        // Closing the tab discards the take; never save behind the user's back.
        teardown(current);
      }
    },
    [teardown],
  );

  const start = useCallback(async () => {
    setError(null);
    if (!transport) {
      setError("Recording needs a backend to save the take.");
      setPhase("error");
      return;
    }
    if (
      typeof AudioContext === "undefined" ||
      typeof navigator === "undefined" ||
      !navigator.mediaDevices?.getUserMedia
    ) {
      setError("Audio recording is not supported in this build.");
      setPhase("error");
      return;
    }
    setPhase("requesting");
    let stream: MediaStream | null = null;
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          echoCancellation: false,
          noiseSuppression: false,
          autoGainControl: false,
        },
      });
      if (!mounted.current) {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      const context = new AudioContext();
      await context.resume();
      const source = context.createMediaStreamSource(stream);
      const processor = context.createScriptProcessor(4096, 1, 1);
      const chunks: Float32Array[] = [];
      processor.onaudioprocess = (event) => {
        // The input buffer is reused by the audio thread: keep a copy.
        chunks.push(new Float32Array(event.inputBuffer.getChannelData(0)));
      };
      // ScriptProcessor only runs while connected to the destination; a zero
      // gain sink keeps the microphone out of the speakers.
      const sink = context.createGain();
      sink.gain.value = 0;
      source.connect(processor);
      processor.connect(sink);
      sink.connect(context.destination);

      active.current = {
        context,
        source,
        processor,
        sink,
        stream,
        chunks,
        startedAt: performance.now(),
      };
      setElapsed(0);
      setDevice(stream.getAudioTracks()[0]?.label || null);
      setPhase("recording");
    } catch (reason) {
      stream?.getTracks().forEach((track) => track.stop());
      if (mounted.current) {
        setError(recordingErrorText(reason));
        setPhase("error");
      }
    }
  }, [transport]);

  const stop = useCallback(() => {
    const current = active.current;
    if (!current) {
      return;
    }
    active.current = null;
    setPhase("saving");
    teardown(current);
    saveRef.current(current.chunks, current.context.sampleRate);
  }, [teardown]);

  const recording = phase === "recording";
  const busy = phase === "requesting" || phase === "saving";

  return (
    <DisplayFrame
      title={display.name}
      kind="recorder"
      state={{ kind: "ready" }}
      footerHints="Microphone audio is saved locally, then opened in a new Spectrogram"
      footerRight={recording ? "Recording" : "Idle"}
    >
      <div className="recorder-panel">
        <div className="recorder-card">
          <button
            type="button"
            className={`record-button${recording ? " recording" : ""}`}
            onClick={recording ? stop : () => void start()}
            disabled={busy}
            aria-pressed={recording}
          >
            <span className="record-dot" aria-hidden="true" />
            {recording
              ? "Stop recording"
              : phase === "requesting"
                ? "Requesting microphone…"
                : phase === "saving"
                  ? "Saving recording…"
                  : "Start recording"}
          </button>
          <div className="recorder-clock" role="status" aria-live="polite">
            {recording
              ? `Recording · ${formatRecordingClock(elapsed)}`
              : phase === "saving"
                ? "Saving recording…"
                : "Ready"}
          </div>
          <p className="recorder-device">
            {device ? `Input: ${device}` : "Default microphone input"}
          </p>
          <p className="recorder-hint">
            The take is saved as a lossless WAV in the app&apos;s recordings
            folder and opened in a Spectrogram. Microphone processing (echo
            cancellation, noise suppression, auto gain) is disabled so the
            spectrum stays unmodified.
          </p>
        </div>
        {error ? (
          <div className="recorder-error">
            <Banner
              tone="error"
              onDismiss={() => {
                setError(null);
                setPhase("idle");
              }}
            >
              {error}
            </Banner>
          </div>
        ) : null}
      </div>
    </DisplayFrame>
  );
}

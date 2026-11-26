// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { MockTransport } from "../transport/mock";
import type { ShellDisplay } from "../shell/session";
import { RecorderPanel } from "./RecorderPanel";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  Object.defineProperty(navigator, "mediaDevices", {
    configurable: true,
    value: undefined,
  });
});

const DISPLAY: ShellDisplay = {
  id: 9,
  parent: null,
  kind: "recorder",
  name: "Recorder",
  state: {},
};

type AudioProcessEvent = {
  inputBuffer: { getChannelData: (channel: number) => Float32Array };
};

class FakeProcessor {
  onaudioprocess: ((event: AudioProcessEvent) => void) | null = null;
  connect = vi.fn();
  disconnect = vi.fn();

  emit(samples: number[]) {
    this.onaudioprocess?.({
      inputBuffer: { getChannelData: () => Float32Array.from(samples) },
    });
  }
}

class FakeAudioContext {
  static instances: FakeAudioContext[] = [];

  readonly sampleRate = 48_000;
  readonly destination = {};
  readonly processor = new FakeProcessor();
  readonly source = { connect: vi.fn(), disconnect: vi.fn() };
  readonly sink = {
    gain: { value: 1 },
    connect: vi.fn(),
    disconnect: vi.fn(),
  };
  resume = vi.fn(async () => {});
  close = vi.fn(async () => {});

  constructor() {
    FakeAudioContext.instances.push(this);
  }

  createMediaStreamSource() {
    return this.source;
  }

  createScriptProcessor() {
    return this.processor;
  }

  createGain() {
    return this.sink;
  }
}

function stubMicrophone(getUserMedia: (constraints: unknown) => Promise<unknown>) {
  Object.defineProperty(navigator, "mediaDevices", {
    configurable: true,
    value: { getUserMedia },
  });
}

function fakeStream() {
  const stop = vi.fn();
  const track = { stop, label: "Fake microphone" };
  return {
    stream: {
      getTracks: () => [track],
      getAudioTracks: () => [track],
    } as unknown as MediaStream,
    stop,
  };
}

function decodeWav(bytes: Uint8Array) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const frames = (bytes.length - 44) / 2;
  return {
    riff: String.fromCharCode(...bytes.slice(0, 4)),
    channels: view.getUint16(22, true),
    sampleRate: view.getUint32(24, true),
    bits: view.getUint16(34, true),
    dataSize: view.getUint32(40, true),
    samples: Array.from(
      { length: frames },
      (_, index) => view.getInt16(44 + index * 2, true),
    ),
  };
}

describe("RecorderPanel", () => {
  it("records PCM, saves a WAV through the transport, and opens it", async () => {
    FakeAudioContext.instances = [];
    const { stream, stop } = fakeStream();
    const getUserMedia = vi.fn(async () => stream);
    stubMicrophone(getUserMedia);
    vi.stubGlobal("AudioContext", FakeAudioContext);
    const transport = new MockTransport({
      "recording.save": async ({ filename }) => ({ path: `/tmp/${filename}` }),
    });
    const onRecorded = vi.fn();

    render(
      <RecorderPanel
        display={DISPLAY}
        transport={transport}
        onRecorded={onRecorded}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Start recording" }));
    await screen.findByRole("button", { name: "Stop recording" });
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: {
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
      },
    });
    expect(screen.getByText("Input: Fake microphone")).toBeTruthy();

    const context = FakeAudioContext.instances.at(-1);
    expect(context).toBeTruthy();
    context?.processor.emit([0.5, -0.5, 1, -1]);
    fireEvent.click(screen.getByRole("button", { name: "Stop recording" }));
    await waitFor(() => expect(onRecorded).toHaveBeenCalledOnce());
    expect(stop).toHaveBeenCalled();

    expect(onRecorded.mock.calls[0][0]).toMatch(
      /^\/tmp\/recording-.*\.wav$/,
    );
    const call = transport.calls.find(
      (command) => command.name === "recording.save",
    );
    expect(call).toBeTruthy();
    const args = call?.args as { dataBase64: string; filename: string };
    expect(args.filename).toMatch(/^recording-.*\.wav$/);
    const bytes = Uint8Array.from(atob(args.dataBase64), (character) =>
      character.charCodeAt(0),
    );
    const wav = decodeWav(bytes);
    expect(wav.riff).toBe("RIFF");
    expect(wav.channels).toBe(1);
    expect(wav.sampleRate).toBe(48_000);
    expect(wav.bits).toBe(16);
    expect(wav.dataSize).toBe(8);
    expect(wav.samples[0]).toBe(16_383);
    expect(wav.samples[1]).toBe(-16_384);
    expect(wav.samples[2]).toBe(32_767);
    expect(wav.samples[3]).toBe(-32_768);
  });

  it("explains a denied microphone permission", async () => {
    stubMicrophone(async () => {
      throw new DOMException("denied", "NotAllowedError");
    });
    vi.stubGlobal("AudioContext", FakeAudioContext);

    render(
      <RecorderPanel
        display={DISPLAY}
        transport={new MockTransport({})}
        onRecorded={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Start recording" }));
    await screen.findByText(/Microphone access was denied/);
    expect(
      screen.getByRole("button", { name: "Start recording" }),
    ).toBeTruthy();
  });

  it("reports builds without recording support", async () => {
    render(
      <RecorderPanel
        display={DISPLAY}
        transport={new MockTransport({})}
        onRecorded={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Start recording" }));
    await screen.findByText(/not supported in this build/);
  });
});

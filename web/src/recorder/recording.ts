/** Audio-recording helpers for the Recorder display. */

/**
 * Timestamped take name, safe as a file name on every platform.
 *
 * Takes are stored as 16-bit PCM WAV: it decodes everywhere the backend runs
 * and keeps the recorded spectrum lossless.
 */
export function recordingFileName(now: Date = new Date()): string {
  const stamp = now.toISOString().replace(/[:.]/g, "-");
  return `recording-${stamp}.wav`;
}

/**
 * Encode mono PCM captured by Web Audio as a 16-bit WAV.
 *
 * Samples are clamped to [-1, 1]; negative samples scale by `0x8000` and
 * positive ones by `0x7fff` so full-scale input does not wrap.
 */
export function encodeWav(
  chunks: readonly Float32Array[],
  sampleRate: number,
): Uint8Array {
  const frames = chunks.reduce((total, chunk) => total + chunk.length, 0);
  const buffer = new ArrayBuffer(44 + frames * 2);
  const view = new DataView(buffer);
  const writeAscii = (offset: number, text: string) => {
    for (let index = 0; index < text.length; index += 1) {
      view.setUint8(offset + index, text.charCodeAt(index));
    }
  };

  writeAscii(0, "RIFF");
  view.setUint32(4, 36 + frames * 2, true);
  writeAscii(8, "WAVE");
  writeAscii(12, "fmt ");
  view.setUint32(16, 16, true); // fmt chunk size
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true); // byte rate
  view.setUint16(32, 2, true); // block align
  view.setUint16(34, 16, true); // bits per sample
  writeAscii(36, "data");
  view.setUint32(40, frames * 2, true);

  let offset = 44;
  for (const chunk of chunks) {
    for (let index = 0; index < chunk.length; index += 1) {
      const sample = Math.max(-1, Math.min(1, chunk[index]));
      view.setInt16(
        offset,
        sample < 0 ? sample * 0x8000 : sample * 0x7fff,
        true,
      );
      offset += 2;
    }
  }
  return new Uint8Array(buffer);
}

/** Base64 for the transport payload; chunked so long takes cannot overflow. */
export function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  const chunk = 0x8000;
  for (let index = 0; index < bytes.length; index += chunk) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunk));
  }
  return btoa(binary);
}

/** `m:ss` recording clock. */
export function formatRecordingClock(seconds: number): string {
  const whole = Math.max(0, Math.floor(seconds));
  const minutes = Math.floor(whole / 60);
  return `${minutes}:${String(whole % 60).padStart(2, "0")}`;
}

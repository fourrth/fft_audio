import { describe, expect, it } from "vitest";
import {
  bytesToBase64,
  encodeWav,
  formatRecordingClock,
  recordingFileName,
} from "./recording";

function viewOf(bytes: Uint8Array): DataView {
  return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

describe("recording helpers", () => {
  it("writes a mono 16-bit WAV header and clamps samples", () => {
    const bytes = encodeWav(
      [Float32Array.from([0, 0.5, -0.5, 1, -1, 2])],
      48_000,
    );
    const view = viewOf(bytes);
    const ascii = (offset: number, length: number) =>
      String.fromCharCode(...bytes.slice(offset, offset + length));

    expect(ascii(0, 4)).toBe("RIFF");
    expect(view.getUint32(4, true)).toBe(bytes.length - 8);
    expect(ascii(8, 4)).toBe("WAVE");
    expect(ascii(12, 4)).toBe("fmt ");
    expect(view.getUint16(20, true)).toBe(1);
    expect(view.getUint16(22, true)).toBe(1);
    expect(view.getUint32(24, true)).toBe(48_000);
    expect(view.getUint32(28, true)).toBe(96_000);
    expect(view.getUint16(32, true)).toBe(2);
    expect(view.getUint16(34, true)).toBe(16);
    expect(ascii(36, 4)).toBe("data");
    expect(view.getUint32(40, true)).toBe(12);
    expect(bytes.length).toBe(44 + 12);
    expect(view.getInt16(44, true)).toBe(0);
    expect(view.getInt16(46, true)).toBe(16_383);
    expect(view.getInt16(48, true)).toBe(-16_384);
    expect(view.getInt16(50, true)).toBe(32_767);
    // Both out-of-range samples clamp to full scale.
    expect(view.getInt16(52, true)).toBe(-32_768);
    expect(view.getInt16(54, true)).toBe(32_767);
  });

  it("concatenates capture chunks and keeps the sample rate", () => {
    const bytes = encodeWav(
      [Float32Array.from([0.25]), Float32Array.from([0.25, 0.25])],
      44_100,
    );
    const view = viewOf(bytes);
    expect(view.getUint32(24, true)).toBe(44_100);
    expect(view.getUint32(40, true)).toBe(6);
    expect(bytes.length).toBe(44 + 6);
  });

  it("names takes with a filesystem-safe timestamp", () => {
    expect(
      recordingFileName(new Date("2026-11-25T19:05:35.123Z")),
    ).toBe("recording-2026-11-25T19-05-35-123Z.wav");
  });

  it("formats the recording clock and base64 payloads", () => {
    expect(formatRecordingClock(0)).toBe("0:00");
    expect(formatRecordingClock(65.9)).toBe("1:05");
    expect(formatRecordingClock(3_600)).toBe("60:00");
    expect(bytesToBase64(Uint8Array.from([1, 2, 3]))).toBe("AQID");
    expect(bytesToBase64(new Uint8Array(0))).toBe("");
  });
});

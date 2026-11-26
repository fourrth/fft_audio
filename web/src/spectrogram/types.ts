import type { AnalysisSettings, TableInfo } from "../settings/model";

export interface SpectrogramMeta {
  frames: number;
  bins: number;
  binHz: number;
  frameS: number;
  durationS: number;
  nyquist: number;
  maxDb: number;
  minDb: number;
  sampleRate: number;
  analysisSettings?: AnalysisSettings;
  /** Present when the signal came from a data table. */
  table?: TableInfo | null;
}

export interface SpectrogramPayload {
  meta: SpectrogramMeta;
  /** Base64 of frames × bins little-endian int16 dB×100 values. */
  data: string;
}

export interface DecodedGrid {
  meta: SpectrogramMeta;
  values: Int16Array;
}

export function decodeSpectrogram(payload: SpectrogramPayload): DecodedGrid {
  const binary = atob(payload.data);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i);
  }
  if (bytes.length % 2 !== 0) {
    throw new Error("spectrogram payload has an odd byte length");
  }
  const values = new Int16Array(bytes.length / 2);
  const view = new DataView(bytes.buffer);
  for (let i = 0; i < values.length; i += 1) {
    values[i] = view.getInt16(i * 2, true);
  }
  return { meta: payload.meta, values };
}

/** dB at one grid cell (0-based frame, bin). */
export function gridDb(grid: DecodedGrid, frame: number, bin: number): number | null {
  if (
    frame < 0 ||
    bin < 0 ||
    frame >= grid.meta.frames ||
    bin >= grid.meta.bins
  ) {
    return null;
  }
  return grid.values[frame * grid.meta.bins + bin] / 100;
}

/** Bilinear dB lookup at a time/frequency, clamped to the grid. */
export function dbAt(
  grid: DecodedGrid,
  timeS: number,
  hz: number,
): number | null {
  const { frames, bins, durationS, nyquist } = grid.meta;
  if (frames === 0 || bins === 0 || !(durationS >= 0) || !(nyquist > 0)) {
    return null;
  }
  if (timeS < 0 || timeS > durationS || hz < 0 || hz > nyquist) {
    return null;
  }
  const fx = durationS > 0 ? (timeS / durationS) * (frames - 1) : 0;
  const bx = nyquist > 0 ? (hz / nyquist) * (bins - 1) : 0;
  const f0 = Math.floor(fx);
  const b0 = Math.floor(bx);
  const f1 = Math.min(frames - 1, f0 + 1);
  const b1 = Math.min(bins - 1, b0 + 1);
  const tf = fx - f0;
  const tb = bx - b0;
  const sample = (f: number, b: number) => grid.values[f * bins + b] / 100;
  const a = sample(f0, b0) * (1 - tf) + sample(f1, b0) * tf;
  const c = sample(f0, b1) * (1 - tf) + sample(f1, b1) * tf;
  return a * (1 - tb) + c * tb;
}

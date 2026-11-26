/** Settings DTOs matching the Rust core serde shape (snake_case). */

import type {
  LineCutSettings,
  OrderCutSettings,
} from "../cuts/settings";

export interface DataSettings {
  column: string | null;
  time_column: string | null;
  sample_rate: number | null;
  remove_mean: boolean;
}

export type WindowKind =
  | "hann"
  | "hamming"
  | "blackman"
  | "nuttall"
  | "triangular"
  | "rectangular"
  | "none";

export interface AnalysisSettings {
  frame: number;
  hop: number | null;
  pad: number;
  window: WindowKind;
  highpass: number | null;
  lowpass: number | null;
  smooth_s: number;
  data: DataSettings;
}

export interface DisplaySettings {
  log_freq: boolean;
  /** Draw tick-aligned grid lines behind plot data. */
  show_grid: boolean;
  max_hz: number | null;
  min_hz?: number | null;
  range_db: number;
  db_min?: number | null;
  db_max?: number | null;
}

export interface AppearanceSettings {
  theme: string;
  ui_scale: number;
  reduce_motion: boolean;
  high_contrast: boolean;
}

export interface SessionSettings {
  reopen_last: boolean;
}

export interface TableInfo {
  rows: number;
  columns: string[];
  signal: string;
  time: string | null;
  notes: string[];
}

/** Analysis presets exposed by the Inspector's main Analysis card. */
export type ResolutionPreset = "high-time" | "balanced" | "high-frequency";

export interface ResolutionPresetSpec {
  id: ResolutionPreset;
  label: string;
  description: string;
  /** Target analysis window in seconds. */
  windowSeconds: number;
}

export const RESOLUTION_PRESETS: ResolutionPresetSpec[] = [
  {
    id: "high-time",
    label: "High time resolution",
    description: "~10 ms window; follows fast changes",
    windowSeconds: 0.01,
  },
  {
    id: "balanced",
    label: "Balanced",
    description: "~50 ms window; good default for audio",
    windowSeconds: 0.05,
  },
  {
    id: "high-frequency",
    label: "High frequency resolution",
    description: "~200 ms window; resolves close tones",
    windowSeconds: 0.2,
  },
];

const MIN_FRAME = 16;
const MAX_FRAME = 65_536;

function nearestPowerOfTwo(value: number): number {
  const lower = Math.floor(Math.log2(value));
  const lowerValue = 2 ** lower;
  const upperValue = 2 ** (lower + 1);
  return value - lowerValue <= upperValue - value ? lowerValue : upperValue;
}

/**
 * Frame/hop/pad for a preset at `sampleRate`, or `null` when the sample rate
 * is unknown. Frames are rounded to powers of two for predictable FFT sizes.
 */
export function resolutionPresetSettings(
  preset: ResolutionPreset,
  sampleRate: number | null | undefined,
): Pick<AnalysisSettings, "frame" | "hop" | "pad"> | null {
  const spec = RESOLUTION_PRESETS.find((entry) => entry.id === preset);
  if (!spec || !sampleRate || !Number.isFinite(sampleRate) || sampleRate <= 0) {
    return null;
  }
  const target = sampleRate * spec.windowSeconds;
  const frame = Math.min(
    MAX_FRAME,
    Math.max(MIN_FRAME, nearestPowerOfTwo(target)),
  );
  return { frame, hop: Math.max(1, Math.floor(frame / 4)), pad: 2 };
}

/** The preset matching the current settings, or `null` for custom settings. */
export function activeResolutionPreset(
  settings: AnalysisSettings,
  sampleRate: number | null | undefined,
): ResolutionPreset | null {
  for (const preset of RESOLUTION_PRESETS) {
    const values = resolutionPresetSettings(preset.id, sampleRate);
    if (
      values &&
      settings.frame === values.frame &&
      settings.pad === values.pad &&
      hopSamples(settings) === values.hop
    ) {
      return preset.id;
    }
  }
  return null;
}

export interface Settings {
  analysis: AnalysisSettings;
  display: DisplaySettings;
  order_cuts: OrderCutSettings;
  line_cuts: LineCutSettings;
  session: SessionSettings;
  appearance: AppearanceSettings;
}

export const WINDOW_KINDS: WindowKind[] = [
  "hann",
  "hamming",
  "blackman",
  "nuttall",
  "triangular",
  "rectangular",
  "none",
];

export function defaultAnalysisSettings(): AnalysisSettings {
  return {
    frame: 2048,
    hop: null,
    pad: 2,
    window: "hann",
    highpass: null,
    lowpass: null,
    smooth_s: 0,
    data: {
      column: null,
      time_column: null,
      sample_rate: null,
      remove_mean: true,
    },
  };
}

export function defaultDisplaySettings(): DisplaySettings {
  return {
    log_freq: true,
    show_grid: true,
    max_hz: null,
    min_hz: null,
    range_db: 80,
    db_min: null,
    db_max: null,
  };
}

export function defaultAppearanceSettings(): AppearanceSettings {
  return {
    theme: "dark",
    ui_scale: 1,
    reduce_motion: false,
    high_contrast: false,
  };
}

export function defaultSettings(): Settings {
  return {
    analysis: defaultAnalysisSettings(),
    display: defaultDisplaySettings(),
    order_cuts: {
      orders: "1,2,4",
      order_width: 0.2,
      freqs: "",
      freq_width_hz: 5,
      axis: "rpm",
      smooth_s: 0,
      peak_snap: true,
      peak_min_db: 1,
      rpm: {
        source: "constant",
        constant: 2000,
        start: 1000,
        end: 3000,
        csv: null,
      },
    },
    line_cuts: {
      line_width_hz: 0,
      line_axis: "auto",
      snap: true,
      snap_deg: 2,
      peak_snap: true,
      peak_min_db: 1,
    },
    session: { reopen_last: true },
    appearance: defaultAppearanceSettings(),
  };
}

/** Clamp a UI scale fraction to the supported 0.75–2.0 range. */
export function clampScale(scale: number): number {
  return Math.min(2, Math.max(0.75, Number.isFinite(scale) ? scale : 1));
}

export function scalePercent(scale: number): number {
  return Math.round(clampScale(scale) * 100);
}

export function scaleFromPercent(percent: number): number {
  return clampScale(percent / 100);
}

/** Apply appearance flags to a document root (DOM or a test fake). */
export function applyAppearance(
  settings: {
    ui_scale: number;
    reduce_motion: boolean;
    high_contrast: boolean;
    theme: string;
  },
  root?: {
    style?: { setProperty(name: string, value: string): void };
    setAttribute?(name: string, value: string): void;
    removeAttribute?(name: string): void;
  },
): void {
  const target =
    root ??
    (typeof document === "undefined" ? undefined : document.documentElement);
  if (!target) {
    return;
  }
  target.style?.setProperty("--ui-scale", String(clampScale(settings.ui_scale)));
  target.style?.setProperty("--anim-ms", settings.reduce_motion ? "0ms" : "140ms");
  target.setAttribute?.("data-theme", settings.theme);
  target.setAttribute?.(
    "data-reduce-motion",
    settings.reduce_motion ? "true" : "false",
  );
  target.setAttribute?.(
    "data-high-contrast",
    settings.high_contrast ? "true" : "false",
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function readAnalysisSettings(state: Record<string, unknown>): AnalysisSettings {
  const value = state.analysisSettings;
  if (!isRecord(value)) {
    return defaultAnalysisSettings();
  }
  const base = defaultAnalysisSettings();
  return {
    ...base,
    ...(typeof value.frame === "number" ? { frame: value.frame } : {}),
    ...(typeof value.hop === "number" || value.hop === null ? { hop: value.hop } : {}),
    ...(typeof value.pad === "number" ? { pad: value.pad } : {}),
    ...(WINDOW_KINDS.includes(value.window as WindowKind)
      ? { window: value.window as WindowKind }
      : {}),
    ...(typeof value.highpass === "number" || value.highpass === null
      ? { highpass: value.highpass }
      : {}),
    ...(typeof value.lowpass === "number" || value.lowpass === null
      ? { lowpass: value.lowpass }
      : {}),
    ...(typeof value.smooth_s === "number" ? { smooth_s: value.smooth_s } : {}),
    ...(isRecord(value.data)
      ? {
          data: {
            column:
              typeof value.data.column === "string" ? value.data.column : null,
            time_column:
              typeof value.data.time_column === "string"
                ? value.data.time_column
                : null,
            sample_rate:
              typeof value.data.sample_rate === "number"
                ? value.data.sample_rate
                : null,
            remove_mean:
              typeof value.data.remove_mean === "boolean"
                ? value.data.remove_mean
                : true,
          },
        }
      : {}),
  };
}

export function readDisplaySettings(state: Record<string, unknown>): DisplaySettings {
  const value = state.displaySettings;
  if (!isRecord(value)) {
    return defaultDisplaySettings();
  }
  return {
    log_freq:
      typeof value.log_freq === "boolean"
        ? value.log_freq
        : true,
    show_grid:
      typeof value.show_grid === "boolean"
        ? value.show_grid
        : true,
    max_hz: typeof value.max_hz === "number" ? value.max_hz : null,
    min_hz: typeof value.min_hz === "number" ? value.min_hz : null,
    range_db:
      typeof value.range_db === "number" ? value.range_db : 80,
    db_min: typeof value.db_min === "number" ? value.db_min : null,
    db_max: typeof value.db_max === "number" ? value.db_max : null,
  };
}

export function analysisSettingsEqual(a: AnalysisSettings, b: AnalysisSettings): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function hopSamples(settings: AnalysisSettings): number {
  return Math.min(
    Math.max(1, settings.hop ?? Math.floor(settings.frame / 4)),
    Math.max(1, settings.frame),
  );
}

/**
 * Expected analysis frames for staged settings. Mirrors the core pipeline:
 * the final frame is end-aligned, so a partial remainder still gets a frame.
 */
export function estimatedFrameCount(
  settings: AnalysisSettings,
  sampleRate: number | null | undefined,
  durationS: number | null | undefined,
): number | null {
  if (
    !sampleRate ||
    !durationS ||
    !(sampleRate > 0) ||
    !(durationS > 0) ||
    !Number.isFinite(sampleRate) ||
    !Number.isFinite(durationS)
  ) {
    return null;
  }
  const samples = Math.round(sampleRate * durationS);
  const frame = Math.max(1, Math.floor(settings.frame));
  const hop = hopSamples(settings);
  if (samples <= 0) {
    return 0;
  }
  if (samples <= frame) {
    return 1;
  }
  return 1 + Math.ceil((samples - frame) / hop);
}

export function frameMilliseconds(
  settings: AnalysisSettings,
  sampleRate: number | null | undefined,
): string {
  if (!sampleRate || sampleRate <= 0) {
    return "—";
  }
  return `${((settings.frame / sampleRate) * 1000).toFixed(1)} ms`;
}

export function frequencyResolution(
  settings: AnalysisSettings,
  sampleRate: number | null | undefined,
): string {
  if (!sampleRate || sampleRate <= 0) {
    return "—";
  }
  return `${(sampleRate / (settings.frame * Math.max(1, settings.pad))).toFixed(2)} Hz`;
}

export interface InspectorMeta {
  sampleRate?: number;
  durationS?: number;
  frames?: number;
  bins?: number;
  table?: TableInfo;
}

export function readInspectorMeta(state: Record<string, unknown>): InspectorMeta {
  const value = state.analysisMeta;
  if (!isRecord(value)) {
    return {};
  }
  const meta: InspectorMeta = {
    sampleRate: typeof value.sampleRate === "number" ? value.sampleRate : undefined,
    durationS: typeof value.durationS === "number" ? value.durationS : undefined,
    frames: typeof value.frames === "number" ? value.frames : undefined,
    bins: typeof value.bins === "number" ? value.bins : undefined,
  };
  if (isRecord(value.table)) {
    const columns = Array.isArray(value.table.columns)
      ? value.table.columns.filter(
          (column): column is string => typeof column === "string",
        )
      : [];
    const notes = Array.isArray(value.table.notes)
      ? value.table.notes.filter(
          (note): note is string => typeof note === "string",
        )
      : [];
    if (
      typeof value.table.rows === "number" &&
      typeof value.table.signal === "string"
    ) {
      meta.table = {
        rows: value.table.rows,
        columns,
        signal: value.table.signal,
        time:
          typeof value.table.time === "string" ? value.table.time : null,
        notes,
      };
    }
  }
  return meta;
}

const SEARCHABLE_LABELS = [
  "Input file",
  "Browse",
  "Path",
  "Frame",
  "Hop",
  "Pad",
  "Resolution",
  "Window",
  "High-pass",
  "Low-pass",
  "Smoothing",
  "Log frequency",
  "Grid lines",
  "Minimum frequency",
  "Maximum frequency",
  "color range",
  "color floor",
  "color ceiling",
  "Nyquist",
  "Hz",
  "dB",
  "Column",
  "Time column",
  "Sample rate",
  "Remove mean",
];

/** Case-insensitive match of a filter query against setting labels. */
export function matchesFilter(label: string, query: string): boolean {
  const q = query.trim().toLowerCase();
  return q.length === 0 || label.toLowerCase().includes(q);
}

export function anyFieldMatches(query: string): boolean {
  return SEARCHABLE_LABELS.some((label) => matchesFilter(label, query));
}

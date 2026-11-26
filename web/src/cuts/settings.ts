import type { LineSpec } from "./model";

export interface RpmSettings {
  source: "constant" | "ramp" | "csv";
  constant: number;
  start: number;
  end: number;
  csv: string | null;
}

/** Matches `fft_runtime::RpmProfileSpec` (tagged, snake_case variants). */
export interface RpmPoint {
  timeS: number;
  rpm: number;
}

export type RpmProfileSpec =
  | { kind: "constant"; value: number }
  | {
      kind: "ramp";
      value: { t0_s: number; t1_s: number; rpm0: number; rpm1: number };
    }
  | { kind: "csv"; value: string }
  | { kind: "points"; value: RpmPoint[] };

/** Matches `fft_core::settings::OrderCutSettings` (snake_case serde keys). */
export interface OrderCutSettings {
  orders: string;
  order_width: number;
  freqs: string;
  freq_width_hz: number;
  axis: "rpm" | "time" | "freq";
  smooth_s: number;
  peak_snap: boolean;
  peak_min_db: number;
  rpm: RpmSettings;
}

/** Matches `fft_core::settings::LineCutSettings`. */
export interface LineCutSettings {
  line_width_hz: number;
  line_axis: "auto" | "time" | "freq";
  snap: boolean;
  snap_deg: number;
  peak_snap: boolean;
  peak_min_db: number;
}

export function defaultOrderSettings(): OrderCutSettings {
  return {
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
  };
}

export function defaultLineSettings(): LineCutSettings {
  return {
    line_width_hz: 0,
    line_axis: "auto",
    snap: true,
    snap_deg: 2,
    peak_snap: true,
    peak_min_db: 1,
  };
}

export function readOrderSettings(state: Record<string, unknown>): OrderCutSettings {
  return isOrderSettings(state.orderCuts)
    ? state.orderCuts
    : defaultOrderSettings();
}

export function readLineSettings(state: Record<string, unknown>): LineCutSettings {
  if (!isLineSettings(state.lineCuts)) {
    return defaultLineSettings();
  }
  // 5° was the old default and produced a very wide snap zone; normalize
  // sessions saved before the tighter default without changing custom values.
  return state.lineCuts.snap_deg === 5
    ? { ...state.lineCuts, snap_deg: 2 }
    : state.lineCuts;
}

export function readLines(state: Record<string, unknown>): LineSpec[] {
  return Array.isArray(state.lines)
    ? (state.lines.filter(isLineSpec) as LineSpec[])
    : [];
}

/** Cut plots show grid lines unless the display state explicitly hides them. */
export function readShowGrid(state: Record<string, unknown>): boolean {
  return state.showGrid !== false;
}

export function readRpmProfile(
  state: Record<string, unknown>,
): RpmProfileSpec | undefined {
  return isRpmProfileSpec(state.rpm) ? state.rpm : undefined;
}

export function isRpmProfileSpec(value: unknown): value is RpmProfileSpec {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const v = value as Record<string, unknown>;
  switch (v.kind) {
    case "constant":
      return typeof v.value === "number";
    case "ramp":
      return typeof v.value === "object" && v.value !== null;
    case "csv":
      return typeof v.value === "string";
    case "points":
      return (
        Array.isArray(v.value) &&
        v.value.every(
          (point) =>
            typeof point === "object" &&
            point !== null &&
            typeof (point as RpmPoint).timeS === "number" &&
            typeof (point as RpmPoint).rpm === "number",
        )
      );
    default:
      return false;
  }
}

export function isOrderSettings(value: unknown): value is OrderCutSettings {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const v = value as Record<string, unknown>;
  return typeof v.orders === "string" && typeof v.order_width === "number";
}

export function isLineSettings(value: unknown): value is LineCutSettings {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const v = value as Record<string, unknown>;
  return typeof v.line_width_hz === "number" && typeof v.line_axis === "string";
}

export function isLineSpec(value: unknown): value is LineSpec {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const v = value as Record<string, unknown>;
  return (
    typeof v.t0 === "number" &&
    typeof v.f0 === "number" &&
    typeof v.t1 === "number" &&
    typeof v.f1 === "number" &&
    typeof v.logF === "boolean"
  );
}

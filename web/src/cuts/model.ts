export type OrderAxis = "rpm" | "time" | "freq";
export type LineAxis = "auto" | "time" | "freq";

export interface CutPoint {
  timeS: number;
  rpm: number;
  freqHz: number;
  db: number;
}

export type CutKind =
  | { type: "order"; value: number }
  | { type: "freq"; value: number }
  | { type: "line"; value: number };

export interface Cut {
  kind: CutKind;
  points: CutPoint[];
}

export interface CutsPayload {
  cuts: Cut[];
  durationS: number;
  nyquist: number;
  binHz: number;
  hopS: number;
}

export interface LineSnapHistory {
  /** Previous exact horizontal line for double-click toggling. */
  horizontal?: LineSpec;
  /** Previous exact vertical line for double-click toggling. */
  vertical?: LineSpec;
}

export interface LineSpec {
  t0: number;
  f0: number;
  t1: number;
  f1: number;
  logF: boolean;
  /**
   * View-only orientation history used by spectrogram double-click toggles.
   * Stripped before line-cut compute requests.
   */
  snapHistory?: LineSnapHistory;
}

/** colorblind-safe fallback; the active TOML theme overrides it. */
export const FALLBACK_PALETTE = [
  "#56b4e9",
  "#e69f00",
  "#009e73",
  "#cc79a7",
  "#f0e442",
  "#0072b2",
  "#d55e00",
  "#e0e0e0",
];

export function seriesColor(palette: readonly string[], index: number): string {
  if (palette.length === 0) {
    return FALLBACK_PALETTE[index % FALLBACK_PALETTE.length];
  }
  return palette[index % palette.length];
}

export function cutLabel(kind: CutKind): string {
  switch (kind.type) {
    case "order":
      return `${kind.value}x`;
    case "freq":
      return `${kind.value} Hz`;
    case "line":
      return `line ${kind.value}`;
  }
}

export function cutX(point: CutPoint, axis: OrderAxis): number {
  switch (axis) {
    case "rpm":
      return point.rpm;
    case "time":
      return point.timeS;
    case "freq":
      return point.freqHz;
  }
}

export function axisFor(orderAxis: OrderAxis, rpmConstant: boolean): {
  axis: OrderAxis;
  note: string | null;
} {
  if (rpmConstant && orderAxis !== "time") {
    return { axis: "time", note: "RPM is constant, so level is shown against time" };
  }
  return { axis: orderAxis, note: null };
}

export function isDegenerate(
  cut: Cut,
  axis: OrderAxis,
  hopS: number,
  binHz: number,
): boolean {
  if (cut.points.length < 2) {
    return true;
  }
  let lo = Number.POSITIVE_INFINITY;
  let hi = Number.NEGATIVE_INFINITY;
  for (const point of cut.points) {
    const x = cutX(point, axis);
    if (x < lo) lo = x;
    if (x > hi) hi = x;
  }
  const tolerance =
    axis === "rpm" ? 0.01 : axis === "time" ? 0.5 * hopS : 0.5 * binHz;
  return hi - lo < tolerance;
}

export function resolveLineAxis(
  cuts: readonly Cut[],
  requested: LineAxis,
  hopS: number,
  binHz: number,
): OrderAxis {
  if (requested === "time") return "time";
  if (requested === "freq") return "freq";
  const visible = (axis: OrderAxis) =>
    cuts.filter((cut) => !isDegenerate(cut, axis, hopS, binHz)).length;
  return visible("freq") > visible("time") ? "freq" : "time";
}

export function flipLine(line: LineSpec): LineSpec {
  return {
    ...stripLineSnapHistory(line),
    t0: line.t1,
    t1: line.t0,
    f0: line.f1,
    f1: line.f0,
  };
}

/** Remove view-only orientation history before a manual edit or compute. */
export function stripLineSnapHistory(line: LineSpec): LineSpec {
  const plain = { ...line };
  delete plain.snapHistory;
  return plain;
}

export function isHorizontalLine(line: LineSpec): boolean {
  return Math.abs(line.f1 - line.f0) <= 1e-9;
}

export function isVerticalLine(line: LineSpec): boolean {
  return Math.abs(line.t1 - line.t0) <= 1e-9;
}

export type LineAnchor = "start" | "end";

/**
 * Double-click orientation toggle in the spectrogram time/frequency plane.
 *
 * Diagonal lines become horizontal using the clicked endpoint as the anchor.
 * Horizontal lines become vertical through their time midpoint and the full
 * frequency range. The previous horizontal/vertical versions are retained so
 * toggling back restores the exact line.
 */
export function toggleLineOrientation(
  line: LineSpec,
  anchor: LineAnchor,
  options: { minHz: number; maxHz: number; durationS: number },
): LineSpec {
  const plain = stripLineSnapHistory(line);
  const history: LineSnapHistory = { ...line.snapHistory };
  const { minHz, maxHz, durationS } = options;
  const midTime = (plain.t0 + plain.t1) / 2;
  const midHz = (plain.f0 + plain.f1) / 2;

  if (isHorizontalLine(plain)) {
    const vertical = history.vertical
      ? stripLineSnapHistory(history.vertical)
      : {
          ...plain,
          t0: midTime,
          t1: midTime,
          f0: minHz,
          f1: maxHz,
        };
    return {
      ...vertical,
      snapHistory: {
        horizontal: history.horizontal ?? stripLineSnapHistory(plain),
        vertical: stripLineSnapHistory(vertical),
      },
    };
  }

  if (isVerticalLine(plain)) {
    const horizontal = history.horizontal
      ? stripLineSnapHistory(history.horizontal)
      : {
          ...plain,
          t0: 0,
          t1: durationS,
          f0: midHz,
          f1: midHz,
        };
    return {
      ...horizontal,
      snapHistory: {
        horizontal: stripLineSnapHistory(horizontal),
        vertical: history.vertical ?? stripLineSnapHistory(plain),
      },
    };
  }

  const horizontal =
    anchor === "start"
      ? { ...plain, f1: plain.f0 }
      : { ...plain, f0: plain.f1 };
  return {
    ...horizontal,
    snapHistory: {
      ...history,
      horizontal: stripLineSnapHistory(horizontal),
    },
  };
}

export function moveLineEndpoint(
  line: LineSpec,
  endpoint: "start" | "end",
  time: number,
  frequency: number,
): LineSpec {
  const plain = stripLineSnapHistory(line);
  if (endpoint === "start") {
    return { ...plain, t0: time, f0: frequency };
  }
  return { ...plain, t1: time, f1: frequency };
}

/**
 * Snap a dragged endpoint to the axis through the fixed endpoint when the
 * screen-space angle is within `snapDeg` of horizontal or vertical.
 */
export function snapLineEndpoint(params: {
  fixedX: number;
  fixedY: number;
  x: number;
  y: number;
  time: number;
  hz: number;
  fixedTime: number;
  fixedHz: number;
  snapDeg: number;
}): { time: number; hz: number } {
  const { fixedX, fixedY, x, y, fixedTime, fixedHz, snapDeg } = params;
  if (!(snapDeg > 0) || !Number.isFinite(snapDeg)) {
    return { time: params.time, hz: params.hz };
  }
  const dx = Math.abs(x - fixedX);
  const dy = Math.abs(y - fixedY);
  if (dx === 0 && dy === 0) {
    return { time: params.time, hz: params.hz };
  }
  const threshold = Math.tan((snapDeg * Math.PI) / 180);
  const horizontal = dy <= dx * threshold;
  const vertical = dx <= dy * threshold;
  if (horizontal && (!vertical || dx >= dy)) {
    return { time: params.time, hz: fixedHz };
  }
  if (vertical) {
    return { time: fixedTime, hz: params.hz };
  }
  return { time: params.time, hz: params.hz };
}

export function makeLine(t0: number, f0: number, t1: number, f1: number): LineSpec {
  return { t0, f0, t1, f1, logF: false };
}

/**
 * Constant-frequency line through a clicked point, spanning the full file.
 * The click time is accepted for call-site context; frequency is clamped to
 * [0, Nyquist]. A click on a log axis maps to the axis floor, so lines can sit
 * all the way down at DC in a linear view.
 */
export function lineAtPoint(
  _time: number,
  hz: number,
  nyquist: number,
  durationS: number,
): LineSpec {
  const top = Number.isFinite(nyquist) && nyquist > 0 ? nyquist : 1;
  const frequency = Number.isFinite(hz)
    ? Math.min(top, Math.max(0, hz))
    : 0;
  const duration =
    Number.isFinite(durationS) && durationS > 0 ? durationS : 0;
  return makeLine(0, frequency, duration, frequency);
}

/** Append a positive order, dropping invalid/duplicate values. */
export function addOrderValue(text: string, value: number): string | null {
  if (!Number.isFinite(value) || value <= 0) {
    return null;
  }
  const values = text
    .split(/[,;\s]+/)
    .map((part) => Number(part))
    .filter((item) => Number.isFinite(item) && item > 0);
  if (values.some((item) => Math.abs(item - value) < 1e-9)) {
    return null;
  }
  return [...values, value].sort((a, b) => a - b).join(",");
}

export function deleteAt<T>(items: readonly T[], index: number): T[] {
  return items.filter((_, i) => i !== index);
}

export function defaultLineFor(payload: CutsPayload): LineSpec {
  const f0 = payload.nyquist * 0.15;
  const f1 = payload.nyquist * 0.3;
  return makeLine(0, f0, payload.durationS, f1);
}

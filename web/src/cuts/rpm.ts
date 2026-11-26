import type { OrderCutSettings, RpmProfileSpec } from "./settings";

export interface OrderGuidePoint {
  timeS: number;
  freqHz: number;
}

function formatNumber(value: number): string {
  if (!Number.isFinite(value)) {
    return "—";
  }
  return Number.isInteger(value) ? String(value) : value.toFixed(1);
}

function formatHz(value: number): string {
  return Number.isFinite(value) ? value.toFixed(1) : "—";
}

/** RPM at `time`, with ramps spanning `durationS` (default one second). */
export function rpmAt(
  time: number,
  settings: OrderCutSettings,
  durationS = 1,
): number {
  const rpm = settings.rpm;
  if (rpm.source === "constant") {
    return rpm.constant;
  }
  if (rpm.source === "csv") {
    return Number.NaN;
  }
  const duration = Number.isFinite(durationS) && durationS > 0 ? durationS : 1;
  const t = Math.min(duration, Math.max(0, Number.isFinite(time) ? time : 0));
  return rpm.start + (rpm.end - rpm.start) * (t / duration);
}

export function orderHz(order: number, rpm: number): number {
  return (order * rpm) / 60;
}

export function rpmLabel(settings: OrderCutSettings): string {
  const rpm = settings.rpm;
  if (rpm.source === "csv") {
    return `CSV ${rpm.csv ?? "(no file)"}`;
  }
  if (rpm.source === "ramp") {
    return `${formatNumber(rpm.start)}→${formatNumber(rpm.end)} RPM`;
  }
  return `${formatNumber(rpm.constant)} RPM`;
}

/** RPM profile plus the order's frequency span (order 1 by default). */
export function orderTrackingLabel(
  order: number,
  settings: OrderCutSettings,
): string {
  if (settings.rpm.source === "csv") {
    return rpmLabel(settings);
  }
  const start = rpmAt(0, settings);
  const end = rpmAt(1, settings, 1);
  return `${rpmLabel(settings)} · order ${formatNumber(order)} = ${formatHz(
    orderHz(order, start),
  )}–${formatHz(orderHz(order, end))} Hz`;
}

/** Short label for a script-provided RPM profile (variable fundamentals). */
export function rpmProfileLabel(profile: RpmProfileSpec): string {
  switch (profile.kind) {
    case "constant":
      return `${formatNumber(profile.value)} RPM`;
    case "ramp":
      return `${formatNumber(profile.value.rpm0)}→${formatNumber(profile.value.rpm1)} RPM`;
    case "csv":
      return `CSV ${profile.value}`;
    case "points":
      return `${profile.value.length}-point RPM profile`;
  }
}

export function parseOrderList(text: string): number[] {
  return text
    .split(/[,;\s]+/)
    .map((part) => Number(part))
    .filter((value) => Number.isFinite(value) && value > 0);
}

/**
 * Points for an order guide. CSV RPM profiles are host-local and cannot be
 * read by the browser, so they intentionally produce no guide.
 */
export function orderPolyline(
  order: number,
  settings: OrderCutSettings,
  durationS: number,
  nyquist: number,
  samples = 32,
): OrderGuidePoint[] {
  if (
    !(order > 0) ||
    !(durationS > 0) ||
    !Number.isFinite(durationS) ||
    !(nyquist > 0) ||
    !Number.isFinite(nyquist) ||
    settings.rpm.source === "csv"
  ) {
    return [];
  }
  if (settings.rpm.source === "constant") {
    const hz = orderHz(order, settings.rpm.constant);
    if (!(hz > 0) || hz > nyquist) {
      return [];
    }
    return [
      { timeS: 0, freqHz: hz },
      { timeS: durationS, freqHz: hz },
    ];
  }

  const steps = Math.max(2, Math.round(samples));
  const points: OrderGuidePoint[] = [];
  for (let index = 0; index <= steps; index += 1) {
    const timeS = (durationS * index) / steps;
    const hz = orderHz(order, rpmAt(timeS, settings, durationS));
    if (hz > 0 && hz <= nyquist) {
      points.push({ timeS, freqHz: hz });
    }
  }
  return points;
}

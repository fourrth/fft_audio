/** Pure axis-tick and compact-label helpers for the spectrogram plot. */

import { logFloor, type Viewport } from "./geometry";

const LOG_TICKS = [
  10, 20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000, 20_000, 50_000,
  100_000, 200_000, 500_000,
];

function niceStep(span: number, target: number): number {
  const rough = span / Math.max(1, target);
  const power = 10 ** Math.floor(Math.log10(rough));
  const fraction = rough / power;
  const nice = fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 5 ? 5 : 10;
  return nice * power;
}

function niceTicks(min: number, max: number, target: number): number[] {
  if (!Number.isFinite(min) || !Number.isFinite(max) || !(max > min)) {
    return [];
  }
  const step = niceStep(max - min, target);
  if (!(step > 0) || !Number.isFinite(step)) {
    return [];
  }
  const start = Math.ceil(min / step - 1e-9) * step;
  const ticks: number[] = [];
  for (let index = 0; index < 1_000; index += 1) {
    const value = start + index * step;
    if (value > max + step * 1e-9) {
      break;
    }
    ticks.push(Number(value.toPrecision(12)));
  }
  return ticks;
}

/** Nice 1/2/5 × 10^n time ticks inside `[t0, t1]`. */
export function niceTimeTicks(
  t0: number,
  t1: number,
  target = 6,
): number[] {
  return niceTicks(t0, t1, target);
}

/** Nice linear frequency ticks, or a log series with a geometric fallback. */
export function frequencyTicks(view: Viewport, target = 6): number[] {
  if (!(view.f1 > view.f0)) {
    return [];
  }
  if (!view.logFreq) {
    const low = Math.max(0, view.f0);
    if (!(view.f1 > low)) {
      return [];
    }
    return niceTicks(low, view.f1, target);
  }

  const filtered = LOG_TICKS.filter(
    (hz) => hz >= view.f0 - 1e-9 && hz <= view.f1 + 1e-9,
  );
  if (filtered.length >= 2) {
    return filtered;
  }

  const low = logFloor(view.f0, view.f1);
  if (!(view.f1 > low)) {
    return [];
  }
  const count = Math.max(2, Math.min(8, Math.round(target)));
  const ratio = (view.f1 / low) ** (1 / (count - 1));
  const ticks: number[] = [];
  for (let index = 0; index < count; index += 1) {
    const value = Number((low * ratio ** index).toPrecision(6));
    if (!ticks.includes(value)) {
      ticks.push(value);
    }
  }
  return ticks;
}

function trimFixed(value: number, digits: number): string {
  return value
    .toFixed(digits)
    .replace(/(\.\d*?)0+$/, "$1")
    .replace(/\.$/, "");
}

/** Compact frequency label: `10`, `500`, `1.2k`, `22.1k`. */
export function formatHz(hz: number): string {
  if (!Number.isFinite(hz)) {
    return "—";
  }
  const sign = hz < 0 ? "-" : "";
  const value = Math.abs(hz);
  if (value >= 1_000) {
    const scaled = value / 1_000;
    return `${sign}${trimFixed(scaled, scaled >= 100 ? 0 : 1)}k`;
  }
  if (value >= 10) {
    return `${sign}${Math.round(value)}`;
  }
  if (value >= 1) {
    return `${sign}${trimFixed(value, 1)}`;
  }
  if (value === 0) {
    return "0";
  }
  return `${sign}${trimFixed(value, 2)}`;
}

/** Compact seconds label; the axis gutter supplies the `s` unit. */
export function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds)) {
    return "—";
  }
  const sign = seconds < 0 ? "-" : "";
  const value = Math.abs(seconds);
  if (value >= 10) {
    return `${sign}${Math.round(value)}`;
  }
  if (value >= 1) {
    return `${sign}${trimFixed(value, 1)}`;
  }
  if (value === 0) {
    return "0";
  }
  if (value >= 0.01) {
    return `${sign}${trimFixed(value, 2)}`;
  }
  return `${sign}${trimFixed(value, 3)}`;
}

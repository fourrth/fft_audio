import { describe, expect, it } from "vitest";
import { colorStop, heatmapPixels } from "./heatmap";
import { inferno } from "./colormap";
import type { Viewport } from "./geometry";
import type { DecodedGrid } from "./types";

const BINS = 101;
const BIN_HZ = 10;
const NYQUIST = 1000;
const WIDTH = 16;
const HEIGHT = 200;

function gridWithPeaks(peaks: ReadonlyArray<readonly [number, number]>): DecodedGrid {
  const frames = Math.max(1, ...peaks.map(([frame]) => frame + 1));
  const values = new Int16Array(frames * BINS).fill(-10_000);
  for (const [frame, bin] of peaks) {
    values[frame * BINS + bin] = 0;
  }
  return {
    meta: {
      frames,
      bins: BINS,
      binHz: BIN_HZ,
      frameS: 1,
      durationS: 1,
      nyquist: NYQUIST,
      maxDb: 0,
      minDb: -100,
      sampleRate: 2000,
    },
    values,
  };
}

function expectedLogRow(hz: number, f0: number, f1: number, height: number): number {
  const lo = Math.max(f0, 10);
  const fraction = Math.log(hz / lo) / Math.log(f1 / lo);
  return (1 - fraction) * height - 0.5;
}

function expectedLinearRow(hz: number, f0: number, f1: number, height: number): number {
  const fraction = (hz - f0) / (f1 - f0);
  return (1 - fraction) * height - 0.5;
}

function rowBrightness(
  pixels: Uint8ClampedArray,
  width: number,
  y: number,
  x0 = 0,
  x1 = width,
): number {
  let total = 0;
  for (let x = x0; x < x1; x += 1) {
    const index = (y * width + x) * 4;
    total += pixels[index] + pixels[index + 1] + pixels[index + 2];
  }
  return total / Math.max(1, x1 - x0);
}

function brightestRow(
  pixels: Uint8ClampedArray,
  width: number,
  height: number,
  x0 = 0,
  x1 = width,
): number {
  let bestRow = 0;
  let best = -Infinity;
  for (let y = 0; y < height; y += 1) {
    const value = rowBrightness(pixels, width, y, x0, x1);
    if (value > best) {
      best = value;
      bestRow = y;
    }
  }
  return bestRow;
}

function brightestRowAtColumn(
  pixels: Uint8ClampedArray,
  width: number,
  height: number,
  x: number,
): number {
  let bestRow = 0;
  let best = -Infinity;
  for (let y = 0; y < height; y += 1) {
    const index = (y * width + x) * 4;
    const value = pixels[index] + pixels[index + 1] + pixels[index + 2];
    if (value > best) {
      best = value;
      bestRow = y;
    }
  }
  return bestRow;
}

describe("heatmapPixels", () => {
  it("maps the same peak to different log and linear rows", () => {
    const grid = gridWithPeaks([[0, 10]]); // 100 Hz
    const logView: Viewport = { t0: 0, t1: 1, f0: 10, f1: 1000, logFreq: true };
    const linearView: Viewport = { t0: 0, t1: 1, f0: 0, f1: 1000, logFreq: false };

    const logPixels = heatmapPixels(grid, logView, WIDTH, HEIGHT, -100, 0);
    const linearPixels = heatmapPixels(grid, linearView, WIDTH, HEIGHT, -100, 0);
    expect(logPixels.length).toBe(WIDTH * HEIGHT * 4);
    expect(linearPixels.length).toBe(WIDTH * HEIGHT * 4);
    for (let index = 3; index < logPixels.length; index += 4) {
      expect(logPixels[index]).toBe(255);
    }

    const logRow = brightestRow(logPixels, WIDTH, HEIGHT);
    const linearRow = brightestRow(linearPixels, WIDTH, HEIGHT);
    const expectedLog = expectedLogRow(100, 10, 1000, HEIGHT);
    const expectedLinear = expectedLinearRow(100, 0, 1000, HEIGHT);
    expect(Math.abs(logRow - expectedLog)).toBeLessThanOrEqual(4);
    expect(Math.abs(linearRow - expectedLinear)).toBeLessThanOrEqual(4);
    expect(Math.abs(logRow - linearRow)).toBeGreaterThan(50);

    // Rows away from the 100 Hz ridge stay at the dark background.
    expect(rowBrightness(logPixels, WIDTH, 20)).toBeLessThan(20);
    expect(rowBrightness(linearPixels, WIDTH, 20)).toBeLessThan(20);
  });

  it("maps viewport time columns to grid frames", () => {
    const grid = gridWithPeaks([
      [0, 10], // frame 0: 100 Hz
      [1, 20], // frame 1: 200 Hz
    ]);
    const view: Viewport = { t0: 0, t1: 1, f0: 0, f1: 1000, logFreq: false };
    const width = 20;
    const pixels = heatmapPixels(grid, view, width, HEIGHT, -100, 0);

    const leftRow = brightestRowAtColumn(pixels, width, HEIGHT, 2);
    const rightRow = brightestRowAtColumn(pixels, width, HEIGHT, 17);
    expect(Math.abs(leftRow - expectedLinearRow(100, 0, 1000, HEIGHT))).toBeLessThanOrEqual(4);
    expect(Math.abs(rightRow - expectedLinearRow(200, 0, 1000, HEIGHT))).toBeLessThanOrEqual(4);
    expect(leftRow - rightRow).toBeGreaterThan(10);
  });

  it("leaves frequency rows beyond the grid Nyquist as background", () => {
    const grid = gridWithPeaks([[0, 10]]);
    const view: Viewport = { t0: 0, t1: 1, f0: 0, f1: 2000, logFreq: false };
    const pixels = heatmapPixels(grid, view, WIDTH, HEIGHT, -100, 0);

    expect(rowBrightness(pixels, WIDTH, 10)).toBeLessThan(20);
    const brightRow = brightestRow(pixels, WIDTH, HEIGHT);
    expect(Math.abs(brightRow - expectedLinearRow(100, 0, 2000, HEIGHT))).toBeLessThanOrEqual(4);
  });

  it("restricts colors to the selected slice of the palette", () => {
    expect(colorStop(0, -60, -40, -100, 0)).toBeCloseTo(0.6, 6);
    expect(colorStop(-100, -60, -40, -100, 0)).toBeCloseTo(0.4, 6);
    expect(colorStop(-50, -60, -40, -100, 0)).toBeCloseTo(0.5, 6);

    const grid = gridWithPeaks([[0, 10]]); // 0 dB peak at 100 Hz
    const view: Viewport = { t0: 0, t1: 1, f0: 0, f1: 1000, logFreq: false };
    const pixels = heatmapPixels(
      grid,
      view,
      WIDTH,
      HEIGHT,
      -60,
      -40,
      -100,
      0,
    );
    const row = brightestRow(pixels, WIDTH, HEIGHT);
    const index = row * WIDTH * 4;
    const [r, g, b] = inferno(0.6);
    expect([pixels[index], pixels[index + 1], pixels[index + 2]]).toEqual([
      r,
      g,
      b,
    ]);
    // The rest of the data is below the selected floor and must stay black.
    expect(rowBrightness(pixels, WIDTH, 20)).toBeLessThan(20);
  });
});

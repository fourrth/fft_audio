import { describe, expect, it } from "vitest";
import {
  LOG_MIN_HZ,
  VIEW_MARGIN,
  clampView,
  dbFromFraction,
  fitView,
  fractionFromDb,
  freqAtFraction,
  freqToY,
  logFloor,
  minimapRect,
  nearestOrder,
  panView,
  timeToX,
  viewFromMinimap,
  viewFromRegion,
  xToTime,
  yToFreq,
  zoomView,
} from "./geometry";

describe("spectrogram geometry", () => {
  it("fits and clamps a view", () => {
    const fitted = fitView(10, 1000, true);
    expect(fitted).toEqual({
      t0: 0,
      t1: 10,
      f0: LOG_MIN_HZ,
      f1: 1000,
      logFreq: true,
    });

    const clamped = clampView(
      { t0: -5, t1: 20, f0: 5, f1: 5000, logFreq: false },
      10,
      1000,
    );
    expect(clamped.t0).toBeCloseTo(-10 * VIEW_MARGIN, 6);
    expect(clamped.t1).toBeCloseTo(10 * (1 + VIEW_MARGIN), 6);
    expect(clamped.f0).toBeCloseTo(-1000 * VIEW_MARGIN, 6);
    expect(clamped.f1).toBeCloseTo(1000 * (1 + VIEW_MARGIN), 6);
  });

  it("fits to an explicit frequency floor", () => {
    expect(fitView(1, 60, false, 10).f0).toBe(10);
    expect(fitView(1, 60, false, 0).f0).toBe(0);
    expect(fitView(1, 60, true, 0).f0).toBe(LOG_MIN_HZ);
    expect(fitView(1, 60, true, 20).f0).toBe(20);
  });

  it("round-trips time and frequency in linear and log modes", () => {
    for (const logFreq of [false, true]) {
      const view = fitView(8, 2000, logFreq);
      for (const t of [0, 1.5, 4, 8]) {
        const x = timeToX(t, view, 800);
        expect(xToTime(x, view, 800)).toBeCloseTo(t, 6);
      }
      for (const hz of logFreq ? [10, 100, 1000, 2000] : [0, 250, 1000, 2000]) {
        const y = freqToY(hz, view, 400);
        expect(yToFreq(y, view, 400)).toBeCloseTo(hz, 6);
      }
    }
  });

  it("puts high frequencies at the top on a log axis", () => {
    const view = fitView(1, 20_000, true);
    expect(freqToY(20_000, view, 400)).toBeCloseTo(0, 6);
    expect(freqToY(10, view, 400)).toBeCloseTo(400, 6);
    expect(freqToY(20_000, view, 400)).toBeLessThan(freqToY(1000, view, 400));
  });

  it("pans and zooms within time and frequency bounds", () => {
    for (const logFreq of [false, true]) {
      const full = fitView(10, 1000, logFreq);
      const anchorF = logFreq ? 300 : 500;
      const zoomed = zoomView(full, 0.5, 5, anchorF, 10, 1000, logFreq);
      expect(zoomed.logFreq).toBe(logFreq);
      expect(zoomed.t0).toBeCloseTo(2.5, 6);
      expect(zoomed.t1).toBeCloseTo(7.5, 6);
      expect(zoomed.f0).toBeGreaterThanOrEqual(logFreq ? LOG_MIN_HZ : 0);
      expect(zoomed.f1).toBeLessThanOrEqual(1000);
      expect(zoomed.f1 - zoomed.f0).toBeLessThan(full.f1 - full.f0);
      expect(zoomed.f0).toBeLessThan(anchorF);
      expect(zoomed.f1).toBeGreaterThan(anchorF);

      const panned = panView(zoomed, 2, 0, 10, 1000);
      expect(panned.t0).toBeCloseTo(4.5, 6);
      expect(panned.t1).toBeCloseTo(9.5, 6);

      const clamped = panView(zoomed, 100, 0, 10, 1000);
      expect(clamped.t0).toBeCloseTo(5.5, 6);
      expect(clamped.t1).toBeCloseTo(10.5, 6);

      const low = panView(zoomed, 0, -10_000, 10, 1000);
      expect(low.f0).toBeCloseTo(
        logFreq ? LOG_MIN_HZ * (1 - VIEW_MARGIN) : -1000 * VIEW_MARGIN,
        6,
      );
      expect(low.f1).toBeLessThanOrEqual(1000 * (1 + VIEW_MARGIN));

      const out = zoomView(full, 4, 5, anchorF, 10, 1000, logFreq);
      expect(out.t0).toBeCloseTo(-0.5, 6);
      expect(out.t1).toBeCloseTo(10.5, 6);
      expect(out.f0).toBeCloseTo(
        logFreq ? LOG_MIN_HZ * (1 - VIEW_MARGIN) : -50,
        6,
      );
      expect(out.f1).toBeCloseTo(1050, 6);
    }
  });

  it("caps repeated zoom-out at the bounded overscan margin", () => {
    const full = fitView(10, 1000, false);
    const once = zoomView(full, 1.25, 5, 500, 10, 1000, false);
    expect(once.t0).toBeCloseTo(-0.5, 6);
    expect(once.t1).toBeCloseTo(10.5, 6);
    expect(once.f0).toBeCloseTo(-50, 6);
    expect(once.f1).toBeCloseTo(1050, 6);

    const again = zoomView(once, 4, 5, 500, 10, 1000, false);
    expect(again.t0).toBeCloseTo(-0.5, 6);
    expect(again.t1).toBeCloseTo(10.5, 6);
    expect(again.f0).toBeCloseTo(-50, 6);
    expect(again.f1).toBeCloseTo(1050, 6);

    const panned = panView(full, -100, 0, 10, 1000);
    expect(panned.t0).toBeCloseTo(-0.5, 6);
    expect(panned.t1).toBeCloseTo(9.5, 6);

    expect(fitView(10, 1000, true)).toEqual({
      t0: 0,
      t1: 10,
      f0: LOG_MIN_HZ,
      f1: 1000,
      logFreq: true,
    });
  });

  it("converts a dragged pixel region into a clamped viewport", () => {
    const linear = fitView(10, 1000, false);
    const selected = viewFromRegion(
      linear,
      { x: 60, y: 60 },
      { x: 20, y: 20 },
      100,
      100,
      10,
      1000,
    );
    expect(selected).not.toBeNull();
    if (selected) {
      expect(selected.t0).toBeCloseTo(2, 6);
      expect(selected.t1).toBeCloseTo(6, 6);
      expect(selected.f0).toBeCloseTo(400, 6);
      expect(selected.f1).toBeCloseTo(800, 6);
      expect(selected.logFreq).toBe(false);
    }

    const logarithmic = fitView(10, 1000, true);
    const logSelected = viewFromRegion(
      logarithmic,
      { x: 20, y: 20 },
      { x: 60, y: 60 },
      100,
      100,
      10,
      1000,
    );
    expect(logSelected).not.toBeNull();
    if (logSelected) {
      expect(logSelected.t0).toBeCloseTo(2, 6);
      expect(logSelected.t1).toBeCloseTo(6, 6);
      expect(logSelected.f0).toBeCloseTo(10 * 100 ** 0.4, 4);
      expect(logSelected.f1).toBeCloseTo(10 * 100 ** 0.8, 4);
      expect(logSelected.logFreq).toBe(true);
    }

    const clamped = viewFromRegion(
      linear,
      { x: -20, y: -20 },
      { x: 120, y: 120 },
      100,
      100,
      10,
      1000,
    );
    expect(clamped).not.toBeNull();
    if (clamped) {
      expect(clamped.t0).toBeCloseTo(0, 6);
      expect(clamped.t1).toBeCloseTo(10, 6);
      expect(clamped.f0).toBeCloseTo(0, 6);
      expect(clamped.f1).toBeCloseTo(1000, 6);
    }

    expect(
      viewFromRegion(
        linear,
        { x: 20, y: 20 },
        { x: 22, y: 22 },
        100,
        100,
        10,
        1000,
      ),
    ).toBeNull();
  });

  it("keeps the mini-map rectangle inside the map", () => {
    const view = clampView(
      { t0: 2, t1: 4, f0: 100, f1: 500, logFreq: true },
      10,
      1000,
    );
    const rect = minimapRect(view, 10, 1000, 200, 36);
    expect(rect.x).toBeGreaterThanOrEqual(0);
    expect(rect.y).toBeGreaterThanOrEqual(0);
    expect(rect.x + rect.w).toBeLessThanOrEqual(200);
    expect(rect.y + rect.h).toBeLessThanOrEqual(36);
    expect(rect.w).toBeGreaterThan(0);
    expect(rect.h).toBeGreaterThan(0);
  });

  it("centres a new view from a mini-map click and clamps it", () => {
    const view = fitView(10, 1000, false);
    const zoomed = { ...view, t0: 2, t1: 4, f0: 100, f1: 300 };
    const moved = viewFromMinimap(0.9, 0.5, zoomed, 10, 1000);
    expect(moved.t1).toBeLessThanOrEqual(10);
    expect(moved.t0).toBeGreaterThanOrEqual(0);
    expect(moved.t1 - moved.t0).toBeCloseTo(2, 6);
    expect(moved.f0).toBeGreaterThanOrEqual(0);
    expect(moved.f1).toBeLessThanOrEqual(1000);
  });

  it("hard-caps zoom and pan at the configured frequency bounds", () => {
    const bounds = { minHz: 100, maxHz: 800 };
    const full = fitView(10, 800, false, 100);
    expect(full.f0).toBe(100);
    expect(full.f1).toBe(800);

    const out = zoomView(full, 4, 5, 450, 10, 800, false, bounds);
    expect(out.f0).toBeCloseTo(100, 6);
    expect(out.f1).toBeCloseTo(800, 6);
    // Time keeps the existing 5% overscan.
    expect(out.t0).toBeCloseTo(-0.5, 6);
    expect(out.t1).toBeCloseTo(10.5, 6);

    const zoomed = { ...full, t0: 2, t1: 8, f0: 200, f1: 600 };
    const pannedUp = panView(zoomed, 0, 500, 10, 800, bounds);
    expect(pannedUp.f0).toBeCloseTo(400, 6);
    expect(pannedUp.f1).toBeCloseTo(800, 6);

    const pannedDown = panView(zoomed, 0, -1_000, 10, 800, bounds);
    expect(pannedDown.f0).toBeCloseTo(100, 6);
    expect(pannedDown.f1).toBeCloseTo(500, 6);

    const oversized = clampView(
      { t0: 0, t1: 10, f0: -100, f1: 1_200, logFreq: false },
      10,
      800,
      bounds,
    );
    expect(oversized.f0).toBeCloseTo(100, 6);
    expect(oversized.f1).toBeCloseTo(800, 6);
  });

  it("clamps dragged regions to the hard frequency bounds", () => {
    const bounds = { minHz: 100, maxHz: 800 };
    const view = { t0: 0, t1: 10, f0: 0, f1: 1_000, logFreq: false };

    const top = viewFromRegion(
      view,
      { x: 0, y: 0 },
      { x: 50, y: 20 },
      100,
      100,
      10,
      1_000,
      4,
      bounds,
    );
    expect(top).not.toBeNull();
    if (top) {
      expect(top.t0).toBeCloseTo(0, 6);
      expect(top.t1).toBeCloseTo(5, 6);
      expect(top.f0).toBeCloseTo(600, 6);
      expect(top.f1).toBeCloseTo(800, 6);
    }

    const bottom = viewFromRegion(
      view,
      { x: 0, y: 80 },
      { x: 50, y: 100 },
      100,
      100,
      10,
      1_000,
      4,
      bounds,
    );
    expect(bottom).not.toBeNull();
    if (bottom) {
      expect(bottom.f0).toBeCloseTo(100, 6);
      expect(bottom.f1).toBeCloseTo(300, 6);
    }
  });

  it("keeps a usable log axis when the top is below LOG_MIN_HZ", () => {
    const view = { t0: 0, t1: 1, f0: 2.5, f1: 5, logFreq: true };
    expect(logFloor(10, 5)).toBeCloseTo(2.5, 6);
    expect(freqAtFraction(0, view)).toBeCloseTo(2.5, 6);
    expect(freqAtFraction(1, view)).toBeCloseTo(5, 6);
    expect(freqAtFraction(0.5, view)).toBeCloseTo(Math.sqrt(12.5), 6);

    const y = freqToY(3.5, view, 100);
    expect(Number.isFinite(y)).toBe(true);
    expect(yToFreq(y, view, 100)).toBeCloseTo(3.5, 6);
    expect(freqToY(5, view, 100)).toBeCloseTo(0, 6);
    expect(freqToY(2.5, view, 100)).toBeCloseTo(100, 6);

    const fitted = fitView(1, 5, true, 0);
    expect(fitted.f0).toBeCloseTo(2.5, 6);
    expect(fitted.f1).toBe(5);
    const clampedLow = clampView(fitted, 1, 5);
    expect(clampedLow.f0).toBeCloseTo(2.5, 6);
    expect(clampedLow.f1).toBeCloseTo(5, 6);
    const zoomed = zoomView(fitted, 0.5, 0.5, 3.5, 1, 5, true);
    expect(Number.isFinite(zoomed.f0)).toBe(true);
    expect(Number.isFinite(zoomed.f1)).toBe(true);
    expect(zoomed.f1).toBeGreaterThan(zoomed.f0);
  });

  it("keeps a requested high log floor for a narrow range", () => {
    const view = { t0: 0, t1: 1, f0: 600, f1: 1_000, logFreq: true };
    expect(logFloor(600, 1_000)).toBe(600);
    expect(freqAtFraction(0, view)).toBeCloseTo(600, 6);
    expect(freqAtFraction(1, view)).toBeCloseTo(1_000, 6);
    const fitted = fitView(1, 1_000, true, 600);
    expect(fitted.f0).toBe(600);
    expect(fitted.f1).toBe(1_000);
  });

  it("uses bounded full views for mini-map resets", () => {
    const bounds = { minHz: 100, maxHz: 800 };
    const full = fitView(10, 800, false, 100);
    const rect = minimapRect(full, 10, 800, 200, 36, bounds);
    expect(rect.x).toBeCloseTo(0, 6);
    expect(rect.y).toBeCloseTo(0, 6);
    expect(rect.w).toBeCloseTo(200, 6);
    expect(rect.h).toBeCloseTo(36, 6);
  });

  it("preserves the log zoom ratio when clicking the mini-map", () => {
    const view = { t0: 2, t1: 4, f0: 100, f1: 400, logFreq: true };
    const moved = viewFromMinimap(0.5, 0.5, view, 10, 1_000);
    expect(moved.logFreq).toBe(true);
    expect(moved.t0).toBeCloseTo(4, 6);
    expect(moved.t1).toBeCloseTo(6, 6);
    expect(moved.f1 / moved.f0).toBeCloseTo(4, 6);
    expect(moved.f0).toBeCloseTo(50, 6);
    expect(moved.f1).toBeCloseTo(200, 6);
  });

  it("maps colorbar fractions to dB bounds", () => {
    expect(dbFromFraction(0, -80, 0)).toBe(0);
    expect(dbFromFraction(1, -80, 0)).toBe(-80);
    expect(dbFromFraction(0.5, -80, 0)).toBe(-40);
    expect(fractionFromDb(0, -80, 0)).toBe(0);
    expect(fractionFromDb(-80, -80, 0)).toBe(1);
    expect(fractionFromDb(50, -80, 0)).toBe(0);
    expect(fractionFromDb(-120, -80, 0)).toBe(1);
  });

  it("finds the nearest tracked order", () => {
    expect(nearestOrder(100, 6000, [1, 2, 4])?.order).toBe(1);
    expect(nearestOrder(205, 6000, [1, 2, 4])?.order).toBe(2);
    expect(nearestOrder(100, 6000, [])).toBeNull();
    expect(nearestOrder(100, 0, [1])).toBeNull();
    const result = nearestOrder(100, 6000, [1]);
    expect(result?.target).toBeCloseTo(100, 6);
    expect(freqAtFraction(0, fitView(1, 1000, true))).toBeCloseTo(LOG_MIN_HZ, 6);
  });
});

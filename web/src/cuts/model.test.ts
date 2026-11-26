import { describe, expect, it } from "vitest";
import {
  FALLBACK_PALETTE,
  addOrderValue,
  axisFor,
  cutLabel,
  cutX,
  defaultLineFor,
  deleteAt,
  flipLine,
  isDegenerate,
  lineAtPoint,
  makeLine,
  moveLineEndpoint,
  resolveLineAxis,
  seriesColor,
  snapLineEndpoint,
  stripLineSnapHistory,
  toggleLineOrientation,
  type Cut,
  type CutPoint,
  type CutsPayload,
} from "./model";

const point = (timeS: number, rpm: number, freqHz: number, db: number): CutPoint => ({
  timeS,
  rpm,
  freqHz,
  db,
});

const cut = (label: "order" | "freq" | "line", value: number, points: CutPoint[]): Cut => ({
  kind: { type: label, value },
  points,
});

describe("cut models and editing", () => {
  it("labels cuts and maps axes", () => {
    expect(cutLabel({ type: "order", value: 2 })).toBe("2x");
    expect(cutLabel({ type: "freq", value: 120 })).toBe("120 Hz");
    expect(cutLabel({ type: "line", value: 3 })).toBe("line 3");
    const p = point(1, 2000, 100, -20);
    expect(cutX(p, "rpm")).toBe(2000);
    expect(cutX(p, "time")).toBe(1);
    expect(cutX(p, "freq")).toBe(100);
  });

  it("uses the theme palette cyclically", () => {
    const palette = ["#111111", "#222222", "#333333"];
    expect(seriesColor(palette, 0)).toBe("#111111");
    expect(seriesColor(palette, 3)).toBe("#111111");
    expect(seriesColor([], 0)).toBe(FALLBACK_PALETTE[0]);
    expect(FALLBACK_PALETTE).toHaveLength(8);
    expect(new Set(FALLBACK_PALETTE).size).toBe(8);
  });

  it("falls back to time when RPM is constant", () => {
    expect(axisFor("rpm", true)).toEqual({
      axis: "time",
      note: "RPM is constant, so level is shown against time",
    });
    expect(axisFor("time", true)).toEqual({ axis: "time", note: null });
    expect(axisFor("rpm", false)).toEqual({ axis: "rpm", note: null });
  });

  it("detects degenerate cuts per axis", () => {
    const vertical = cut("line", 1, [point(0.5, 1000, 10, -10), point(0.5, 1000, 100, -20)]);
    expect(isDegenerate(vertical, "time", 0.1, 1)).toBe(true);
    expect(isDegenerate(vertical, "freq", 0.1, 1)).toBe(false);
    const fixed = cut("freq", 100, [point(0, 1000, 100, -10), point(1, 2000, 100, -20)]);
    expect(isDegenerate(fixed, "freq", 0.1, 1)).toBe(true);
    expect(isDegenerate(fixed, "rpm", 0.1, 1)).toBe(false);
  });

  it("resolves the Auto line axis from drawable cuts", () => {
    const vertical = cut("line", 1, [point(0.5, 0, 10, -10), point(0.5, 0, 100, -20)]);
    const sloped = cut("line", 2, [point(0, 0, 10, -10), point(1, 0, 100, -20)]);
    expect(resolveLineAxis([vertical], "auto", 0.1, 1)).toBe("freq");
    expect(resolveLineAxis([sloped], "auto", 0.1, 1)).toBe("time");
    expect(resolveLineAxis([], "auto", 0.1, 1)).toBe("time");
    expect(resolveLineAxis([vertical], "time", 0.1, 1)).toBe("time");
    expect(resolveLineAxis([vertical], "freq", 0.1, 1)).toBe("freq");
  });

  it("flips and moves line endpoints", () => {
    const line = makeLine(1, 100, 2, 200);
    expect(flipLine(line)).toEqual(makeLine(2, 200, 1, 100));
    expect(moveLineEndpoint(line, "start", 0.5, 50)).toEqual(
      makeLine(0.5, 50, 2, 200),
    );
    expect(moveLineEndpoint(line, "end", 3, 300)).toEqual(
      makeLine(1, 100, 3, 300),
    );
  });

  it("creates a clamped horizontal line through a clicked point", () => {
    expect(lineAtPoint(1.5, 400, 1000, 8)).toEqual(
      makeLine(0, 400, 8, 400),
    );
    expect(lineAtPoint(-1, 990, 1000, 2.5)).toEqual(
      makeLine(0, 990, 2.5, 990),
    );
    expect(lineAtPoint(2, -100, 1000, 3)).toEqual(makeLine(0, 0, 3, 0));
    expect(lineAtPoint(1, 2000, 1000, 4)).toEqual(
      makeLine(0, 1000, 4, 1000),
    );
    // No artificial low-frequency floor: sub-hertz values are kept as-is.
    expect(lineAtPoint(0, 0.5, 1000, 1).f0).toBe(0.5);

    const edge = lineAtPoint(0, 0, 1000, 1);
    expect(edge.t0).toBe(0);
    expect(edge.t1).toBe(1);
    expect(edge.f0).toBe(0);
    expect(edge.f0).toBe(edge.f1);
    expect(edge.f1).toBeLessThanOrEqual(1000);
    expect(edge.logF).toBe(false);
  });

  it("appends sorted unique order values", () => {
    expect(addOrderValue("1,2,4", 5)).toBe("1,2,4,5");
    expect(addOrderValue("4, 1", 2.5)).toBe("1,2.5,4");
    expect(addOrderValue("1,2,4", 2)).toBeNull();
    expect(addOrderValue("1,2,4", 0)).toBeNull();
    expect(addOrderValue("1,2,4", Number.NaN)).toBeNull();
  });

  it("deletes selected cuts and seeds new lines from payload bounds", () => {
    expect(deleteAt(["a", "b", "c"], 1)).toEqual(["a", "c"]);
    const payload: CutsPayload = {
      cuts: [],
      durationS: 10,
      nyquist: 1000,
      binHz: 1,
      hopS: 0.1,
    };
    expect(defaultLineFor(payload)).toEqual(makeLine(0, 150, 10, 300));
  });

  it("snaps an endpoint near the horizontal or vertical axis", () => {
    const base = {
      fixedX: 100,
      fixedY: 100,
      time: 0.7,
      hz: 400,
      fixedTime: 0.2,
      fixedHz: 500,
      snapDeg: 5,
    };
    expect(
      snapLineEndpoint({ ...base, x: 240, y: 103 }),
    ).toEqual({ time: 0.7, hz: 500 });
    expect(
      snapLineEndpoint({ ...base, x: 103, y: 240 }),
    ).toEqual({ time: 0.2, hz: 400 });
    expect(
      snapLineEndpoint({ ...base, x: 240, y: 160 }),
    ).toEqual({ time: 0.7, hz: 400 });
    expect(
      snapLineEndpoint({ ...base, x: 240, y: 103, snapDeg: 0 }),
    ).toEqual({ time: 0.7, hz: 400 });
  });

  it("toggles a line horizontal, vertical and back with exact history", () => {
    const options = { minHz: 10, maxHz: 1000, durationS: 10 };
    const diagonal = makeLine(0, 100, 10, 200);

    const horizontalStart = toggleLineOrientation(diagonal, "start", options);
    expect(horizontalStart.f0).toBe(100);
    expect(horizontalStart.f1).toBe(100);

    const horizontalEnd = toggleLineOrientation(diagonal, "end", options);
    expect(horizontalEnd.f0).toBe(200);
    expect(horizontalEnd.f1).toBe(200);

    const vertical = toggleLineOrientation(horizontalStart, "start", options);
    expect(vertical.t0).toBe(5);
    expect(vertical.t1).toBe(5);
    expect(vertical.f0).toBe(10);
    expect(vertical.f1).toBe(1000);

    const restoredHorizontal = toggleLineOrientation(vertical, "start", options);
    expect(restoredHorizontal).toEqual(
      expect.objectContaining({
        t0: horizontalStart.t0,
        t1: horizontalStart.t1,
        f0: 100,
        f1: 100,
      }),
    );

    const restoredVertical = toggleLineOrientation(
      restoredHorizontal,
      "start",
      options,
    );
    expect(restoredVertical).toEqual(
      expect.objectContaining({
        t0: vertical.t0,
        t1: vertical.t1,
        f0: vertical.f0,
        f1: vertical.f1,
      }),
    );
  });

  it("drops orientation history when a line is moved manually", () => {
    const options = { minHz: 10, maxHz: 1000, durationS: 10 };
    const snapped = toggleLineOrientation(
      makeLine(0, 100, 10, 200),
      "start",
      options,
    );
    const moved = moveLineEndpoint(snapped, "end", 8, 300);
    expect(moved.snapHistory).toBeUndefined();
    expect(stripLineSnapHistory(moved)).toEqual(moved);
  });
});

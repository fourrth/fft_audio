import { describe, expect, it } from "vitest";
import {
  frequencyTicks,
  formatHz,
  formatTime,
  niceTimeTicks,
} from "./ticks";

describe("spectrogram ticks", () => {
  it("uses 1/2/5 × 10^n time ticks", () => {
    expect(niceTimeTicks(0, 10, 5)).toEqual([0, 2, 4, 6, 8, 10]);
    expect(niceTimeTicks(0.3, 1.7, 4)).toEqual([0.5, 1, 1.5]);
    expect(niceTimeTicks(2, 2, 4)).toEqual([]);
  });

  it("uses nice linear frequency ticks", () => {
    expect(
      frequencyTicks({ t0: 0, t1: 1, f0: 0, f1: 1_000, logFreq: false }, 5),
    ).toEqual([0, 200, 400, 600, 800, 1_000]);
    expect(
      frequencyTicks({ t0: 0, t1: 1, f0: -50, f1: 50, logFreq: false }, 4),
    ).toEqual([0, 20, 40]);
  });

  it("uses the canonical log series and falls back geometrically", () => {
    const logTicks = frequencyTicks(
      { t0: 0, t1: 1, f0: 10, f1: 20_000, logFreq: true },
      6,
    );
    expect(logTicks).toContain(10);
    expect(logTicks).toContain(1_000);
    expect(logTicks).toContain(20_000);
    expect(logTicks.every((hz) => hz >= 10 && hz <= 20_000)).toBe(true);

    const narrow = frequencyTicks(
      { t0: 0, t1: 1, f0: 2.5, f1: 5, logFreq: true },
      4,
    );
    expect(narrow.length).toBeGreaterThanOrEqual(2);
    expect(narrow[0]).toBeCloseTo(2.5, 6);
    expect(narrow[narrow.length - 1]).toBeCloseTo(5, 6);
  });

  it("formats compact Hz and time labels", () => {
    expect(formatHz(10)).toBe("10");
    expect(formatHz(500)).toBe("500");
    expect(formatHz(1_200)).toBe("1.2k");
    expect(formatHz(22_100)).toBe("22.1k");
    expect(formatHz(0)).toBe("0");
    expect(formatTime(0)).toBe("0");
    expect(formatTime(0.5)).toBe("0.5");
    expect(formatTime(2.25)).toBe("2.3");
    expect(formatTime(12.4)).toBe("12");
  });
});

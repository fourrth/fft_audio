import { describe, expect, it } from "vitest";
import {
  orderHz,
  orderPolyline,
  orderTrackingLabel,
  parseOrderList,
  rpmAt,
  rpmLabel,
} from "./rpm";
import { defaultOrderSettings, type RpmSettings } from "./settings";

function settingsWithRpm(rpm: Partial<RpmSettings>) {
  const settings = defaultOrderSettings();
  settings.rpm = { ...settings.rpm, ...rpm };
  return settings;
}

describe("RPM and order tracking", () => {
  it("evaluates constant RPM and the base order frequency", () => {
    const settings = settingsWithRpm({ source: "constant", constant: 2000 });
    expect(rpmAt(0, settings)).toBe(2000);
    expect(rpmAt(5, settings, 10)).toBe(2000);
    expect(orderHz(1, 2000)).toBeCloseTo(33.3333, 3);
    expect(rpmLabel(settings)).toBe("2000 RPM");
    expect(orderTrackingLabel(1, settings)).toBe(
      "2000 RPM · order 1 = 33.3–33.3 Hz",
    );
    const guide = orderPolyline(1, settings, 4, 1000);
    expect(guide).toEqual([
      { timeS: 0, freqHz: 2000 / 60 },
      { timeS: 4, freqHz: 2000 / 60 },
    ]);
  });

  it("interpolates ramp RPM and derives the order-1 range", () => {
    const settings = settingsWithRpm({
      source: "ramp",
      start: 2000,
      end: 2400,
    });
    expect(rpmAt(0, settings, 4)).toBe(2000);
    expect(rpmAt(2, settings, 4)).toBe(2200);
    expect(rpmAt(9, settings, 4)).toBe(2400);
    expect(rpmLabel(settings)).toBe("2000→2400 RPM");
    expect(orderTrackingLabel(1, settings)).toBe(
      "2000→2400 RPM · order 1 = 33.3–40.0 Hz",
    );
    const guide = orderPolyline(1, settings, 4, 1000, 8);
    expect(guide).toHaveLength(9);
    expect(guide[0].freqHz).toBeCloseTo(2000 / 60, 6);
    expect(guide[guide.length - 1].freqHz).toBeCloseTo(40, 6);
    expect(guide[0].timeS).toBe(0);
    expect(guide[guide.length - 1].timeS).toBe(4);
  });

  it("exposes CSV paths without fabricating a guide", () => {
    const settings = settingsWithRpm({
      source: "csv",
      csv: "/tmp/rpm.csv",
    });
    expect(Number.isNaN(rpmAt(1, settings))).toBe(true);
    expect(rpmLabel(settings)).toBe("CSV /tmp/rpm.csv");
    expect(orderTrackingLabel(1, settings)).toBe("CSV /tmp/rpm.csv");
    expect(orderPolyline(1, settings, 4, 1000)).toEqual([]);
  });

  it("parses positive order lists", () => {
    expect(parseOrderList("1, 2.5;3 x -1,0")).toEqual([1, 2.5, 3]);
  });
});

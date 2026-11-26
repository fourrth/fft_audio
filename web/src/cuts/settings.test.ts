import { describe, expect, it } from "vitest";
import { defaultLineSettings, readLineSettings } from "./settings";

describe("line cut settings", () => {
  it("defaults line snapping to two degrees", () => {
    expect(defaultLineSettings().snap_deg).toBe(2);
    expect(readLineSettings({}).snap_deg).toBe(2);
  });

  it("normalizes the old five-degree default but keeps custom values", () => {
    const base = {
      line_width_hz: 0,
      line_axis: "auto",
      snap: true,
      peak_snap: true,
      peak_min_db: 1,
    };
    expect(
      readLineSettings({
        lineCuts: { ...base, snap_deg: 5 },
      }).snap_deg,
    ).toBe(2);
    expect(
      readLineSettings({
        lineCuts: { ...base, snap_deg: 8 },
      }).snap_deg,
    ).toBe(8);
  });
});

import { describe, expect, it } from "vitest";
import {
  RESOLUTION_PRESETS,
  WINDOW_KINDS,
  activeResolutionPreset,
  analysisSettingsEqual,
  anyFieldMatches,
  applyAppearance,
  clampScale,
  defaultAnalysisSettings,
  defaultDisplaySettings,
  defaultSettings,
  estimatedFrameCount,
  frameMilliseconds,
  frequencyResolution,
  hopSamples,
  matchesFilter,
  readAnalysisSettings,
  readDisplaySettings,
  readInspectorMeta,
  resolutionPresetSettings,
  scaleFromPercent,
  scalePercent,
} from "./model";

describe("settings model", () => {
  it("provides the core defaults", () => {
    const analysis = defaultAnalysisSettings();
    expect(analysis.frame).toBe(2048);
    expect(analysis.hop).toBeNull();
    expect(hopSamples(analysis)).toBe(512);
    expect(analysis.window).toBe("hann");
    expect(defaultDisplaySettings()).toEqual({
      log_freq: true,
      show_grid: true,
      max_hz: null,
      min_hz: null,
      range_db: 80,
      db_min: null,
      db_max: null,
    });
    expect(WINDOW_KINDS).toHaveLength(7);
    expect(WINDOW_KINDS).toContain("none");
    expect(defaultAnalysisSettings().window).toBe("hann");
  });

  it("reads partial persisted settings with defaults", () => {
    const analysis = readAnalysisSettings({
      analysisSettings: {
        frame: 8192,
        hop: 1024,
        window: "blackman",
        highpass: 20,
        data: { column: "ax", remove_mean: false },
      },
    });
    expect(analysis.frame).toBe(8192);
    expect(analysis.hop).toBe(1024);
    expect(analysis.window).toBe("blackman");
    expect(analysis.highpass).toBe(20);
    expect(analysis.data.column).toBe("ax");
    expect(analysis.data.remove_mean).toBe(false);
    expect(analysis.pad).toBe(2);

    expect(readAnalysisSettings({ analysisSettings: { window: "bogus" } }).window).toBe(
      "hann",
    );
    expect(
      readAnalysisSettings({ analysisSettings: { window: "none" } }).window,
    ).toBe("none");
    expect(readDisplaySettings({})).toEqual(defaultDisplaySettings());
    expect(
      readDisplaySettings({
        displaySettings: {
          log_freq: false,
          max_hz: 4000,
          min_hz: 20,
          range_db: 60,
          db_min: -90,
          db_max: -10,
        },
      }),
    ).toEqual({
      log_freq: false,
      show_grid: true,
      max_hz: 4000,
      min_hz: 20,
      range_db: 60,
      db_min: -90,
      db_max: -10,
    });
    expect(
      readDisplaySettings({ displaySettings: { show_grid: false } }).show_grid,
    ).toBe(false);
  });

  it("tracks dirty analysis settings structurally", () => {
    const a = defaultAnalysisSettings();
    const b = defaultAnalysisSettings();
    expect(analysisSettingsEqual(a, b)).toBe(true);
    b.frame = 4096;
    expect(analysisSettingsEqual(a, b)).toBe(false);
    b.frame = 2048;
    b.data.column = "ax";
    expect(analysisSettingsEqual(a, b)).toBe(false);
  });

  it("estimates frame counts from staged framing", () => {
    const settings = defaultAnalysisSettings();
    expect(estimatedFrameCount(settings, 16000, 1)).toBe(
      1 + Math.ceil((16000 - 2048) / 512),
    );
    expect(estimatedFrameCount({ ...settings, frame: 4096 }, 16000, 1)).toBe(
      1 + Math.ceil((16000 - 4096) / 1024),
    );
    expect(estimatedFrameCount({ ...settings, frame: 32000 }, 16000, 1)).toBe(1);
    expect(estimatedFrameCount(settings, null, 1)).toBeNull();
    expect(estimatedFrameCount(settings, 16000, 0)).toBeNull();
  });

  it("derives frame, hop and resolution chips", () => {
    const settings = defaultAnalysisSettings();
    expect(hopSamples(settings)).toBe(512);
    expect(frameMilliseconds(settings, 16000)).toBe("128.0 ms");
    expect(frequencyResolution(settings, 16000)).toBe("3.91 Hz");
    expect(frameMilliseconds(settings, null)).toBe("—");
    expect(frequencyResolution(settings, 0)).toBe("—");
  });

  it("reads analysis metadata for derived chips", () => {
    const meta = readInspectorMeta({
      analysisMeta: { sampleRate: 16000, durationS: 2, frames: 20, bins: 513 },
    });
    expect(meta).toEqual({
      sampleRate: 16000,
      durationS: 2,
      frames: 20,
      bins: 513,
    });
    expect(readInspectorMeta({})).toEqual({});

    const tableMeta = readInspectorMeta({
      analysisMeta: {
        sampleRate: 2500,
        table: {
          rows: 2500,
          columns: ["time_s", "amplitude"],
          signal: "amplitude",
          time: "time_s",
          notes: [],
        },
      },
    });
    expect(tableMeta.table).toEqual({
      rows: 2500,
      columns: ["time_s", "amplitude"],
      signal: "amplitude",
      time: "time_s",
      notes: [],
    });
  });

  it("calculates resolution presets from the sample rate", () => {
    expect(resolutionPresetSettings("high-time", 2500)).toEqual({
      frame: 32,
      hop: 8,
      pad: 2,
    });
    expect(resolutionPresetSettings("balanced", 2500)).toEqual({
      frame: 128,
      hop: 32,
      pad: 2,
    });
    expect(resolutionPresetSettings("high-frequency", 2500)).toEqual({
      frame: 512,
      hop: 128,
      pad: 2,
    });
    expect(resolutionPresetSettings("balanced", 48000)?.frame).toBe(2048);
    expect(resolutionPresetSettings("balanced", null)).toBeNull();
    expect(resolutionPresetSettings("balanced", 0)).toBeNull();
    expect(RESOLUTION_PRESETS.map((preset) => preset.id)).toEqual([
      "high-time",
      "balanced",
      "high-frequency",
    ]);

    const settings = {
      ...defaultAnalysisSettings(),
      ...resolutionPresetSettings("balanced", 2500)!,
    };
    expect(activeResolutionPreset(settings, 2500)).toBe("balanced");
    expect(activeResolutionPreset(defaultAnalysisSettings(), 2500)).toBeNull();
  });

  it("filters case-insensitively and recognizes known labels", () => {
    expect(matchesFilter("High-pass", "")).toBe(true);
    expect(matchesFilter("High-pass", "PASS")).toBe(true);
    expect(matchesFilter("High-pass", "frame")).toBe(false);
    expect(anyFieldMatches("frame")).toBe(true);
    expect(anyFieldMatches("zzz")).toBe(false);
  });

  it("clamps UI scale to 75–200%", () => {
    expect(clampScale(0.4)).toBe(0.75);
    expect(clampScale(2.5)).toBe(2);
    expect(clampScale(Number.NaN)).toBe(1);
    expect(scalePercent(1.25)).toBe(125);
    expect(scaleFromPercent(75)).toBe(0.75);
    expect(scaleFromPercent(500)).toBe(2);
  });

  it("builds complete default settings and applies appearance flags", () => {
    const settings = defaultSettings();
    expect(settings.appearance.theme).toBe("dark");
    expect(settings.session.reopen_last).toBe(true);
    expect(settings.order_cuts.orders).toBe("1,2,4");
    expect(settings.line_cuts.line_axis).toBe("auto");

    const values = new Map<string, string>();
    const attrs = new Map<string, string>();
    const root = {
      style: { setProperty: (name: string, value: string) => values.set(name, value) },
      setAttribute: (name: string, value: string) => attrs.set(name, value),
    };
    applyAppearance(
      {
        theme: "light",
        ui_scale: 1.5,
        reduce_motion: true,
        high_contrast: true,
      },
      root,
    );
    expect(values.get("--ui-scale")).toBe("1.5");
    expect(attrs.get("data-theme")).toBe("light");
    expect(attrs.get("data-reduce-motion")).toBe("true");
    expect(attrs.get("data-high-contrast")).toBe("true");
  });
});

import { describe, expect, it } from "vitest";
import { MockTransport } from "../transport/mock";
import { loadAndApplyTheme } from "./boot";
import { applyTheme, themeToCssVars, type StyleTarget } from "./css";
import type { ResolvedTheme } from "./types";

const theme: ResolvedTheme = {
  name: "Test",
  appearance: "light",
  colors: {
    "bg-canvas": "#eef2f6",
    "font-sans": "Inter, sans-serif",
  },
  plotPalette: [
    "#0072b2",
    "#e69f00",
    "#009e73",
    "#cc79a7",
    "#f0e442",
    "#56b4e9",
    "#d55e00",
    "#334155",
  ],
  status: {
    info: { bg: "rgba(29, 78, 216, 0.10)", fg: "#1e40af" },
    success: { bg: "rgba(5, 150, 105, 0.12)", fg: "#047857" },
    warn: { bg: "rgba(180, 83, 9, 0.12)", fg: "#92400e" },
    error: { bg: "rgba(220, 38, 38, 0.12)", fg: "#b91c1c" },
  },
};

function fakeTarget() {
  const values = new Map<string, string>();
  const attrs = new Map<string, string>();
  const target: StyleTarget = {
    style: {
      setProperty(name, value) {
        values.set(name, value);
      },
    },
    setAttribute(name, value) {
      attrs.set(name, value);
    },
  };
  return { target, values, attrs };
}

describe("theme CSS bridge", () => {
  it("maps tokens, palette and status tones to CSS variables", () => {
    const vars = themeToCssVars(theme);

    expect(vars["--bg-canvas"]).toBe("#eef2f6");
    expect(vars["--font-sans"]).toBe("Inter, sans-serif");
    expect(vars["--plot-palette-1"]).toBe("#0072b2");
    expect(vars["--plot-palette-8"]).toBe("#334155");
    expect(vars["--plot-palette"]).toContain("#d55e00");
    expect(vars["--status-success-fg"]).toBe("#047857");
    expect(vars["--status-info-bg"]).toBe("rgba(29, 78, 216, 0.10)");
    expect(vars["--status-warn-fg"]).toBe("#92400e");
    expect(vars["--status-error-bg"]).toBe("rgba(220, 38, 38, 0.12)");
  });

  it("injects variables and sets the appearance attribute", () => {
    const { target, values, attrs } = fakeTarget();
    applyTheme(theme, target);

    expect(values.get("--bg-canvas")).toBe("#eef2f6");
    expect(values.get("--plot-palette-8")).toBe("#334155");
    expect(values.get("--status-info-fg")).toBe("#1e40af");
    expect(attrs.get("data-theme")).toBe("light");
  });

  it("refuses to inject without a DOM target", () => {
    expect(() => applyTheme(theme)).toThrow(/DOM element/);
  });

  it("fetches a theme through the Transport and applies it", async () => {
    const { target, values } = fakeTarget();
    const transport = new MockTransport({
      "theme.get": () => theme,
    });

    const applied = await loadAndApplyTheme(transport, "light", target);

    expect(applied).toBe(theme);
    expect(values.get("--accent-primary")).toBeUndefined();
    expect(values.get("--bg-canvas")).toBe("#eef2f6");
    expect(transport.calls).toEqual([
      { name: "theme.get", args: { name: "light" } },
    ]);
  });

  it("requests the default theme with empty args", async () => {
    const transport = new MockTransport({
      "theme.get": () => theme,
    });

    await loadAndApplyTheme(transport, undefined, fakeTarget().target);

    expect(transport.calls).toEqual([{ name: "theme.get", args: {} }]);
  });
});

// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MockTransport } from "../transport/mock";
import { SettingsContent } from "./SettingsContent";
import { defaultSettings } from "./model";
import type { ResolvedTheme } from "../theme/types";

afterEach(cleanup);

const theme: ResolvedTheme = {
  name: "Dark",
  appearance: "dark",
  colors: {
    "bg-canvas": "#0f1217",
    "bg-panel": "#151921",
    "bg-card": "#1c222d",
    "text-primary": "#e6edf7",
    "accent-primary": "#4ea1ff",
  },
  plotPalette: [],
  status: {
    info: { bg: "#000", fg: "#fff" },
    success: { bg: "#000", fg: "#fff" },
    warn: { bg: "#000", fg: "#fff" },
    error: { bg: "#000", fg: "#fff" },
  },
};

function makeTransport() {
  const transport = new MockTransport({
    "settings.get": () => defaultSettings(),
    "settings.save": () => undefined,
    "config.paths": () => ({
      configDir: "/tmp/fft_audio",
      settingsPath: "/tmp/fft_audio/settings.toml",
      sessionPath: "/tmp/fft_audio/session.json",
      themesDir: "/tmp/fft_audio/themes",
    }),
    "app.info": () => ({
      name: "fft_audio",
      version: "0.12.0",
      mode: "desktop",
      capabilities: {
        displays: true,
        scripting: "python",
        tableFormats: ["csv", "parquet", "arrow"],
      },
    }),
    "theme.list": () => [
      {
        id: "dark",
        name: "Dark",
        builtin: true,
        appearance: "dark",
        preview: {
          bg: "#0f1217",
          panel: "#151921",
          card: "#1c222d",
          text: "#e6edf7",
          accent: "#4ea1ff",
        },
        parseError: null,
      },
      {
        id: "light",
        name: "Light",
        builtin: true,
        appearance: "light",
        preview: {
          bg: "#eef2f6",
          panel: "#f7f9fc",
          card: "#ffffff",
          text: "#111827",
          accent: "#1d4ed8",
        },
        parseError: null,
      },
    ],
    "theme.get": () => theme,
    openPath: () => undefined,
  });
  return transport;
}

describe("SettingsContent", () => {
  it("loads settings, paths, themes and app info", async () => {
    const transport = makeTransport();
    render(<SettingsContent transport={transport} />);

    expect(await screen.findByRole("heading", { name: "General" })).toBeTruthy();
    expect(transport.calls.some((call) => call.name === "settings.get")).toBe(true);
    expect(transport.calls.some((call) => call.name === "config.paths")).toBe(true);
    expect(transport.calls.some((call) => call.name === "theme.list")).toBe(true);
  });

  it("changes the UI scale and saves it", async () => {
    const transport = makeTransport();
    render(<SettingsContent transport={transport} />);
    await screen.findByRole("heading", { name: "General" });
    fireEvent.click(screen.getByRole("button", { name: /Appearance/ }));

    const scale = screen.getByRole("slider", { name: "UI scale" });
    fireEvent.change(scale, { target: { value: "150" } });
    await waitFor(() => {
      const saves = transport.calls.filter((call) => call.name === "settings.save");
      expect(saves.length).toBeGreaterThan(0);
      const args = saves.at(-1)?.args as {
        settings: { appearance: { ui_scale: number } };
      };
      expect(args.settings.appearance.ui_scale).toBe(1.5);
    });
    expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe(
      "1.5",
    );
  });

  it("filters the shortcut table", async () => {
    const transport = makeTransport();
    render(<SettingsContent transport={transport} />);
    await screen.findByRole("heading", { name: "General" });
    fireEvent.click(screen.getByRole("button", { name: /Keyboard/ }));
    expect(screen.getByText("Command palette")).toBeTruthy();
    fireEvent.change(screen.getByRole("searchbox", { name: "Filter shortcuts" }), {
      target: { value: "palette" },
    });
    expect(screen.getByText("Command palette")).toBeTruthy();
    expect(screen.queryByText("Fit view")).toBeNull();
  });

  it("shows file paths and the About stack", async () => {
    const transport = makeTransport();
    render(<SettingsContent transport={transport} />);
    await screen.findByRole("heading", { name: "General" });

    fireEvent.click(screen.getByRole("button", { name: /Files/ }));
    expect(screen.getByText("/tmp/fft_audio/settings.toml")).toBeTruthy();
    expect(screen.getByText("/tmp/fft_audio/themes")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /About/ }));
    expect(screen.getByText("0.12.0")).toBeTruthy();
    expect(screen.getByText("Tauri 2 · React · Vite")).toBeTruthy();
  });

  it("applies a theme from the theme grid", async () => {
    const transport = makeTransport();
    render(<SettingsContent transport={transport} />);
    await screen.findByRole("heading", { name: "General" });
    fireEvent.click(screen.getByRole("button", { name: /Appearance/ }));

    fireEvent.click(screen.getByRole("radio", { name: /Light/ }));
    await waitFor(() => {
      expect(
        transport.calls.some(
          (call) =>
            call.name === "theme.get" &&
            (call.args as { name?: string }).name === "light",
        ),
      ).toBe(true);
    });
    await waitFor(() => {
      const args = transport.calls
        .filter((call) => call.name === "settings.save")
        .at(-1)?.args as { settings: { appearance: { theme: string } } };
      expect(args.settings.appearance.theme).toBe("light");
    });
  });
});

// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ShellDisplay } from "../shell/session";
import { MockTransport } from "../transport/mock";
import { InspectorContent } from "./InspectorContent";
import {
  defaultAnalysisSettings,
  defaultDisplaySettings,
} from "../settings/model";
import { defaultOrderSettings } from "../cuts/settings";

afterEach(cleanup);

function display(): ShellDisplay {
  return {
    id: 3,
    parent: null,
    kind: "spectrogram",
    name: "Spectrogram 3",
    state: {
      path: "/audio/kick.wav",
      analysisSettings: defaultAnalysisSettings(),
      displaySettings: defaultDisplaySettings(),
      analysisMeta: { sampleRate: 16000, frames: 20, bins: 513, durationS: 2 },
    },
  };
}

describe("InspectorContent", () => {
  it("renders cards, derived chips and the live filter", () => {
    render(
      <InspectorContent
        display={display()}
        transport={null}
        onUpdateState={() => {}}
        onBrowse={() => {}}
      />,
    );
    expect(screen.getByTestId("inspector")).toBeTruthy();
    expect(screen.getByText("Preview mode: settings are not persisted.")).toBeTruthy();
    expect(screen.getByText("128.0 ms")).toBeTruthy();
    expect(screen.getByText("3.91 Hz")).toBeTruthy();
    expect(screen.getByRole("spinbutton", { name: "Frame" })).toBeTruthy();

    fireEvent.change(screen.getByRole("searchbox", { name: "Filter settings" }), {
      target: { value: "frame" },
    });
    expect(screen.getByRole("spinbutton", { name: "Frame" })).toBeTruthy();
    expect(screen.queryByText("Log frequency")).toBeNull();
    expect(screen.queryByText("Remove mean")).toBeNull();
  });

  it("stages DSP edits, reverts and applies them", () => {
    const onUpdateState = vi.fn();
    render(
      <InspectorContent
        display={display()}
        transport={null}
        onUpdateState={onUpdateState}
        onBrowse={() => {}}
      />,
    );

    const frame = screen.getByRole("spinbutton", { name: "Frame" });
    fireEvent.change(frame, { target: { value: "4096" } });
    expect(screen.getByText(/Unapplied DSP edits/)).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Apply & re-analyse" }),
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Revert" }));
    expect(screen.queryByText(/Unapplied DSP edits/)).toBeNull();
    expect((frame as HTMLInputElement).value).toBe("2048");

    fireEvent.change(frame, { target: { value: "8192" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply & re-analyse" }));
    expect(onUpdateState).toHaveBeenCalledWith({
      analysisSettings: expect.objectContaining({ frame: 8192 }),
    });
  });

  it("renders the frame size and estimates the frame count while staged", () => {
    render(
      <InspectorContent
        display={display()}
        transport={null}
        onUpdateState={() => {}}
        onBrowse={() => {}}
      />,
    );

    expect(screen.getByText("2048 samples")).toBeTruthy();
    expect(screen.getByText("20 frames")).toBeTruthy();

    fireEvent.change(screen.getByRole("spinbutton", { name: "Frame" }), {
      target: { value: "4096" },
    });
    expect(screen.getByText("4096 samples")).toBeTruthy();
    expect(screen.getByText("~29 frames")).toBeTruthy();
  });

  it("shows table-only settings only for table inputs", () => {
    render(
      <InspectorContent
        display={display()}
        transport={null}
        onUpdateState={() => {}}
        onBrowse={() => {}}
      />,
    );
    expect(screen.queryByRole("textbox", { name: "Column" })).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Time column" })).toBeNull();
    expect(screen.queryByRole("spinbutton", { name: "Sample rate" })).toBeNull();
    expect(screen.queryByRole("switch", { name: "Remove mean" })).toBeNull();

    cleanup();
    const shell = display();
    shell.state = {
      ...shell.state,
      path: "/data/sinusoids.csv",
      analysisMeta: {
        sampleRate: 2500,
        frames: 2,
        bins: 2049,
        durationS: 1,
        table: {
          rows: 2500,
          columns: ["time_s", "amplitude"],
          signal: "amplitude",
          time: "time_s",
          notes: [],
        },
      },
    };
    render(
      <InspectorContent
        display={shell}
        transport={null}
        onUpdateState={() => {}}
        onBrowse={() => {}}
      />,
    );
    expect(screen.getByRole("textbox", { name: "Column" })).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "Time column" })).toBeTruthy();
    expect(screen.getByRole("spinbutton", { name: "Sample rate" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Remove mean" })).toBeTruthy();
  });

  it("applies resolution presets from the sample rate", () => {
    const onUpdateState = vi.fn();
    render(
      <InspectorContent
        display={display()}
        transport={null}
        onUpdateState={onUpdateState}
        onBrowse={() => {}}
      />,
    );

    const preset = screen.getByRole("combobox", { name: "Resolution preset" });
    fireEvent.change(preset, { target: { value: "high-time" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply & re-analyse" }));
    expect(onUpdateState).toHaveBeenCalledWith({
      analysisSettings: expect.objectContaining({
        frame: 128,
        hop: 32,
        pad: 2,
      }),
    });
  });

  it("applies display-only changes immediately", () => {
    const onUpdateState = vi.fn();
    render(
      <InspectorContent
        display={display()}
        transport={null}
        onUpdateState={onUpdateState}
        onBrowse={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("switch", { name: "Log frequency axis" }));
    expect(onUpdateState).toHaveBeenCalledWith({
      displaySettings: { ...defaultDisplaySettings(), log_freq: false },
    });

    fireEvent.click(screen.getByRole("switch", { name: "Show grid lines" }));
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...defaultDisplaySettings(), show_grid: false },
    });

    fireEvent.change(
      screen.getByRole("spinbutton", { name: "color range" }),
      { target: { value: "60" } },
    );
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...defaultDisplaySettings(), range_db: 60 },
    });
  });

  it("persists minimum/maximum frequency and clamps to Nyquist", () => {
    const onUpdateState = vi.fn();
    render(
      <InspectorContent
        display={display()}
        transport={null}
        onUpdateState={onUpdateState}
        onBrowse={() => {}}
      />,
    );

    const min = screen.getByRole("spinbutton", { name: "Minimum frequency" });
    const max = screen.getByRole("spinbutton", { name: "Maximum frequency" });
    expect(min).toBeTruthy();
    expect(max).toBeTruthy();

    fireEvent.change(min, { target: { value: "120" } });
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...defaultDisplaySettings(), min_hz: 120 },
    });

    fireEvent.change(min, { target: { value: "9000" } });
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...defaultDisplaySettings(), min_hz: 7999 },
    });

    fireEvent.change(max, { target: { value: "4000" } });
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...defaultDisplaySettings(), max_hz: 4000 },
    });

    fireEvent.change(max, { target: { value: "20000" } });
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...defaultDisplaySettings(), max_hz: 8000 },
    });

    fireEvent.click(screen.getByRole("button", { name: "Use Nyquist" }));
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...defaultDisplaySettings(), max_hz: null },
    });
  });

  it("edits absolute color bounds and clears them back to a range", () => {
    const onUpdateState = vi.fn();
    const settings = { ...defaultDisplaySettings(), db_min: 0 };
    const shell = display();
    shell.state = { ...shell.state, displaySettings: settings };
    render(
      <InspectorContent
        display={shell}
        transport={null}
        onUpdateState={onUpdateState}
        onBrowse={() => {}}
      />,
    );

    const range = screen.getByRole("spinbutton", { name: "color range" });
    expect((range as HTMLInputElement).disabled).toBe(true);
    expect(
      screen.getByText(/Absolute color bounds override the color range/),
    ).toBeTruthy();

    fireEvent.change(
      screen.getByRole("spinbutton", { name: "color ceiling" }),
      { target: { value: "-10" } },
    );
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...settings, db_max: 1 },
    });

    fireEvent.change(
      screen.getByRole("spinbutton", { name: "color floor" }),
      { target: { value: "-65" } },
    );
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...settings, db_min: -65 },
    });

    fireEvent.click(screen.getByRole("button", { name: "Use range" }));
    expect(onUpdateState).toHaveBeenLastCalledWith({
      displaySettings: { ...settings, db_min: null, db_max: null },
    });
  });

  it("finds the display card and its units by search", () => {
    render(
      <InspectorContent
        display={display()}
        transport={null}
        onUpdateState={() => {}}
        onBrowse={() => {}}
      />,
    );
    fireEvent.change(screen.getByRole("searchbox", { name: "Filter settings" }), {
      target: { value: "dB" },
    });
    expect(
      screen.getByRole("spinbutton", { name: "color range" }),
    ).toBeTruthy();
    expect(screen.queryByText("Log frequency")).toBeTruthy();
    expect(screen.queryByText("Frame")).toBeNull();
  });

  it("persists card state and can use settings as defaults", async () => {
    const transport = new MockTransport({
      "settings.setSpectrogramDefaults": () => undefined,
    });
    const onUpdateState = vi.fn();
    render(
      <InspectorContent
        display={display()}
        transport={transport}
        onUpdateState={onUpdateState}
        onBrowse={() => {}}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Analysis/ }));
    expect(onUpdateState).toHaveBeenCalledWith({
      inspectorCards: { analysis: false },
    });

    fireEvent.click(
      screen.getByRole("button", {
        name: "Use as default for new spectrograms",
      }),
    );
    expect(transport.calls[0]).toEqual({
      name: "settings.setSpectrogramDefaults",
      args: {
        analysis: defaultAnalysisSettings(),
        display: defaultDisplaySettings(),
      },
    });
  });

  it("shows only input controls for a launcher", () => {
    render(
      <InspectorContent
        display={{
          id: 2,
          parent: null,
          kind: "launcher",
          name: "Launcher",
          state: {},
        }}
        transport={null}
        onUpdateState={() => {}}
        onBrowse={() => {}}
      />,
    );

    expect(screen.getByRole("textbox", { name: "Input file" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Browse…" })).toBeTruthy();
    expect(screen.queryByRole("spinbutton", { name: "Frame" })).toBeNull();
    expect(screen.queryByLabelText("Derived values")).toBeNull();
    expect(screen.queryByText("Analysis")).toBeNull();
  });

  it("makes order cut settings editable in its own inspector", () => {
    const onUpdateState = vi.fn();
    render(
      <InspectorContent
        display={{
          id: 5,
          parent: 3,
          kind: "order_cuts",
          name: "Order Cuts",
          state: {},
        }}
        transport={null}
        onUpdateState={onUpdateState}
        onBrowse={() => {}}
      />,
    );

    expect(screen.getByText("Cut tracking")).toBeTruthy();
    expect(screen.getByRole("combobox", { name: "Inspector order axis" })).toBeTruthy();
    expect(screen.getByRole("combobox", { name: "Inspector RPM source" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Grid lines" })).toBeTruthy();

    fireEvent.change(
      screen.getByRole("combobox", { name: "Inspector order axis" }),
      { target: { value: "time" } },
    );
    expect(onUpdateState).toHaveBeenCalledWith({
      orderCuts: { ...defaultOrderSettings(), axis: "time" },
    });
  });

  it("makes line cut settings editable in its own inspector", () => {
    const onUpdateState = vi.fn();
    render(
      <InspectorContent
        display={{
          id: 6,
          parent: 3,
          kind: "line_cuts",
          name: "Line Cuts",
          state: {},
        }}
        transport={null}
        onUpdateState={onUpdateState}
        onBrowse={() => {}}
      />,
    );

    expect(
      screen.getByRole("combobox", { name: "Inspector line axis" }),
    ).toBeTruthy();
    expect(
      screen.getByRole("spinbutton", { name: "Inspector line snap degrees" }),
    ).toBeTruthy();
    expect(screen.getByRole("switch", { name: "Grid lines" })).toBeTruthy();
  });

  it("does not show analysis controls for a scripting display", () => {
    render(
      <InspectorContent
        display={{
          id: 4,
          parent: null,
          kind: "scripting",
          name: "Scripting",
          state: {},
        }}
        transport={null}
        onUpdateState={() => {}}
        onBrowse={() => {}}
      />,
    );

    expect(
      screen.getByText("Scripting displays have no analysis or display properties."),
    ).toBeTruthy();
    expect(screen.queryByRole("spinbutton", { name: "Frame" })).toBeNull();
    expect(screen.queryByText("Display")).toBeNull();
  });
});

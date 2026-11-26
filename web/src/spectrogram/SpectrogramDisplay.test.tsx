// @vitest-environment jsdom
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { SpectrogramDisplay } from "./SpectrogramDisplay";
import { defaultDisplaySettings } from "../settings/model";
import type { SpectrogramPayload } from "./types";

function payload(): SpectrogramPayload {
  const frames = 2;
  const bins = 2;
  const values = new Int16Array([0, -1000, -2000, -3000]);
  const bytes = new Uint8Array(values.buffer);
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return {
    meta: {
      frames,
      bins,
      binHz: 500,
      frameS: 1,
      durationS: 1,
      nyquist: 1000,
      maxDb: 0,
      minDb: -30,
      sampleRate: 2000,
    },
    data: btoa(binary),
  };
}

beforeAll(() => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
    () =>
      ({
        createImageData: (width: number, height: number) => ({
          data: new Uint8ClampedArray(width * height * 4),
          width,
          height,
        }),
        putImageData: vi.fn(),
        clearRect: vi.fn(),
        drawImage: vi.fn(),
        set imageSmoothingEnabled(_value: boolean) {},
      }) as unknown as CanvasRenderingContext2D,
  );
});

afterAll(() => vi.restoreAllMocks());
afterEach(cleanup);

describe("SpectrogramDisplay", () => {
  it("renders the ready heatmap controls", () => {
    const onExport = vi.fn();
    const onReanalyse = vi.fn();
    render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
        onExport={onExport}
        onReanalyse={onReanalyse}
      />,
    );

    expect(screen.getByText("Spectrogram 1")).toBeTruthy();
    expect(screen.getByTestId("spectrogram-plot")).toBeTruthy();
    expect(screen.getByTestId("frequency-axis")).toBeTruthy();
    expect(screen.getByTestId("time-axis")).toBeTruthy();
    expect(screen.getByTestId("colorbar")).toBeTruthy();
    expect(screen.getByTestId("minimap")).toBeTruthy();

    const scales = screen.getByTestId("spectrogram-scales");
    expect(scales.textContent).toContain("Range 80 dB");
    expect(scales.textContent).toContain("Log frequency");
    expect(scales.textContent).toContain("10–1k Hz");
    expect(scales.textContent).toContain("0–1 s");

    fireEvent.click(screen.getByRole("radio", { name: "Linear" }));
    expect(scales.textContent).toContain("Linear frequency");
    fireEvent.click(screen.getByRole("button", { name: "Export image" }));
    fireEvent.click(screen.getByRole("button", { name: "Re-analyse" }));
    expect(onExport).toHaveBeenCalledOnce();
    expect(onReanalyse).toHaveBeenCalledOnce();
  });

  it("persists color-bar commits and resets absolute bounds", () => {
    const onDisplaySettingsChange = vi.fn();
    const displaySettings = defaultDisplaySettings();
    render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
        displaySettings={displaySettings}
        onDisplaySettingsChange={onDisplaySettingsChange}
      />,
    );

    fireEvent.keyDown(screen.getByRole("slider", { name: "Maximum dB" }), {
      key: "ArrowUp",
    });
    expect(onDisplaySettingsChange).toHaveBeenLastCalledWith({
      ...displaySettings,
      db_min: -80,
      db_max: 1,
    });

    fireEvent.click(
      screen.getByRole("button", { name: "Reset color range" }),
    );
    expect(onDisplaySettingsChange).toHaveBeenLastCalledWith({
      ...displaySettings,
      db_min: null,
      db_max: null,
    });
  });

  it("shows the maximum frequency as the hard view cap", () => {
    render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
        displaySettings={{ ...defaultDisplaySettings(), max_hz: 500 }}
      />,
    );
    const scales = screen.getByTestId("spectrogram-scales");
    expect(scales.textContent).toContain("10–500 Hz");
    fireEvent.click(screen.getByRole("button", { name: "Zoom out" }));
    expect(scales.textContent).toContain("500 Hz");
  });

  it("toggles grid lines while keeping the axis ticks", () => {
    const onDisplaySettingsChange = vi.fn();
    const displaySettings = defaultDisplaySettings();
    const { container, rerender } = render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
        displaySettings={displaySettings}
        onDisplaySettingsChange={onDisplaySettingsChange}
      />,
    );

    expect(
      container.querySelectorAll(".spectrogram-grid-line").length,
    ).toBeGreaterThan(0);
    const toggle = screen.getByRole("button", { name: "Grid lines" });
    expect(toggle.getAttribute("aria-pressed")).toBe("true");

    fireEvent.click(toggle);
    expect(onDisplaySettingsChange).toHaveBeenLastCalledWith({
      ...displaySettings,
      show_grid: false,
    });

    rerender(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
        displaySettings={{ ...displaySettings, show_grid: false }}
        onDisplaySettingsChange={onDisplaySettingsChange}
      />,
    );
    expect(container.querySelectorAll(".spectrogram-grid-line")).toHaveLength(0);
    expect(screen.getByTestId("frequency-axis")).toBeTruthy();
    expect(screen.getByTestId("time-axis")).toBeTruthy();
  });

  it("applies absolute color floor and ceiling settings", () => {
    const displaySettings = defaultDisplaySettings();
    const { rerender } = render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
        displaySettings={{ ...displaySettings, db_min: -20, db_max: -5 }}
      />,
    );

    expect(screen.getByTestId("colorbar-min-value").textContent).toBe("-20");
    expect(screen.getByTestId("colorbar-max-value").textContent).toBe("-5");

    rerender(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
        displaySettings={{ ...displaySettings, db_min: -35, db_max: -12 }}
      />,
    );
    expect(screen.getByTestId("colorbar-min-value").textContent).toBe("-35");
    expect(screen.getByTestId("colorbar-max-value").textContent).toBe("-12");
  });

  it("pans with the wheel and zooms with the modifier", () => {
    const { container } = render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
      />,
    );
    const canvas = container.querySelector(".plot-canvas") as HTMLCanvasElement;
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      top: 0,
      left: 0,
      width: 800,
      height: 400,
      right: 800,
      bottom: 400,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);
    const scales = screen.getByTestId("spectrogram-scales");
    const initial = scales.textContent;

    // Fit fills the available frequency bounds, so zoom in before panning.
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    const zoomed = scales.textContent;
    fireEvent.wheel(canvas, { deltaY: 200, clientX: 400, clientY: 200 });
    expect(scales.textContent).not.toBe(zoomed);

    fireEvent.click(screen.getByRole("button", { name: "Fit view" }));
    fireEvent.wheel(canvas, {
      deltaY: -100,
      clientX: 400,
      clientY: 200,
      ctrlKey: true,
    });
    expect(scales.textContent).not.toBe(initial);
  });

  it("keeps the right-click read-out available while the menu is open", async () => {
    const { container } = render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
      />,
    );
    const canvas = container.querySelector(".plot-canvas") as HTMLCanvasElement;
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      top: 0,
      left: 0,
      width: 800,
      height: 400,
      right: 800,
      bottom: 400,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);

    fireEvent.pointerMove(canvas, { clientX: 100, clientY: 100 });
    await waitFor(() => expect(screen.getByTestId("readout")).toBeTruthy());

    fireEvent.contextMenu(canvas, { clientX: 100, clientY: 100 });
    const copy = screen.getByRole("menuitem", { name: "Copy read-out" });
    expect((copy as HTMLButtonElement).disabled).toBe(false);

    fireEvent.pointerLeave(canvas);
    expect((copy as HTMLButtonElement).disabled).toBe(false);
  });

  it("resets the view when the plot background is double-clicked", () => {
    render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
      />,
    );
    const scales = screen.getByTestId("spectrogram-scales");
    const initial = scales.textContent;

    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    expect(scales.textContent).not.toBe(initial);

    fireEvent.doubleClick(screen.getByTestId("spectrogram-plot"));
    expect(scales.textContent).toBe(initial);
  });

  it("toggles a line cut horizontal from its spectrogram overlay", () => {
    const onUpdateLineCut = vi.fn();
    const { container } = render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="ready"
        payload={payload()}
        lineCutOverlays={[
          {
            panelId: "panel-1",
            lineIndex: 0,
            color: "#56b4e9",
            snap: true,
            snapDeg: 2,
            line: { t0: 0, f0: 100, t1: 1, f1: 200, logF: false },
          },
        ]}
        onUpdateLineCut={onUpdateLineCut}
      />,
    );
    const group = container.querySelector('[data-line-key="panel-1:0"]');
    expect(group).toBeTruthy();
    fireEvent.doubleClick(group!);
    expect(onUpdateLineCut).toHaveBeenCalledWith(
      "panel-1",
      0,
      expect.objectContaining({ t0: 0, f0: 100, t1: 1, f1: 100 }),
    );
  });

  it("renders loading progress and calls cancel", () => {
    const onCancel = vi.fn();
    render(
      <SpectrogramDisplay
        title="Spectrogram 1"
        path="/audio/kick.wav"
        phase="loading"
        payload={null}
        progress={{ framesDone: 5, framesTotal: 10, etaSeconds: 2 }}
        onCancel={onCancel}
      />,
    );
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe(
      "50",
    );
    fireEvent.click(screen.getByRole("button", { name: "Cancel analysis" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });
});

// @vitest-environment jsdom
import { useState } from "react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import type { DockviewApi, IDockviewPanelProps } from "dockview-react";
import type { ShellDisplay } from "../shell/session";
import { MockTransport } from "../transport/mock";
import { defaultAnalysisSettings } from "../settings/model";
import {
  DisplayActionsContext,
  DisplayPanel,
  type DisplayActions,
} from "./DisplayPanel";
import type { SpectrogramPayload } from "../spectrogram/types";

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

function payload(): SpectrogramPayload {
  const bytes = new Uint8Array(new Int16Array([0, -1000]).buffer);
  return {
    meta: {
      frames: 1,
      bins: 1,
      binHz: 500,
      frameS: 1,
      durationS: 1,
      nyquist: 1000,
      maxDb: 0,
      minDb: -30,
      sampleRate: 2000,
    },
    data: btoa(String.fromCharCode(...bytes)),
  };
}

function actions(transport: MockTransport): DisplayActions {
  return {
    recentFiles: [],
    transport,
    scriptingEnabled: false,
    plotPalette: [],
    browseForAudio: async () => null,
    openPath: () => {},
    savePng: async () => null,
    exportCuts: async () => null,
    spawnChild: () => {},
    lineCutSiblings: () => [],
    moveLineCut: () => {},
    updateLineCut: () => {},
    saveSession: () => {},
    toggleInspector: () => {},
  };
}

function panelProps(display: ShellDisplay): IDockviewPanelProps<{
  display: ShellDisplay;
}> {
  const containerApi = {
    panels: [],
    onDidLayoutChange: () => ({ dispose: vi.fn() }),
  } as unknown as DockviewApi;
  return {
    params: { display },
    api: {
      id: `panel-${display.id}`,
      title: display.name,
      updateParameters: vi.fn(),
    },
    containerApi,
  } as unknown as IDockviewPanelProps<{ display: ShellDisplay }>;
}

describe("DisplayPanel backend attach", () => {
  it("attaches without opening and fetches again on analysis.done", async () => {
    const transport = new MockTransport({
      "spectrogram.get": () => payload(),
    });
    const display: ShellDisplay = {
      id: 1_000_000,
      parent: null,
      kind: "spectrogram",
      name: "tone.wav",
      state: { analysisId: 4, sourceKind: "backend", scripted: true },
    };
    const { unmount } = render(
      <DisplayActionsContext.Provider value={actions(transport)}>
        <DisplayPanel {...panelProps(display)} />
      </DisplayActionsContext.Provider>,
    );

    await waitFor(() =>
      expect(
        transport.calls.filter((call) => call.name === "spectrogram.get"),
      ).toHaveLength(1),
    );
    expect(transport.calls.some((call) => call.name === "spectrogram.open")).toBe(
      false,
    );

    transport.emit("analysis.done", { displayId: display.id });
    await waitFor(() =>
      expect(
        transport.calls.filter((call) => call.name === "spectrogram.get"),
      ).toHaveLength(2),
    );

    unmount();
    expect(
      transport.calls.some((call) => call.name === "analysis.cancel"),
    ).toBe(false);
  });

  it("adopts analysis settings from the payload for backend displays", async () => {
    const settings = {
      ...defaultAnalysisSettings(),
      frame: 8192,
      hop: 2048,
      pad: 1,
      window: "rectangular" as const,
    };
    const transport = new MockTransport({
      "spectrogram.get": () => ({
        ...payload(),
        meta: { ...payload().meta, analysisSettings: settings },
      }),
    });
    const display: ShellDisplay = {
      id: 1_000_000,
      parent: null,
      kind: "spectrogram",
      name: "scripted.wav",
      state: { analysisId: 4, sourceKind: "backend", scripted: true },
    };
    const updateParameters = vi.fn();
    const props = panelProps(display);
    props.api.updateParameters = updateParameters as never;
    const { unmount } = render(
      <DisplayActionsContext.Provider value={actions(transport)}>
        <DisplayPanel {...props} />
      </DisplayActionsContext.Provider>,
    );

    await waitFor(() => expect(updateParameters).toHaveBeenCalled());
    const updated = updateParameters.mock.calls.at(-1)?.[0] as {
      display: ShellDisplay;
    };
    expect(updated.display.state.analysisSettings).toMatchObject({
      frame: 8192,
      hop: 2048,
      pad: 1,
      window: "rectangular",
    });

    unmount();
  });

  it("retries the attach fetch while an analysis is still pending", async () => {
    let attempts = 0;
    const transport = new MockTransport({
      "spectrogram.get": () => {
        attempts += 1;
        if (attempts === 1) {
          throw new Error("the analysis is not ready yet");
        }
        return payload();
      },
    });
    const display: ShellDisplay = {
      id: 1_000_000,
      parent: null,
      kind: "spectrogram",
      name: "pending.wav",
      state: { analysisId: 4, sourceKind: "backend", scripted: true },
    };
    const { unmount } = render(
      <DisplayActionsContext.Provider value={actions(transport)}>
        <DisplayPanel {...panelProps(display)} />
      </DisplayActionsContext.Provider>,
    );

    await waitFor(
      () =>
        expect(
          transport.calls.filter((call) => call.name === "spectrogram.get"),
        ).toHaveLength(2),
      { timeout: 1500 },
    );

    unmount();
    expect(
      transport.calls.some((call) => call.name === "analysis.cancel"),
    ).toBe(false);
  });

  it("opens by path for a normal user display", async () => {
    const transport = new MockTransport({
      "spectrogram.open": () => ({ displayId: 2 }),
    });
    const display: ShellDisplay = {
      id: 2,
      parent: null,
      kind: "spectrogram",
      name: "tone.wav",
      state: { path: "/audio/tone.wav" },
    };
    render(
      <DisplayActionsContext.Provider value={actions(transport)}>
        <DisplayPanel {...panelProps(display)} />
      </DisplayActionsContext.Provider>,
    );

    await waitFor(() =>
      expect(
        transport.calls.some((call) => call.name === "spectrogram.open"),
      ).toBe(true),
    );
    expect(transport.calls.some((call) => call.name === "spectrogram.get")).toBe(
      false,
    );
  });

  it("keeps the empty state while the picker is open and after it is cancelled", async () => {
    const transport = new MockTransport();
    let resolvePick: (value: string | null) => void = () => {};
    const browseForAudio = vi.fn(
      () =>
        new Promise<string | null>((resolve) => {
          resolvePick = resolve;
        }),
    );
    const display: ShellDisplay = {
      id: 2,
      parent: null,
      kind: "spectrogram",
      name: "Spectrogram",
      state: {},
    };
    render(
      <DisplayActionsContext.Provider
        value={{ ...actions(transport), browseForAudio }}
      >
        <DisplayPanel {...panelProps(display)} />
      </DisplayActionsContext.Provider>,
    );

    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    expect(browseForAudio).toHaveBeenCalledTimes(1);
    expect(screen.getByText("No file loaded")).toBeTruthy();
    expect(screen.queryByText(/Analysing/)).toBeNull();

    await act(async () => {
      resolvePick(null);
      await Promise.resolve();
    });
    expect(screen.getByText("No file loaded")).toBeTruthy();
    expect(screen.queryByText(/Analysing/)).toBeNull();
    expect(
      transport.calls.some((call) => call.name === "spectrogram.open"),
    ).toBe(false);
  });

  it("starts loading only after the picker returns a path", async () => {
    const transport = new MockTransport({
      "spectrogram.open": () => ({ displayId: 2 }),
    });
    const initial: ShellDisplay = {
      id: 2,
      parent: null,
      kind: "spectrogram",
      name: "Spectrogram",
      state: {},
    };
    function Harness() {
      const [display, setDisplay] = useState(initial);
      const base = actions(transport);
      return (
        <DisplayActionsContext.Provider
          value={{
            ...base,
            browseForAudio: async () => "/audio/picked.wav",
            openPath: (_panelId, path) =>
              setDisplay((current) => ({
                ...current,
                name: "picked.wav",
                state: { ...current.state, path },
              })),
          }}
        >
          <DisplayPanel {...panelProps(display)} />
        </DisplayActionsContext.Provider>
      );
    }
    render(<Harness />);

    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    expect(screen.queryByText(/Analysing/)).toBeNull();
    await waitFor(() =>
      expect(
        transport.calls.some((call) => call.name === "spectrogram.open"),
      ).toBe(true),
    );
    expect(screen.getByText(/Analysing/)).toBeTruthy();
  });
});

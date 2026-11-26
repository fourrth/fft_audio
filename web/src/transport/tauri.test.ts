import { describe, expect, it, vi } from "vitest";
import {
  COMMAND_NAMES,
  EVENT_NAMES,
  createTauriTransport,
  type TauriDeps,
} from "./tauri";
import type { EventMap } from "./types";

function fakeDeps() {
  const calls: Array<{ command: string; args?: Record<string, unknown> }> = [];
  const listeners = new Map<string, (event: { payload: unknown }) => void>();
  const unlisten = vi.fn();

  const deps: TauriDeps = {
    invoke: async <T>(command: string, args?: Record<string, unknown>) => {
      calls.push({ command, args });
      return {
        name: "fft_audio",
        version: "0.12.0",
        mode: "desktop",
        capabilities: {
          displays: true,
          scripting: "python",
          tableFormats: ["csv", "parquet", "arrow"],
        },
      } as T;
    },
    listen: async <T>(
      event: string,
      handler: (event: { payload: T }) => void,
    ) => {
      listeners.set(event, handler as (event: { payload: unknown }) => void);
      return unlisten;
    },
  };

  return { calls, listeners, unlisten, deps };
}

describe("TauriTransport", () => {
  it("maps dotted command names to Rust identifiers", async () => {
    const { calls, deps } = fakeDeps();
    const transport = createTauriTransport(deps);

    await transport.invoke({ name: "app.info", args: {} });
    await transport.invoke({
      name: "spectrogram.open",
      args: { displayId: 3, path: "a.wav" },
    });

    expect(transport.mode).toBe("desktop");
    expect(calls).toEqual([
      { command: "app_info", args: {} },
      { command: "spectrogram_open", args: { displayId: 3, path: "a.wav" } },
    ]);
    expect(COMMAND_NAMES["analysis.cancel"]).toBe("analysis_cancel");
    expect(COMMAND_NAMES["display.list"]).toBe("display_list");
    expect(COMMAND_NAMES["display.get"]).toBe("display_get");
    expect(COMMAND_NAMES["display.close"]).toBe("display_close");
    expect(COMMAND_NAMES["image.savePng"]).toBe("image_save_png");
    expect(COMMAND_NAMES["display.showSpectrogram"]).toBe(
      "display_show_spectrogram",
    );
    expect(COMMAND_NAMES["script.listExamples"]).toBe(
      "script_list_examples",
    );
    expect(COMMAND_NAMES["script.createExample"]).toBe(
      "script_create_example",
    );
  });

  it("forwards display command arguments unchanged", async () => {
    const { calls, deps } = fakeDeps();
    const transport = createTauriTransport(deps);

    await transport.invoke({ name: "display.list", args: {} });
    await transport.invoke({
      name: "display.get",
      args: { displayId: 1_000_000 },
    });
    await transport.invoke({
      name: "display.close",
      args: { displayId: 1_000_000 },
    });
    await transport.invoke({
      name: "display.showSpectrogram",
      args: { analysisId: 5 },
    });

    expect(calls).toEqual([
      { command: "display_list", args: {} },
      { command: "display_get", args: { displayId: 1_000_000 } },
      { command: "display_close", args: { displayId: 1_000_000 } },
      {
        command: "display_show_spectrogram",
        args: { analysisId: 5 },
      },
    ]);
  });

  it("maps contract events to valid Tauri event names", () => {
    expect(EVENT_NAMES["analysis.progress"]).toBe("analysis-progress");
    expect(EVENT_NAMES["display.open"]).toBe("display-open");
    expect(EVENT_NAMES["display.updated"]).toBe("display-updated");
    expect(EVENT_NAMES["display.closed"]).toBe("display-closed");
    for (const name of Object.values(EVENT_NAMES)) {
      expect(name).toMatch(/^[A-Za-z0-9_/:-]+$/);
    }
  });

  it("forwards event payloads and unsubscribes", async () => {
    const { listeners, unlisten, deps } = fakeDeps();
    const transport = createTauriTransport(deps);
    const received: EventMap["analysis.progress"][] = [];
    const off = transport.on("analysis.progress", (payload) => {
      received.push(payload);
    });

    await Promise.resolve();
    const handler = listeners.get("analysis-progress");
    expect(handler).toBeTypeOf("function");
    handler?.({
      payload: {
        displayId: 3,
        framesDone: 12,
        framesTotal: 100,
        etaSeconds: 4,
      },
    });

    expect(received).toEqual([
      { displayId: 3, framesDone: 12, framesTotal: 100, etaSeconds: 4 },
    ]);

    off();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("unlistens immediately when subscription closes before registration", async () => {
    const { deps, unlisten } = fakeDeps();
    const transport = createTauriTransport(deps);
    const off = transport.on("notification", () => {});
    off();
    await vi.waitFor(() => expect(unlisten).toHaveBeenCalledOnce());
  });
});

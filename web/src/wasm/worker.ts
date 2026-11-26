/// <reference lib="webworker" />

/**
 * Local WASM worker.
 *
 * The main thread never decodes or analyses files; it only stores settings in
 * browser storage and posts bytes here. The generated wasm-bindgen glue lives
 * in `web/public/wasm/` and is loaded lazily so normal HTTP/Tauri builds do
 * not need the WASM toolchain.
 */

import type { WasmEngine, WorkerInbound, WorkerOutbound } from "./protocol";

const scope = self as unknown as DedicatedWorkerGlobalScope;

let wasmBase = "";
let enginePromise: Promise<WasmEngine> | null = null;
let initError: string | null = null;
let resolveReady!: () => void;
let ready = new Promise<void>((resolve) => {
  resolveReady = resolve;
});
const files = new Map<string, Uint8Array>();

function post(message: WorkerOutbound): void {
  scope.postMessage(message);
}

function emit(event: string, payload: unknown): void {
  post({ kind: "event", event, payload });
}

function ensureEngine(): Promise<WasmEngine> {
  if (!enginePromise) {
    enginePromise = (async () => {
      const glueUrl = `${wasmBase}fft_wasm.js`;
      const wasmUrl = `${wasmBase}fft_wasm_bg.wasm`;
      // A runtime URL keeps the generated package out of the normal Vite
      // dependency graph; it is copied from web/public by the static build.
      const module = (await import(/* @vite-ignore */ glueUrl)) as {
        default: (input: { module_or_path: string }) => Promise<unknown>;
        FftWasm: new () => WasmEngine;
      };
      await module.default({ module_or_path: wasmUrl });
      return new module.FftWasm();
    })();
  }
  return enginePromise;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

async function dispatch(
  engine: WasmEngine,
  command: string,
  payload: unknown,
): Promise<unknown> {
  switch (command) {
    case "storeFile": {
      const value = payload as { name?: unknown; bytes?: unknown };
      if (typeof value?.name !== "string" || value.name.length === 0) {
        throw new Error("file has no name");
      }
      if (!(value.bytes instanceof ArrayBuffer)) {
        throw new Error("file contents are missing");
      }
      files.set(value.name, new Uint8Array(value.bytes));
      return { path: value.name };
    }
    case "spectrogram.open": {
      const value = payload as {
        displayId: number;
        path: string;
        settings?: unknown;
      };
      const bytes = files.get(value.path);
      if (!bytes) {
        throw new Error(
          `File "${value.path}" is no longer available; choose it again.`,
        );
      }
      const started = Date.now();
      let lastProgress = 0;
      const progress = (framesDone: number, framesTotal: number | null) => {
        const now = Date.now();
        // The core estimate can lag the actual trailing partial frame; clamp it
        // so the UI never shows "29/28 frames".
        const total =
          framesTotal === null ? null : Math.max(framesTotal, framesDone);
        if (total !== null && framesDone < total && now - lastProgress < 75) {
          return;
        }
        lastProgress = now;
        const etaSeconds =
          total !== null && framesDone > 0
            ? Math.max(
                0,
                ((now - started) / 1000) *
                  ((total - framesDone) / framesDone),
              )
            : null;
        emit("analysis.progress", {
          displayId: value.displayId,
          framesDone,
          framesTotal: total,
          etaSeconds,
        });
      };
      engine.open_with_progress(
        value.displayId,
        value.path,
        bytes,
        JSON.stringify(value.settings ?? {}),
        progress,
      );
      // The async desktop/headless transports publish this after their job
      // completes. The WASM work is synchronous, but the UI still expects it.
      emit("analysis.done", { displayId: value.displayId });
      return { displayId: value.displayId, analysisId: value.displayId };
    }
    case "spectrogram.get": {
      const value = payload as { displayId: number };
      return JSON.parse(engine.payload(value.displayId, 0, 0)) as unknown;
    }
    case "spectrogram.exportPng": {
      const value = payload as { displayId: number };
      return engine.export_png(value.displayId, "{}");
    }
    case "orderCuts.compute": {
      return JSON.parse(engine.order_cuts(JSON.stringify(payload))) as unknown;
    }
    case "lineCuts.compute": {
      return JSON.parse(engine.line_cuts(JSON.stringify(payload))) as unknown;
    }
    case "cuts.exportCsv": {
      const value = payload as { cuts: unknown };
      return engine.cuts_csv(JSON.stringify({ cuts: value.cuts }));
    }
    case "theme.get": {
      const value = payload as {
        name?: string;
        customThemes?: unknown;
      };
      return JSON.parse(
        engine.theme_get(
          value.name ?? "dark",
          JSON.stringify(value.customThemes ?? {}),
        ),
      ) as unknown;
    }
    case "theme.list": {
      const value = payload as { customThemes?: unknown };
      return JSON.parse(
        engine.theme_list(JSON.stringify(value.customThemes ?? {})),
      ) as unknown;
    }
    case "theme.duplicate": {
      const value = payload as {
        source: string;
        name: string;
        customThemes?: unknown;
      };
      return JSON.parse(
        engine.theme_duplicate(
          value.source,
          value.name,
          JSON.stringify(value.customThemes ?? {}),
        ),
      ) as unknown;
    }
    case "analysis.close": {
      const value = payload as { displayId: number };
      engine.close(value.displayId);
      return undefined;
    }
    default:
      throw new Error(`unknown WASM worker command "${command}"`);
  }
}

async function handle(message: WorkerInbound): Promise<void> {
  if (message.kind === "init") {
    wasmBase = message.wasmBase.endsWith("/")
      ? message.wasmBase
      : `${message.wasmBase}/`;
    try {
      await ensureEngine();
    } catch (error) {
      initError = errorText(error);
    } finally {
      resolveReady();
    }
    return;
  }
  await ready;
  if (initError) {
    post({
      kind: "response",
      id: message.id,
      ok: false,
      error: `Could not load the browser analysis engine: ${initError}`,
    });
    return;
  }
  try {
    const engine = await ensureEngine();
    const result = await dispatch(engine, message.command, message.payload);
    post({ kind: "response", id: message.id, ok: true, result });
  } catch (error) {
    post({
      kind: "response",
      id: message.id,
      ok: false,
      error: errorText(error),
    });
  }
}

scope.onmessage = (event: MessageEvent<WorkerInbound>) => {
  void handle(event.data);
};

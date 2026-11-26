import type {
  AppInfo,
  Command,
  CommandMap,
  CommandName,
  EventMap,
  EventName,
  Transport,
  Unsubscribe,
} from "./types";
import type { SessionDocument } from "../shell/session";
import { defaultSettings, type Settings } from "../settings/model";

import { triggerDownload } from "./download";
import { UPLOAD_ACCEPT } from "./tableExtensions";
import type {
  WorkerEvent,
  WorkerInbound,
  WorkerOutbound,
} from "../wasm/protocol";

const SETTINGS_KEY = "fft_audio.settings.v1";
const SESSION_KEY = "fft_audio.session.v1";
const THEMES_KEY = "fft_audio.themes.v1";

type StoredSettings = {
  analysis?: Partial<Settings["analysis"]> & {
    data?: Partial<Settings["analysis"]["data"]>;
  };
  display?: Partial<Settings["display"]>;
  order_cuts?: Partial<Settings["order_cuts"]> & {
    rpm?: Partial<Settings["order_cuts"]["rpm"]>;
  };
  line_cuts?: Partial<Settings["line_cuts"]>;
  session?: Partial<Settings["session"]>;
  appearance?: Partial<Settings["appearance"]>;
};

type PickedFile = { name: string; bytes: ArrayBuffer };

export interface BrowserTransportOptions {
  /** Test seam; production uses a module worker. */
  workerFactory?: () => Worker;
  /** Test seam; production uses `window.localStorage`. */
  storage?: Storage;
  /** Test seam; production opens a browser file picker. */
  pickFile?: () => Promise<PickedFile | null>;
}

/** True when the bundle was produced by the static browser build. */
export function isBrowserRuntime(): boolean {
  return (
    import.meta.env.MODE === "browser" ||
    import.meta.env.VITE_FFT_BROWSER === "1"
  );
}

function pageUrl(): string {
  return typeof window !== "undefined"
    ? window.location.href
    : "http://localhost/";
}

function readJson<T>(storage: Storage, key: string): T | null {
  try {
    const raw = storage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : null;
  } catch {
    return null;
  }
}

function writeJson(storage: Storage, key: string, value: unknown): void {
  try {
    storage.setItem(key, JSON.stringify(value));
  } catch (error) {
    throw new Error(
      `could not store browser data: ${
        error instanceof Error ? error.message : String(error)
      }`,
    );
  }
}

function loadSettings(storage: Storage): Settings {
  const defaults = defaultSettings();
  const stored = readJson<StoredSettings>(storage, SETTINGS_KEY);
  if (!stored) {
    return defaults;
  }
  return {
    ...defaults,
    ...stored,
    analysis: {
      ...defaults.analysis,
      ...stored.analysis,
      data: {
        ...defaults.analysis.data,
        ...stored.analysis?.data,
      },
    },
    display: { ...defaults.display, ...stored.display },
    order_cuts: {
      ...defaults.order_cuts,
      ...stored.order_cuts,
      rpm: { ...defaults.order_cuts.rpm, ...stored.order_cuts?.rpm },
    },
    line_cuts: { ...defaults.line_cuts, ...stored.line_cuts },
    session: { ...defaults.session, ...stored.session },
    appearance: { ...defaults.appearance, ...stored.appearance },
  };
}

function loadSession(storage: Storage): SessionDocument | null {
  return readJson<SessionDocument>(storage, SESSION_KEY);
}

function loadCustomThemes(storage: Storage): Record<string, string> {
  return readJson<Record<string, string>>(storage, THEMES_KEY) ?? {};
}

function saveCustomThemes(
  storage: Storage,
  themes: Record<string, string>,
): void {
  writeJson(storage, THEMES_KEY, themes);
}

async function pickFileWithInput(): Promise<PickedFile | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = UPLOAD_ACCEPT;
    let settled = false;
    const finish = (file: PickedFile | null) => {
      if (!settled) {
        settled = true;
        resolve(file);
      }
    };
    input.addEventListener("cancel", () => finish(null));
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) {
        finish(null);
        return;
      }
      finish({ name: file.name, bytes: await file.arrayBuffer() });
    };
    try {
      input.click();
    } catch {
      finish(null);
    }
  });
}

class WorkerClient {
  private readonly worker: Worker;
  private nextId = 1;
  private readonly pending = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: Error) => void }
  >();
  private readonly listeners = new Map<
    string,
    Set<(payload: unknown) => void>
  >();

  constructor(factory: () => Worker, wasmBase: string) {
    this.worker = factory();
    this.worker.onmessage = (event: MessageEvent<WorkerOutbound>) => {
      this.handle(event.data);
    };
    this.worker.onerror = (event) => {
      this.failAll(event.message || "WASM worker failed");
    };
    this.worker.postMessage({
      kind: "init",
      wasmBase,
    } satisfies WorkerInbound);
  }

  private handle(message: WorkerOutbound): void {
    if (message.kind === "event") {
      const event = message as WorkerEvent;
      const listeners = this.listeners.get(event.event);
      if (listeners) {
        for (const listener of [...listeners]) {
          listener(event.payload);
        }
      }
      return;
    }
    const entry = this.pending.get(message.id);
    if (!entry) {
      return;
    }
    this.pending.delete(message.id);
    if (message.ok) {
      entry.resolve(message.result);
    } else {
      entry.reject(new Error(message.error));
    }
  }

  private failAll(message: string): void {
    for (const entry of this.pending.values()) {
      entry.reject(new Error(message));
    }
    this.pending.clear();
  }

  request<T>(command: string, payload?: unknown): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, {
        resolve: (value) => resolve(value as T),
        reject,
      });
      this.worker.postMessage({
        kind: "request",
        id,
        command,
        payload,
      } satisfies WorkerInbound);
    });
  }

  onEvent(
    event: EventName,
    listener: (payload: unknown) => void,
  ): Unsubscribe {
    let listeners = this.listeners.get(event);
    if (!listeners) {
      listeners = new Set();
      this.listeners.set(event, listeners);
    }
    listeners.add(listener);
    return () => {
      listeners?.delete(listener);
    };
  }

  close(): void {
    this.failAll("browser transport closed");
    this.worker.terminate();
    this.listeners.clear();
  }
}

function defaultWorkerFactory(): Worker {
  return new Worker(new URL("../wasm/worker.ts", import.meta.url), {
    type: "module",
    name: "fft-wasm",
  });
}

function browserAppInfo(): AppInfo {
  return {
    name: "fft_audio",
    version: "0.13.0",
    mode: "browser",
    capabilities: {
      displays: true,
      scripting: null,
      tableFormats: ["csv", "json", "jsonl"],
    },
  };
}

/**
 * Transport for the static browser/WASM build.
 *
 * Files are read with a browser picker and kept in the worker's memory;
 * settings, session, and user themes use local storage. There is intentionally
 * no upload endpoint, host path, script engine, or server-side state.
 */
export function createBrowserTransport(
  options: BrowserTransportOptions = {},
): Transport {
  const storage =
    options.storage ??
    (typeof window !== "undefined" ? window.localStorage : undefined);
  if (!storage) {
    throw new Error("browser storage is not available");
  }
  const wasmBase = new URL(
    `${import.meta.env.BASE_URL}wasm/`,
    pageUrl(),
  ).href;
  const worker = new WorkerClient(
    options.workerFactory ?? defaultWorkerFactory,
    wasmBase,
  );
  const pickFile = options.pickFile ?? pickFileWithInput;

  const request = async (
    command: string,
    payload?: unknown,
  ): Promise<unknown> => worker.request(command, payload);

  return {
    mode: "browser",
    async invoke<K extends CommandName>(
      command: Command<K>,
    ): Promise<CommandMap[K]["result"]> {
      const args = command.args as Record<string, unknown>;
      switch (command.name) {
        case "app.info":
          return browserAppInfo() as CommandMap[K]["result"];
        case "app.ping":
          return {
            nonce: typeof args.nonce === "string" ? args.nonce : null,
            at: Date.now(),
          } as CommandMap[K]["result"];
        case "settings.get":
          return loadSettings(storage) as CommandMap[K]["result"];
        case "settings.save":
          writeJson(storage, SETTINGS_KEY, args.settings);
          return undefined as CommandMap[K]["result"];
        case "settings.setSpectrogramDefaults": {
          const current = loadSettings(storage);
          const next: Settings = {
            ...current,
            analysis: args.analysis as Settings["analysis"],
            display: args.display as Settings["display"],
          };
          writeJson(storage, SETTINGS_KEY, next);
          return undefined as CommandMap[K]["result"];
        }
        case "session.get":
          return loadSession(storage) as CommandMap[K]["result"];
        case "session.save":
          writeJson(storage, SESSION_KEY, args.session);
          return undefined as CommandMap[K]["result"];
        case "config.paths":
          return {
            configDir: "Browser local storage",
            settingsPath: SETTINGS_KEY,
            sessionPath: SESSION_KEY,
            themesDir: THEMES_KEY,
          } as CommandMap[K]["result"];
        case "theme.get": {
          const theme = await request("theme.get", {
            name: typeof args.name === "string" ? args.name : undefined,
            customThemes: loadCustomThemes(storage),
          });
          return theme as CommandMap[K]["result"];
        }
        case "theme.list": {
          const themes = await request("theme.list", {
            customThemes: loadCustomThemes(storage),
          });
          return themes as CommandMap[K]["result"];
        }
        case "theme.duplicate": {
          const customThemes = loadCustomThemes(storage);
          const created = (await request("theme.duplicate", {
            source: args.source,
            name: args.name,
            customThemes,
          })) as { id: string; toml: string };
          customThemes[created.id] = created.toml;
          saveCustomThemes(storage, customThemes);
          return created.id as CommandMap[K]["result"];
        }
        case "dialog.openAudio": {
          const picked = await pickFile();
          if (!picked) {
            return null as CommandMap[K]["result"];
          }
          const stored = (await request("storeFile", {
            name: picked.name,
            bytes: picked.bytes,
          })) as { path: string };
          return stored as CommandMap[K]["result"];
        }
        case "spectrogram.open": {
          const result = await request("spectrogram.open", {
            displayId: args.displayId,
            path: args.path,
            settings: args.settings ?? {},
          });
          return result as CommandMap[K]["result"];
        }
        case "spectrogram.get": {
          const payload = await request("spectrogram.get", {
            displayId: args.displayId,
          });
          return payload as CommandMap[K]["result"];
        }
        case "spectrogram.exportImage": {
          const bytes = (await request("spectrogram.exportPng", {
            displayId: args.displayId,
          })) as Uint8Array;
          const buffer = bytes.buffer.slice(
            bytes.byteOffset,
            bytes.byteOffset + bytes.byteLength,
          ) as ArrayBuffer;
          triggerDownload(
            new Blob([buffer], { type: "image/png" }),
            "spectrogram.png",
          );
          return { path: "spectrogram.png" } as CommandMap[K]["result"];
        }
        case "recording.save": {
          const { dataBase64, filename } = args as {
            dataBase64: string;
            filename: string;
          };
          const binary = atob(dataBase64);
          const bytes = new Uint8Array(binary.length);
          for (let index = 0; index < binary.length; index += 1) {
            bytes[index] = binary.charCodeAt(index);
          }
          const stored = (await request("storeFile", {
            name: filename,
            bytes: bytes.buffer,
          })) as { path: string };
          return stored as CommandMap[K]["result"];
        }
        case "image.savePng": {
          const { dataBase64, filename } = args as {
            dataBase64: string;
            filename: string;
          };
          const binary = atob(dataBase64);
          const bytes = new Uint8Array(binary.length);
          for (let index = 0; index < binary.length; index += 1) {
            bytes[index] = binary.charCodeAt(index);
          }
          triggerDownload(new Blob([bytes], { type: "image/png" }), filename);
          return { path: filename } as CommandMap[K]["result"];
        }
        case "analysis.cancel":
          // WASM analysis is synchronous inside the worker; the UI cannot
          // interrupt a decode already in progress. Nothing is left running.
          return { cancelled: false } as CommandMap[K]["result"];
        case "display.list":
          return [] as CommandMap[K]["result"];
        case "display.get":
          throw new Error("backend displays are not used in the browser build");
        case "display.close":
          await request("analysis.close", { displayId: args.displayId });
          return undefined as CommandMap[K]["result"];
        case "display.showSpectrogram":
          throw new Error("backend displays are not used in the browser build");
        case "orderCuts.compute": {
          const result = await request("orderCuts.compute", args);
          return result as CommandMap[K]["result"];
        }
        case "lineCuts.compute": {
          const result = await request("lineCuts.compute", args);
          return result as CommandMap[K]["result"];
        }
        case "cuts.exportCsv": {
          const csv = (await request("cuts.exportCsv", args)) as string;
          triggerDownload(
            new Blob([csv], { type: "text/csv;charset=utf-8" }),
            "cuts.csv",
          );
          return { path: "cuts.csv" } as CommandMap[K]["result"];
        }
        case "plot.load":
          throw new Error(
            "opening local table files for a plot is not available in the browser build",
          );
        case "openPath":
          throw new Error(
            "opening host paths is not available in the browser build",
          );
        case "script.pickFile":
        case "script.readFile":
        case "script.writeFile":
        case "script.listExamples":
        case "script.createExample":
        case "script.run":
        case "script.stop":
        case "script.stat":
          throw new Error(
            `${command.name} is only available in the desktop app; the static browser build does not run scripts`,
          );
      }
    },
    on<K extends EventName>(
      event: K,
      listener: (payload: EventMap[K]) => void,
    ): Unsubscribe {
      return worker.onEvent(event, listener as (payload: unknown) => void);
    },
    close() {
      worker.close();
    },
  };
}

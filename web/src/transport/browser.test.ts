import { describe, expect, it } from "vitest";
import { createBrowserTransport } from "./browser";
import type { WorkerInbound, WorkerOutbound } from "../wasm/protocol";
import { defaultSettings } from "../settings/model";

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();

  get length(): number {
    return this.values.size;
  }

  clear(): void {
    this.values.clear();
  }

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  key(index: number): string | null {
    return [...this.values.keys()][index] ?? null;
  }

  removeItem(key: string): void {
    this.values.delete(key);
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

class FakeWorker {
  onmessage: ((event: MessageEvent<WorkerOutbound>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  readonly sent: WorkerInbound[] = [];
  private readonly answered = new Set<number>();

  postMessage(message: unknown): void {
    this.sent.push(message as WorkerInbound);
  }

  terminate(): void {}

  requests(): Array<{ id: number; command: string; payload: unknown }> {
    return this.sent
      .filter(
        (message): message is Extract<WorkerInbound, { kind: "request" }> =>
          message.kind === "request" && !this.answered.has(message.id),
      )
      .map((message) => ({
        id: message.id,
        command: message.command,
        payload: message.payload,
      }));
  }

  respond(id: number, result: unknown): void {
    this.answered.add(id);
    this.onmessage?.({
      data: { kind: "response", id, ok: true, result },
    } as MessageEvent<WorkerOutbound>);
  }

  emit(event: string, payload: unknown): void {
    this.onmessage?.({
      data: { kind: "event", event, payload },
    } as MessageEvent<WorkerOutbound>);
  }
}

function setUp() {
  const worker = new FakeWorker();
  const storage = new MemoryStorage();
  const transport = createBrowserTransport({
    storage,
    workerFactory: () => worker as unknown as Worker,
    pickFile: async () => ({
      name: "tone.wav",
      bytes: new Uint8Array([1, 2, 3]).buffer,
    }),
  });
  return { worker, storage, browser: transport };
}

async function call(
  worker: FakeWorker,
  promise: Promise<unknown>,
  result: unknown,
): Promise<{
  value: unknown;
  request: { id: number; command: string; payload: unknown };
}> {
  let request = worker.requests().at(-1);
  for (let attempt = 0; attempt < 20 && !request; attempt += 1) {
    await Promise.resolve();
    request = worker.requests().at(-1);
  }
  expect(request).toBeDefined();
  worker.respond(request!.id, result);
  return { value: await promise, request: request! };
}

describe("browser WASM transport", () => {
  it("reports the browser mode and no scripting capability", async () => {
    const { browser } = setUp();
    const info = await browser.invoke({ name: "app.info", args: {} });
    expect(info.mode).toBe("browser");
    expect(info.capabilities.scripting).toBeNull();
    expect(info.capabilities.tableFormats).toEqual(["csv", "json", "jsonl"]);
  });

  it("stores settings and session in browser storage", async () => {
    const { browser, storage } = setUp();
    const settings = defaultSettings();
    settings.appearance.theme = "light";
    await browser.invoke({ name: "settings.save", args: { settings } });
    expect(await browser.invoke({ name: "settings.get", args: {} })).toEqual(
      settings,
    );
    expect(storage.getItem("fft_audio.settings.v1")).toContain('"light"');

    const session = {
      version: 1,
      nextId: 1,
      displays: [],
      recentFiles: [],
      layoutPresets: {},
      layout: null,
    };
    await browser.invoke({ name: "session.save", args: { session } });
    expect(await browser.invoke({ name: "session.get", args: {} })).toEqual(
      session,
    );
  });

  it("sends picked file bytes to the worker and opens the analysis", async () => {
    const { browser, worker } = setUp();
    const picked = browser.invoke({ name: "dialog.openAudio", args: {} });
    const stored = { path: "tone.wav" };
    const storedCall = await call(worker, picked, stored);
    expect(storedCall.value).toEqual(stored);
    expect(storedCall.request.command).toBe("storeFile");
    expect(storedCall.request.payload).toMatchObject({ name: "tone.wav" });

    const open = browser.invoke({
      name: "spectrogram.open",
      args: { displayId: 42, path: "tone.wav", settings: defaultSettings().analysis },
    });
    const openCall = await call(worker, open, {
      displayId: 42,
      analysisId: 42,
    });
    expect(openCall.value).toEqual({ displayId: 42, analysisId: 42 });
    expect(openCall.request.command).toBe("spectrogram.open");
  });

  it("dispatches worker events to transport listeners", () => {
    const { browser, worker } = setUp();
    const seen: number[] = [];
    const off = browser.on("analysis.done", (payload) => {
      seen.push(payload.displayId);
    });
    worker.emit("analysis.done", { displayId: 7 });
    worker.emit("analysis.done", { displayId: 9 });
    off();
    worker.emit("analysis.done", { displayId: 11 });
    expect(seen).toEqual([7, 9]);
  });

  it("rejects scripting and host-path commands", async () => {
    const { browser } = setUp();
    await expect(
      browser.invoke({ name: "script.run", args: { displayId: 1, path: "x.py" } }),
    ).rejects.toThrow(/static browser build/);
    await expect(
      browser.invoke({ name: "script.listExamples", args: {} }),
    ).rejects.toThrow(/static browser build/);
    await expect(
      browser.invoke({ name: "openPath", args: { path: "/tmp" } }),
    ).rejects.toThrow(/browser build/);
  });
});

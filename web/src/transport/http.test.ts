import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createHttpTransport,
  isHeadlessRuntime,
  loopbackApiBase,
  requestFor,
} from "./http";
import { TABLE_EXTENSIONS, UPLOAD_ACCEPT, isTablePath } from "./tableExtensions";
import type { Command } from "./types";

describe("headless HTTP request mapping", () => {
  it("maps core commands to loopback API routes", () => {
    expect(requestFor({ name: "app.info", args: {} })).toEqual({
      method: "GET",
      path: "/api/app/info",
    });
    expect(
      requestFor({
        name: "spectrogram.get",
        args: { displayId: 7 },
      }),
    ).toEqual({ method: "GET", path: "/api/spectrogram/7" });
    expect(
      requestFor({
        name: "spectrogram.open",
        args: { displayId: 3, path: "/a.wav" },
      }).path,
    ).toBe("/api/spectrogram/open");
    expect(
      requestFor({ name: "theme.get", args: { name: "light" } }).path,
    ).toBe("/api/theme?name=light");
    expect(requestFor({ name: "theme.list", args: {} }).path).toBe("/api/themes");
    expect(
      requestFor({
        name: "settings.save",
        args: {
          settings: {
            analysis: {
              frame: 2048,
              hop: null,
              pad: 2,
              window: "hann",
              highpass: null,
              lowpass: null,
              smooth_s: 0,
              data: {
                column: null,
                time_column: null,
                sample_rate: null,
                remove_mean: true,
              },
            },
            display: { log_freq: true, show_grid: true, max_hz: null, range_db: 80 },
            order_cuts: {
              orders: "1",
              order_width: 0.2,
              freqs: "",
              freq_width_hz: 5,
              axis: "rpm",
              smooth_s: 0,
              peak_snap: true,
              peak_min_db: 1,
              rpm: {
                source: "constant",
                constant: 2000,
                start: 1000,
                end: 3000,
                csv: null,
              },
            },
            line_cuts: {
              line_width_hz: 0,
              line_axis: "auto",
              snap: true,
              snap_deg: 2,
              peak_snap: true,
              peak_min_db: 1,
            },
            session: { reopen_last: true },
            appearance: {
              theme: "dark",
              ui_scale: 1,
              reduce_motion: false,
              high_contrast: false,
            },
          },
        },
      }).method,
    ).toBe("PUT");
  });

  it("maps display registry commands to exact routes and payloads", () => {
    expect(requestFor({ name: "display.list", args: {} })).toEqual({
      method: "GET",
      path: "/api/displays",
    });
    expect(
      requestFor({ name: "display.get", args: { displayId: 1_000_000 } }),
    ).toEqual({ method: "GET", path: "/api/displays/1000000" });
    expect(
      requestFor({ name: "display.close", args: { displayId: 7 } }),
    ).toEqual({
      method: "POST",
      path: "/api/displays/close",
      body: { id: 7 },
    });
    expect(
      requestFor({
        name: "display.showSpectrogram",
        args: { analysisId: 9 },
      }),
    ).toEqual({
      method: "POST",
      path: "/api/displays/spectrogram",
      body: { analysisId: 9 },
    });
    expect(
      requestFor({
        name: "display.showSpectrogram",
        args: { analysisId: 9, ephemeral: false },
      }),
    ).toEqual({
      method: "POST",
      path: "/api/displays/spectrogram",
      body: { analysisId: 9, ephemeral: false },
    });
  });

  it("dispatches dotted display events from WebSocket frames", () => {
    const sockets: Array<{
      onmessage: ((event: { data: string }) => void) | null;
      onclose: (() => void) | null;
    }> = [];
    class FakeWebSocket {
      onmessage: ((event: { data: string }) => void) | null = null;
      onclose: (() => void) | null = null;
      constructor(readonly url: string) {
        sockets.push(this);
      }
      close() {
        this.onclose?.();
      }
    }
    const transport = createHttpTransport({
      baseUrl: "http://127.0.0.1:8765",
      WebSocketImpl: FakeWebSocket as unknown as typeof WebSocket,
    });
    const opened: number[] = [];
    const closed: Array<{ id: number; children: number[] }> = [];
    const offOpen = transport.on("display.open", ({ display }) => {
      opened.push(display.id);
    });
    const offClosed = transport.on("display.closed", (payload) => {
      closed.push(payload);
    });

    expect(sockets).toHaveLength(1);
    sockets[0].onmessage?.({
      data: JSON.stringify({
        type: "display.open",
        display: {
          id: 1_000_000,
          parent: null,
          kind: "spectrogram",
          name: "tone.wav",
          state: { analysisId: 4 },
          visible: true,
          ephemeral: false,
        },
      }),
    });
    sockets[0].onmessage?.({
      data: JSON.stringify({
        type: "display.closed",
        id: 1_000_000,
        children: [1_000_001],
      }),
    });

    expect(opened).toEqual([1_000_000]);
    expect(closed).toEqual([{ id: 1_000_000, children: [1_000_001] }]);
    offOpen();
    offClosed();
  });

  it("rejects every script command as desktop-only", async () => {
    const commands: Command[] = [
      { name: "script.readFile", args: { path: "/tmp/x.py" } },
      { name: "script.writeFile", args: { path: "/tmp/x.py", contents: "print(1)" } },
      { name: "script.listExamples", args: {} },
      { name: "script.createExample", args: { filename: "harmonics_chirp_example.py" } },
      { name: "script.run", args: { displayId: 7, path: "/tmp/x.py" } },
      { name: "script.stop", args: { displayId: 7 } },
      { name: "script.stat", args: { path: "/tmp/x.py" } },
      { name: "script.pickFile", args: {} },
    ];
    for (const command of commands) {
      expect(() => requestFor(command)).toThrow(/desktop app/);
    }

    const fetchImpl = vi.fn(async () => new Response("{}"));
    const transport = createHttpTransport({
      baseUrl: "http://127.0.0.1:8765",
      fetchImpl: fetchImpl as unknown as typeof fetch,
    });
    for (const command of commands) {
      await expect(transport.invoke(command)).rejects.toThrow(/desktop app/);
    }
    expect(fetchImpl).not.toHaveBeenCalled();
  });

  it("refuses the desktop-only dialog command", () => {
    expect(() => requestFor({ name: "dialog.openAudio", args: {} })).toThrow();
  });

  it("keeps the upload picker browser-handled", () => {
    expect(() =>
      requestFor({ name: "dialog.openAudio", args: {} }),
    ).toThrow(/browser picker/);
  });

  it("offers every shipping table format to the upload picker", () => {
    expect(TABLE_EXTENSIONS).toEqual([
      "csv",
      "tsv",
      "tab",
      "txt",
      "dat",
      "parquet",
      "pq",
      "json",
      "ndjson",
      "jsonl",
      "arrow",
      "ipc",
      "feather",
    ]);
    const accepted = UPLOAD_ACCEPT.split(",");
    for (const extension of TABLE_EXTENSIONS) {
      expect(accepted).toContain(`.${extension}`);
    }
    expect(isTablePath("/data/SINUSOIDS.CSV")).toBe(true);
    expect(isTablePath("C:\\data\\table.parquet")).toBe(true);
    expect(isTablePath("/audio/kick.wav")).toBe(false);
    expect(isTablePath("/data/no-extension")).toBe(false);
  });
});

describe("headless loopback API base", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("accepts loopback origins only", () => {
    expect(loopbackApiBase("http://127.0.0.1:8765")).toBe(
      "http://127.0.0.1:8765",
    );
    expect(loopbackApiBase("http://localhost:8765")).toBe(
      "http://localhost:8765",
    );
    expect(loopbackApiBase("http://[::1]:8765")).toBe("http://[::1]:8765");
    expect(loopbackApiBase("https://example.com")).toBeNull();
    expect(loopbackApiBase("http://0.0.0.0:8765")).toBeNull();
    expect(loopbackApiBase("not a url")).toBeNull();
  });

  it("detects the server-injected headless flag", () => {
    vi.stubGlobal("window", {
      __FFT_HEADLESS__: true,
      location: { origin: "http://127.0.0.1:8765", search: "" },
    });
    expect(isHeadlessRuntime()).toBe(true);
  });

  it("detects a valid injected API base", () => {
    vi.stubGlobal("window", {
      __FFT_HEADLESS_API__: "http://localhost:8765",
    });
    expect(isHeadlessRuntime()).toBe(true);
  });

  it("detects headless from a valid ?api= override", () => {
    vi.stubGlobal("window", {
      location: {
        origin: "http://localhost:5173",
        search: "?api=http%3A%2F%2F127.0.0.1%3A8765",
      },
    });
    expect(isHeadlessRuntime()).toBe(true);
  });

  it("ignores non-loopback ?api= overrides", () => {
    vi.stubGlobal("window", {
      location: {
        origin: "http://localhost:5173",
        search: "?api=https%3A%2F%2Fexample.com",
      },
    });
    expect(isHeadlessRuntime()).toBe(false);
  });
});

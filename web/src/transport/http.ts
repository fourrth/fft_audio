import type {
  Command,
  CommandMap,
  CommandName,
  EventMap,
  EventName,
  Transport,
  Unsubscribe,
} from "./types";
import { triggerDownload } from "./download";
import { UPLOAD_ACCEPT } from "./tableExtensions";

declare global {
  interface Window {
    __FFT_HEADLESS__?: boolean;
    __FFT_HEADLESS_API__?: string;
  }
}

export interface HttpTransportOptions {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
  WebSocketImpl?: typeof WebSocket;
}

export interface HttpRequest {
  method: "GET" | "POST" | "PUT";
  path: string;
  body?: unknown;
}

const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]", "::1"]);

/**
 * Normalize a loopback-only HTTP(S) API base to its origin.
 * Returns `null` for anything else so a hostile `?api=` cannot redirect the
 * UI at a remote host.
 */
export function loopbackApiBase(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  if (trimmed === "") {
    return null;
  }
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return null;
  }
  if (!LOOPBACK_HOSTS.has(url.hostname.toLowerCase())) {
    return null;
  }
  return url.origin;
}

function headlessApiBase(): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  let queryApi: string | null = null;
  try {
    queryApi = new URLSearchParams(window.location?.search ?? "").get("api");
  } catch {
    // A broken location should not prevent the injected API base from working.
  }
  return (
    loopbackApiBase(queryApi) ?? loopbackApiBase(window.__FFT_HEADLESS_API__)
  );
}

export function isHeadlessRuntime(): boolean {
  return (
    typeof window !== "undefined" &&
    (window.__FFT_HEADLESS__ === true || headlessApiBase() !== null)
  );
}

/** Map the shared command names to the headless HTTP API. */
export function requestFor(command: Command): HttpRequest {
  const args = command.args as unknown as Record<string, unknown>;
  switch (command.name) {
    case "app.info":
      return { method: "GET", path: "/api/app/info" };
    case "app.ping":
      return { method: "POST", path: "/api/app/ping", body: args };
    case "settings.get":
      return { method: "GET", path: "/api/settings" };
    case "settings.save":
      return { method: "PUT", path: "/api/settings", body: args.settings };
    case "settings.setSpectrogramDefaults":
      return {
        method: "POST",
        path: "/api/settings/defaults",
        body: args,
      };
    case "config.paths":
      return { method: "GET", path: "/api/config/paths" };
    case "openPath":
      return { method: "POST", path: "/api/open-path", body: args };
    case "theme.get": {
      const name =
        typeof args.name === "string"
          ? `?name=${encodeURIComponent(args.name)}`
          : "";
      return { method: "GET", path: `/api/theme${name}` };
    }
    case "theme.list":
      return { method: "GET", path: "/api/themes" };
    case "theme.duplicate":
      return {
        method: "POST",
        path: "/api/themes/duplicate",
        body: args,
      };
    case "session.get":
      return { method: "GET", path: "/api/session" };
    case "session.save":
      return { method: "PUT", path: "/api/session", body: args.session };
    case "spectrogram.open":
      return {
        method: "POST",
        path: "/api/spectrogram/open",
        body: args,
      };
    case "spectrogram.get":
      return {
        method: "GET",
        path: `/api/spectrogram/${args.displayId}`,
      };
    case "spectrogram.exportImage":
      return {
        method: "GET",
        path: `/api/spectrogram/${args.displayId}/image`,
      };
    case "image.savePng":
      throw new Error("image.savePng is handled by the browser download");
    case "display.list":
      return { method: "GET", path: "/api/displays" };
    case "display.get":
      return { method: "GET", path: `/api/displays/${args.displayId}` };
    case "display.close":
      // The Rust route takes `{id}`; the shared command stays `displayId`.
      return {
        method: "POST",
        path: "/api/displays/close",
        body: { id: args.displayId },
      };
    case "display.showSpectrogram":
      return {
        method: "POST",
        path: "/api/displays/spectrogram",
        body: {
          analysisId: args.analysisId,
          ...(args.ephemeral === undefined
            ? {}
            : { ephemeral: args.ephemeral }),
        },
      };
    case "analysis.cancel":
      return {
        method: "POST",
        path: "/api/analysis/cancel",
        body: args,
      };
    case "orderCuts.compute":
      return { method: "POST", path: "/api/cuts/order", body: args };
    case "lineCuts.compute":
      return { method: "POST", path: "/api/cuts/line", body: args };
    case "cuts.exportCsv":
      return { method: "POST", path: "/api/cuts/csv", body: args };
    case "plot.load":
      return { method: "POST", path: "/api/plot", body: args };
    case "dialog.openAudio":
      throw new Error("dialog.openAudio is handled by the browser picker");
    case "recording.save":
      throw new Error("recording.save is handled by the upload path");
    case "script.pickFile":
    case "script.readFile":
    case "script.writeFile":
    case "script.listExamples":
    case "script.createExample":
    case "script.run":
    case "script.stop":
    case "script.stat":
      throw new Error(
        `${command.name} is only available in the desktop app; the headless WebUI does not execute, read, or write scripts`,
      );
  }
}

async function jsonOrText(response: Response): Promise<unknown> {
  if (response.status === 204) {
    return undefined;
  }
  const type = response.headers.get("content-type") ?? "";
  if (type.includes("application/json")) {
    return response.json();
  }
  return response.text();
}

/** Include the HTTP status and server error text in browser-side failures. */
async function responseError(response: Response): Promise<string> {
  const status = `${response.status}${
    response.statusText ? ` (${response.statusText})` : ""
  }`;
  try {
    const text = await response.text();
    if (text) {
      try {
        const body = JSON.parse(text) as { error?: unknown };
        if (typeof body.error === "string" && body.error) {
          return `${status}: ${body.error}`;
        }
      } catch {
        // Not JSON; fall through to the raw body.
      }
      return `${status}: ${text.slice(0, 300)}`;
    }
  } catch {
    // The body could not be read; the status is still useful.
  }
  return status;
}

async function uploadAudio(baseUrl: string, fetchImpl: typeof fetch) {
  return new Promise<{ path: string } | null>((resolve, reject) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = UPLOAD_ACCEPT;
    let settled = false;
    const finish = (value: { path: string } | null) => {
      if (settled) {
        return;
      }
      settled = true;
      resolve(value);
    };
    const fail = (reason: unknown) => {
      if (settled) {
        return;
      }
      settled = true;
      reject(reason);
    };
    // Browser file pickers fire `cancel` when the user dismisses the dialog.
    // Without this, `dialog.openAudio` would stay pending forever.
    input.addEventListener("cancel", () => finish(null));
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) {
        finish(null);
        return;
      }
      try {
        const form = new FormData();
        form.append("file", file);
        const response = await fetchImpl(`${baseUrl}/api/upload`, {
          method: "POST",
          body: form,
        });
        if (!response.ok) {
          fail(new Error(`upload failed: ${await responseError(response)}`));
          return;
        }
        const body = (await response.json()) as { path: string };
        finish({ path: body.path });
      } catch (error) {
        fail(error);
      }
    };
    try {
      input.click();
    } catch (error) {
      fail(error);
    }
  });
}

/** Transport for the headless WebUI (HTTP commands + WS events). */
export function createHttpTransport(
  options: HttpTransportOptions = {},
): Transport {
  const baseUrl = (
    options.baseUrl ??
    headlessApiBase() ??
    window.location.origin
  ).replace(/\/$/, "");
  const fetchImpl = options.fetchImpl ?? fetch;
  const WebSocketImpl = options.WebSocketImpl ?? WebSocket;
  let socket: WebSocket | null = null;
  let reconnectTimer: number | undefined;
  const listeners = new Map<EventName, Set<(payload: unknown) => void>>();

  const dispatch = (message: MessageEvent) => {
    try {
      const parsed = JSON.parse(String(message.data)) as {
        type?: string;
        [key: string]: unknown;
      };
      if (!parsed.type) {
        return;
      }
      const event = parsed.type as EventName;
      const set = listeners.get(event);
      if (!set) {
        return;
      }
      const payload = { ...parsed };
      delete payload.type;
      for (const listener of [...set]) {
        listener(payload);
      }
    } catch {
      // Ignore malformed frames; the server only sends JSON.
    }
  };

  const ensureSocket = () => {
    if (socket || listeners.size === 0) {
      return;
    }
    const wsUrl = baseUrl.replace(/^http/, "ws") + "/ws";
    socket = new WebSocketImpl(wsUrl);
    socket.onmessage = dispatch;
    socket.onclose = () => {
      socket = null;
      if (listeners.size > 0) {
        reconnectTimer = window.setTimeout(ensureSocket, 1000);
      }
    };
  };

  const download = async (
    request: HttpRequest,
    filename: string,
  ): Promise<{ path: string }> => {
    const response = await fetchImpl(`${baseUrl}${request.path}`, {
      method: request.method,
    });
    if (!response.ok) {
      throw new Error(`${request.path}: ${response.status}`);
    }
    triggerDownload(await response.blob(), filename);
    return { path: filename };
  };

  return {
    mode: "headless",
    async invoke<K extends CommandName>(
      command: Command<K>,
    ): Promise<CommandMap[K]["result"]> {
      if (command.name === "dialog.openAudio") {
        return (await uploadAudio(baseUrl, fetchImpl)) as CommandMap[K]["result"];
      }
      if (command.name === "spectrogram.exportImage") {
        return (await download(
          {
            method: "GET",
            path: `/api/spectrogram/${
              (command.args as { displayId: number }).displayId
            }/image`,
          },
          "spectrogram.png",
        )) as CommandMap[K]["result"];
      }
      if (command.name === "recording.save") {
        const { dataBase64, filename } = command.args as {
          dataBase64: string;
          filename: string;
        };
        const binary = atob(dataBase64);
        const bytes = new Uint8Array(binary.length);
        for (let index = 0; index < binary.length; index += 1) {
          bytes[index] = binary.charCodeAt(index);
        }
        const form = new FormData();
        form.append("file", new Blob([bytes]), filename);
        const response = await fetchImpl(`${baseUrl}/api/upload`, {
          method: "POST",
          body: form,
        });
        if (!response.ok) {
          throw new Error(
            `recording upload failed: ${await responseError(response)}`,
          );
        }
        const saved = (await response.json()) as { path: string };
        return { path: saved.path } as CommandMap[K]["result"];
      }
      if (command.name === "image.savePng") {
        const { dataBase64, filename } = command.args as {
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
      if (command.name === "cuts.exportCsv") {
        return (await download(
          { method: "POST", path: "/api/cuts/csv" },
          "cuts.csv",
        )) as CommandMap[K]["result"];
      }
      const request = requestFor(command);
      const response = await fetchImpl(`${baseUrl}${request.path}`, {
        method: request.method,
        headers:
          request.body === undefined
            ? undefined
            : { "content-type": "application/json" },
        body:
          request.body === undefined ? undefined : JSON.stringify(request.body),
      });
      if (!response.ok) {
        let detail = `${response.status}`;
        try {
          const body = (await response.json()) as { error?: string };
          detail = body.error ?? detail;
        } catch {
          // keep the status
        }
        throw new Error(`${command.name}: ${detail}`);
      }
      return (await jsonOrText(response)) as CommandMap[K]["result"];
    },
    on<K extends EventName>(
      event: K,
      listener: (payload: EventMap[K]) => void,
    ): Unsubscribe {
      let set = listeners.get(event);
      if (!set) {
        set = new Set();
        listeners.set(event, set);
      }
      set.add(listener as (payload: unknown) => void);
      ensureSocket();
      return () => {
        set?.delete(listener as (payload: unknown) => void);
        if (listeners.size === 0) {
          if (reconnectTimer !== undefined) {
            window.clearTimeout(reconnectTimer);
          }
          socket?.close();
          socket = null;
        }
      };
    },
    close() {
      if (reconnectTimer !== undefined) {
        window.clearTimeout(reconnectTimer);
      }
      socket?.close();
      socket = null;
      listeners.clear();
    },
  };
}

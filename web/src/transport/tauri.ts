import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type {
  Command,
  CommandMap,
  CommandName,
  EventMap,
  EventName,
  Transport,
  Unsubscribe,
} from "./types";

/**
 * The shared contract uses dotted names for readability. Tauri command names
 * must be Rust identifiers and event names may not contain dots.
 */
export const COMMAND_NAMES = {
  "app.info": "app_info",
  "app.ping": "app_ping",
  "spectrogram.open": "spectrogram_open",
  "spectrogram.get": "spectrogram_get",
  "spectrogram.exportImage": "spectrogram_export_image",
  "image.savePng": "image_save_png",
  "recording.save": "recording_save",
  "display.list": "display_list",
  "display.get": "display_get",
  "display.close": "display_close",
  "display.showSpectrogram": "display_show_spectrogram",
  "orderCuts.compute": "order_cuts_compute",
  "lineCuts.compute": "line_cuts_compute",
  "cuts.exportCsv": "cuts_export_csv",
  "plot.load": "plot_load",
  "analysis.cancel": "analysis_cancel",
  "theme.get": "theme_get",
  "theme.list": "theme_list",
  "session.get": "session_get",
  "session.save": "session_save",
  "dialog.openAudio": "dialog_open_audio",
  "settings.get": "settings_get",
  "settings.save": "settings_save",
  "settings.setSpectrogramDefaults": "settings_set_spectrogram_defaults",
  "config.paths": "config_paths",
  openPath: "open_path",
  "theme.duplicate": "theme_duplicate",
  "script.run": "script_run",
  "script.stop": "script_stop",
  "script.stat": "script_stat",
  "script.readFile": "script_read_file",
  "script.writeFile": "script_write_file",
  "script.listExamples": "script_list_examples",
  "script.pickFile": "script_pick_file",
  "script.createExample": "script_create_example",
} satisfies Record<CommandName, string>;

export const EVENT_NAMES = {
  "analysis.progress": "analysis-progress",
  "analysis.done": "analysis-done",
  "analysis.error": "analysis-error",
  "display.open": "display-open",
  "display.updated": "display-updated",
  "display.closed": "display-closed",
  "script.output": "script-output",
  "script.exit": "script-exit",
  notification: "notification",
} satisfies Record<EventName, string>;

export type TauriInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export type TauriListen = <T>(
  event: string,
  handler: (event: { payload: T }) => void,
) => Promise<() => void>;

export interface TauriDeps {
  invoke: TauriInvoke;
  listen: TauriListen;
}

export const defaultTauriDeps: TauriDeps = {
  invoke: (command, args) => tauriInvoke(command, args),
  listen: (event, handler) => tauriListen(event, handler),
};

/** True inside the Tauri webview (desktop); false in a plain browser/test. */
export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
  );
}

/** Transport backed by Tauri 2 IPC and plugin events. */
export function createTauriTransport(
  deps: TauriDeps = defaultTauriDeps,
): Transport {
  return {
    mode: "desktop",
    async invoke<K extends CommandName>(
      command: Command<K>,
    ): Promise<CommandMap[K]["result"]> {
      return deps.invoke<CommandMap[K]["result"]>(
        COMMAND_NAMES[command.name],
        command.args,
      );
    },
    on<K extends EventName>(
      event: K,
      listener: (payload: EventMap[K]) => void,
    ): Unsubscribe {
      let unlisten: (() => void) | undefined;
      let closed = false;

      void deps
        .listen<EventMap[K]>(EVENT_NAMES[event], (message) => {
          listener(message.payload);
        })
        .then(
          (stop) => {
            if (closed) {
              stop();
            } else {
              unlisten = stop;
            }
          },
          (error: unknown) => {
            console.error(`could not listen for ${event}`, error);
          },
        );

      return () => {
        closed = true;
        unlisten?.();
      };
    },
    close() {
      // Tauri unlistens per subscription; the webview lifetime owns the rest.
    },
  };
}

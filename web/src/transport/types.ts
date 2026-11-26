import type { CutsPayload, Cut, LineSpec } from "../cuts/model";
import type {
  LineCutSettings,
  OrderCutSettings,
  RpmProfileSpec,
} from "../cuts/settings";
import type { PlotLayout, TablePlot } from "../plot/types";
import type {
  AnalysisSettings,
  DisplaySettings,
  Settings,
} from "../settings/model";
import type { SessionDocument } from "../shell/session";
import type { SpectrogramPayload } from "../spectrogram/types";
import type { ResolvedTheme, ThemeEntry } from "../theme/types";

export type AppMode = "desktop" | "headless" | "browser" | "mock";

export type ScriptLanguage = "python" | "lua";

export interface AppCapabilities {
  displays: boolean;
  /** Scripting *UI* offered by this build; null means no editor/console. */
  scripting: ScriptLanguage | null;
  /** Stable table format families: csv, parquet, json, jsonl, arrow. */
  tableFormats: string[];
}

export interface AppInfo {
  name: string;
  version: string;
  mode: Exclude<AppMode, "mock">;
  capabilities: AppCapabilities;
}

/** Registry metadata shared by HTTP `/api/displays` and Tauri display commands. */
export interface DisplayInfo {
  id: number;
  parent: number | null;
  kind: "spectrogram" | "order_cuts" | "line_cuts";
  name: string;
  state: Record<string, unknown>;
  visible: boolean;
  ephemeral: boolean;
}

/** One Python example embedded in the desktop binary. */
export interface ScriptExampleInfo {
  /** Flattened file name copied into the config directory's examples folder. */
  filename: string;
}

export type CommandMap = {
  "app.info": { args: Record<string, never>; result: AppInfo };
  "app.ping": { args: { nonce?: string }; result: { nonce: string | null; at: number } };
  "spectrogram.open": {
    args: { displayId: number; path: string; settings?: AnalysisSettings };
    /// `analysisId` is the runtime analysis created by the open request.
    result: { displayId: number; analysisId?: number };
  };
  "display.list": { args: Record<string, never>; result: DisplayInfo[] };
  "display.get": { args: { displayId: number }; result: DisplayInfo };
  "display.close": { args: { displayId: number }; result: void };
  "display.showSpectrogram": {
    args: { analysisId: number; ephemeral?: boolean };
    result: DisplayInfo;
  };
  "spectrogram.get": {
    args: { displayId: number };
    result: SpectrogramPayload;
  };
  "spectrogram.exportImage": {
    args: { displayId: number };
    result: { path: string } | null;
  };
  "image.savePng": {
    args: { dataBase64: string; filename: string };
    result: { path: string } | null;
  };
  "recording.save": {
    args: { dataBase64: string; filename: string };
    result: { path: string };
  };
  "analysis.cancel": { args: { displayId: number }; result: { cancelled: boolean } };
  "orderCuts.compute": {
    args: { parentId: number; settings: OrderCutSettings; rpm?: RpmProfileSpec };
    result: CutsPayload;
  };
  "lineCuts.compute": {
    args: { parentId: number; settings: LineCutSettings; lines: LineSpec[] };
    result: CutsPayload;
  };
  "cuts.exportCsv": {
    args: { cuts: Cut[] };
    result: { path: string } | null;
  };
  "plot.load": {
    args: { path: string; layout: PlotLayout };
    result: TablePlot;
  };
  "theme.get": { args: { name?: string }; result: ResolvedTheme };
  "theme.list": { args: Record<string, never>; result: ThemeEntry[] };
  "session.get": { args: Record<string, never>; result: SessionDocument | null };
  "session.save": { args: { session: SessionDocument }; result: void };
  "dialog.openAudio": { args: Record<string, never>; result: { path: string } | null };
  "settings.get": { args: Record<string, never>; result: Settings };
  "settings.save": { args: { settings: Settings }; result: void };
  "settings.setSpectrogramDefaults": {
    args: { analysis: AnalysisSettings; display: DisplaySettings };
    result: void;
  };
  "config.paths": {
    args: Record<string, never>;
    result: {
      configDir: string;
      settingsPath: string;
      sessionPath: string;
      themesDir: string;
    };
  };
  openPath: { args: { path: string }; result: void };
  "theme.duplicate": {
    args: { source: string; name: string };
    result: string;
  };
  "script.run": {
    args: { displayId: number; path: string };
    result: void;
  };
  "script.stop": { args: { displayId: number }; result: void };
  "script.stat": {
    args: { path: string };
    result: { exists: boolean; mtimeMs: number | null };
  };
  "script.readFile": { args: { path: string }; result: string };
  "script.writeFile": {
    args: { path: string; contents: string };
    result: void;
  };
  "script.pickFile": {
    args: Record<string, never>;
    result: { path: string } | null;
  };
  "script.listExamples": {
    args: Record<string, never>;
    result: ScriptExampleInfo[];
  };
  "script.createExample": {
    args: { filename: string };
    result: { path: string };
  };
};

export type EventMap = {
  "analysis.progress": { displayId: number; framesDone: number; framesTotal: number | null; etaSeconds: number | null };
  "analysis.done": { displayId: number };
  "analysis.error": { displayId: number; message: string };
  "display.open": { display: DisplayInfo };
  "display.updated": { display: DisplayInfo };
  "display.closed": { id: number; children: number[] };
  "script.output": {
    displayId: number;
    stream: "stdout" | "stderr" | "system";
    line: string;
  };
  "script.exit": {
    displayId: number;
    code: number | null;
    success: boolean;
  };
  "notification": { level: "info" | "success" | "warn" | "error"; message: string };
};

export type CommandName = keyof CommandMap;
export type Command<K extends CommandName = CommandName> = { name: K; args: CommandMap[K]["args"] };
export type EventName = keyof EventMap;
export type Unsubscribe = () => void;

export interface Transport {
  readonly mode: AppMode;
  invoke<K extends CommandName>(command: Command<K>): Promise<CommandMap[K]["result"]>;
  on<K extends EventName>(event: K, listener: (payload: EventMap[K]) => void): Unsubscribe;
  close?(): void;
}

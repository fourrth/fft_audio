/** Launcher state rules: recent files, tips and root-only tiles. */

export const MAX_RECENT_FILES = 8;

export interface LauncherActions {
  recentFiles: string[];
  /** Whether this build offers the Scripting editor/console. */
  scriptingEnabled: boolean;
  /** Replace this launcher tab with a new Spectrogram. */
  openSpectrogram: (panelId: string) => void;
  /** Open a Spectrogram in a new split beside this launcher. */
  openSpectrogramInSplit: (panelId: string) => void;
  /** Replace this launcher tab with the Recorder display. */
  openRecorder: (panelId: string) => void;
  /** Replace this launcher tab with the Scripting display. */
  openScripting: (panelId: string) => void;
  /** Replace this launcher tab with a Spectrogram for `path`. */
  openFile: (panelId: string, path: string) => void;
}

export interface LauncherTile {
  id: "spectrogram" | "recorder" | "scripting";
  title: string;
  description: string;
  icon: string;
  kind: "spec" | "recorder" | "launcher";
  root: boolean;
  soon?: boolean;
}

/**
 * Root displays only. Order Cuts and Line Cuts are children of a Spectrogram
 * and must never appear here.
 */
export const LAUNCHER_TILES: LauncherTile[] = [
  {
    id: "spectrogram",
    title: "Spectrogram",
    description: "Analyse audio or a data table and inspect the spectrum.",
    icon: "▤",
    kind: "spec",
    root: true,
  },
  {
    id: "recorder",
    title: "Recorder",
    description: "Record the microphone, then analyse the take.",
    icon: "●",
    kind: "recorder",
    root: true,
  },
  {
    id: "scripting",
    title: "Scripting",
    description: "Edit and run local Python in-process against the display API.",
    icon: "⌘",
    kind: "launcher",
    root: true,
  },
];

/** Root tiles offered by this build. Never mutate `LAUNCHER_TILES` in place. */
export function launcherTiles(scriptingEnabled: boolean): LauncherTile[] {
  return scriptingEnabled
    ? LAUNCHER_TILES
    : LAUNCHER_TILES.filter((tile) => tile.id !== "scripting");
}

export function isSpawnableTile(tile: LauncherTile): boolean {
  return tile.root && !tile.soon;
}

export const USAGE_TIPS = [
  "Press F to fit the spectrogram, and drag the color bar handles to tune the dB range.",
  "Drag a tab onto another pane edge to split the dock; double-click a title to rename it.",
  "Ctrl+K opens the command palette from anywhere in the app.",
  "Order Cuts and Line Cuts are children: use the + tab action on a Spectrogram.",
  "Drop a file anywhere on the window to open it in a new Spectrogram.",
] as const;

export function tipAt(index: number): string {
  const count = USAGE_TIPS.length;
  return USAGE_TIPS[((index % count) + count) % count] ?? USAGE_TIPS[0];
}

export function nextTipIndex(index: number): number {
  return (index + 1) % USAGE_TIPS.length;
}

/** Trim, drop empties, keep the newest occurrence first, cap at 8. */
export function normalizeRecentFiles(values: unknown): string[] {
  if (!Array.isArray(values)) {
    return [];
  }
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (typeof value !== "string") {
      continue;
    }
    const path = value.trim();
    if (path.length === 0 || seen.has(path)) {
      continue;
    }
    seen.add(path);
    result.push(path);
    if (result.length === MAX_RECENT_FILES) {
      break;
    }
  }
  return result;
}

export function addRecentFile(current: readonly string[], path: string): string[] {
  return normalizeRecentFiles([path, ...current]);
}

export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function folderOf(path: string): string {
  const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return cut > 0 ? path.slice(0, cut) : "";
}

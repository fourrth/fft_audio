/**
 * `session.json` document shape shared with the Rust core.
 *
 * `layout` is Dockview's own serialized layout, stored opaquely: no custom
 * dock tree is persisted here.
 */
import { normalizeRecentFiles } from "../launcher/recent";

export const SESSION_VERSION = 1;

export type DisplayKind =
  | "launcher"
  | "spectrogram"
  | "order_cuts"
  | "line_cuts"
  | "plot"
  | "recorder"
  | "scripting"
  | "script_console";

export interface ShellDisplay {
  id: number;
  parent: number | null;
  kind: DisplayKind;
  name: string;
  state: Record<string, unknown>;
}

export interface SessionDocument {
  version: number;
  nextId: number;
  displays: ShellDisplay[];
  recentFiles: string[];
  layoutPresets: Record<string, unknown>;
  layout: unknown;
}

export function emptySession(): SessionDocument {
  return {
    version: SESSION_VERSION,
    nextId: 1,
    displays: [],
    recentFiles: [],
    layoutPresets: {},
    layout: null,
  };
}

export function createSession(
  layout: unknown,
  displays: ShellDisplay[],
  nextId: number,
  recentFiles: readonly string[] = [],
  layoutPresets: Record<string, unknown> = {},
): SessionDocument {
  return {
    version: SESSION_VERSION,
    nextId: Math.max(1, Math.trunc(nextId)),
    displays,
    recentFiles: normalizeRecentFiles(recentFiles),
    layoutPresets: { ...layoutPresets },
    layout,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isShellDisplay(value: unknown): value is ShellDisplay {
  return (
    isRecord(value) &&
    typeof value.id === "number" &&
    (value.parent === null || typeof value.parent === "number") &&
    typeof value.kind === "string" &&
    typeof value.name === "string" &&
    isRecord(value.state)
  );
}

/** Accept a saved document only when its display metadata is well formed. */
export function parseSession(value: unknown): SessionDocument | null {
  if (!isRecord(value) || !Array.isArray(value.displays)) {
    return null;
  }
  if (
    typeof value.version !== "number" ||
    typeof value.nextId !== "number" ||
    !value.displays.every(isShellDisplay)
  ) {
    return null;
  }
  return {
    version: value.version,
    nextId: Math.max(1, value.nextId),
    displays: value.displays,
    recentFiles: normalizeRecentFiles(value.recentFiles),
    layoutPresets: isRecord(value.layoutPresets)
      ? { ...value.layoutPresets }
      : {},
    layout: value.layout ?? null,
  };
}

/** Pull the display metadata Dockview stores in panel params. */
export function displayFromPanel(
  panel: { title?: string; params?: unknown },
  fallbackId: number,
): ShellDisplay {
  const params = isRecord(panel.params) ? panel.params : {};
  const display = params.display;
  if (isShellDisplay(display)) {
    return display;
  }
  return {
    id: fallbackId,
    parent: null,
    kind: "spectrogram",
    name: panel.title ?? `Display ${fallbackId}`,
    state: {},
  };
}

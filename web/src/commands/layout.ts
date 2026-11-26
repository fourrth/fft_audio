import type { ShellDisplay } from "../shell/session";

export type LayoutPresets = Record<string, unknown>;

export const BUILTIN_PRESETS = ["Analysis", "Cuts side by side", "Focus"] as const;

export const UNDO_TTL_MS = 8000;

export function savePreset(
  presets: LayoutPresets,
  rawName: string,
  layout: unknown,
): LayoutPresets {
  const name = rawName.trim();
  if (name.length === 0) {
    return { ...presets };
  }
  return { ...presets, [name]: layout };
}

export function removePreset(presets: LayoutPresets, name: string): LayoutPresets {
  const next = { ...presets };
  delete next[name];
  return next;
}

export function presetNames(presets: LayoutPresets): string[] {
  return Object.keys(presets).sort((a, b) => a.localeCompare(b));
}

export function uniquePresetName(
  presets: LayoutPresets,
  rawName: string,
): string {
  const base = rawName.trim() || "Preset";
  let name = base;
  let index = 2;
  while (name in presets) {
    name = `${base} ${index}`;
    index += 1;
  }
  return name;
}

export interface ClosePlan {
  parentId: number;
  childIds: number[];
  allIds: number[];
  childCount: number;
}

export function planClose(
  parent: ShellDisplay,
  displays: readonly ShellDisplay[],
): ClosePlan {
  const children = displays.filter((display) => display.parent === parent.id);
  return {
    parentId: parent.id,
    childIds: children.map((child) => child.id),
    allIds: [parent.id, ...children.map((child) => child.id)],
    childCount: children.length,
  };
}

export interface UndoSnapshot {
  layout: unknown;
  displayIds: number[];
  createdAt: number;
}

export function undoAvailable(
  snapshot: UndoSnapshot | null,
  now: number,
): boolean {
  return Boolean(snapshot && now - snapshot.createdAt < UNDO_TTL_MS);
}

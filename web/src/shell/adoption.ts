/**
 * Pure display-adoption helpers shared by the shell and tests.
 *
 * The backend registry is authoritative for backend/script displays while the
 * React session stays authoritative for user ids and panel metadata. These
 * helpers keep the two id spaces apart and make adoption idempotent.
 */
import type { DisplayInfo } from "../transport/types";
import type { ShellDisplay } from "./session";

/** First id reserved for backend/script displays. */
export const SCRIPTED_DISPLAY_ID_BASE = 1_000_000;

/** Highest id a frontend-created display may use. */
export const MAX_USER_DISPLAY_ID = SCRIPTED_DISPLAY_ID_BASE - 1;

/** Local state that backend updates must not overwrite. */
const PRESERVED_LOCAL_STATE_KEYS = [
  "path",
  "analysisSettings",
  "displaySettings",
  "orderCuts",
  "lineCuts",
  "lines",
  "rpm",
  "showGrid",
  "analysisMeta",
] as const;

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isReservedDisplayId(id: number): boolean {
  return !Number.isSafeInteger(id) || id >= SCRIPTED_DISPLAY_ID_BASE;
}

export function displayPath(display: Pick<ShellDisplay, "state">): string | null {
  const value = display.state.path;
  return typeof value === "string" && value.length > 0 ? value : null;
}

/**
 * Detect a display whose data belongs to the backend (registry, script or an
 * attached analysis) rather than to the local open-by-path flow.
 */
export function isBackendOwnedDisplay(
  display: Pick<ShellDisplay, "id" | "state">,
): boolean {
  return (
    isReservedDisplayId(display.id) ||
    display.state.sourceKind === "backend" ||
    display.state.scripted === true
  );
}

/** Does this display need the attach path (no `spectrogram.open`)? */
export function shouldAttachAnalysis(
  display: Pick<ShellDisplay, "id" | "state">,
): boolean {
  if (isBackendOwnedDisplay(display)) {
    return true;
  }
  return (
    typeof display.state.analysisId === "number" &&
    displayPath(display) === null
  );
}

function convertedState(info: DisplayInfo): Record<string, unknown> {
  const state: Record<string, unknown> = { ...info.state };
  if (info.ephemeral) {
    state.ephemeral = true;
  } else {
    delete state.ephemeral;
  }
  const settings = state.settings;
  if (isRecord(settings)) {
    if (info.kind === "spectrogram" && !isRecord(state.displaySettings)) {
      state.displaySettings = settings;
    } else if (info.kind === "order_cuts" && !isRecord(state.orderCuts)) {
      state.orderCuts = settings;
    } else if (info.kind === "line_cuts" && !isRecord(state.lineCuts)) {
      state.lineCuts = settings;
    }
  }
  return state;
}

/** Map registry metadata to the Dockview panel display shape. */
export function displayFromInfo(info: DisplayInfo): ShellDisplay {
  return {
    id: info.id,
    parent: info.parent,
    kind: info.kind,
    name: info.name,
    state: convertedState(info),
  };
}

/**
 * Merge an `display.open`/`display.updated` payload into an existing panel.
 * Backend state wins, except for local path/settings/line edits.
 */
export function mergeDisplayInfo(
  existing: ShellDisplay,
  info: DisplayInfo,
): ShellDisplay {
  const state: Record<string, unknown> = {
    ...existing.state,
    ...info.state,
  };
  for (const key of PRESERVED_LOCAL_STATE_KEYS) {
    if (key in existing.state) {
      state[key] = existing.state[key];
    }
  }
  if (info.ephemeral) {
    state.ephemeral = true;
  } else {
    delete state.ephemeral;
  }
  // A local display may not have a display-specific copy yet; adopt the
  // registry's settings without overwriting later local edits.
  const settings = state.settings;
  if (isRecord(settings)) {
    if (info.kind === "spectrogram" && !isRecord(existing.state.displaySettings)) {
      state.displaySettings = settings;
    } else if (info.kind === "order_cuts" && !isRecord(existing.state.orderCuts)) {
      state.orderCuts = settings;
    } else if (info.kind === "line_cuts" && !isRecord(existing.state.lineCuts)) {
      state.lineCuts = settings;
    }
  }
  return {
    id: info.id,
    parent: info.parent,
    kind: info.kind,
    name: info.name,
    state,
  };
}

export interface DisplayUpsert {
  panelId: string;
  display: ShellDisplay;
  isNew: boolean;
}

/** Decide whether a display event adds a panel or updates the existing one. */
export function upsertDisplay(
  existing: ShellDisplay | undefined,
  info: DisplayInfo,
): DisplayUpsert {
  return existing
    ? {
        panelId: panelIdForDisplay(info.id),
        display: mergeDisplayInfo(existing, info),
        isNew: false,
      }
    : {
        panelId: panelIdForDisplay(info.id),
        display: displayFromInfo(info),
        isNew: true,
      };
}

export function panelIdForDisplay(id: number): string {
  return `panel-${id}`;
}

/**
 * Parent-before-child order for adopting a registry snapshot at startup.
 * Dockview can only place a child under an already-added parent, so incoming
 * child-before-parent lists are normalized here. Stable by ascending id.
 */
export function displayAdoptionOrder(
  displays: readonly DisplayInfo[],
): DisplayInfo[] {
  const byId = new Map(displays.map((display) => [display.id, display]));
  const depth = (display: DisplayInfo): number => {
    let current = display;
    let levels = 0;
    const seen = new Set<number>();
    while (current.parent !== null && !seen.has(current.parent)) {
      seen.add(current.parent);
      const parent = byId.get(current.parent);
      if (!parent) {
        break;
      }
      levels += 1;
      current = parent;
    }
    return levels;
  };
  return [...displays].sort(
    (left, right) => depth(left) - depth(right) || left.id - right.id,
  );
}

/** Root + descendants removed by one `display.closed` event. */
export function closedDisplayIds(id: number, children: readonly number[]): number[] {
  return [...new Set([id, ...children])];
}

/**
 * Close-event ids expanded through local parent chains. The backend lists only
 * its own registry children; locally-spawned cut panels may sit under a closed
 * backend display without appearing in `children`, so they are removed too.
 */
export function closedDisplayTreeIds(
  id: number,
  children: readonly number[],
  displays: readonly Pick<ShellDisplay, "id" | "parent">[],
): number[] {
  const removed = new Set<number>(closedDisplayIds(id, children));
  let changed = true;
  while (changed) {
    changed = false;
    for (const display of displays) {
      if (
        !removed.has(display.id) &&
        display.parent !== null &&
        removed.has(display.parent)
      ) {
        removed.add(display.id);
        changed = true;
      }
    }
  }
  return [...removed];
}

export function closedTreePanelIds(
  id: number,
  children: readonly number[],
  displays: readonly Pick<ShellDisplay, "id" | "parent">[],
): string[] {
  return closedDisplayTreeIds(id, children, displays).map(panelIdForDisplay);
}

export function closedPanelIds(id: number, children: readonly number[]): string[] {
  return closedDisplayIds(id, children).map(panelIdForDisplay);
}

export function isEphemeralDisplay(display: Pick<ShellDisplay, "state">): boolean {
  return display.state.ephemeral === true;
}

/** Displays that may be written to `session.json` (ephemeral ones never are). */
export function sessionDisplays(
  displays: readonly ShellDisplay[],
): ShellDisplay[] {
  return displays.filter((display) => !isEphemeralDisplay(display));
}

export function highestUserDisplayId(ids: Iterable<number>): number {
  let highest = 0;
  for (const id of ids) {
    if (Number.isSafeInteger(id) && id > 0 && id < SCRIPTED_DISPLAY_ID_BASE) {
      highest = Math.max(highest, id);
    }
  }
  return highest;
}

/**
 * Clamp a saved/next counter below the reserved range and keep it ahead of
 * every persisted user display. Callers should log when the saved value was
 * already in the reserved range.
 */
export function sanitizeNextId(
  savedNextId: number | undefined,
  displayIds: Iterable<number>,
): number {
  const candidate =
    typeof savedNextId === "number" && Number.isSafeInteger(savedNextId)
      ? Math.max(1, savedNextId)
      : 1;
  const highest = highestUserDisplayId(displayIds);
  return Math.min(
    MAX_USER_DISPLAY_ID,
    Math.max(candidate, highest + 1, 1),
  );
}

/**
 * Pick the next free user display id. If the counter reached the reserved
 * range, reuse the lowest free id below it instead.
 */
export function nextUserDisplayId(
  counter: number,
  usedIds: Iterable<number> = [],
): number | null {
  const used = new Set<number>();
  for (const id of usedIds) {
    if (Number.isSafeInteger(id) && id > 0) {
      used.add(id);
    }
  }
  const start =
    Number.isSafeInteger(counter) &&
    counter >= 1 &&
    counter < SCRIPTED_DISPLAY_ID_BASE
      ? counter
      : 1;
  for (let id = start; id < SCRIPTED_DISPLAY_ID_BASE; id += 1) {
    if (!used.has(id)) {
      return id;
    }
  }
  for (let id = 1; id < start; id += 1) {
    if (!used.has(id)) {
      return id;
    }
  }
  return null;
}

export type RestoreDisposition = "keep" | "drop" | "recreate";

/**
 * Restored backend displays cannot be attached after a restart: their
 * analyses are gone. File-backed spectrograms are recreated as user panels;
 * everything else in the reserved/backend space is dropped.
 */
export function restoreDisposition(
  display: Pick<ShellDisplay, "id" | "kind" | "state">,
): RestoreDisposition {
  if (display.state.ephemeral === true) {
    return "drop";
  }
  const backendIdentity =
    isReservedDisplayId(display.id) ||
    display.state.sourceKind === "backend" ||
    display.state.scripted === true;
  const path =
    typeof display.state.path === "string" && display.state.path.length > 0;
  if (backendIdentity) {
    return display.kind === "spectrogram" && path ? "recreate" : "drop";
  }
  // A user display with an analysis id but no path cannot be restored either;
  // an empty Spectrogram (no analysis, no path) is a valid user panel.
  if (typeof display.state.analysisId === "number" && !path) {
    return "drop";
  }
  return "keep";
}

/**
 * Restore ids that must be removed. A child cut whose parent is dropped is
 * removed with it, even when the child's own metadata looks restorable. A
 * child under a kept/recreated parent stays (and is reparented by the shell
 * when the parent receives a new user id).
 */
export function restoreDropIds(
  displays: readonly Pick<ShellDisplay, "id" | "parent" | "kind" | "state">[],
): Set<number> {
  const dropped = new Set<number>();
  for (const display of displays) {
    if (restoreDisposition(display) === "drop") {
      dropped.add(display.id);
    }
  }
  let changed = true;
  while (changed) {
    changed = false;
    for (const display of displays) {
      if (
        !dropped.has(display.id) &&
        display.parent !== null &&
        dropped.has(display.parent)
      ) {
        dropped.add(display.id);
        changed = true;
      }
    }
  }
  return dropped;
}

/** Convert a restored backend spectrogram into a fresh user spectrogram. */
export function recreatedUserDisplay(
  display: ShellDisplay,
  id: number,
): ShellDisplay {
  const state: Record<string, unknown> = {
    ...display.state,
    sourceKind: "user",
    scripted: false,
  };
  delete state.analysisId;
  delete state.signalId;
  delete state.cutId;
  delete state.ephemeral;
  const settings = state.settings;
  if (isRecord(settings) && !isRecord(state.displaySettings)) {
    state.displaySettings = settings;
  }
  return {
    id,
    parent: null,
    kind: "spectrogram",
    name: display.name,
    state,
  };
}

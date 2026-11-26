import { useCallback, useSyncExternalStore } from "react";

export type ScriptConsoleStream = "stdout" | "stderr" | "system";

export interface ScriptConsoleLine {
  stream: ScriptConsoleStream;
  text: string;
  at: string;
}

export interface ScriptConsoleExit {
  code: number | null;
  success: boolean;
}

export interface ScriptConsoleSnapshot {
  lines: readonly ScriptConsoleLine[];
  running: boolean;
  exit: ScriptConsoleExit | null;
}

interface ScriptConsoleEntry {
  lines: readonly ScriptConsoleLine[];
  running: boolean;
  exit: ScriptConsoleExit | null;
  snapshot: ScriptConsoleSnapshot;
  listeners: Set<() => void>;
}

const MAX_LINES = 500;

const EMPTY_LINES: readonly ScriptConsoleLine[] = [];
const EMPTY_SNAPSHOT: ScriptConsoleSnapshot = {
  lines: EMPTY_LINES,
  running: false,
  exit: null,
};

const entries = new Map<number, ScriptConsoleEntry>();

function timestamp(): string {
  return new Date().toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function freshEntry(): ScriptConsoleEntry {
  return {
    lines: EMPTY_LINES,
    running: false,
    exit: null,
    snapshot: EMPTY_SNAPSHOT,
    listeners: new Set(),
  };
}

function getEntry(id: number): ScriptConsoleEntry {
  let entry = entries.get(id);
  if (!entry) {
    entry = freshEntry();
    entries.set(id, entry);
  }
  return entry;
}

function notify(entry: ScriptConsoleEntry): void {
  entry.snapshot = {
    lines: entry.lines,
    running: entry.running,
    exit: entry.exit,
  };
  for (const listener of entry.listeners) {
    listener();
  }
}

export function appendScriptConsoleLine(
  id: number,
  line: { stream: ScriptConsoleStream; text: string; at?: string },
): void {
  const entry = getEntry(id);
  const next = [...entry.lines, { ...line, at: line.at ?? timestamp() }];
  entry.lines = next.slice(-MAX_LINES);
  notify(entry);
}

export function setScriptConsoleRunning(id: number, running: boolean): void {
  const entry = entries.get(id);
  if (!entry) {
    if (!running) {
      return;
    }
    const created = getEntry(id);
    created.running = true;
    notify(created);
    return;
  }
  if (entry.running === running) {
    return;
  }
  entry.running = running;
  notify(entry);
}

export function setScriptConsoleExit(
  id: number,
  exit: ScriptConsoleExit,
): void {
  const entry = getEntry(id);
  if (
    entry.exit?.code === exit.code &&
    entry.exit.success === exit.success
  ) {
    return;
  }
  entry.exit = exit;
  notify(entry);
}

export function clearScriptConsole(id: number): void {
  const entry = entries.get(id);
  if (!entry || entry.lines.length === 0) {
    return;
  }
  entry.lines = EMPTY_LINES;
  notify(entry);
}

function subscribe(id: number, listener: () => void): () => void {
  const entry = getEntry(id);
  entry.listeners.add(listener);
  return () => {
    entry.listeners.delete(listener);
  };
}

function snapshotFor(id: number): ScriptConsoleSnapshot {
  return entries.get(id)?.snapshot ?? EMPTY_SNAPSHOT;
}

export function useScriptConsole(id: number | null): ScriptConsoleSnapshot {
  const subscribeToStore = useCallback(
    (listener: () => void) => (id === null ? () => {} : subscribe(id, listener)),
    [id],
  );
  const getSnapshot = useCallback(
    () => (id === null ? EMPTY_SNAPSHOT : snapshotFor(id)),
    [id],
  );
  return useSyncExternalStore(subscribeToStore, getSnapshot, getSnapshot);
}

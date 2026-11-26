import type { ShellDisplay } from "../shell/session";

export type ChildKind = "order_cuts" | "line_cuts" | "script_console";

export function childLabel(kind: ChildKind): string {
  if (kind === "order_cuts") {
    return "Order Cuts";
  }
  if (kind === "line_cuts") {
    return "Line Cuts";
  }
  return "Console";
}

export function canSpawnChild(
  parentKind: ShellDisplay["kind"],
  childKind: ChildKind,
): boolean {
  if (childKind === "script_console") {
    return parentKind === "scripting";
  }
  return parentKind === "spectrogram";
}

/** Child metadata: always linked to the display that spawned it. */
export function makeChildDisplay(
  parent: ShellDisplay,
  kind: ChildKind,
  id: number,
): ShellDisplay {
  const name = `${parent.name} > ${childLabel(kind)}`;
  return {
    id,
    parent: parent.id,
    kind,
    name,
    // A child of an in-memory backend display is in-memory too; never persist
    // it without a restorable parent.
    state: parent.state.ephemeral === true ? { ephemeral: true } : {},
  };
}

import { describe, expect, it } from "vitest";
import {
  BUILTIN_PRESETS,
  UNDO_TTL_MS,
  planClose,
  presetNames,
  removePreset,
  savePreset,
  undoAvailable,
  uniquePresetName,
} from "./layout";
import type { ShellDisplay } from "../shell/session";

const parent: ShellDisplay = {
  id: 1,
  parent: null,
  kind: "spectrogram",
  name: "Spectrogram 1",
  state: {},
};
const order: ShellDisplay = {
  id: 2,
  parent: 1,
  kind: "order_cuts",
  name: "Spectrogram 1 > Order Cuts",
  state: {},
};
const line: ShellDisplay = {
  id: 3,
  parent: 1,
  kind: "line_cuts",
  name: "Spectrogram 1 > Line Cuts",
  state: {},
};

describe("layout presets and close undo", () => {
  it("saves, lists, overwrites and removes custom presets", () => {
    let presets = savePreset({}, "  My layout  ", { grid: 1 });
    expect(presetNames(presets)).toEqual(["My layout"]);
    presets = savePreset(presets, "Another", { grid: 2 });
    expect(presetNames(presets)).toEqual(["Another", "My layout"]);
    presets = savePreset(presets, "My layout", { grid: 3 });
    expect(presets["My layout"]).toEqual({ grid: 3 });
    presets = removePreset(presets, "Another");
    expect(presetNames(presets)).toEqual(["My layout"]);
    expect(savePreset(presets, "   ", {})).toEqual(presets);
    expect(BUILTIN_PRESETS).toEqual(["Analysis", "Cuts side by side", "Focus"]);
  });

  it("makes duplicate preset names unique", () => {
    const presets = { Preset: {}, "Preset 2": {} };
    expect(uniquePresetName(presets, "Preset")).toBe("Preset 3");
    expect(uniquePresetName({}, "  ")).toBe("Preset");
  });

  it("plans closing a Spectrogram and all children", () => {
    const plan = planClose(parent, [parent, order, line]);
    expect(plan.parentId).toBe(1);
    expect(plan.childIds).toEqual([2, 3]);
    expect(plan.allIds).toEqual([1, 2, 3]);
    expect(plan.childCount).toBe(2);
    expect(planClose(order, [parent, order, line]).childCount).toBe(0);
  });

  it("expires close-undo after eight seconds", () => {
    const snapshot = { layout: {}, displayIds: [1, 2], createdAt: 1000 };
    expect(undoAvailable(snapshot, 1000)).toBe(true);
    expect(undoAvailable(snapshot, 1000 + UNDO_TTL_MS - 1)).toBe(true);
    expect(undoAvailable(snapshot, 1000 + UNDO_TTL_MS)).toBe(false);
    expect(undoAvailable(null, 1000)).toBe(false);
  });
});

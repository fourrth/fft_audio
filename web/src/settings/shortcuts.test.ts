import { describe, expect, it } from "vitest";
import {
  DEFAULT_SHORTCUTS,
  filterShortcuts,
  shortcutsByCategory,
} from "./shortcuts";

describe("keyboard shortcuts", () => {
  it("lists the required defaults", () => {
    const keys = DEFAULT_SHORTCUTS.map((row) => row.keys);
    for (const required of [
      "Ctrl+T",
      "Ctrl+W",
      "Ctrl+Tab",
      "Ctrl+Shift+Tab",
      "Ctrl+O",
      "Ctrl+K",
      "Ctrl+,",
      "P",
      "F",
      "M",
      "Esc",
      "F1",
    ]) {
      expect(keys).toContain(required);
    }
  });

  it("filters by title, keys or category", () => {
    expect(filterShortcuts("").length).toBe(DEFAULT_SHORTCUTS.length);
    expect(filterShortcuts("palette").map((row) => row.id)).toEqual(["palette"]);
    expect(filterShortcuts("ctrl+w").map((row) => row.id)).toEqual(["close-tab"]);
    expect(filterShortcuts("cuts").map((row) => row.id)).toEqual([
      "shortcuts",
    ]);
    expect(filterShortcuts("nope")).toEqual([]);
  });

  it("groups rows by category in a stable order", () => {
    const groups = shortcutsByCategory(filterShortcuts(""));
    expect(groups[0].category).toBe("Global");
    expect(groups.map((group) => group.category)).toEqual([
      "Global",
      "Tabs",
      "Display",
      "Help",
    ]);
  });
});

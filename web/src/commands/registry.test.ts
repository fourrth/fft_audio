import { describe, expect, it, vi } from "vitest";
import {
  CommandRegistry,
  fuzzyScore,
  shortcutMatches,
  type UiCommand,
} from "./registry";

function command(partial: Partial<UiCommand> & { id: string }): UiCommand {
  return {
    title: partial.id,
    category: "Test",
    run: () => {},
    ...partial,
  };
}

describe("CommandRegistry", () => {
  it("runs enabled commands and refuses disabled/unknown ones", async () => {
    const run = vi.fn();
    const disabled = vi.fn();
    const registry = new CommandRegistry([
      command({ id: "open", title: "Open audio", run }),
      command({ id: "secret", enabled: () => false, run: disabled }),
    ]);

    await expect(registry.run("open")).resolves.toBe(true);
    await expect(registry.run("secret")).resolves.toBe(false);
    await expect(registry.run("missing")).resolves.toBe(false);
    expect(run).toHaveBeenCalledOnce();
    expect(disabled).not.toHaveBeenCalled();
    expect(registry.available().map((row) => row.id)).toEqual(["open"]);
  });

  it("fuzzy-searches titles, categories and keywords", () => {
    const registry = new CommandRegistry([
      command({ id: "fit", title: "Fit view", keywords: ["zoom"] }),
      command({ id: "palette", title: "Command palette", category: "Global" }),
      command({ id: "close", title: "Close tab", category: "Tabs" }),
    ]);
    expect(registry.search("fit").map((row) => row.id)).toContain("fit");
    expect(registry.search("zoom").map((row) => row.id)).toEqual(["fit"]);
    expect(registry.search("tab").map((row) => row.id)).toEqual(["close"]);
    expect(registry.search("zzzz")).toEqual([]);
    expect(registry.search("").length).toBe(3);
    expect(fuzzyScore("fv", "Fit view")).toBeGreaterThan(0);
    expect(fuzzyScore("xyz", "Fit view")).toBe(0);
  });

  it("matches shortcuts exactly", () => {
    expect(shortcutMatches({ key: "p" }, "P")).toBe(true);
    expect(shortcutMatches({ key: "p", ctrlKey: true }, "P")).toBe(false);
    expect(shortcutMatches({ key: "Tab", ctrlKey: true }, "Ctrl+Tab")).toBe(true);
    expect(
      shortcutMatches(
        { key: "Tab", ctrlKey: true, shiftKey: true },
        "Ctrl+Shift+Tab",
      ),
    ).toBe(true);
    expect(
      shortcutMatches({ key: "Tab", ctrlKey: true }, "Ctrl+Shift+Tab"),
    ).toBe(false);
    expect(shortcutMatches({ key: "Escape" }, "Esc")).toBe(true);
    expect(shortcutMatches({ key: "," , ctrlKey: true }, "Ctrl+,")).toBe(true);
  });
});

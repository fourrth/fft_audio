// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { CommandPalette } from "./CommandPalette";
import { ShortcutsModal } from "./ShortcutsModal";
import { CommandRegistry } from "./registry";

afterEach(cleanup);

const registry = new CommandRegistry([
  {
    id: "fit",
    title: "Fit view",
    category: "Display",
    shortcut: "F",
    run: () => {},
  },
  {
    id: "palette",
    title: "Command palette",
    category: "View",
    shortcut: "Ctrl+K",
    run: () => {},
  },
]);

describe("command palette and shortcuts modal", () => {
  it("filters commands and runs the chosen one", () => {
    const onRun = vi.fn();
    const onClose = vi.fn();
    render(
      <CommandPalette registry={registry} onRun={onRun} onClose={onClose} />,
    );
    expect(screen.getByRole("dialog", { name: "Command palette" })).toBeTruthy();
    fireEvent.change(screen.getByRole("textbox", { name: "Search commands" }), {
      target: { value: "fit" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Fit view/ }));
    expect(onRun).toHaveBeenCalledWith("fit");
    expect(onClose).toHaveBeenCalled();
  });

  it("runs the highlighted command with Enter", () => {
    const onRun = vi.fn();
    render(
      <CommandPalette
        registry={registry}
        onRun={onRun}
        onClose={() => {}}
      />,
    );
    const input = screen.getByRole("textbox", { name: "Search commands" });
    fireEvent.change(input, { target: { value: "palette" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onRun).toHaveBeenCalledWith("palette");
  });

  it("renders the shortcuts modal and filters it", () => {
    render(<ShortcutsModal onClose={() => {}} />);
    expect(
      screen.getByRole("dialog", { name: "Keyboard shortcuts" }),
    ).toBeTruthy();
    expect(screen.getByText("Command palette")).toBeTruthy();
    fireEvent.change(screen.getByRole("searchbox", { name: "Filter shortcuts" }), {
      target: { value: "fit" },
    });
    expect(screen.getByText("Fit view")).toBeTruthy();
    expect(screen.queryByText("Command palette")).toBeNull();
  });
});

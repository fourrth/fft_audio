// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MenuBar, type MenuBarMenu } from "./MenuBar";

afterEach(cleanup);

function menus(onNew: () => void, onRecent: () => void): MenuBarMenu[] {
  return [
    {
      id: "file",
      label: "File",
      items: [
        {
          kind: "item",
          id: "new",
          label: "New Spectrogram",
          shortcut: "Ctrl+T",
          onSelect: onNew,
        },
        { kind: "separator", id: "sep" },
        {
          kind: "submenu",
          id: "recent",
          label: "Recent",
          items: [
            {
              kind: "item",
              id: "tone",
              label: "tone.wav",
              onSelect: onRecent,
            },
          ],
        },
      ],
    },
    {
      id: "help",
      label: "Help",
      items: [
        {
          kind: "item",
          id: "shortcuts",
          label: "Keyboard shortcuts",
          onSelect: () => {},
        },
      ],
    },
  ];
}

describe("MenuBar", () => {
  it("opens menus and runs the selected item", () => {
    const onNew = vi.fn();
    const onRecent = vi.fn();
    render(<MenuBar menus={menus(onNew, onRecent)} />);

    fireEvent.click(screen.getByRole("menuitem", { name: "File" }));
    expect(screen.getByRole("menu", { name: "File" })).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitem", { name: /New Spectrogram/ }));
    expect(onNew).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu", { name: "File" })).toBeNull();

    fireEvent.click(screen.getByRole("menuitem", { name: "File" }));
    fireEvent.pointerEnter(screen.getByRole("menuitem", { name: "Recent" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "tone.wav" }));
    expect(onRecent).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu", { name: "File" })).toBeNull();
  });

  it("supports keyboard opening, arrow navigation and Escape", async () => {
    const onNew = vi.fn();
    render(<MenuBar menus={menus(onNew, vi.fn())} />);
    const file = screen.getByRole("menuitem", { name: "File" });

    fireEvent.keyDown(file, { key: "ArrowDown" });
    const item = screen.getByRole("menuitem", { name: /New Spectrogram/ });
    await waitFor(() => expect(document.activeElement).toBe(item));

    fireEvent.keyDown(item, { key: "Escape" });
    expect(screen.queryByRole("menu", { name: "File" })).toBeNull();
  });

  it("switches between top-level menus and closes on outside click", () => {
    render(<MenuBar menus={menus(vi.fn(), vi.fn())} />);

    fireEvent.click(screen.getByRole("menuitem", { name: "File" }));
    expect(screen.getByRole("menu", { name: "File" })).toBeTruthy();

    fireEvent.click(screen.getByRole("menuitem", { name: "Help" }));
    expect(screen.getByRole("menu", { name: "Help" })).toBeTruthy();
    expect(screen.queryByRole("menu", { name: "File" })).toBeNull();

    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("menu", { name: "Help" })).toBeNull();
  });
});

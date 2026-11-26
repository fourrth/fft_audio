// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ScriptEditor } from "./ScriptEditor";

afterEach(cleanup);

function renderEditor(overrides: Partial<Parameters<typeof ScriptEditor>[0]> = {}) {
  const props = {
    value: "print('one')\n",
    dirty: false,
    busy: false,
    onChange: vi.fn(),
    onSave: vi.fn(),
    ...overrides,
  };
  const view = render(<ScriptEditor {...props} />);
  return { ...view, props };
}

describe("ScriptEditor", () => {
  it("renders a controlled monospace textarea", () => {
    const onChange = vi.fn();
    const { rerender } = renderEditor({ onChange });
    const textarea = screen.getByLabelText("Script text") as HTMLTextAreaElement;

    expect(textarea.value).toBe("print('one')\n");
    expect(textarea.getAttribute("spellcheck")).toBe("false");
    fireEvent.change(textarea, { target: { value: "print('two')\n" } });
    expect(onChange).toHaveBeenCalledWith("print('two')\n");

    rerender(
      <ScriptEditor
        value={"print('two')\n"}
        dirty={false}
        busy={false}
        onChange={onChange}
        onSave={vi.fn()}
      />,
    );
    expect(
      (screen.getByLabelText("Script text") as HTMLTextAreaElement).value,
    ).toBe("print('two')\n");
  });

  it("shows the dirty badge only for unsaved changes", () => {
    const { rerender } = renderEditor({ dirty: true });
    expect(screen.getByTestId("scripting-editor-dirty").textContent).toContain(
      "Unsaved changes",
    );

    rerender(
      <ScriptEditor
        value={"print('one')\n"}
        dirty={false}
        busy={false}
        onChange={vi.fn()}
        onSave={vi.fn()}
      />,
    );
    expect(screen.queryByTestId("scripting-editor-dirty")).toBeNull();
  });

  it("saves with Ctrl/Cmd+S only when dirty and idle", () => {
    const onSave = vi.fn();
    const { rerender } = renderEditor({ dirty: true, onSave });
    const textarea = screen.getByLabelText("Script text");

    fireEvent.keyDown(textarea, { key: "s", ctrlKey: true });
    expect(onSave).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(textarea, { key: "s", metaKey: true });
    expect(onSave).toHaveBeenCalledTimes(2);

    rerender(
      <ScriptEditor
        value={"print('one')\n"}
        dirty={false}
        busy={false}
        onChange={vi.fn()}
        onSave={onSave}
      />,
    );
    fireEvent.keyDown(screen.getByLabelText("Script text"), {
      key: "s",
      ctrlKey: true,
    });
    expect(onSave).toHaveBeenCalledTimes(2);

    rerender(
      <ScriptEditor
        value={"print('one')\n"}
        dirty
        busy
        onChange={vi.fn()}
        onSave={onSave}
      />,
    );
    fireEvent.keyDown(screen.getByLabelText("Script text"), {
      key: "s",
      ctrlKey: true,
    });
    expect(onSave).toHaveBeenCalledTimes(2);
  });

  it("marks the editor busy and read-only while loading or saving", () => {
    renderEditor({ dirty: true, busy: true });
    const textarea = screen.getByLabelText("Script text") as HTMLTextAreaElement;

    expect(textarea.readOnly).toBe(true);
    expect(screen.getByText("Working…")).toBeTruthy();
    expect(
      textarea.closest(".script-editor")?.getAttribute("aria-busy"),
    ).toBe("true");
  });
});

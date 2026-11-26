// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { ShellDisplay } from "../shell/session";
import { MockTransport } from "../transport/mock";
import { ScriptingPanel } from "./ScriptingPanel";
import { clearScriptConsole, useScriptConsole } from "./console";

afterEach(() => {
  cleanup();
  clearScriptConsole(7);
  vi.restoreAllMocks();
});

function ConsoleProbe({ id }: { id: number }) {
  const snapshot = useScriptConsole(id);
  return (
    <div>
      <span data-testid="console-running">{String(snapshot.running)}</span>
      <span data-testid="console-exit">
        {snapshot.exit
          ? `${snapshot.exit.success}:${snapshot.exit.code ?? "null"}`
          : ""}
      </span>
      <div data-testid="console-lines">
        {snapshot.lines.map((line, index) => (
          <div key={`${line.at}-${index}`}>{line.text}</div>
        ))}
      </div>
    </div>
  );
}

const display: ShellDisplay = {
  id: 7,
  parent: null,
  kind: "scripting",
  name: "Scripting",
  state: {},
};

function withPath(path: string): ShellDisplay {
  return { ...display, state: { path } };
}

const EXAMPLES = [
  { filename: "harmonics_chirp_example.py" },
  { filename: "three_tones_example.py" },
];

function makeTransport() {
  return new MockTransport({
    "script.pickFile": () => ({ path: "/tmp/demo.py" }),
    "script.run": () => undefined,
    "script.stop": () => undefined,
    "script.stat": () => ({ exists: true, mtimeMs: 1 }),
    "script.readFile": () => "print('hello')\n",
    "script.writeFile": () => undefined,
    "script.listExamples": () => EXAMPLES,
    "script.createExample": (args) => ({
      path: `/tmp/scripting-examples/${args.filename}`,
    }),
  });
}

function renderPanel(
  transport: MockTransport,
  options: {
    shellDisplay?: ShellDisplay;
    onPathChange?: (path: string) => void;
    onOpenConsole?: (options?: { focus?: boolean }) => void;
    runnerWorkspaceEnabled?: boolean;
  } = {},
) {
  return render(
    <ScriptingPanel
      display={options.shellDisplay ?? display}
      transport={transport}
      onPathChange={options.onPathChange ?? (() => {})}
      onOpenConsole={options.onOpenConsole}
      runnerWorkspaceEnabled={options.runnerWorkspaceEnabled}
    />,
  );
}

function editor(): HTMLTextAreaElement {
  return screen.getByLabelText("Script text") as HTMLTextAreaElement;
}

async function waitForEditor(value: string) {
  await waitFor(() => expect(editor().value).toBe(value));
}

describe("ScriptingPanel", () => {
  it("opens in the editor and auto-loads the selected script", async () => {
    const transport = makeTransport();
    renderPanel(transport, { shellDisplay: withPath("/tmp/demo.py") });

    await waitFor(() =>
      expect(
        transport.calls.some(
          (call) =>
            call.name === "script.readFile" &&
            (call.args as { path?: string }).path === "/tmp/demo.py",
        ),
      ).toBe(true),
    );
    await waitForEditor("print('hello')\n");
    expect(screen.queryByRole("radio", { name: "Editor" })).toBeNull();
    expect(screen.queryByTestId("scripting-editor-dirty")).toBeNull();
    expect(screen.getByRole("button", { name: "Console" })).toBeTruthy();
    expect(
      screen.queryByRole("log", { name: "Script output" }),
    ).toBeNull();
  });

  it("shows open and example actions when no script is selected", async () => {
    const transport = makeTransport();
    renderPanel(transport);

    expect(screen.getByRole("heading", { name: "No script selected" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open script…" })).toBeTruthy();
    expect(screen.getByLabelText("Load example")).toBeTruthy();
    expect(
      screen.getByText(/embedded Python engine/),
    ).toBeTruthy();
    expect(screen.queryByLabelText("Script text")).toBeNull();
    expect(
      transport.calls.some((call) => call.name === "script.readFile"),
    ).toBe(false);
  });

  it("saves dirty edits, then runs and streams the output", async () => {
    const transport = makeTransport();
    renderPanel(transport, { shellDisplay: withPath("/tmp/demo.py") });
    await waitForEditor("print('hello')\n");

    fireEvent.change(editor(), {
      target: { value: "print('updated')\n" },
    });
    expect(screen.getByTestId("scripting-editor-dirty").textContent).toContain(
      "Unsaved changes",
    );
    const run = screen.getByRole("button", { name: "Run" }) as HTMLButtonElement;
    expect(run.disabled).toBe(true);
    expect(run.title).toBe("Save before running");

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(
        transport.calls.some(
          (call) =>
            call.name === "script.writeFile" &&
            (call.args as { path?: string }).path === "/tmp/demo.py" &&
            (call.args as { contents?: string }).contents ===
              "print('updated')\n",
        ),
      ).toBe(true),
    );
    await waitFor(() =>
      expect(screen.queryByTestId("scripting-editor-dirty")).toBeNull(),
    );
    expect(
      (screen.getByRole("button", { name: "Run" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);

    render(<ConsoleProbe id={7} />);
    expect(screen.getByTestId("console-lines").textContent).toContain(
      "saved /tmp/demo.py",
    );

    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() =>
      expect(
        transport.calls.some(
          (call) =>
            call.name === "script.run" &&
            (call.args as { path?: string }).path === "/tmp/demo.py",
        ),
      ).toBe(true),
    );
    await waitFor(() =>
      expect(screen.getByTestId("console-lines").textContent).toContain(
        "> /tmp/demo.py",
      ),
    );
    expect(
      screen.queryByRole("log", { name: "Script output" }),
    ).toBeNull();
    act(() => {
      transport.emit("script.output", {
        displayId: 7,
        stream: "stdout",
        line: "hello from the script",
      });
    });
    expect(screen.getByTestId("console-lines").textContent).toContain(
      "hello from the script",
    );

    act(() => {
      transport.emit("script.exit", {
        displayId: 7,
        code: 0,
        success: true,
      });
    });
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Run" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
    expect(screen.getByTestId("console-running").textContent).toBe("false");
    expect(screen.getByTestId("console-exit").textContent).toBe("true:0");
    expect(screen.getByTestId("console-lines").textContent).toContain(
      "script finished (ok, code 0)",
    );
    expect(screen.queryByText(/process exited/)).toBeNull();
    expect(
      transport.calls.some((call) => call.name.startsWith("code.")),
    ).toBe(false);
  });

  it("opens the console from the toolbar and on run without stealing focus", async () => {
    const transport = makeTransport();
    const onOpenConsole = vi.fn();
    renderPanel(transport, {
      shellDisplay: withPath("/tmp/demo.py"),
      onOpenConsole,
    });
    await waitForEditor("print('hello')\n");

    expect(screen.queryByRole("log", { name: "Script output" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Console" }));
    expect(onOpenConsole).toHaveBeenCalledWith();

    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    await waitFor(() =>
      expect(onOpenConsole).toHaveBeenCalledWith({ focus: false }),
    );
    expect(screen.queryByRole("log", { name: "Script output" })).toBeNull();
  });

  it("loads a bundled example without running it", async () => {
    const transport = makeTransport();
    const onPathChange = vi.fn();
    renderPanel(transport, { onPathChange });

    const picker = screen.getByLabelText(
      "Scripting example",
    ) as HTMLSelectElement;
    await waitFor(() =>
      expect(
        within(picker).getByRole("option", {
          name: "three_tones_example.py",
        }),
      ).toBeTruthy(),
    );
    fireEvent.change(picker, {
      target: { value: "three_tones_example.py" },
    });

    const path = "/tmp/scripting-examples/three_tones_example.py";
    await waitFor(() => expect(onPathChange).toHaveBeenCalledWith(path));
    await waitFor(() =>
      expect(
        transport.calls.some(
          (call) =>
            call.name === "script.createExample" &&
            (call.args as { filename?: string }).filename ===
              "three_tones_example.py",
        ),
      ).toBe(true),
    );
    await waitFor(() =>
      expect(
        transport.calls.some(
          (call) =>
            call.name === "script.readFile" &&
            (call.args as { path?: string }).path === path,
        ),
      ).toBe(true),
    );
    expect(screen.getByTitle(path)).toBeTruthy();
    expect(screen.getByTestId("scripting-example-hint").textContent).toContain(
      "three_tones_example.py",
    );
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Run" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );
    expect(transport.calls.some((call) => call.name === "script.run")).toBe(false);
  });

  it("populates the example picker from the backend list", async () => {
    const transport = makeTransport();
    transport.setHandler("script.listExamples", () => [
      { filename: "nested_example.py" },
      { filename: "other_example.py" },
    ]);
    renderPanel(transport);

    const picker = screen.getByLabelText(
      "Scripting example",
    ) as HTMLSelectElement;
    await waitFor(() =>
      expect(
        within(picker).getByRole("option", { name: "nested_example.py" }),
      ).toBeTruthy(),
    );
    expect(
      within(picker).getByRole("option", { name: "other_example.py" }),
    ).toBeTruthy();
    expect(
      transport.calls.some((call) => call.name === "script.listExamples"),
    ).toBe(true);
  });

  it("shows read errors and keeps the prior buffer", async () => {
    const transport = makeTransport();
    const onPathChange = vi.fn();
    transport.setHandler("script.pickFile", () => ({ path: "/tmp/broken.py" }));
    transport.setHandler("script.readFile", ({ path }) => {
      if (path === "/tmp/broken.py") {
        throw new Error("cannot read script");
      }
      return "print('hello')\n";
    });
    renderPanel(transport, {
      shellDisplay: withPath("/tmp/demo.py"),
      onPathChange,
    });
    await waitForEditor("print('hello')\n");

    fireEvent.click(screen.getByRole("button", { name: "Open…" }));
    await waitFor(() => expect(onPathChange).toHaveBeenCalledWith("/tmp/broken.py"));
    expect(await screen.findByText("cannot read script")).toBeTruthy();
    expect(editor().value).toBe("print('hello')\n");
    expect(screen.queryByTestId("scripting-editor-dirty")).toBeNull();
  });

  it("keeps the buffer and dirty state when saving fails", async () => {
    const transport = makeTransport();
    transport.setHandler("script.writeFile", () => {
      throw new Error("disk full");
    });
    renderPanel(transport, { shellDisplay: withPath("/tmp/demo.py") });
    await waitForEditor("print('hello')\n");

    fireEvent.change(editor(), { target: { value: "print('draft')\n" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("disk full")).toBeTruthy();
    expect(editor().value).toBe("print('draft')\n");
    expect(screen.getByTestId("scripting-editor-dirty")).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Save" }) as HTMLButtonElement)
        .disabled,
    ).toBe(false);
  });

  it("confirms before discarding dirty edits when changing path", async () => {
    const transport = makeTransport();
    const onPathChange = vi.fn();
    transport.setHandler("script.pickFile", () => ({ path: "/tmp/other.py" }));
    transport.setHandler("script.readFile", ({ path }) =>
      path === "/tmp/other.py" ? "print('other')\n" : "print('hello')\n",
    );
    renderPanel(transport, {
      shellDisplay: withPath("/tmp/demo.py"),
      onPathChange,
    });
    await waitForEditor("print('hello')\n");

    fireEvent.change(editor(), { target: { value: "print('unsaved')\n" } });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    fireEvent.click(screen.getByRole("button", { name: "Open…" }));
    await waitFor(() => expect(confirm).toHaveBeenCalled());
    expect(onPathChange).not.toHaveBeenCalled();
    expect(screen.getByTitle("/tmp/demo.py")).toBeTruthy();
    expect(editor().value).toBe("print('unsaved')\n");
    expect(
      transport.calls.filter((call) => call.name === "script.readFile"),
    ).toHaveLength(1);

    confirm.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "Open…" }));
    await waitFor(() => expect(onPathChange).toHaveBeenCalledWith("/tmp/other.py"));
    await waitForEditor("print('other')\n");
    expect(screen.queryByTestId("scripting-editor-dirty")).toBeNull();
  });

  it("reloads only after confirming a dirty buffer", async () => {
    const transport = makeTransport();
    renderPanel(transport, { shellDisplay: withPath("/tmp/demo.py") });
    await waitForEditor("print('hello')\n");
    fireEvent.change(editor(), { target: { value: "print('unsaved')\n" } });

    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    await waitFor(() => expect(confirm).toHaveBeenCalled());
    expect(
      transport.calls.filter((call) => call.name === "script.readFile"),
    ).toHaveLength(1);
    expect(editor().value).toBe("print('unsaved')\n");

    confirm.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    await waitForEditor("print('hello')\n");
    expect(screen.queryByTestId("scripting-editor-dirty")).toBeNull();
    expect(
      transport.calls.filter((call) => call.name === "script.readFile"),
    ).toHaveLength(2);
  });

  it("keeps saving the buffer's loaded path after an external path change", async () => {
    const transport = makeTransport();
    const { rerender } = renderPanel(transport, {
      shellDisplay: withPath("/tmp/demo.py"),
    });
    await waitForEditor("print('hello')\n");

    fireEvent.change(editor(), { target: { value: "print('draft')\n" } });
    rerender(
      <ScriptingPanel
        display={withPath("/tmp/other.py")}
        transport={transport}
        onPathChange={() => {}}
      />,
    );
    await waitFor(() => expect(screen.getByTitle("/tmp/other.py")).toBeTruthy());
    expect(editor().value).toBe("print('draft')\n");

    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(
        transport.calls.some(
          (call) =>
            call.name === "script.writeFile" &&
            (call.args as { path?: string }).path === "/tmp/demo.py" &&
            (call.args as { contents?: string }).contents ===
              "print('draft')\n",
        ),
      ).toBe(true),
    );
    expect(
      transport.calls.some(
        (call) =>
          call.name === "script.writeFile" &&
          (call.args as { path?: string }).path === "/tmp/other.py",
      ),
    ).toBe(false);
  });

  it("confirms before replacing a dirty buffer with an example", async () => {
    const transport = makeTransport();
    renderPanel(transport, { shellDisplay: withPath("/tmp/demo.py") });
    await waitForEditor("print('hello')\n");
    fireEvent.change(editor(), { target: { value: "print('unsaved')\n" } });

    const select = screen.getByLabelText(
      "Scripting example",
    ) as HTMLSelectElement;
    await waitFor(() =>
      expect(
        within(select).getByRole("option", {
          name: "three_tones_example.py",
        }),
      ).toBeTruthy(),
    );
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    fireEvent.change(select, { target: { value: "three_tones_example.py" } });
    expect(confirm).toHaveBeenCalled();
    expect(
      transport.calls.some((call) => call.name === "script.createExample"),
    ).toBe(false);
    expect(editor().value).toBe("print('unsaved')\n");
    expect(screen.getByTitle("/tmp/demo.py")).toBeTruthy();
    expect(select.value).toBe("");
  });

  it("stops only the script when the panel unmounts", async () => {
    const transport = makeTransport();
    const { unmount } = renderPanel(transport, {
      shellDisplay: withPath("/tmp/demo.py"),
    });
    await waitForEditor("print('hello')\n");

    unmount();
    await waitFor(() =>
      expect(
        transport.calls.some((call) => call.name === "script.stop"),
      ).toBe(true),
    );
    expect(
      transport.calls.some((call) => call.name.startsWith("code.")),
    ).toBe(false);
  });

  it("keeps the buffer when switching to the gated runner workspace", async () => {
    const transport = makeTransport();
    renderPanel(transport, {
      shellDisplay: withPath("/tmp/demo.py"),
      runnerWorkspaceEnabled: true,
    });
    await waitForEditor("print('hello')\n");

    const modes = screen.getByRole("radiogroup", { name: "Scripting mode" });
    expect(modes).toBeTruthy();
    expect(
      (screen.getByRole("radio", { name: "Editor" }) as HTMLElement).getAttribute(
        "aria-checked",
      ),
    ).toBe("true");

    fireEvent.change(editor(), { target: { value: "print('kept')\n" } });
    fireEvent.click(screen.getByRole("radio", { name: "Runner" }));

    expect(screen.queryByLabelText("Script text")).toBeNull();
    expect(screen.getByLabelText("Runner example")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Choose…" })).toBeTruthy();
    expect(screen.getByRole("log", { name: "Script output" })).toBeTruthy();
    const runnerRun = screen.getByRole("button", {
      name: "Run",
    }) as HTMLButtonElement;
    expect(runnerRun.disabled).toBe(false);

    fireEvent.click(screen.getByRole("radio", { name: "Editor" }));
    await waitForEditor("print('kept')\n");
    expect(screen.getByTestId("scripting-editor-dirty")).toBeTruthy();
  });
});

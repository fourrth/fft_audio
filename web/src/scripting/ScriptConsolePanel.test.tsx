// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ShellDisplay } from "../shell/session";
import { MockTransport } from "../transport/mock";
import { ScriptConsolePanel } from "./ScriptConsolePanel";
import {
  appendScriptConsoleLine,
  clearScriptConsole,
  setScriptConsoleRunning,
} from "./console";

const PARENT_ID = 21;

const display: ShellDisplay = {
  id: 30,
  parent: PARENT_ID,
  kind: "script_console",
  name: "Scripting > Console",
  state: {},
};

afterEach(() => {
  cleanup();
  clearScriptConsole(PARENT_ID);
  setScriptConsoleRunning(PARENT_ID, false);
});

function makeTransport() {
  return new MockTransport({
    "script.stop": () => undefined,
  });
}

describe("ScriptConsolePanel", () => {
  it("renders an empty independent console", () => {
    render(<ScriptConsolePanel display={display} transport={makeTransport()} />);

    expect(screen.getByRole("log", { name: "Script output" })).toBeTruthy();
    expect(screen.getByText(/Script output appears here/)).toBeTruthy();
    expect(screen.getByText(/independent panel/)).toBeTruthy();
    expect(screen.getByText("Idle")).toBeTruthy();
    expect(screen.getByText(/parent 21/)).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Clear" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("shows stored lines and clears them", () => {
    act(() => {
      appendScriptConsoleLine(PARENT_ID, {
        stream: "stdout",
        text: "hello from the script",
        at: "10:00:00",
      });
      appendScriptConsoleLine(PARENT_ID, {
        stream: "stderr",
        text: "warn from the script",
        at: "10:00:01",
      });
    });
    render(<ScriptConsolePanel display={display} transport={makeTransport()} />);

    expect(screen.getByText("hello from the script")).toBeTruthy();
    expect(screen.getByText("warn from the script")).toBeTruthy();
    expect(screen.getByText(/2 lines/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(screen.queryByText("hello from the script")).toBeNull();
    expect(screen.getByText(/Script output appears here/)).toBeTruthy();
    expect(screen.getByText(/0 lines/)).toBeTruthy();
  });

  it("stops the parent script and reports stopped", async () => {
    const transport = makeTransport();
    act(() => setScriptConsoleRunning(PARENT_ID, true));
    render(<ScriptConsolePanel display={display} transport={transport} />);

    expect(screen.getByText("Running")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));

    expect(
      transport.calls.some(
        (call) =>
          call.name === "script.stop" &&
          (call.args as { displayId?: number }).displayId === PARENT_ID,
      ),
    ).toBe(true);
    expect(screen.getByText("stopped")).toBeTruthy();
    expect(screen.getByText("Idle")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Stop" })).toBeNull();
  });

  it("shows a clear error state without a parent", () => {
    render(
      <ScriptConsolePanel
        display={{ ...display, parent: null }}
        transport={makeTransport()}
      />,
    );

    expect(screen.getByRole("heading", { name: "Console has no parent" })).toBeTruthy();
    expect(screen.queryByRole("log", { name: "Script output" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Clear" })).toBeNull();
    expect(screen.getByText("no parent")).toBeTruthy();
  });
});

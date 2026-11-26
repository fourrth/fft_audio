import { useEffect, useRef } from "react";
import type { ShellDisplay } from "../shell/session";
import type { Transport } from "../transport/types";
import { DisplayFrame } from "../display/DisplayFrame";
import { EmptyState } from "../ui/EmptyState";
import {
  appendScriptConsoleLine,
  clearScriptConsole,
  setScriptConsoleRunning,
  useScriptConsole,
} from "./console";
import "./scripting.css";

export interface ScriptConsolePanelProps {
  display: ShellDisplay;
  transport: Transport | null;
}

function statusLabel(
  running: boolean,
  exit: { success: boolean } | null,
): string {
  if (running) {
    return "Running";
  }
  if (exit) {
    return exit.success ? "Finished" : "Failed";
  }
  return "Idle";
}

/** Independent child panel that mirrors a Scripting display's output store. */
export function ScriptConsolePanel({
  display,
  transport,
}: ScriptConsolePanelProps) {
  const snapshot = useScriptConsole(display.parent);
  const parent = display.parent;
  const logRef = useRef<HTMLDivElement>(null);
  const lines = snapshot.lines;

  useEffect(() => {
    logRef.current?.scrollTo?.({ top: logRef.current.scrollHeight });
  }, [lines]);

  const clear = () => {
    if (parent !== null) {
      clearScriptConsole(parent);
    }
  };

  const stop = () => {
    if (parent === null || !transport) {
      return;
    }
    void transport
      .invoke({ name: "script.stop", args: { displayId: parent } })
      .catch(() => {});
    setScriptConsoleRunning(parent, false);
    appendScriptConsoleLine(parent, { stream: "system", text: "stopped" });
  };

  const label = statusLabel(snapshot.running, snapshot.exit);
  const status = snapshot.running
    ? "running"
    : snapshot.exit
      ? snapshot.exit.success
        ? "finished"
        : "failed"
      : "idle";

  return (
    <DisplayFrame
      title={display.name}
      kind="console"
      state={{ kind: "ready" }}
      footerHints={`${lines.length} line${
        lines.length === 1 ? "" : "s"
      } · Output is not persisted across restarts`}
      footerRight={parent === null ? "no parent" : `parent ${parent}`}
      actions={
        parent === null ? undefined : (
          <div className="script-console-actions">
            <span className={`script-console-status status-${status}`}>
              {label}
            </span>
            {snapshot.running ? (
              <button
                type="button"
                className="secondary-button"
                onClick={stop}
                disabled={!transport}
              >
                Stop
              </button>
            ) : null}
            <button
              type="button"
              className="secondary-button"
              onClick={clear}
              disabled={lines.length === 0}
            >
              Clear
            </button>
          </div>
        )
      }
    >
      {parent === null ? (
        <EmptyState
          icon="⌘"
          title="Console has no parent"
          description="This console is not linked to a Scripting display, so there is no script to run or stop."
          hint="Open Scripting, then use its Console button to create a linked console."
        />
      ) : (
        <div className="script-console-panel">
          <div
            className="script-console"
            role="log"
            aria-label="Script output"
            ref={logRef}
          >
            {lines.length === 0 ? (
              <div className="script-console-empty">
                <span className="script-muted">
                  Script output appears here when the Scripting panel runs a
                  script.
                </span>
                <span className="script-muted">
                  This console is an independent panel: move, tab, or close it
                  without stopping the editor.
                </span>
              </div>
            ) : (
              lines.map((line, index) => (
                <div
                  key={`${line.at}-${index}`}
                  className={`script-line ${line.stream}`}
                >
                  <span className="script-time">{line.at}</span>
                  <span>{line.text}</span>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </DisplayFrame>
  );
}

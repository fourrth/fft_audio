import { useCallback, useEffect, useRef, useState } from "react";
import { watchImmediate } from "@tauri-apps/plugin-fs";
import type { ShellDisplay } from "../shell/session";
import type { ScriptExampleInfo, Transport } from "../transport/types";
import { isTauriRuntime } from "../transport/tauri";
import { fileName } from "../launcher/recent";
import { DisplayFrame } from "../display/DisplayFrame";
import { Banner } from "../ui/Banner";
import { EmptyState } from "../ui/EmptyState";
import { Segmented } from "../ui/Segmented";
import { Toggle } from "../ui/Toggle";
import { useToast } from "../ui/toast";
import { RUNNER_WORKSPACE_ENABLED } from "./features";
import { ScriptEditor } from "./ScriptEditor";
import {
  appendScriptConsoleLine,
  setScriptConsoleExit,
  setScriptConsoleRunning,
  useScriptConsole,
  type ScriptConsoleStream,
} from "./console";
import "./scripting.css";

type ScriptingMode = "editor" | "runner";

export interface ScriptingPanelProps {
  display: ShellDisplay;
  panelId?: string;
  transport: Transport | null;
  onPathChange: (path: string) => void;
  onOpenConsole?: (options?: { focus?: boolean }) => void;
  runnerWorkspaceEnabled?: boolean;
}

const DISCARD_MESSAGE = "Discard unsaved changes?";

function errorText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

/** Editor-first scripting workspace; the runner view is feature-gated. */
export function ScriptingPanel({
  display,
  panelId: _panelId,
  transport,
  onPathChange,
  onOpenConsole,
  runnerWorkspaceEnabled = RUNNER_WORKSPACE_ENABLED,
}: ScriptingPanelProps) {
  const storedPath =
    typeof display.state.path === "string" ? display.state.path : "";
  const [path, setPath] = useState(storedPath);
  const [examples, setExamples] = useState<ScriptExampleInfo[]>([]);
  const [exampleId, setExampleId] = useState("");
  const [exampleBusy, setExampleBusy] = useState(false);
  const [examplesLoading, setExamplesLoading] = useState(false);
  const [mode, setMode] = useState<ScriptingMode>("editor");
  const [autoRun, setAutoRun] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const consoleSnapshot = useScriptConsole(display.id);
  const lines = consoleSnapshot.lines;
  const running = consoleSnapshot.running;

  const [editorText, setEditorText] = useState("");
  const [baseline, setBaseline] = useState("");
  const [loadedPath, setLoadedPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);

  const lastMtime = useRef<number | null>(null);
  const loadSeq = useRef(0);
  const consoleRef = useRef<HTMLDivElement>(null);
  const dirtyRef = useRef(false);
  const runningRef = useRef(false);
  const runRef = useRef<() => void>(() => {});

  const { pushToast } = useToast();
  const dirty = editorText !== baseline;
  dirtyRef.current = dirty;
  runningRef.current = running;

  const append = useCallback(
    (line: { stream: ScriptConsoleStream; text: string }) => {
      appendScriptConsoleLine(display.id, line);
    },
    [display.id],
  );

  useEffect(() => {
    if (storedPath) {
      setPath(storedPath);
    }
  }, [storedPath]);

  useEffect(() => {
    if (!runnerWorkspaceEnabled) {
      setMode("editor");
    }
  }, [runnerWorkspaceEnabled]);

  useEffect(() => {
    lastMtime.current = null;
  }, [path]);

  useEffect(() => {
    if (!transport) {
      setExamples([]);
      return;
    }
    let cancelled = false;
    setExamplesLoading(true);
    void transport
      .invoke({ name: "script.listExamples", args: {} })
      .then((list) => {
        if (!cancelled) {
          setExamples(list);
        }
      })
      .catch((reason) => {
        if (!cancelled) {
          setError(errorText(reason));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setExamplesLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [transport]);

  const load = useCallback(
    async (target: string) => {
      if (!transport || !target) {
        return;
      }
      const seq = ++loadSeq.current;
      setLoading(true);
      setError(null);
      try {
        const contents = await transport.invoke({
          name: "script.readFile",
          args: { path: target },
        });
        if (seq !== loadSeq.current) {
          return;
        }
        setEditorText(contents);
        setBaseline(contents);
        setLoadedPath(target);
        append({ stream: "system", text: `loaded ${target}` });
      } catch (reason) {
        if (seq !== loadSeq.current) {
          return;
        }
        setError(errorText(reason));
      } finally {
        if (seq === loadSeq.current) {
          setLoading(false);
        }
      }
    },
    [append, transport],
  );

  useEffect(() => {
    if (!transport || !path || path === loadedPath || dirty) {
      return;
    }
    void load(path);
  }, [dirty, load, loadedPath, path, transport]);

  useEffect(() => {
    if (!transport) {
      return;
    }
    const offOutput = transport.on("script.output", (event) => {
      if (event.displayId === display.id) {
        append({ stream: event.stream, text: event.line });
      }
    });
    const offExit = transport.on("script.exit", (event) => {
      if (event.displayId !== display.id) {
        return;
      }
      setScriptConsoleRunning(display.id, false);
      setScriptConsoleExit(display.id, {
        code: event.code,
        success: event.success,
      });
      append({
        stream: "system",
        text: `script finished (${event.success ? "ok" : "failed"}${
          event.code === null ? "" : `, code ${event.code}`
        })`,
      });
      pushToast({
        tone: event.success ? "success" : "error",
        message: event.success ? "Script finished" : "Script failed",
      });
    });
    return () => {
      offOutput();
      offExit();
      setScriptConsoleRunning(display.id, false);
      void transport
        .invoke({ name: "script.stop", args: { displayId: display.id } })
        .catch(() => {});
    };
  }, [append, display.id, pushToast, transport]);

  useEffect(() => {
    if (mode === "runner") {
      consoleRef.current?.scrollTo?.({ top: consoleRef.current.scrollHeight });
    }
  }, [lines, mode]);

  const confirmDiscard = () =>
    !dirtyRef.current || window.confirm(DISCARD_MESSAGE);

  const applyPath = (next: string, nextExampleId: string) => {
    setExampleId(nextExampleId);
    setPath(next);
    onPathChange(next);
    if (dirtyRef.current) {
      void load(next);
    }
  };

  const browse = async () => {
    if (!transport) {
      return;
    }
    try {
      const picked = await transport.invoke({ name: "script.pickFile", args: {} });
      if (!picked?.path || picked.path === path) {
        return;
      }
      if (!confirmDiscard()) {
        return;
      }
      applyPath(picked.path, "");
      append({ stream: "system", text: `selected ${picked.path}` });
    } catch (reason) {
      setError(errorText(reason));
    }
  };

  const loadExample = async (filename: string) => {
    if (!transport || !filename) {
      return;
    }
    setExampleBusy(true);
    try {
      const created = await transport.invoke({
        name: "script.createExample",
        args: { filename },
      });
      setError(null);
      applyPath(created.path, filename);
      append({
        stream: "system",
        text: `loaded example: ${filename}`,
      });
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setExampleBusy(false);
    }
  };

  const handleExampleChange = (value: string, select: HTMLSelectElement) => {
    if (!value) {
      setExampleId("");
      return;
    }
    if (!confirmDiscard()) {
      select.value = exampleId;
      return;
    }
    void loadExample(value);
  };

  const reload = () => {
    if (!transport || !path || loading) {
      return;
    }
    if (!confirmDiscard()) {
      return;
    }
    void load(path);
  };

  const save = async () => {
    if (!transport || !loadedPath || !dirty || busy || loading) {
      return;
    }
    const contents = editorText;
    setBusy(true);
    setError(null);
    try {
      await transport.invoke({
        name: "script.writeFile",
        args: { path: loadedPath, contents },
      });
      dirtyRef.current = false;
      setBaseline(contents);
      append({ stream: "system", text: `saved ${loadedPath}` });
      pushToast({
        tone: "success",
        message: `Saved ${fileName(loadedPath)}`,
      });
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setBusy(false);
    }
  };

  const run = useCallback(async () => {
    // Manual Run is disabled by canRun while the buffer is dirty. Auto-run
    // intentionally executes the on-disk file and must not be blocked by a
    // dirty buffer, otherwise the file-change event can race the save.
    if (!transport || !path || runningRef.current) {
      return;
    }
    setScriptConsoleRunning(display.id, true);
    onOpenConsole?.({ focus: false });
    try {
      setError(null);
      await transport.invoke({
        name: "script.run",
        args: { displayId: display.id, path },
      });
      append({ stream: "system", text: `> ${path}` });
    } catch (reason) {
      setScriptConsoleRunning(display.id, false);
      setError(errorText(reason));
    }
  }, [append, display.id, onOpenConsole, path, transport]);

  runRef.current = () => {
    void run();
  };

  const stop = async () => {
    if (!transport) {
      return;
    }
    await transport
      .invoke({ name: "script.stop", args: { displayId: display.id } })
      .catch(() => {});
    setScriptConsoleRunning(display.id, false);
    append({ stream: "system", text: "stopped" });
  };

  useEffect(() => {
    if (!autoRun || !transport || !path || !isTauriRuntime()) {
      return;
    }
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void watchImmediate(path, (event) => {
      const touched =
        event.type === "any" ||
        (typeof event.type === "object" &&
          ("modify" in event.type || "create" in event.type));
      if (!runningRef.current && touched) {
        runRef.current();
      }
    })
      .then((stopWatching) => {
        if (cancelled) {
          stopWatching();
        } else {
          unlisten = stopWatching;
        }
      })
      .catch(() => {
        // The polling path below still covers browser previews; on a
        // permission error the user can re-select the file.
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [autoRun, path, transport]);

  useEffect(() => {
    if (!autoRun || !transport || !path || isTauriRuntime()) {
      return;
    }
    const timer = window.setInterval(() => {
      void transport
        .invoke({ name: "script.stat", args: { path } })
        .then((stat) => {
          if (!stat.exists || stat.mtimeMs === null) {
            return;
          }
          if (lastMtime.current === null) {
            lastMtime.current = stat.mtimeMs;
            return;
          }
          if (stat.mtimeMs !== lastMtime.current) {
            lastMtime.current = stat.mtimeMs;
            if (!runningRef.current) {
              runRef.current();
            }
          }
        })
        .catch(() => {});
    }, 1000);
    return () => window.clearInterval(timer);
  }, [autoRun, path, transport]);

  const editorBusy = busy || loading || exampleBusy;
  const canSave = Boolean(
    transport && loadedPath && dirty && !busy && !loading && !exampleBusy,
  );
  // The editor must be saved before Run. The gated runner workspace has no
  // buffer of its own, so a dirty editor buffer does not block its Run.
  const dirtyBlocksRun = mode === "editor" && dirty;
  const canRun = Boolean(
    transport &&
      path &&
      !dirtyBlocksRun &&
      !running &&
      !busy &&
      !loading &&
      !exampleBusy,
  );
  const runTitle = dirtyBlocksRun ? "Save before running" : undefined;

  const consoleLines = (
    <>
      {lines.length === 0 ? (
        <span className="script-muted">
          Output appears here. Scripts run in-process with the embedded Python
          engine.
        </span>
      ) : null}
      {lines.map((line, index) => (
        <div key={`${line.at}-${index}`} className={`script-line ${line.stream}`}>
          <span className="script-time">{line.at}</span>
          <span>{line.text}</span>
        </div>
      ))}
    </>
  );

  const consoleButton = (
    <button
      type="button"
      className="secondary-button script-console-open"
      aria-label="Console"
      onClick={() => onOpenConsole?.()}
      disabled={!onOpenConsole}
    >
      Console
      {lines.length > 0 ? (
        <span className="script-console-count" aria-hidden="true">
          {lines.length}
        </span>
      ) : null}
    </button>
  );

  const errorBanner = error ? (
    <div className="script-error">
      <Banner tone="error" onDismiss={() => setError(null)}>
        {error}
      </Banner>
    </div>
  ) : null;

  const exampleOptions = examples.map((example) => (
    <option key={example.filename} value={example.filename}>
      {example.filename}
    </option>
  ));

  const editorToolbar = (
    <div className="script-toolbar">
      <div className="script-path" title={path}>
        {path || "No script selected"}
      </div>
      <button
        type="button"
        className="secondary-button"
        onClick={() => void browse()}
        disabled={!transport}
      >
        Open…
      </button>
      <button
        type="button"
        className="secondary-button"
        onClick={() => void save()}
        disabled={!canSave}
        title="Save (Ctrl/Cmd+S)"
      >
        Save
      </button>
      <button
        type="button"
        className="secondary-button"
        onClick={reload}
        disabled={!transport || !path || editorBusy}
      >
        Reload
      </button>
      {running ? (
        <button
          type="button"
          className="secondary-button"
          onClick={() => void stop()}
        >
          Stop
        </button>
      ) : (
        <button
          type="button"
          className="primary-button"
          onClick={() => void run()}
          disabled={!canRun}
          title={runTitle}
        >
          Run
        </button>
      )}
      {consoleButton}
      <Toggle checked={autoRun} onChange={setAutoRun} label="Auto-run on save" />
      <div className="script-example-picker">
        <select
          aria-label="Scripting example"
          value={exampleId}
          onChange={(event) =>
            handleExampleChange(event.target.value, event.currentTarget)
          }
          disabled={!transport || exampleBusy || examplesLoading}
        >
          <option value="">Load example…</option>
          {exampleOptions}
        </select>
      </div>
    </div>
  );

  const editorWorkspace = (
    <div className="script-workspace">
      {errorBanner}
      {editorToolbar}
      {loading ? (
        <div className="script-loading" role="status">
          Loading script…
        </div>
      ) : null}
      {exampleId ? (
        <div
          className="script-example-hint"
          data-testid="scripting-example-hint"
        >
          Bundled example: <code>{exampleId}</code>
        </div>
      ) : null}
      {path ? (
        <>
          <ScriptEditor
            value={editorText}
            dirty={dirty}
            busy={editorBusy}
            onChange={setEditorText}
            onSave={() => void save()}
          />
        </>
      ) : (
        <EmptyState
          icon="⌘"
          title="No script selected"
          description="Open a local Python file, or start from a bundled example, then edit and run it in-process against the display API."
          actions={
            <>
              <button
                type="button"
                className="primary-button"
                onClick={() => void browse()}
                disabled={!transport}
              >
                Open script…
              </button>
              <select
                aria-label="Load example"
                className="script-empty-example"
                value=""
                onChange={(event) =>
                  handleExampleChange(event.target.value, event.currentTarget)
                }
                disabled={!transport || exampleBusy || examplesLoading}
              >
                <option value="">Load example…</option>
                {exampleOptions}
              </select>
            </>
          }
          hint="Runs use the embedded Python engine; auto-run watches the file while enabled."
        />
      )}
    </div>
  );

  const runnerWorkspace = (
    <div className="script-runner">
      {errorBanner}
      <div className="script-toolbar">
        <div className="script-example-picker">
          <select
            aria-label="Runner example"
            value={exampleId}
            onChange={(event) =>
              handleExampleChange(event.target.value, event.currentTarget)
            }
            disabled={!transport || exampleBusy || examplesLoading}
          >
            <option value="">Load example…</option>
            {exampleOptions}
          </select>
        </div>
        <div className="script-path" title={path}>
          {path || "No script selected"}
        </div>
        <Toggle checked={autoRun} onChange={setAutoRun} label="Auto-run on save" />
        <button
          type="button"
          className="secondary-button"
          onClick={() => void browse()}
          disabled={!transport}
        >
          Choose…
        </button>
        {running ? (
          <button
            type="button"
            className="secondary-button"
            onClick={() => void stop()}
          >
            Stop
          </button>
        ) : (
          <button
            type="button"
            className="primary-button"
            onClick={() => void run()}
            disabled={!canRun}
            title={runTitle}
          >
            Run
          </button>
        )}
        {consoleButton}
      </div>
      {exampleId ? (
        <div className="script-example-hint" data-testid="scripting-example-hint">
          Bundled example: <code>{exampleId}</code>
        </div>
      ) : null}
      {path ? (
        <div
          className="script-console script-runner-console"
          role="log"
          aria-label="Script output"
          ref={consoleRef}
        >
          {consoleLines}
        </div>
      ) : (
        <EmptyState
          icon="⌘"
          title="No script selected"
          description="Load a bundled example from the toolbar, or choose a local Python file. It runs in-process against the UI-free core APIs."
          actions={
            <button
              type="button"
              className="primary-button"
              onClick={() => void browse()}
              disabled={!transport}
            >
              Choose script…
            </button>
          }
          hint="Runs use the embedded Python engine; auto-run watches the file while enabled."
        />
      )}
    </div>
  );

  return (
    <DisplayFrame
      title={display.name}
      kind="spec"
      state={{ kind: "ready" }}
      footerHints={
        dirty
          ? "Save before running (Ctrl/Cmd+S) · output opens in the Console child panel"
          : "Embedded Python · output opens in the Console child panel"
      }
      footerRight={path ? fileName(path) : "no script"}
      actions={
        runnerWorkspaceEnabled ? (
          <Segmented
            label="Scripting mode"
            value={mode}
            options={[
              { value: "editor", label: "Editor" },
              { value: "runner", label: "Runner" },
            ]}
            onChange={setMode}
          />
        ) : undefined
      }
    >
      {mode === "runner" && runnerWorkspaceEnabled
        ? runnerWorkspace
        : editorWorkspace}
    </DisplayFrame>
  );
}

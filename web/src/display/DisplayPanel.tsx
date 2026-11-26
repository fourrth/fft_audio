import { createContext, useContext, useEffect, useRef, useState } from "react";
import type { IDockviewPanelProps } from "dockview-react";
import type { Transport } from "../transport/types";
import { seriesColor, type Cut, type LineSpec } from "../cuts/model";
import { LineCutsPanel } from "../cuts/LineCutsPanel";
import { OrderCutsPanel } from "../cuts/OrderCutsPanel";
import { readLines, readLineSettings, readOrderSettings } from "../cuts/settings";
import { ScriptingPanel } from "../scripting/ScriptingPanel";
import { ScriptConsolePanel } from "../scripting/ScriptConsolePanel";
import { PlotPanel } from "../plot/PlotPanel";
import { RecorderPanel } from "../recorder/RecorderPanel";
import { fileName } from "../launcher/recent";
import {
  SpectrogramDisplay,
  type SpectrogramPhase,
} from "../spectrogram/SpectrogramDisplay";
import type {
  SpectrogramLineCut,
  SpectrogramOrderSource,
} from "../spectrogram/overlays";
import type { SpectrogramPayload } from "../spectrogram/types";
import type { DisplayProgressView } from "./DisplayFrame";
import type { ShellDisplay } from "../shell/session";
import { shouldAttachAnalysis } from "../shell/adoption";
import {
  readAnalysisSettings,
  readDisplaySettings,
} from "../settings/model";
import { useToast } from "../ui/toast";
import type { ChildKind } from "./children";

const EMPTY_PALETTE: string[] = [];

export interface LineCutSibling {
  panelId: string;
  displayId: number;
  name: string;
}

export interface DisplayActions {
  recentFiles: string[];
  transport: Transport | null;
  /** Whether this build offers the Scripting editor/console. */
  scriptingEnabled: boolean;
  plotPalette: string[];
  browseForAudio: () => Promise<string | null>;
  openPath: (panelId: string, path: string) => void;
  savePng: (dataUrl: string, filename: string) => Promise<string | null>;
  exportCuts: (cuts: Cut[]) => Promise<string | null>;
  spawnChild: (
    panelId: string,
    kind: ChildKind,
    initialState?: Record<string, unknown>,
    options?: { focus?: boolean },
  ) => void;
  lineCutSiblings: (panelId: string) => LineCutSibling[];
  moveLineCut: (
    fromPanelId: string,
    lineIndex: number,
    toPanelId: string | null,
  ) => void;
  updateLineCut: (
    panelId: string,
    lineIndex: number,
    line: LineSpec,
  ) => void;
  saveSession: () => void;
  toggleInspector: (displayId: number) => void;
}

export const DisplayActionsContext = createContext<DisplayActions | null>(null);

function pathOf(display: ShellDisplay | undefined): string | null {
  const value = display?.state.path;
  return typeof value === "string" && value.length > 0 ? value : null;
}

function errorText(reason: unknown): string {
  return reason instanceof Error ? reason.message : String(reason);
}

/** Dockview panel adapter for Spectrogram and (Stage 9) child displays. */
export function DisplayPanel(props: IDockviewPanelProps<{ display: ShellDisplay }>) {
  const display = props.params?.display;
  const actions = useContext(DisplayActionsContext);
  const { pushToast } = useToast();
  const path = pathOf(display);
  // Only Spectrograms have an analysis to attach; backend plot displays keep
  // their table path in state instead.
  const attach =
    display?.kind === "spectrogram" ? shouldAttachAnalysis(display) : false;
  const [phase, setPhase] = useState<SpectrogramPhase>(
    path || attach ? "loading" : "empty",
  );
  const [payload, setPayload] = useState<SpectrogramPayload | null>(null);
  const [progress, setProgress] = useState<DisplayProgressView | null>(null);
  const [analysisError, setAnalysisError] = useState<string | null>(null);
  const [runId, setRunId] = useState(0);
  const [browseError, setBrowseError] = useState<string | null>(null);
  const [childOverlays, setChildOverlays] = useState<{
    lines: SpectrogramLineCut[];
    orders: SpectrogramOrderSource[];
  }>({ lines: [], orders: [] });
  const cancelled = useRef(false);

  const transport = actions?.transport ?? null;
  const palette = actions?.plotPalette;
  const isSpectrogram = display?.kind === "spectrogram";
  const analysisSettings = readAnalysisSettings(display?.state ?? {});
  const settingsKey = JSON.stringify(analysisSettings);

  useEffect(() => {
    if (!isSpectrogram || !display) {
      setChildOverlays({ lines: [], orders: [] });
      return;
    }
    const api = props.containerApi;
    const parentId = display.id;
    let childSubscriptions: Array<{ dispose(): void }> = [];

    const readOverlays = () => {
      const colors = palette ?? EMPTY_PALETTE;
      const lines: SpectrogramLineCut[] = [];
      const orders: SpectrogramOrderSource[] = [];
      for (const panel of api.panels) {
        const child = (
          panel.params as { display?: ShellDisplay } | undefined
        )?.display;
        if (!child || child.parent !== parentId) {
          continue;
        }
        if (child.kind === "line_cuts") {
          const lineSettings = readLineSettings(child.state);
          readLines(child.state).forEach((line, lineIndex) => {
            lines.push({
              panelId: panel.id,
              lineIndex,
              line,
              color: seriesColor(colors, lineIndex),
              snap: lineSettings.snap,
              snapDeg: lineSettings.snap_deg,
            });
          });
        } else if (child.kind === "order_cuts") {
          orders.push({
            panelId: panel.id,
            settings: readOrderSettings(child.state),
          });
        }
      }
      setChildOverlays({ lines, orders });
    };

    const subscribeChildren = () => {
      for (const subscription of childSubscriptions) {
        subscription.dispose();
      }
      childSubscriptions = [];
      for (const panel of api.panels) {
        const child = (
          panel.params as { display?: ShellDisplay } | undefined
        )?.display;
        if (child?.parent === parentId) {
          childSubscriptions.push(
            panel.api.onDidParametersChange(() => readOverlays()),
          );
        }
      }
    };

    const layout = api.onDidLayoutChange(() => {
      readOverlays();
      subscribeChildren();
    });
    subscribeChildren();
    readOverlays();
    return () => {
      layout.dispose();
      for (const subscription of childSubscriptions) {
        subscription.dispose();
      }
    };
  }, [display?.id, isSpectrogram, palette, props.containerApi]);

  useEffect(() => {
    if (!isSpectrogram || !display || !attach) {
      return;
    }
    if (!transport) {
      setPhase("loading");
      return;
    }

    let disposed = false;
    let retryTimer: number | undefined;
    const displayId = display.id;

    const clearRetry = () => {
      if (retryTimer !== undefined) {
        window.clearTimeout(retryTimer);
        retryTimer = undefined;
      }
    };
    const scheduleRetry = () => {
      if (disposed || retryTimer !== undefined) {
        return;
      }
      // Script-created displays may be published before their analysis job is
      // ready, and adapter job events can be lost when the display and job are
      // registered through different paths. Poll until the payload exists.
      retryTimer = window.setTimeout(() => {
        retryTimer = undefined;
        fetchPayload();
      }, 100);
    };
    const fetchPayload = () => {
      void transport
        .invoke({ name: "spectrogram.get", args: { displayId } })
        .then((result) => {
          if (!disposed) {
            clearRetry();
            setPayload(result);
            setProgress(null);
            setPhase("ready");
          }
        })
        .catch((reason: unknown) => {
          if (disposed) {
            return;
          }
          const message = errorText(reason);
          // A pending/shared analysis is expected while attaching; a done
          // event (possibly addressed to another display sharing this
          // analysis) triggers the next fetch.
          if (/not ready|not found/i.test(message)) {
            scheduleRetry();
            return;
          }
          clearRetry();
          setAnalysisError(message);
          setPhase("error");
        });
    };

    setPhase("loading");
    setPayload(null);
    setAnalysisError(null);
    setProgress(null);

    const offProgress = transport.on("analysis.progress", (event) => {
      if (!disposed && event.displayId === displayId) {
        setProgress({
          framesDone: event.framesDone,
          framesTotal: event.framesTotal,
          etaSeconds: event.etaSeconds,
        });
      }
    });
    const offDone = transport.on("analysis.done", () => {
      // A backend display may share an analysis whose job events still carry
      // the original display id, so any done can materialize this payload.
      if (!disposed) {
        clearRetry();
        fetchPayload();
      }
    });
    const offError = transport.on("analysis.error", (event) => {
      if (!disposed && event.displayId === displayId) {
        clearRetry();
        setAnalysisError(event.message);
        setPhase("error");
      }
    });

    fetchPayload();

    return () => {
      disposed = true;
      clearRetry();
      offProgress();
      offDone();
      offError();
      // Backend/script analyses are owned by the app; never cancel on unmount.
    };
  }, [attach, display?.id, isSpectrogram, runId, transport]);

  useEffect(() => {
    if (!isSpectrogram || attach) {
      return;
    }
    if (!path) {
      setPhase("empty");
      setPayload(null);
      setProgress(null);
      return;
    }
    if (!transport) {
      setPhase("empty");
      return;
    }

    let disposed = false;
    cancelled.current = false;
    setPhase("loading");
    setPayload(null);
    setAnalysisError(null);
    setProgress({ framesDone: 0, framesTotal: null, etaSeconds: null });

    const offProgress = transport.on("analysis.progress", (event) => {
      if (!disposed && event.displayId === display?.id) {
        setProgress({
          framesDone: event.framesDone,
          framesTotal: event.framesTotal,
          etaSeconds: event.etaSeconds,
        });
      }
    });
    const offDone = transport.on("analysis.done", (event) => {
      if (disposed || event.displayId !== display?.id) {
        return;
      }
      void transport
        .invoke({
          name: "spectrogram.get",
          args: { displayId: event.displayId },
        })
        .then((result) => {
          if (!disposed) {
            setPayload(result);
            setPhase("ready");
          }
        })
        .catch((reason: unknown) => {
          if (!disposed) {
            setAnalysisError(errorText(reason));
            setPhase("error");
          }
        });
    });
    const offError = transport.on("analysis.error", (event) => {
      if (
        !disposed &&
        !cancelled.current &&
        event.displayId === display?.id
      ) {
        setAnalysisError(event.message);
        setPhase("error");
      }
    });

    void transport
      .invoke({
        name: "spectrogram.open",
        args: { displayId: display!.id, path, settings: analysisSettings },
      })
      .catch((reason: unknown) => {
        if (!disposed) {
          setAnalysisError(errorText(reason));
          setPhase("error");
        }
      });

    return () => {
      disposed = true;
      offProgress();
      offDone();
      offError();
      void transport
        .invoke({ name: "analysis.cancel", args: { displayId: display!.id } })
        .catch(() => {});
    };
  }, [
    attach,
    display?.id,
    display?.kind,
    isSpectrogram,
    path,
    runId,
    settingsKey,
    transport,
  ]);

  useEffect(() => {
    if (!payload) {
      return;
    }
    const current = props.params?.display;
    if (!current) {
      return;
    }
    const { analysisSettings, ...analysisMeta } = payload.meta;
    const existing = current.state.analysisMeta as
      | { frames?: number; bins?: number; sampleRate?: number; table?: unknown }
      | undefined;
    const metaMatches =
      existing &&
      existing.frames === analysisMeta.frames &&
      existing.bins === analysisMeta.bins &&
      existing.sampleRate === analysisMeta.sampleRate &&
      JSON.stringify(existing.table ?? null) ===
        JSON.stringify(analysisMeta.table ?? null);
    const hasSettings = current.state.analysisSettings !== undefined;
    if (metaMatches && (!analysisSettings || hasSettings)) {
      return;
    }
    props.api.updateParameters({
      display: {
        ...current,
        state: {
          ...current.state,
          analysisMeta,
          ...(analysisSettings ? { analysisSettings } : {}),
        },
      },
    });
    // The payload is the trigger; updating params must not loop this effect.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [payload]);

  const browse = async () => {
    if (!actions) {
      return;
    }
    setBrowseError(null);
    try {
      const picked = await actions.browseForAudio();
      // Only a chosen path starts the loading phase. Opening or dismissing the
      // picker leaves the current empty/ready state untouched.
      if (picked) {
        actions.openPath(props.api.id, picked);
      }
    } catch (reason) {
      const message = errorText(reason);
      setBrowseError(message);
      pushToast({
        tone: "error",
        message: `Could not open the picker: ${message}`,
      });
    }
  };

  const cancel = () => {
    cancelled.current = true;
    if (transport && display) {
      void transport
        .invoke({ name: "analysis.cancel", args: { displayId: display.id } })
        .catch(() => {});
    }
    if (payload) {
      setPhase("ready");
    } else {
      setPhase("empty");
    }
  };

  const exportImage = () => {
    window.dispatchEvent(
      new CustomEvent("fft:export-image", {
        detail: { panelId: props.api.id },
      }),
    );
  };

  const updateChildState = (patch: Record<string, unknown>) => {
    if (!display) {
      return;
    }
    props.api.updateParameters({
      display: {
        ...display,
        state: { ...display.state, ...patch },
      },
    });
    actions?.saveSession();
  };

  const exportCuts = async (cutsToSave: Cut[]) => {
    if (!actions) {
      return;
    }
    try {
      const saved = await actions.exportCuts(cutsToSave);
      if (saved) {
        pushToast({ tone: "success", message: `Exported ${fileName(saved)}` });
      }
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `CSV export failed: ${errorText(reason)}`,
      });
    }
  };

  if (display && display.kind === "scripting") {
    if (!actions?.scriptingEnabled) {
      // Defense in depth: a restored session must never mount the runner in a
      // build without the scripting UI, even if a panel slipped through.
      return (
        <div className="panel-empty" data-testid="scripting-unavailable">
          Scripting is not included in this build
        </div>
      );
    }
    return (
      <ScriptingPanel
        display={display}
        panelId={props.api.id}
        transport={transport}
        onPathChange={(path) => updateChildState({ path })}
        onOpenConsole={(options) =>
          actions?.spawnChild(
            props.api.id,
            "script_console",
            undefined,
            options,
          )
        }
      />
    );
  }

  if (display && display.kind === "script_console") {
    if (!actions?.scriptingEnabled) {
      return (
        <div className="panel-empty" data-testid="script-console-unavailable">
          Scripting is not included in this build
        </div>
      );
    }
    return <ScriptConsolePanel display={display} transport={transport} />;
  }

  if (display && display.kind === "recorder") {
    return (
      <RecorderPanel
        display={display}
        transport={transport}
        onRecorded={(recordedPath) =>
          actions?.openPath(props.api.id, recordedPath)
        }
      />
    );
  }

  if (display && display.kind === "plot") {
    return (
      <PlotPanel
        display={display}
        panelId={props.api.id}
        transport={transport}
        palette={actions?.plotPalette ?? []}
        onStateChange={updateChildState}
        onSavePng={actions?.savePng}
      />
    );
  }

  if (display && display.kind === "order_cuts") {
    return (
      <OrderCutsPanel
        display={display}
        panelId={props.api.id}
        parentId={display.parent}
        transport={transport}
        palette={actions?.plotPalette ?? []}
        onStateChange={updateChildState}
        onSavePng={actions?.savePng}
        onToggleInspector={
          display ? () => actions?.toggleInspector(display.id) : undefined
        }
        onExport={(cutsToSave) => void exportCuts(cutsToSave)}
      />
    );
  }

  if (display && display.kind === "line_cuts") {
    return (
      <LineCutsPanel
        display={display}
        panelId={props.api.id}
        parentId={display.parent}
        transport={transport}
        palette={actions?.plotPalette ?? []}
        onStateChange={updateChildState}
        onSavePng={actions?.savePng}
        onToggleInspector={
          display ? () => actions?.toggleInspector(display.id) : undefined
        }
        onExport={(cutsToSave) => void exportCuts(cutsToSave)}
        lineCutSiblings={(panelId) =>
          actions?.lineCutSiblings(panelId) ?? []
        }
        moveLineCut={(fromPanelId, lineIndex, toPanelId) =>
          actions?.moveLineCut(fromPanelId, lineIndex, toPanelId)
        }
      />
    );
  }

  const visibleError = browseError ?? analysisError;

  return (
    <SpectrogramDisplay
      panelId={props.api.id}
      displayId={display?.id}
      title={display?.name ?? props.api.title ?? "Spectrogram"}
      path={path}
      phase={phase}
      payload={payload}
      progress={progress}
      error={visibleError}
      recentFiles={actions?.recentFiles ?? []}
      displaySettings={readDisplaySettings(display?.state ?? {})}
      onDisplaySettingsChange={(next) =>
        updateChildState({ displaySettings: next })
      }
      onToggleInspector={
        display ? () => actions?.toggleInspector(display.id) : undefined
      }
      onBrowse={() => void browse()}
      onOpenPath={(value) => actions?.openPath(props.api.id, value)}
      onOpenRecent={(value) => actions?.openPath(props.api.id, value)}
      onCancel={attach ? undefined : cancel}
      onRetry={() => setRunId((value) => value + 1)}
      onExport={() => void exportImage()}
      onSavePng={actions?.savePng}
      onReanalyse={() => setRunId((value) => value + 1)}
      onSpawnChild={(kind, initialState) =>
        actions?.spawnChild(props.api.id, kind, initialState)
      }
      lineCutOverlays={childOverlays.lines}
      orderSources={childOverlays.orders}
      palette={palette ?? EMPTY_PALETTE}
      onUpdateLineCut={(panelId, lineIndex, line) =>
        actions?.updateLineCut(panelId, lineIndex, line)
      }
    />
  );
}

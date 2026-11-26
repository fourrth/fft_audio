import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { lineAtPoint, seriesColor, type LineSpec } from "../cuts/model";
import {
  orderPolyline,
  orderTrackingLabel,
  parseOrderList,
} from "../cuts/rpm";
import { fileName } from "../launcher/recent";
import { DisplayFrame, type DisplayProgressView, type DisplayState } from "../display/DisplayFrame";
import { IconButton } from "../ui/IconButton";
import { PlotContextMenu } from "../ui/PlotContextMenu";
import { Segmented } from "../ui/Segmented";
import { useToast } from "../ui/toast";
import { ColorBar } from "./ColorBar";
import {
  fitView,
  freqAtFraction,
  withLogFreq,
  zoomView,
  type FrequencyBounds,
  type Viewport,
} from "./geometry";
import type { DisplaySettings } from "../settings/model";
import { MiniMap } from "./MiniMap";
import { formatHz, formatTime } from "./ticks";
import {
  type SpectrogramLineCut,
  type SpectrogramOrderGuide,
  type SpectrogramOrderSource,
} from "./overlays";
import { SpectrogramCanvas } from "./SpectrogramCanvas";
import { decodeSpectrogram, dbAt, type SpectrogramPayload } from "./types";
import "./spectrogram.css";

export type SpectrogramPhase = "empty" | "loading" | "ready" | "error";

export interface SpectrogramDisplayProps {
  panelId?: string;
  displayId?: number;
  title: string;
  path: string | null;
  phase: SpectrogramPhase;
  payload: SpectrogramPayload | null;
  progress?: DisplayProgressView | null;
  error?: string | null;
  recentFiles?: string[];
  stale?: boolean;
  orders?: readonly number[];
  rpm?: number | null;
  displaySettings?: DisplaySettings;
  onDisplaySettingsChange?: (settings: DisplaySettings) => void;
  onToggleInspector?: () => void;
  onBrowse?: () => void;
  onOpenPath?: (path: string) => void;
  onOpenRecent?: (path: string) => void;
  onCancel?: () => void;
  onRetry?: () => void;
  onExport?: () => void;
  onSavePng?: (dataUrl: string, filename: string) => Promise<string | null>;
  onReanalyse?: () => void;
  onSpawnChild?: (
    kind: "order_cuts" | "line_cuts",
    initialState?: Record<string, unknown>,
  ) => void;
  lineCutOverlays?: SpectrogramLineCut[];
  orderSources?: SpectrogramOrderSource[];
  palette?: string[];
  onUpdateLineCut?: (
    panelId: string,
    lineIndex: number,
    line: LineSpec,
  ) => void;
}

function pngFilename(title: string): string {
  const safe = title.replace(/[\\/:*?"<>|]+/g, "-").trim();
  return `${safe || "spectrogram"}.png`;
}

export function SpectrogramDisplay({
  panelId,
  displayId,
  title,
  path,
  phase,
  payload,
  progress,
  error,
  recentFiles = [],
  stale = false,
  orders = [],
  rpm,
  displaySettings,
  onDisplaySettingsChange,
  onToggleInspector,
  onBrowse,
  onOpenPath,
  onOpenRecent,
  onCancel,
  onRetry,
  onExport,
  onSavePng,
  onReanalyse,
  onSpawnChild,
  lineCutOverlays = [],
  orderSources = [],
  palette = [],
  onUpdateLineCut,
}: SpectrogramDisplayProps) {
  const grid = useMemo(() => {
    if (!payload) {
      return null;
    }
    try {
      return decodeSpectrogram(payload);
    } catch {
      return null;
    }
  }, [payload]);

  const orderGuides = useMemo<SpectrogramOrderGuide[]>(() => {
    if (!grid) {
      return [];
    }
    const guides: SpectrogramOrderGuide[] = [];
    for (const source of orderSources) {
      const orders = parseOrderList(source.settings.orders);
      orders.forEach((order, index) => {
        const points = orderPolyline(
          order,
          source.settings,
          grid.meta.durationS,
          grid.meta.nyquist,
        );
        if (points.length > 0) {
          guides.push({
            id: `${source.panelId}-order-${order}`,
            panelId: source.panelId,
            order,
            color: seriesColor(palette, index),
            points,
          });
        }
      });
    }
    return guides;
  }, [grid, orderSources, palette]);

  const trackingLabels = useMemo(
    () => orderSources.map((source) => orderTrackingLabel(1, source.settings)),
    [orderSources],
  );

  const [internalLog, setInternalLog] = useState(true);
  const logFreq = displaySettings?.log_freq ?? internalLog;
  const [internalGrid, setInternalGrid] = useState(true);
  const showGrid = displaySettings?.show_grid ?? internalGrid;
  const rangeDb = Math.max(1, displaySettings?.range_db ?? 80);
  const minHz = displaySettings?.min_hz ?? 0;
  const dbMin = displaySettings?.db_min ?? null;
  const dbMax = displaySettings?.db_max ?? null;
  const [view, setView] = useState<Viewport | null>(null);
  const viewRef = useRef<HTMLDivElement>(null);
  const { pushToast } = useToast();
  const [colorMin, setColorMin] = useState(-80);
  const [colorMax, setColorMax] = useState(0);
  const [readoutText, setReadoutText] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<{
    x: number;
    y: number;
    point: { time: number; hz: number } | null;
    readout: string | null;
  } | null>(null);
  const [minimapVisible, setMinimapVisible] = useState(true);
  const minimapHideTimer = useRef<number | undefined>(undefined);
  const minimapArmed = useRef(false);

  const showMinimap = useCallback(() => {
    setMinimapVisible(true);
    if (minimapHideTimer.current !== undefined) {
      window.clearTimeout(minimapHideTimer.current);
    }
    minimapHideTimer.current = window.setTimeout(() => {
      setMinimapVisible(false);
      minimapHideTimer.current = undefined;
    }, 2500);
  }, []);

  useEffect(
    () => () => {
      if (minimapHideTimer.current !== undefined) {
        window.clearTimeout(minimapHideTimer.current);
      }
    },
    [],
  );

  useEffect(() => {
    if (!grid) {
      return;
    }
    minimapArmed.current = false;
    setMinimapVisible(true);
    if (minimapHideTimer.current !== undefined) {
      window.clearTimeout(minimapHideTimer.current);
      minimapHideTimer.current = undefined;
    }
  }, [grid]);

  useEffect(() => {
    if (!view) {
      return;
    }
    if (!minimapArmed.current) {
      minimapArmed.current = true;
      return;
    }
    showMinimap();
  }, [showMinimap, view]);

  const topHz =
    displaySettings?.max_hz && displaySettings.max_hz > 0 && grid
      ? Math.min(displaySettings.max_hz, grid.meta.nyquist)
      : grid?.meta.nyquist ?? 1;
  const bounds: FrequencyBounds = { minHz, maxHz: topHz };

  // Stable color domain with headroom above/below the data so both handles
  // can be dragged outward as well as inward. Do not add padding around the
  // committed bounds, or the handles would jump outward on pointer release.
  const colorDomain = useMemo(() => {
    if (!grid) {
      return { minDb: -100, maxDb: 0 };
    }
    const dataSpan = Math.max(1, grid.meta.maxDb - grid.meta.minDb);
    const pad = Math.max(10, dataSpan * 0.1);
    const minDb = Math.min(
      grid.meta.minDb - pad,
      grid.meta.maxDb - rangeDb,
      dbMin ?? Number.POSITIVE_INFINITY,
    );
    const maxDb = Math.max(
      grid.meta.maxDb + pad,
      dbMax ?? Number.NEGATIVE_INFINITY,
    );
    return { minDb, maxDb: maxDb > minDb ? maxDb : minDb + 1 };
  }, [dbMax, dbMin, grid, rangeDb]);

  useEffect(() => {
    if (!grid) {
      setView(null);
      return;
    }
    setView(fitView(grid.meta.durationS, topHz, logFreq, minHz));
    // Reset only when a new analysis / axis limit arrives.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [grid, minHz, topHz]);

  useEffect(() => {
    setView((current) =>
      current && current.logFreq !== logFreq
        ? withLogFreq(current, logFreq, topHz, minHz)
        : current,
    );
  }, [grid, logFreq, minHz, topHz]);

  useEffect(() => {
    if (grid) {
      const top = dbMax ?? grid.meta.maxDb;
      setColorMax(top);
      setColorMin(dbMin ?? top - rangeDb);
    }
  }, [dbMax, dbMin, grid, rangeDb]);

  const resetColorRange = () => {
    if (displaySettings && onDisplaySettingsChange) {
      onDisplaySettingsChange({
        ...displaySettings,
        db_min: null,
        db_max: null,
      });
    }
    if (grid) {
      setColorMax(grid.meta.maxDb);
      setColorMin(grid.meta.maxDb - rangeDb);
    }
  };

  const commitColorRange = (min: number, max: number) => {
    if (displaySettings && onDisplaySettingsChange) {
      onDisplaySettingsChange({
        ...displaySettings,
        db_min: min,
        db_max: max,
      });
    }
  };

  const fit = () => {
    if (grid) {
      setView(fitView(grid.meta.durationS, topHz, logFreq, minHz));
    }
  };

  const zoomBy = (factor: number) => {
    if (!grid || !view) {
      return;
    }
    setView(
      zoomView(
        view,
        factor,
        (view.t0 + view.t1) / 2,
        freqAtFraction(0.5, view),
        grid.meta.durationS,
        grid.meta.nyquist,
        logFreq,
        bounds,
      ),
    );
  };

  useEffect(() => {
    if (phase !== "ready") {
      return;
    }
    const onFit = (event: Event) => {
      const detail = (event as CustomEvent<{ panelId?: string }>).detail;
      if (detail?.panelId && panelId && detail.panelId !== panelId) {
        return;
      }
      fit();
    };
    window.addEventListener("fft:fit-view", onFit);
    return () => window.removeEventListener("fft:fit-view", onFit);
  }, [grid, logFreq, minHz, panelId, phase, topHz]);

  const changeLog = (next: boolean) => {
    if (displaySettings && onDisplaySettingsChange) {
      onDisplaySettingsChange({ ...displaySettings, log_freq: next });
    } else {
      setInternalLog(next);
    }
    if (grid && view) {
      setView(withLogFreq(view, next, topHz, minHz));
    }
  };

  const changeGrid = (next: boolean) => {
    if (displaySettings && onDisplaySettingsChange) {
      onDisplaySettingsChange({ ...displaySettings, show_grid: next });
    } else {
      setInternalGrid(next);
    }
  };

  let state: DisplayState;
  if (phase === "loading") {
    state = {
      kind: "loading",
      progress: progress ?? { framesDone: 0, framesTotal: null },
      onCancel,
    };
  } else if (phase === "error") {
    state = {
      kind: "error",
      message: error ?? "The analysis failed.",
      onRetry,
      onChooseFile: onBrowse,
    };
  } else if (phase === "empty") {
    state = {
      kind: "empty",
      recentFiles,
      onBrowse,
      onOpenPath,
      onOpenRecent,
    };
  } else if (!grid || !view) {
    state = {
      kind: "error",
      message: "The spectrogram payload could not be decoded.",
      onRetry,
      onChooseFile: onBrowse,
    };
  } else {
    state = { kind: "ready" };
  }

  const quickExport = useCallback(async () => {
    const canvas = viewRef.current?.querySelector<HTMLCanvasElement>(
      "canvas.plot-canvas",
    );
    if (!canvas) {
      return;
    }
    try {
      const dataUrl = canvas.toDataURL("image/png");
      const filename = pngFilename(title);
      if (onSavePng) {
        const saved = await onSavePng(dataUrl, filename);
        if (saved) {
          pushToast({ tone: "success", message: `Exported ${filename}` });
        }
        return;
      }
      onExport?.();
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `Export failed: ${
          reason instanceof Error ? reason.message : String(reason)
        }`,
      });
    }
  }, [onExport, onSavePng, pushToast, title]);

  useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ panelId?: string }>).detail;
      if (detail?.panelId && panelId && detail.panelId !== panelId) {
        return;
      }
      void quickExport();
    };
    window.addEventListener("fft:export-image", handler);
    return () => window.removeEventListener("fft:export-image", handler);
  }, [panelId, quickExport]);

  const actions =
    state.kind === "ready" ? (
      <>
        <Segmented
          label="Frequency axis"
          value={logFreq ? "log" : "linear"}
          options={[
            { value: "log", label: "Log" },
            { value: "linear", label: "Linear" },
          ]}
          onChange={(value) => changeLog(value === "log")}
        />
        <IconButton
          label="Grid lines"
          pressed={showGrid}
          onClick={() => changeGrid(!showGrid)}
        >
          ▦
        </IconButton>
        <IconButton label="Zoom out" onClick={() => zoomBy(1.25)}>
          −
        </IconButton>
        <IconButton label="Zoom in" onClick={() => zoomBy(0.8)}>
          ＋
        </IconButton>
        <IconButton label="Fit view" kbd="F" onClick={fit}>
          ⤢
        </IconButton>
        <IconButton label="Export image" onClick={() => void quickExport()}>
          ⇩
        </IconButton>
        <IconButton label="Re-analyse" onClick={onReanalyse}>
          ↻
        </IconButton>
        <IconButton
          label="Toggle inspector"
          kbd="P"
          onClick={onToggleInspector}
          disabled={!onToggleInspector}
        >
          ⓘ
        </IconButton>
      </>
    ) : phase === "loading" ? (
      <IconButton label="Cancel analysis" onClick={onCancel}>
        ×
      </IconButton>
    ) : null;

  return (
    <DisplayFrame
      displayId={displayId}
      title={title}
      kind="spec"
      state={state}
      actions={actions}
      stale={stale}
      footerHints="Wheel pan · Shift+wheel time pan · Ctrl/Cmd+wheel zoom · double-click/F fit · M maximize"
      footerRight={
        grid
          ? `${grid.meta.frames}×${grid.meta.bins} · ${grid.meta.durationS.toFixed(2)} s`
          : path
            ? fileName(path)
            : "no input"
      }
    >
      {state.kind === "ready" && grid && view ? (
        <div
          className="spectrogram-view"
          ref={viewRef}
          onContextMenu={(event) => {
            event.preventDefault();
            let point: { time: number; hz: number } | null = null;
            const target =
              event.target instanceof Element
                ? event.target.closest(".plot-canvas, .spectrogram-overlays")
                : null;
            if (grid && view && target) {
              const rect = target.getBoundingClientRect();
              if (rect.width > 0 && rect.height > 0) {
                const x = Math.min(
                  Math.max(event.clientX - rect.left, 0),
                  rect.width,
                );
                const y = Math.min(
                  Math.max(event.clientY - rect.top, 0),
                  rect.height,
                );
                point = {
                  time: view.t0 + (x / rect.width) * (view.t1 - view.t0),
                  hz: freqAtFraction(1 - y / rect.height, view),
                };
              }
            }
            const readout = point
              ? (() => {
                  const db = dbAt(grid, point.time, point.hz);
                  return `t ${point.time.toFixed(3)} s · f ${point.hz.toFixed(
                    1,
                  )} Hz · ${db === null ? "—" : `${db.toFixed(1)} dB`}`;
                })()
              : readoutText;
            setContextMenu({
              x: event.clientX,
              y: event.clientY,
              point,
              // Keep the value at the right-click position. Moving onto the
              // menu clears the live hover readout.
              readout,
            });
          }}
        >
          {trackingLabels.length > 0 ? (
            <div
              className="spectrogram-tracking"
              data-testid="spectrogram-tracking"
              aria-label="Order tracking"
            >
              {trackingLabels.map((label, index) => (
                <span
                  key={`${index}-${label}`}
                  data-testid="spectrogram-tracking-label"
                >
                  {label}
                </span>
              ))}
            </div>
          ) : null}
          <div
            className="spectrogram-scales"
            data-testid="spectrogram-scales"
            aria-label="Spectrogram scales"
          >
            <span>
              {formatHz(view.f0)}–{formatHz(view.f1)} Hz
            </span>
            <span>
              {formatTime(view.t0)}–{formatTime(view.t1)} s
            </span>
            <span>Range {Math.round(colorMax - colorMin)} dB</span>
            <span>{logFreq ? "Log frequency" : "Linear frequency"}</span>
          </div>
          <div className="spectrogram-main">
            <SpectrogramCanvas
              grid={grid}
              view={view}
              minDb={colorMin}
              maxDb={colorMax}
              domainMinDb={colorDomain.minDb}
              domainMaxDb={colorDomain.maxDb}
              minHz={minHz}
              maxHz={topHz}
              showGrid={showGrid}
              onResetView={fit}
              onViewChange={setView}
              orders={orders}
              rpm={rpm}
              onReadoutChange={setReadoutText}
              lineCuts={lineCutOverlays}
              orderGuides={orderGuides}
              onUpdateLineCut={onUpdateLineCut}
            />
            <ColorBar
              minDb={colorMin}
              maxDb={colorMax}
              domainMinDb={colorDomain.minDb}
              domainMaxDb={colorDomain.maxDb}
              onChange={(min, max) => {
                setColorMin(min);
                setColorMax(max);
              }}
              onCommit={commitColorRange}
              onReset={resetColorRange}
            />
          </div>
          <MiniMap
            grid={grid}
            view={view}
            minDb={colorMin}
            maxDb={colorMax}
            domainMinDb={colorDomain.minDb}
            domainMaxDb={colorDomain.maxDb}
            minHz={minHz}
            maxHz={topHz}
            visible={minimapVisible}
            onViewChange={setView}
            onInteraction={showMinimap}
          />
          {contextMenu ? (
            <PlotContextMenu
              x={contextMenu.x}
              y={contextMenu.y}
              label="Plot actions"
              onClose={() => setContextMenu(null)}
              readout={contextMenu.readout}
              onResetView={fit}
              onExportImage={() => void quickExport()}
            >
              <button
                type="button"
                role="menuitem"
                disabled={!contextMenu?.point}
                onClick={() => {
                  const point = contextMenu?.point;
                  setContextMenu(null);
                  if (!point || !grid) {
                    return;
                  }
                  onSpawnChild?.("line_cuts", {
                    lines: [
                      lineAtPoint(
                        point.time,
                        point.hz,
                        grid.meta.nyquist,
                        grid.meta.durationS,
                      ),
                    ],
                  });
                }}
              >
                Add line here
              </button>
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setContextMenu(null);
                  onSpawnChild?.("order_cuts");
                }}
              >
                Add frequency cut here
              </button>
            </PlotContextMenu>
          ) : null}
        </div>
      ) : null}
    </DisplayFrame>
  );
}

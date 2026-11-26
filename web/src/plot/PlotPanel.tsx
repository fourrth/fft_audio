import { useEffect, useMemo, useState } from "react";
import { DisplayFrame, type DisplayState } from "../display/DisplayFrame";
import type { ShellDisplay } from "../shell/session";
import type { Transport } from "../transport/types";
import { IconButton } from "../ui/IconButton";
import { Segmented } from "../ui/Segmented";
import { PlotContextMenu } from "../ui/PlotContextMenu";
import { CutPlot, type PlotSeries as CutSeries } from "../cuts/CutPlot";
import { seriesColor } from "../cuts/model";
import type { PlotLayout } from "./types";
import { readPlotLayout, readShowGrid } from "./settings";
import { usePlotData } from "./usePlotData";

export interface PlotPanelProps {
  display: ShellDisplay;
  panelId?: string;
  transport: Transport | null;
  palette: string[];
  onStateChange: (patch: Record<string, unknown>) => void;
  onSavePng?: (dataUrl: string, filename: string) => Promise<string | null>;
}

const LAYOUT_OPTIONS: Array<{ value: PlotLayout; label: string }> = [
  { value: "columns", label: "Shared x" },
  { value: "pairs", label: "x/y pairs" },
];

/** Generic table plot: one line per numeric y column or x/y column pair. */
export function PlotPanel({
  display,
  panelId,
  transport,
  palette,
  onStateChange,
  onSavePng,
}: PlotPanelProps) {
  const path = typeof display.state.path === "string" ? display.state.path : "";
  const layout = readPlotLayout(display.state);
  const showGrid = readShowGrid(display.state);
  const data = usePlotData(transport, path, layout);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [notesDismissed, setNotesDismissed] = useState(false);
  const [plotMenu, setPlotMenu] = useState<{ x: number; y: number } | null>(
    null,
  );

  useEffect(() => {
    setHidden(new Set());
    setSelectedId(null);
    setNotesDismissed(false);
    setPlotMenu(null);
  }, [layout, path]);

  const series: CutSeries[] = useMemo(
    () =>
      (data.payload?.series ?? []).map((line, index) => {
        const id = `line-${index}`;
        return {
          id,
          label: line.label,
          color: seriesColor(palette, index),
          hidden: hidden.has(id),
          points: line.points,
        };
      }),
    [data.payload, hidden, palette],
  );

  let state: DisplayState;
  if (data.phase === "error") {
    state = {
      kind: "error",
      message: data.error ?? "Could not read the table.",
      onRetry: data.reload,
    };
  } else if (data.phase === "loading" || !data.payload) {
    state = {
      kind: "loading",
      progress: { framesDone: 0, framesTotal: null, label: "Loading table…" },
    };
  } else {
    state = { kind: "ready" };
  }

  const yLabel =
    data.payload?.series.length === 1 ? data.payload.series[0].label : "Value";
  const footerRight = data.payload
    ? `${data.payload.series.length} ${
        data.payload.series.length === 1 ? "line" : "lines"
      } · ${data.payload.rows} ${
        data.payload.rows === 1 ? "row" : "rows"
      }`
    : "loading";

  return (
    <DisplayFrame
      title={display.name}
      kind="plot"
      state={state}
      footerHints="Drag to zoom · double-click to reset · legend toggles lines"
      footerRight={footerRight}
      actions={
        <>
          <Segmented
            label="Plot columns"
            value={layout}
            options={LAYOUT_OPTIONS}
            onChange={(value) => onStateChange({ plotLayout: value })}
          />
          <IconButton
            label="Grid lines"
            pressed={showGrid}
            onClick={() => onStateChange({ showGrid: !showGrid })}
          >
            ▦
          </IconButton>
          <IconButton
            label="Export image"
            onClick={() =>
              window.dispatchEvent(
                new CustomEvent("fft:export-image", {
                  detail: { panelId },
                }),
              )
            }
          >
            ⇩
          </IconButton>
        </>
      }
    >
      {state.kind === "ready" && data.payload ? (
        <div className="cut-panel">
          <CutPlot
            series={series}
            kind="plot"
            panelId={panelId}
            onSavePng={onSavePng}
            axisLabel={data.payload.xLabel || "x"}
            yLabel={yLabel}
            selectedId={selectedId}
            onSelect={setSelectedId}
            onToggleVisibility={(id) =>
              setHidden((current) => {
                const next = new Set(current);
                if (next.has(id)) next.delete(id);
                else next.add(id);
                return next;
              })
            }
            onPlotContextMenu={(x, y) => setPlotMenu({ x, y })}
            warning={
              !notesDismissed && data.payload.notes.length > 0
                ? data.payload.notes.join(" · ")
                : null
            }
            onDismissWarning={() => setNotesDismissed(true)}
            showGrid={showGrid}
          />

          {plotMenu ? (
            <PlotContextMenu
              x={plotMenu.x}
              y={plotMenu.y}
              label="Plot actions"
              onClose={() => setPlotMenu(null)}
              onResetView={() =>
                window.dispatchEvent(
                  new CustomEvent("fft:reset-plot-view", {
                    detail: { panelId },
                  }),
                )
              }
              onExportImage={() =>
                window.dispatchEvent(
                  new CustomEvent("fft:export-image", {
                    detail: { panelId },
                  }),
                )
              }
            />
          ) : null}
        </div>
      ) : null}
    </DisplayFrame>
  );
}

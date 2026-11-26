import { useMemo, useState } from "react";
import { DisplayFrame, type DisplayState } from "../display/DisplayFrame";
import type { LineCutSibling } from "../display/DisplayPanel";
import type { ShellDisplay } from "../shell/session";
import type { Transport } from "../transport/types";
import { IconButton } from "../ui/IconButton";
import { Segmented } from "../ui/Segmented";
import { PlotContextMenu } from "../ui/PlotContextMenu";
import { CutPlot, type PlotSeries } from "./CutPlot";
import {
  cutLabel,
  cutX,
  deleteAt,
  flipLine,
  isDegenerate,
  moveLineEndpoint,
  resolveLineAxis,
  seriesColor,
  stripLineSnapHistory,
  type Cut,
  type LineAxis,
  type LineSpec,
  type OrderAxis,
} from "./model";
import {
  readLineSettings,
  readLines,
  readShowGrid,
  type LineCutSettings,
} from "./settings";
import { useChildCuts } from "./useChildCuts";

export interface LineCutsPanelProps {
  display: ShellDisplay;
  panelId?: string;
  parentId: number | null;
  transport: Transport | null;
  palette: string[];
  onStateChange: (patch: Record<string, unknown>) => void;
  onSavePng?: (dataUrl: string, filename: string) => Promise<string | null>;
  onToggleInspector?: () => void;
  onExport: (cuts: Cut[]) => void;
  lineCutSiblings?: (panelId: string) => LineCutSibling[];
  moveLineCut?: (
    fromPanelId: string,
    lineIndex: number,
    toPanelId: string | null,
  ) => void;
}

export function LineCutsPanel({
  display,
  panelId,
  parentId,
  transport,
  palette,
  onStateChange,
  onSavePng,
  onToggleInspector,
  onExport,
  lineCutSiblings,
  moveLineCut,
}: LineCutsPanelProps) {
  const settings = readLineSettings(display.state);
  const lines = readLines(display.state);
  const showGrid = readShowGrid(display.state);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [warningDismissed, setWarningDismissed] = useState(false);
  const [editing, setEditing] = useState(false);
  const [edit, setEdit] = useState({ t0: "", f0: "", t1: "", f1: "" });
  const [lineMenu, setLineMenu] = useState<{
    lineIndex: number;
    x: number;
    y: number;
    siblings: LineCutSibling[];
  } | null>(null);
  const [plotMenu, setPlotMenu] = useState<{ x: number; y: number } | null>(
    null,
  );

  const cuts = useChildCuts(
    parentId,
    transport,
    () =>
      transport!.invoke({
        name: "lineCuts.compute",
        args: {
          parentId: parentId!,
          settings,
          lines: lines.map(stripLineSnapHistory),
        },
      }),
    JSON.stringify({
      settings,
      lines: lines.map(stripLineSnapHistory),
    }),
  );

  const axis: OrderAxis = cuts.payload
    ? resolveLineAxis(
        cuts.payload.cuts,
        settings.line_axis,
        cuts.payload.hopS,
        cuts.payload.binHz,
      )
    : "time";

  const save = (nextSettings: LineCutSettings, nextLines: LineSpec[]) => {
    onStateChange({ lineCuts: nextSettings, lines: nextLines });
  };

  const openLineMenu = (id: string, x: number, y: number) => {
    const lineIndex = Number(id.replace("cut-", ""));
    if (
      !panelId ||
      !Number.isInteger(lineIndex) ||
      lineIndex < 0 ||
      lineIndex >= lines.length
    ) {
      return;
    }
    setSelectedId(id);
    setLineMenu({
      lineIndex,
      x,
      y,
      siblings: lineCutSiblings?.(panelId) ?? [],
    });
  };

  const moveOpenLine = (toPanelId: string | null) => {
    const menu = lineMenu;
    setLineMenu(null);
    if (!menu || !panelId) {
      return;
    }
    setSelectedId(null);
    setHidden(new Set());
    moveLineCut?.(panelId, menu.lineIndex, toPanelId);
  };

  const series: PlotSeries[] = useMemo(
    () =>
      (cuts.payload?.cuts ?? []).map((cut, index) => {
        const id = `cut-${index}`;
        return {
          id,
          label: cutLabel(cut.kind),
          color: seriesColor(palette, index),
          hidden: hidden.has(id),
          points: cut.points.map((point) => ({
            x: cutX(point, axis),
            y: point.db,
          })),
        };
      }),
    [axis, cuts.payload, hidden, palette],
  );

  const selectedIndex = selectedId ? Number(selectedId.replace("cut-", "")) : -1;
  const selectedLine =
    selectedIndex >= 0 && selectedIndex < lines.length
      ? lines[selectedIndex]
      : null;

  const deleteSelected = () => {
    if (selectedIndex < 0) {
      return;
    }
    save(settings, deleteAt(lines, selectedIndex));
    setSelectedId(null);
    setEditing(false);
  };

  const flipSelected = () => {
    if (!selectedLine) {
      return;
    }
    const next = [...lines];
    next[selectedIndex] = flipLine(selectedLine);
    save(settings, next);
  };

  const moveHandle = (id: string, endpoint: "start" | "end", x: number) => {
    const index = Number(id.replace("cut-", ""));
    const line = lines[index];
    if (!line) {
      return;
    }
    const moved =
      axis === "freq"
        ? moveLineEndpoint(
            line,
            endpoint,
            endpoint === "start" ? line.t0 : line.t1,
            x,
          )
        : moveLineEndpoint(
            line,
            endpoint,
            x,
            endpoint === "start" ? line.f0 : line.f1,
          );
    const next = [...lines];
    next[index] = moved;
    save(settings, next);
  };

  const beginEdit = () => {
    if (!selectedLine) {
      return;
    }
    setEdit({
      t0: String(selectedLine.t0),
      f0: String(selectedLine.f0),
      t1: String(selectedLine.t1),
      f1: String(selectedLine.f1),
    });
    setEditing(true);
  };

  const applyEdit = () => {
    if (!selectedLine) {
      return;
    }
    const t0 = Number(edit.t0);
    const f0 = Number(edit.f0);
    const t1 = Number(edit.t1);
    const f1 = Number(edit.f1);
    if (![t0, f0, t1, f1].every(Number.isFinite)) {
      return;
    }
    const next = [...lines];
    next[selectedIndex] = stripLineSnapHistory({
      ...selectedLine,
      t0,
      f0,
      t1,
      f1,
    });
    save(settings, next);
    setEditing(false);
  };

  const degenerate = cuts.payload
    ? cuts.payload.cuts.filter((cut) =>
        isDegenerate(cut, axis, cuts.payload!.hopS, cuts.payload!.binHz),
      ).length
    : 0;
  const warning =
    !warningDismissed && cuts.payload && degenerate > 0
      ? `${degenerate} line(s) cannot be drawn on the ${axis} axis.`
      : null;

  let state: DisplayState;
  if (cuts.phase === "error") {
    state = {
      kind: "error",
      message: cuts.error ?? "Could not compute the line cuts.",
      onRetry: cuts.reload,
    };
  } else if (cuts.phase === "loading" || !cuts.payload) {
    state = { kind: "loading", progress: { framesDone: 0, framesTotal: null } };
  } else {
    state = { kind: "ready" };
  }

  const axisOptions: Array<{ value: LineAxis; label: string }> = [
    { value: "auto", label: "Auto" },
    { value: "time", label: "Time" },
    { value: "freq", label: "Freq" },
  ];

  return (
    <DisplayFrame
      title={display.name}
      kind="line"
      parentLabel="child of Spectrogram"
      state={state}
      stale={cuts.stale}
      footerHints="Click a curve to select · drag endpoint handles"
      footerRight={cuts.payload ? `${lines.length} lines` : "computing"}
      actions={
        <>
          <IconButton
            label="Delete selected line"
            onClick={deleteSelected}
            disabled={!selectedLine}
          >
            ×
          </IconButton>
          <Segmented
            label="Line cut axis"
            value={settings.line_axis}
            options={axisOptions}
            onChange={(value) =>
              save({ ...settings, line_axis: value }, lines)
            }
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
          <IconButton
            label="Export CSV"
            onClick={() => onExport(cuts.payload?.cuts ?? [])}
            disabled={!cuts.payload || cuts.payload.cuts.length === 0}
          >
            CSV
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
      }
    >
      {state.kind === "ready" && cuts.payload ? (
        <div className="cut-panel">
          <CutPlot
            series={series}
            kind="line"
            panelId={panelId}
            onSavePng={onSavePng}
            axisLabel={
              axis === "freq" ? "Frequency (Hz)" : "Time (s)"
            }
            selectedId={selectedId}
            onContextMenu={
              moveLineCut && panelId
                ? (id, x, y) => openLineMenu(id, x, y)
                : undefined
            }
            onPlotContextMenu={(x, y) => setPlotMenu({ x, y })}
            onSelect={(id) => {
              setSelectedId(id);
              setEditing(false);
            }}
            onToggleVisibility={(id) =>
              setHidden((current) => {
                const next = new Set(current);
                if (next.has(id)) next.delete(id);
                else next.add(id);
                return next;
              })
            }
            onMoveHandle={moveHandle}
            warning={warning}
            onDismissWarning={() => setWarningDismissed(true)}
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

          {lineMenu ? (
            <PlotContextMenu
              x={lineMenu.x}
              y={lineMenu.y}
              label="Line cut actions"
              onClose={() => setLineMenu(null)}
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
            >
              <div className="context-section-label">
                Move to another window
              </div>
              {lineMenu.siblings.map((sibling) => (
                <button
                  key={sibling.panelId}
                  type="button"
                  role="menuitem"
                  onClick={() => moveOpenLine(sibling.panelId)}
                >
                  {sibling.name}
                </button>
              ))}
              <button
                type="button"
                role="menuitem"
                onClick={() => moveOpenLine(null)}
              >
                New Line Cuts window
              </button>
            </PlotContextMenu>
          ) : null}

          {lines.length === 0 ? (
            <div className="cut-empty-hint" role="status">
              No lines yet. Right-click the Spectrogram and choose
              “Add line here”.
            </div>
          ) : null}

          {selectedLine ? (
            <div className="cut-action-bar" role="toolbar" aria-label="Selected line">
              <button type="button" className="text-button" onClick={deleteSelected}>
                Delete
              </button>
              <button type="button" className="text-button" onClick={flipSelected}>
                Flip
              </button>
              <button type="button" className="text-button" onClick={beginEdit}>
                Edit numbers…
              </button>
            </div>
          ) : null}

          {editing && selectedLine ? (
            <form
              className="cut-editor"
              onSubmit={(event) => {
                event.preventDefault();
                applyEdit();
              }}
            >
              {(["t0", "f0", "t1", "f1"] as const).map((field) => (
                <label key={field}>
                  {field === "t0" || field === "t1"
                    ? `${field} (s)`
                    : `${field} (Hz)`}
                  <input
                    aria-label={field}
                    type="number"
                    step="any"
                    value={edit[field]}
                    onChange={(event) =>
                      setEdit((current) => ({
                        ...current,
                        [field]: event.target.value,
                      }))
                    }
                  />
                </label>
              ))}
              <button type="submit" className="primary-button">
                Apply
              </button>
              <button
                type="button"
                className="secondary-button"
                onClick={() => setEditing(false)}
              >
                Cancel
              </button>
            </form>
          ) : null}
        </div>
      ) : null}
    </DisplayFrame>
  );
}

import { useEffect, useMemo, useState } from "react";
import { DisplayFrame, type DisplayState } from "../display/DisplayFrame";
import type { ShellDisplay } from "../shell/session";
import type { Transport } from "../transport/types";
import { IconButton } from "../ui/IconButton";
import { Segmented } from "../ui/Segmented";
import { PlotContextMenu } from "../ui/PlotContextMenu";
import { CutPlot, type PlotSeries } from "./CutPlot";
import {
  addOrderValue,
  axisFor,
  cutLabel,
  cutX,
  isDegenerate,
  seriesColor,
  type Cut,
  type OrderAxis,
} from "./model";
import {
  readOrderSettings,
  readRpmProfile,
  readShowGrid,
  type OrderCutSettings,
  type RpmSettings,
} from "./settings";
import { orderTrackingLabel, rpmProfileLabel } from "./rpm";
import { useChildCuts } from "./useChildCuts";

export interface OrderCutsPanelProps {
  display: ShellDisplay;
  panelId?: string;
  parentId: number | null;
  transport: Transport | null;
  palette: string[];
  onStateChange: (patch: Record<string, unknown>) => void;
  onSavePng?: (dataUrl: string, filename: string) => Promise<string | null>;
  onToggleInspector?: () => void;
  onExport: (cuts: Cut[]) => void;
}

function parseList(text: string): number[] {
  return text
    .split(/[,;\s]+/)
    .map((part) => Number(part))
    .filter((value) => Number.isFinite(value) && value > 0);
}

function replaceValue(text: string, oldValue: number, newValue: number): string {
  const values = parseList(text).map((value) =>
    Math.abs(value - oldValue) < 1e-9 ? newValue : value,
  );
  return values.join(",");
}

function removeValue(text: string, value: number): string {
  return parseList(text)
    .filter((item) => Math.abs(item - value) > 1e-9)
    .join(",");
}

export function OrderCutsPanel({
  display,
  panelId,
  parentId,
  transport,
  palette,
  onStateChange,
  onSavePng,
  onToggleInspector,
  onExport,
}: OrderCutsPanelProps) {
  const settings = readOrderSettings(display.state);
  const rpmProfile = readRpmProfile(display.state);
  const showGrid = readShowGrid(display.state);
  const [rpmDraft, setRpmDraft] = useState<RpmSettings>(settings.rpm);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    setRpmDraft(settings.rpm);
  }, [JSON.stringify(settings.rpm)]);
  const [hidden, setHidden] = useState<Set<string>>(new Set());
  const [warningDismissed, setWarningDismissed] = useState(false);
  const [editing, setEditing] = useState(false);
  const [editValue, setEditValue] = useState("");
  const [orderMenu, setOrderMenu] = useState<{ x: number; y: number } | null>(
    null,
  );
  const [addingOrder, setAddingOrder] = useState(false);
  const [newOrder, setNewOrder] = useState("");
  const [addOrderError, setAddOrderError] = useState<string | null>(null);

  const cuts = useChildCuts(
    parentId,
    transport,
    () =>
      transport!.invoke({
        name: "orderCuts.compute",
        args: {
          parentId: parentId!,
          settings,
          ...(rpmProfile ? { rpm: rpmProfile } : {}),
        },
      }),
    JSON.stringify({ settings, rpm: rpmProfile ?? null }),
  );

  const rpmIsConstant = !rpmProfile && settings.rpm.source === "constant";
  const resolved = axisFor(settings.axis, rpmIsConstant);
  const axis: OrderAxis = resolved.axis;

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
  const selectedCut =
    selectedIndex >= 0 ? cuts.payload?.cuts[selectedIndex] : undefined;

  const updateSettings = (next: OrderCutSettings) => {
    onStateChange({ orderCuts: next });
  };

  const applyRpm = () => {
    onStateChange({
      orderCuts: {
        ...settings,
        rpm: {
          ...rpmDraft,
          constant: Number(rpmDraft.constant) || 0,
          start: Number(rpmDraft.start) || 0,
          end: Number(rpmDraft.end) || 0,
          csv: rpmDraft.csv?.trim() ? rpmDraft.csv.trim() : null,
        },
      },
      // A manual tracking edit replaces the script-provided profile; keeping it
      // would silently override the new settings on the next compute.
      rpm: null,
    });
  };

  const closeOrderMenu = () => {
    setOrderMenu(null);
    setAddingOrder(false);
    setNewOrder("");
    setAddOrderError(null);
  };

  const openOrderMenu = (x: number, y: number) => {
    setOrderMenu({ x, y });
    setAddingOrder(false);
    setNewOrder("");
    setAddOrderError(null);
  };

  const addOrder = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const value = Number(newOrder.trim());
    if (!newOrder.trim() || !Number.isFinite(value) || value <= 0) {
      setAddOrderError("Enter a positive order value.");
      return;
    }
    const next = addOrderValue(settings.orders, value);
    if (next === null) {
      setAddOrderError("That order is already in the list.");
      return;
    }
    updateSettings({ ...settings, orders: next });
    closeOrderMenu();
  };

  const deleteCut = () => {
    if (!selectedCut) {
      return;
    }
    if (selectedCut.kind.type === "order") {
      updateSettings({
        ...settings,
        orders: removeValue(settings.orders, selectedCut.kind.value),
      });
    } else if (selectedCut.kind.type === "freq") {
      updateSettings({
        ...settings,
        freqs: removeValue(settings.freqs, selectedCut.kind.value),
      });
    }
    setSelectedId(null);
    setEditing(false);
  };

  const saveEdit = () => {
    if (!selectedCut) {
      return;
    }
    const value = Number(editValue);
    if (!Number.isFinite(value) || value <= 0) {
      return;
    }
    if (selectedCut.kind.type === "order") {
      updateSettings({
        ...settings,
        orders: replaceValue(settings.orders, selectedCut.kind.value, value),
      });
    } else if (selectedCut.kind.type === "freq") {
      updateSettings({
        ...settings,
        freqs: replaceValue(settings.freqs, selectedCut.kind.value, value),
      });
    }
    setEditing(false);
    setSelectedId(null);
  };

  const degenerateCount = cuts.payload
    ? cuts.payload.cuts.filter(
        (cut) =>
          isDegenerate(cut, axis, cuts.payload!.hopS, cuts.payload!.binHz),
      ).length
    : 0;
  const warning =
    !warningDismissed &&
    cuts.payload &&
    cuts.payload.cuts.length > 0 &&
    degenerateCount > 0
      ? `${degenerateCount} cut(s) cannot be drawn on the ${axis} axis.`
      : null;

  let state: DisplayState;
  if (cuts.phase === "error") {
    state = {
      kind: "error",
      message: cuts.error ?? "Could not compute the order cuts.",
      onRetry: cuts.reload,
    };
  } else if (cuts.phase === "loading" || !cuts.payload) {
    state = { kind: "loading", progress: { framesDone: 0, framesTotal: null } };
  } else {
    state = { kind: "ready" };
  }

  const axisOptions: Array<{ value: OrderAxis; label: string }> = [
    { value: "rpm", label: "RPM" },
    { value: "time", label: "Time" },
    { value: "freq", label: "Order freq" },
  ];

  return (
    <DisplayFrame
      title={display.name}
      kind="order"
      parentLabel="child of Spectrogram"
      state={state}
      stale={cuts.stale}
      footerHints="Click a curve to select · legend toggles visibility"
      footerRight={
        cuts.payload ? `${cuts.payload.cuts.length} cuts` : "computing"
      }
      actions={
        <>
          <Segmented
            label="Order cut axis"
            value={settings.axis}
            options={axisOptions}
            onChange={(value) => updateSettings({ ...settings, axis: value })}
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
          <div className="order-tracking" data-testid="order-tracking">
            <label>
              RPM source
              <select
                aria-label="RPM source"
                value={rpmDraft.source}
                onChange={(event) =>
                  setRpmDraft({
                    ...rpmDraft,
                    source: event.target.value as RpmSettings["source"],
                  })
                }
              >
                <option value="constant">Constant</option>
                <option value="ramp">Ramp</option>
                <option value="csv">CSV</option>
              </select>
            </label>
            {rpmDraft.source === "constant" ? (
              <label>
                Constant RPM
                <input
                  aria-label="RPM constant"
                  type="number"
                  step="any"
                  value={rpmDraft.constant}
                  onChange={(event) =>
                    setRpmDraft({
                      ...rpmDraft,
                      constant: Number(event.target.value) || 0,
                    })
                  }
                />
              </label>
            ) : null}
            {rpmDraft.source === "ramp" ? (
              <>
                <label>
                  Start RPM
                  <input
                    aria-label="RPM start"
                    type="number"
                    step="any"
                    value={rpmDraft.start}
                    onChange={(event) =>
                      setRpmDraft({
                        ...rpmDraft,
                        start: Number(event.target.value) || 0,
                      })
                    }
                  />
                </label>
                <label>
                  End RPM
                  <input
                    aria-label="RPM end"
                    type="number"
                    step="any"
                    value={rpmDraft.end}
                    onChange={(event) =>
                      setRpmDraft({
                        ...rpmDraft,
                        end: Number(event.target.value) || 0,
                      })
                    }
                  />
                </label>
              </>
            ) : null}
            {rpmDraft.source === "csv" ? (
              <label>
                CSV path
                <input
                  aria-label="RPM CSV path"
                  value={rpmDraft.csv ?? ""}
                  onChange={(event) =>
                    setRpmDraft({ ...rpmDraft, csv: event.target.value })
                  }
                />
              </label>
            ) : null}
            <button
              type="button"
              className="secondary-button"
              onClick={applyRpm}
            >
              Apply tracking
            </button>
            <span
              className="order-tracking-summary"
              data-testid="order-tracking-summary"
            >
              {rpmProfile
                ? rpmProfileLabel(rpmProfile)
                : orderTrackingLabel(1, { ...settings, rpm: rpmDraft })}
            </span>
          </div>
          {resolved.note ? (
            <div className="cut-axis-note" role="note">
              {resolved.note}
            </div>
          ) : null}
          <CutPlot
            series={series}
            kind="order"
            panelId={panelId}
            onSavePng={onSavePng}
            axisLabel={
              axis === "rpm" ? "RPM" : axis === "time" ? "Time (s)" : "Order frequency (Hz)"
            }
            selectedId={selectedId}
            onPlotContextMenu={(x, y) => openOrderMenu(x, y)}
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
            warning={warning}
            onDismissWarning={() => setWarningDismissed(true)}
            showGrid={showGrid}
          />

          {orderMenu ? (
            <PlotContextMenu
              x={orderMenu.x}
              y={orderMenu.y}
              label="Order cut actions"
              onClose={closeOrderMenu}
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
              {addingOrder ? (
                <form className="order-add-form" onSubmit={addOrder}>
                  <input
                    aria-label="New order value"
                    type="number"
                    min="0"
                    step="any"
                    autoFocus
                    value={newOrder}
                    onChange={(event) => {
                      setNewOrder(event.target.value);
                      setAddOrderError(null);
                    }}
                  />
                  <button
                    type="submit"
                    className="primary-button"
                    disabled={!newOrder.trim()}
                  >
                    Add
                  </button>
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={closeOrderMenu}
                  >
                    Cancel
                  </button>
                  {addOrderError ? (
                    <span className="order-add-error" role="alert">
                      {addOrderError}
                    </span>
                  ) : null}
                </form>
              ) : (
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => setAddingOrder(true)}
                >
                  Add order cut
                </button>
              )}
            </PlotContextMenu>
          ) : null}

          {selectedCut ? (
            <div className="cut-action-bar" role="toolbar" aria-label="Selected cut">
              <button type="button" className="text-button" onClick={deleteCut}>
                Delete
              </button>
              <button
                type="button"
                className="text-button"
                disabled
                title="Order cuts follow RPM; flipping is only for line cuts"
              >
                Flip
              </button>
              <button
                type="button"
                className="text-button"
                onClick={() => {
                  setEditing(true);
                  setEditValue(
                    selectedCut.kind.type === "line"
                      ? ""
                      : String(selectedCut.kind.value),
                  );
                }}
              >
                Edit numbers…
              </button>
            </div>
          ) : null}

          {editing && selectedCut ? (
            <form
              className="cut-editor"
              onSubmit={(event) => {
                event.preventDefault();
                saveEdit();
              }}
            >
              <label>
                {selectedCut.kind.type === "freq" ? "Frequency (Hz)" : "Order"}
                <input
                  aria-label="Cut value"
                  type="number"
                  min="0"
                  step="any"
                  value={editValue}
                  onChange={(event) => setEditValue(event.target.value)}
                />
              </label>
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

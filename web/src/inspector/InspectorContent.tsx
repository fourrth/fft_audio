import { useEffect, useState } from "react";
import type { ShellDisplay } from "../shell/session";
import type { Transport } from "../transport/types";
import {
  orderTrackingLabel,
  parseOrderList,
} from "../cuts/rpm";
import {
  readLineSettings,
  readLines,
  readOrderSettings,
  readShowGrid,
  type LineCutSettings,
  type OrderCutSettings,
  type RpmSettings,
} from "../cuts/settings";
import {
  activeResolutionPreset,
  analysisSettingsEqual,
  estimatedFrameCount,
  frameMilliseconds,
  frequencyResolution,
  matchesFilter,
  readAnalysisSettings,
  readDisplaySettings,
  readInspectorMeta,
  resolutionPresetSettings,
  RESOLUTION_PRESETS,
  WINDOW_KINDS,
  type AnalysisSettings,
  type DisplaySettings,
  type ResolutionPreset,
  type WindowKind,
} from "../settings/model";
import { isTablePath } from "../transport/tableExtensions";
import { Banner } from "../ui/Banner";
import { Card } from "../ui/Card";
import { Chip } from "../ui/Chip";
import { Field } from "../ui/Field";
import { IconButton } from "../ui/IconButton";
import { Toggle } from "../ui/Toggle";
import { Tooltip } from "../ui/Tooltip";
import { useToast } from "../ui/toast";
import "./inspector.css";

export interface InspectorContentProps {
  display: ShellDisplay;
  transport: Transport | null;
  onUpdateState: (patch: Record<string, unknown>) => void;
  onBrowse: () => void;
  onClose?: () => void;
}

type CardId = "input" | "analysis" | "smoothing" | "display" | "advanced";

const CARD_LABELS: Record<CardId, string[]> = {
  input: ["Input file", "Path", "Browse"],
  analysis: ["Resolution", "Window", "High-pass", "Low-pass"],
  smoothing: ["Smoothing", "Smooth"],
  display: [
    "Log frequency",
    "Grid lines",
    "Minimum frequency",
    "Maximum frequency",
    "color range",
    "color floor",
    "color ceiling",
    "Nyquist",
    "Hz",
    "dB",
  ],
  advanced: ["Frame", "Hop", "Pad", "Column", "Time column", "Sample rate", "Remove mean"],
};

const ADVANCED_AUDIO_LABELS = ["Frame", "Hop", "Pad"];
const ADVANCED_TABLE_LABELS = [
  "Frame",
  "Hop",
  "Pad",
  "Column",
  "Time column",
  "Sample rate",
  "Remove mean",
];

const CARD_ORDER: CardId[] = ["input", "analysis", "smoothing", "display", "advanced"];
const CARD_TITLES: Record<CardId, string> = {
  input: "Input",
  analysis: "Analysis",
  smoothing: "Smoothing",
  display: "Display",
  advanced: "Advanced",
};

function readCards(state: Record<string, unknown>): Partial<Record<CardId, boolean>> {
  const value = state.inspectorCards;
  return typeof value === "object" && value !== null
    ? (value as Partial<Record<CardId, boolean>>)
    : {};
}

function numberValue(value: string): number | null {
  const parsed = Number(value);
  return value.trim() === "" || !Number.isFinite(parsed) ? null : parsed;
}

function cardLabels(card: CardId, isTableInput: boolean): string[] {
  if (card === "advanced") {
    return isTableInput ? ADVANCED_TABLE_LABELS : ADVANCED_AUDIO_LABELS;
  }
  return CARD_LABELS[card];
}

function cardVisible(
  card: CardId,
  query: string,
  isTableInput: boolean,
): boolean {
  return cardLabels(card, isTableInput).some((label) =>
    matchesFilter(label, query),
  );
}

export function InspectorContent({
  display,
  transport,
  onUpdateState,
  onBrowse,
  onClose,
}: InspectorContentProps) {
  const applied = readAnalysisSettings(display.state);
  const [staged, setStaged] = useState<AnalysisSettings>(applied);
  const [query, setQuery] = useState("");
  const cards = readCards(display.state);
  const displaySettings = readDisplaySettings(display.state);
  const meta = readInspectorMeta(display.state);
  const path = typeof display.state.path === "string" ? display.state.path : "";
  const isTableInput =
    meta.table !== undefined || (path.length > 0 && isTablePath(path));
  const kind = display.kind;
  const isSpectrogram = kind === "spectrogram";
  const isLauncher = kind === "launcher";
  const isCuts = kind === "order_cuts" || kind === "line_cuts";
  const availableCards: CardId[] = isSpectrogram
    ? CARD_ORDER
    : isLauncher
      ? ["input"]
      : [];
  const nyquist =
    meta.sampleRate && meta.sampleRate > 0 ? meta.sampleRate / 2 : null;
  const hasAbsolutecolor =
    displaySettings.db_min != null || displaySettings.db_max != null;
  const { pushToast } = useToast();

  useEffect(() => {
    setStaged(readAnalysisSettings(display.state));
    // External applies replace the baseline; local staging stays until then.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [display.id, JSON.stringify(applied)]);

  const dirty = !analysisSettingsEqual(staged, applied);
  const activePreset = activeResolutionPreset(staged, meta.sampleRate);
  const estimatedFrames = estimatedFrameCount(
    staged,
    meta.sampleRate,
    meta.durationS,
  );
  const framesLabel =
    dirty && estimatedFrames !== null
      ? `~${estimatedFrames} frames`
      : meta.frames !== undefined
        ? `${meta.frames} frames`
        : null;

  const applyResolutionPreset = (value: string) => {
    if (value === "custom") {
      return;
    }
    const values = resolutionPresetSettings(
      value as ResolutionPreset,
      meta.sampleRate,
    );
    if (values) {
      setStaged({ ...staged, ...values });
    }
  };

  const setCard = (card: CardId, open: boolean) => {
    onUpdateState({ inspectorCards: { ...cards, [card]: open } });
  };

  const updateDisplay = (patch: Partial<DisplaySettings>) => {
    onUpdateState({ displaySettings: { ...displaySettings, ...patch } });
  };

  const updateMinHz = (raw: string) => {
    if (raw.trim() === "") {
      updateDisplay({ min_hz: null });
      return;
    }
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      return;
    }
    const ceiling = displaySettings.max_hz ?? nyquist;
    const next =
      ceiling !== null && Number.isFinite(ceiling)
        ? Math.min(Math.max(0, parsed), Math.max(0, ceiling - 1))
        : Math.max(0, parsed);
    updateDisplay({ min_hz: next });
  };

  const updateMaxHz = (raw: string) => {
    if (raw.trim() === "") {
      updateDisplay({ max_hz: null });
      return;
    }
    const parsed = Number(raw);
    if (!Number.isFinite(parsed) || parsed < 1) {
      return;
    }
    let next =
      nyquist !== null ? Math.min(parsed, nyquist) : parsed;
    const patch: Partial<DisplaySettings> = { max_hz: next };
    const floor = displaySettings.min_hz;
    if (floor != null && next <= floor) {
      if (nyquist !== null && floor >= nyquist) {
        patch.min_hz = Math.max(0, next - 1);
      } else {
        next = Math.min(nyquist ?? Number.POSITIVE_INFINITY, floor + 1);
        patch.max_hz = next;
      }
    }
    updateDisplay(patch);
  };

  const updateRangeDb = (raw: string) => {
    if (raw.trim() === "") {
      updateDisplay({ range_db: 80 });
      return;
    }
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      return;
    }
    updateDisplay({ range_db: Math.min(160, Math.max(1, parsed)) });
  };

  const updateDbMin = (raw: string) => {
    if (raw.trim() === "") {
      updateDisplay({ db_min: null });
      return;
    }
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      return;
    }
    const ceiling = displaySettings.db_max;
    const next = ceiling != null ? Math.min(parsed, ceiling - 1) : parsed;
    updateDisplay({ db_min: next });
  };

  const updateDbMax = (raw: string) => {
    if (raw.trim() === "") {
      updateDisplay({ db_max: null });
      return;
    }
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      return;
    }
    const floor = displaySettings.db_min;
    const next = floor != null ? Math.max(parsed, floor + 1) : parsed;
    updateDisplay({ db_max: next });
  };

  const useAsDefault = async () => {
    if (!transport) {
      return;
    }
    try {
      await transport.invoke({
        name: "settings.setSpectrogramDefaults",
        args: { analysis: staged, display: displaySettings },
      });
      pushToast({
        tone: "success",
        message: "Saved as the default for new spectrograms",
      });
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `Could not save defaults: ${
          reason instanceof Error ? reason.message : String(reason)
        }`,
      });
    }
  };

  const apply = () => {
    onUpdateState({ analysisSettings: staged });
    pushToast({
      tone: "info",
      message: "Analysis settings applied; re-analysing the file",
    });
  };

  const sr = meta.sampleRate;
  const orderSettings =
    display.kind === "order_cuts" ? readOrderSettings(display.state) : null;
  const lineSettings =
    display.kind === "line_cuts" ? readLineSettings(display.state) : null;
  const lineCount =
    display.kind === "line_cuts" ? readLines(display.state).length : 0;
  const orderCutCount = orderSettings
    ? parseOrderList(orderSettings.orders).length +
      parseOrderList(orderSettings.freqs).length
    : 0;
  const cutShowGrid = readShowGrid(display.state);

  const updateOrder = (
    patch: Partial<OrderCutSettings>,
    clearProfile = false,
  ) => {
    if (!orderSettings) {
      return;
    }
    onUpdateState({
      orderCuts: { ...orderSettings, ...patch },
      ...(clearProfile ? { rpm: null } : {}),
    });
  };
  const updateRpm = (patch: Partial<RpmSettings>) => {
    if (!orderSettings) {
      return;
    }
    updateOrder({ rpm: { ...orderSettings.rpm, ...patch } }, true);
  };
  const updateLine = (patch: Partial<LineCutSettings>) => {
    if (!lineSettings) {
      return;
    }
    onUpdateState({ lineCuts: { ...lineSettings, ...patch } });
  };

  return (
    <div className="inspector" data-testid="inspector" data-ui-id="inspector">
      <header className="inspector-header" data-ui-id="inspector.header">
        <div className="inspector-heading">
          <strong>Inspector</strong>
          <span title={display.name}>{display.name}</span>
          {dirty ? (
            <span
              className="inspector-dirty-dot"
              title="Unapplied DSP edits"
              aria-label="Unapplied DSP edits"
            />
          ) : null}
        </div>
        {onClose ? (
          <IconButton label="Close inspector" onClick={onClose}>
            ×
          </IconButton>
        ) : null}
      </header>

      {isSpectrogram || isLauncher ? (
        <label className="inspector-search">
          <span className="sr-only">Filter settings</span>
          <input
            type="search"
            aria-label="Filter settings"
            placeholder="Filter settings…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
      ) : null}

      {isSpectrogram ? (
        <div className="inspector-chips" aria-label="Derived values">
          <Tooltip label="FFT window applied to each frame. It reduces spectral leakage; Hann is the default.">
            <Chip tone={staged.window === "hann" ? "spec" : "neutral"}>
              {staged.window}
            </Chip>
          </Tooltip>
          <Tooltip label="Analysis window length in time: frame samples divided by the sample rate.">
            <Chip>{frameMilliseconds(staged, sr)}</Chip>
          </Tooltip>
          <Tooltip label="Spacing between FFT bins: sample rate divided by frame size times zero-padding.">
            <Chip>{frequencyResolution(staged, sr)}</Chip>
          </Tooltip>
          <Tooltip label="Samples in each analysis frame.">
            <Chip>{staged.frame} samples</Chip>
          </Tooltip>
          {framesLabel ? (
            <Tooltip
              label={
                dirty && estimatedFrames !== null
                  ? "Estimated frame count for the staged framing. It becomes exact after Apply & re-analyse."
                  : "Number of analysis frames in the current spectrogram."
              }
            >
              <Chip>{framesLabel}</Chip>
            </Tooltip>
          ) : null}
          {sr ? (
            <Tooltip label="Source sample rate.">
              <Chip>{Math.round(sr)} Hz</Chip>
            </Tooltip>
          ) : null}
        </div>
      ) : null}

      <div className="inspector-scroll">
        {!transport ? (
          <Banner tone="info">Preview mode: settings are not persisted.</Banner>
        ) : null}

        {kind === "scripting" || kind === "script_console" ? (
          <Banner tone="info">
            Scripting displays have no analysis or display properties.
          </Banner>
        ) : null}

        {isCuts && (orderSettings || lineSettings) ? (
          <Card title="Cut tracking">
            <div
              className="inspector-tracking"
              data-testid="inspector-cut-tracking"
            >
              {orderSettings ? (
                <>
                  <div>
                    <strong>Tracking</strong>
                    <span>{orderTrackingLabel(1, orderSettings)}</span>
                  </div>
                  <div>
                    <strong>Cuts</strong>
                    <span>{orderCutCount}</span>
                  </div>
                </>
              ) : null}
              {lineSettings ? (
                <div>
                  <strong>Line cuts</strong>
                  <span>
                    {lineCount} lines · {lineSettings.line_axis} axis
                  </span>
                </div>
              ) : null}
            </div>

            {orderSettings ? (
              <>
                <Field label="X axis">
                  <select
                    aria-label="Inspector order axis"
                    value={orderSettings.axis}
                    onChange={(event) =>
                      updateOrder({
                        axis: event.target.value as OrderCutSettings["axis"],
                      })
                    }
                  >
                    <option value="rpm">RPM</option>
                    <option value="time">Time</option>
                    <option value="freq">Order frequency</option>
                  </select>
                </Field>
                <Field label="Orders">
                  <input
                    aria-label="Inspector order values"
                    value={orderSettings.orders}
                    onChange={(event) =>
                      updateOrder({ orders: event.target.value })
                    }
                  />
                </Field>
                <Field label="Order width">
                  <input
                    aria-label="Inspector order width"
                    type="number"
                    min={0}
                    step="any"
                    value={orderSettings.order_width}
                    onChange={(event) =>
                      updateOrder({
                        order_width: numberValue(event.target.value) ?? 0,
                      })
                    }
                  />
                </Field>
                <Field label="Smoothing" unit="s">
                  <input
                    aria-label="Inspector order smoothing"
                    type="number"
                    min={0}
                    step="any"
                    value={orderSettings.smooth_s}
                    onChange={(event) =>
                      updateOrder({
                        smooth_s: numberValue(event.target.value) ?? 0,
                      })
                    }
                  />
                </Field>
                <Field label="RPM source">
                  <select
                    aria-label="Inspector RPM source"
                    value={orderSettings.rpm.source}
                    onChange={(event) =>
                      updateRpm({
                        source: event.target.value as RpmSettings["source"],
                      })
                    }
                  >
                    <option value="constant">Constant</option>
                    <option value="ramp">Ramp</option>
                    <option value="csv">CSV</option>
                  </select>
                </Field>
                {orderSettings.rpm.source === "constant" ? (
                  <Field label="Constant RPM">
                    <input
                      aria-label="Inspector constant RPM"
                      type="number"
                      step="any"
                      value={orderSettings.rpm.constant}
                      onChange={(event) =>
                        updateRpm({
                          constant: numberValue(event.target.value) ?? 0,
                        })
                      }
                    />
                  </Field>
                ) : null}
                {orderSettings.rpm.source === "ramp" ? (
                  <>
                    <Field label="Start RPM">
                      <input
                        aria-label="Inspector RPM start"
                        type="number"
                        step="any"
                        value={orderSettings.rpm.start}
                        onChange={(event) =>
                          updateRpm({
                            start: numberValue(event.target.value) ?? 0,
                          })
                        }
                      />
                    </Field>
                    <Field label="End RPM">
                      <input
                        aria-label="Inspector RPM end"
                        type="number"
                        step="any"
                        value={orderSettings.rpm.end}
                        onChange={(event) =>
                          updateRpm({
                            end: numberValue(event.target.value) ?? 0,
                          })
                        }
                      />
                    </Field>
                  </>
                ) : null}
                {orderSettings.rpm.source === "csv" ? (
                  <Field label="RPM CSV">
                    <input
                      aria-label="Inspector RPM CSV"
                      value={orderSettings.rpm.csv ?? ""}
                      onChange={(event) =>
                        updateRpm({ csv: event.target.value || null })
                      }
                    />
                  </Field>
                ) : null}
                <Toggle
                  label="Peak snap"
                  checked={orderSettings.peak_snap}
                  onChange={(checked) =>
                    updateOrder({ peak_snap: checked })
                  }
                />
                <Field label="Peak minimum" unit="dB">
                  <input
                    aria-label="Inspector order peak minimum"
                    type="number"
                    step="any"
                    value={orderSettings.peak_min_db}
                    onChange={(event) =>
                      updateOrder({
                        peak_min_db: numberValue(event.target.value) ?? 0,
                      })
                    }
                  />
                </Field>
              </>
            ) : null}

            {lineSettings ? (
              <>
                <Field label="X axis">
                  <select
                    aria-label="Inspector line axis"
                    value={lineSettings.line_axis}
                    onChange={(event) =>
                      updateLine({
                        line_axis: event.target
                          .value as LineCutSettings["line_axis"],
                      })
                    }
                  >
                    <option value="auto">Auto</option>
                    <option value="time">Time</option>
                    <option value="freq">Frequency</option>
                  </select>
                </Field>
                <Field label="Line width" unit="Hz">
                  <input
                    aria-label="Inspector line width"
                    type="number"
                    min={0}
                    step="any"
                    value={lineSettings.line_width_hz}
                    onChange={(event) =>
                      updateLine({
                        line_width_hz: numberValue(event.target.value) ?? 0,
                      })
                    }
                  />
                </Field>
                <Toggle
                  label="Snap line angle"
                  checked={lineSettings.snap}
                  onChange={(checked) => updateLine({ snap: checked })}
                />
                <Field label="Snap zone" unit="deg">
                  <input
                    aria-label="Inspector line snap degrees"
                    type="number"
                    min={0}
                    step="any"
                    disabled={!lineSettings.snap}
                    value={lineSettings.snap_deg}
                    onChange={(event) =>
                      updateLine({
                        snap_deg: numberValue(event.target.value) ?? 0,
                      })
                    }
                  />
                </Field>
                <Toggle
                  label="Peak snap"
                  checked={lineSettings.peak_snap}
                  onChange={(checked) => updateLine({ peak_snap: checked })}
                />
                <Field label="Peak minimum" unit="dB">
                  <input
                    aria-label="Inspector line peak minimum"
                    type="number"
                    step="any"
                    value={lineSettings.peak_min_db}
                    onChange={(event) =>
                      updateLine({
                        peak_min_db: numberValue(event.target.value) ?? 0,
                      })
                    }
                  />
                </Field>
              </>
            ) : null}
          </Card>
        ) : null}

        {isCuts ? (
          <Card title="Display">
            <Toggle
              label="Grid lines"
              checked={cutShowGrid}
              onChange={(checked) => onUpdateState({ showGrid: checked })}
            />
          </Card>
        ) : null}

        {availableCards
          .filter((card) => cardVisible(card, query, isTableInput))
          .map((card) => (
          <Card
            key={card}
            title={CARD_TITLES[card]}
            open={cards[card] ?? true}
            onOpenChange={(open) => setCard(card, open)}
            actions={
              card === "analysis" ? (
                <IconButton label="Use as default for new spectrograms" onClick={useAsDefault}>
                  ★
                </IconButton>
              ) : null
            }
          >
            {card === "input" ? (
              <>
                {matchesFilter("Input file", query) ? (
                  <Field label="Input file">
                    <input
                      aria-label="Input file"
                      readOnly
                      value={path || "No file loaded"}
                      title={path}
                    />
                  </Field>
                ) : null}
                {matchesFilter("Browse", query) ? (
                  <button type="button" className="primary-button" onClick={onBrowse}>
                    Browse…
                  </button>
                ) : null}
              </>
            ) : null}

            {card === "analysis" ? (
              <>
                {matchesFilter("Resolution", query) ? (
                  <Field
                    label="Resolution"
                    help="Presets choose frame, hop and pad from the sample rate. Use Advanced for manual control."
                  >
                    <select
                      aria-label="Resolution preset"
                      value={activePreset ?? "custom"}
                      disabled={sr == null}
                      onChange={(event) =>
                        applyResolutionPreset(event.target.value)
                      }
                    >
                      {activePreset === null ? (
                        <option value="custom" disabled>
                          Custom
                        </option>
                      ) : null}
                      {RESOLUTION_PRESETS.map((preset) => (
                        <option key={preset.id} value={preset.id}>
                          {preset.label}
                        </option>
                      ))}
                    </select>
                  </Field>
                ) : null}
                {sr == null ? (
                  <p className="inspector-hint">
                    Open a file to calculate presets from its sample rate.
                  </p>
                ) : activePreset ? (
                  <p className="inspector-hint">
                    {
                      RESOLUTION_PRESETS.find(
                        (preset) => preset.id === activePreset,
                      )?.description
                    }
                  </p>
                ) : (
                  <p className="inspector-hint">
                    Custom frame, hop and pad; edit them in Advanced.
                  </p>
                )}
                {matchesFilter("Window", query) ? (
                  <Field label="Window">
                    <select
                      aria-label="Window"
                      value={staged.window}
                      onChange={(event) =>
                        setStaged({
                          ...staged,
                          window: event.target.value as WindowKind,
                        })
                      }
                    >
                      {WINDOW_KINDS.map((kind) => (
                        <option key={kind} value={kind}>
                          {kind === "none" ? "None" : kind}
                        </option>
                      ))}
                    </select>
                  </Field>
                ) : null}
                {matchesFilter("High-pass", query) ? (
                  <Field label="High-pass" unit="Hz">
                    <input
                      aria-label="High-pass"
                      type="number"
                      min={0}
                      value={staged.highpass ?? ""}
                      placeholder="off"
                      onChange={(event) =>
                        setStaged({
                          ...staged,
                          highpass: numberValue(event.target.value),
                        })
                      }
                    />
                  </Field>
                ) : null}
                {matchesFilter("Low-pass", query) ? (
                  <Field label="Low-pass" unit="Hz">
                    <input
                      aria-label="Low-pass"
                      type="number"
                      min={0}
                      value={staged.lowpass ?? ""}
                      placeholder="off"
                      onChange={(event) =>
                        setStaged({
                          ...staged,
                          lowpass: numberValue(event.target.value),
                        })
                      }
                    />
                  </Field>
                ) : null}
              </>
            ) : null}

            {card === "smoothing" ? (
              <Field label="Smoothing" unit="s">
                <input
                  aria-label="Smoothing"
                  type="number"
                  min={0}
                  step={0.05}
                  value={staged.smooth_s}
                  onChange={(event) =>
                    setStaged({
                      ...staged,
                      smooth_s: Number(event.target.value) || 0,
                    })
                  }
                />
              </Field>
            ) : null}

            {card === "display" ? (
              <>
                <Field label="Log frequency">
                  <Toggle
                    checked={displaySettings.log_freq}
                    onChange={(checked) => updateDisplay({ log_freq: checked })}
                    label="Log frequency axis"
                  />
                </Field>
                <Field label="Grid lines">
                  <Toggle
                    checked={displaySettings.show_grid}
                    onChange={(checked) => updateDisplay({ show_grid: checked })}
                    label="Show grid lines"
                  />
                </Field>
                <Field label="Minimum frequency" unit="Hz">
                  <input
                    aria-label="Minimum frequency"
                    type="number"
                    min={0}
                    max={nyquist ?? undefined}
                    value={displaySettings.min_hz ?? ""}
                    placeholder="0"
                    onChange={(event) => updateMinHz(event.target.value)}
                  />
                </Field>
                <Field
                  label="Maximum frequency"
                  unit="Hz"
                  help="The maximum frequency is a hard view cap. Values above Nyquist are clamped."
                >
                  <input
                    aria-label="Maximum frequency"
                    type="number"
                    min={1}
                    max={nyquist ?? undefined}
                    value={displaySettings.max_hz ?? ""}
                    placeholder="Nyquist"
                    onChange={(event) => updateMaxHz(event.target.value)}
                  />
                </Field>
                {nyquist !== null ? (
                  <p className="inspector-hint">
                    Hard view cap. Nyquist is {Math.round(nyquist)} Hz.
                  </p>
                ) : (
                  <p className="inspector-hint">
                    Hard view cap; clamped to Nyquist when known.
                  </p>
                )}
                <button
                  type="button"
                  className="secondary-button"
                  title="Clear the maximum frequency and fit to Nyquist"
                  onClick={() => updateDisplay({ max_hz: null })}
                >
                  Use Nyquist
                </button>
                <Field
                  label="color range"
                  unit="dB"
                  help="Absolute color floor/ceiling bounds override this range."
                  banner={
                    hasAbsolutecolor
                      ? {
                          tone: "info",
                          message:
                            "Absolute color bounds override the color range.",
                        }
                      : undefined
                  }
                >
                  <input
                    aria-label="color range"
                    type="number"
                    min={1}
                    max={160}
                    value={displaySettings.range_db}
                    disabled={hasAbsolutecolor}
                    onChange={(event) => updateRangeDb(event.target.value)}
                  />
                </Field>
                <Field
                  label="color floor"
                  unit="dB"
                  help="Optional absolute low end for the color scale. Must stay below the ceiling."
                >
                  <input
                    aria-label="color floor"
                    type="number"
                    value={displaySettings.db_min ?? ""}
                    placeholder="range"
                    onChange={(event) => updateDbMin(event.target.value)}
                  />
                </Field>
                <Field
                  label="color ceiling"
                  unit="dB"
                  help="Optional absolute high end for the color scale. Must stay above the floor."
                >
                  <input
                    aria-label="color ceiling"
                    type="number"
                    value={displaySettings.db_max ?? ""}
                    placeholder="data max"
                    onChange={(event) => updateDbMax(event.target.value)}
                  />
                </Field>
                <button
                  type="button"
                  className="secondary-button"
                  title="Clear absolute color bounds and use the color range"
                  onClick={() =>
                    updateDisplay({ db_min: null, db_max: null })
                  }
                >
                  Use range
                </button>
                <p className="inspector-hint">
                  Color range is the span below the loudest bin. Color floor and
                  ceiling pin absolute dB ends; setting either switches to
                  absolute bounds.
                </p>
              </>
            ) : null}

            {card === "advanced" ? (
              <>
                <p className="inspector-hint">
                  Manual FFT framing. Prefer the presets in Analysis unless you
                  need exact sample counts.
                </p>
                {matchesFilter("Frame", query) ? (
                  <Field label="Frame" unit="samples">
                    <input
                      aria-label="Frame"
                      type="number"
                      min={16}
                      value={staged.frame}
                      onChange={(event) =>
                        setStaged({
                          ...staged,
                          frame: Number(event.target.value) || 16,
                        })
                      }
                    />
                  </Field>
                ) : null}
                {matchesFilter("Hop", query) ? (
                  <Field label="Hop" unit="samples">
                    <input
                      aria-label="Hop"
                      type="number"
                      min={1}
                      value={staged.hop ?? ""}
                      placeholder="frame / 4"
                      onChange={(event) =>
                        setStaged({
                          ...staged,
                          hop: numberValue(event.target.value),
                        })
                      }
                    />
                  </Field>
                ) : null}
                {matchesFilter("Pad", query) ? (
                  <Field label="Pad" unit="× frame">
                    <input
                      aria-label="Pad"
                      type="number"
                      min={1}
                      value={staged.pad}
                      onChange={(event) =>
                        setStaged({
                          ...staged,
                          pad: Number(event.target.value) || 1,
                        })
                      }
                    />
                  </Field>
                ) : null}
                {isTableInput ? (
                  <>
                    {meta.table ? (
                      <p className="inspector-hint">
                        Table: {meta.table.rows} rows · signal{" "}
                        {meta.table.signal} · time{" "}
                        {meta.table.time ?? "assumed"}
                      </p>
                    ) : null}
                    {matchesFilter("Column", query) ? (
                      <Field label="Column">
                        <input
                          aria-label="Column"
                          value={staged.data.column ?? ""}
                          placeholder="first numeric column"
                          onChange={(event) =>
                            setStaged({
                              ...staged,
                              data: {
                                ...staged.data,
                                column: event.target.value || null,
                              },
                            })
                          }
                        />
                      </Field>
                    ) : null}
                    {matchesFilter("Time column", query) ? (
                      <Field label="Time column">
                        <input
                          aria-label="Time column"
                          value={staged.data.time_column ?? ""}
                          placeholder="auto"
                          onChange={(event) =>
                            setStaged({
                              ...staged,
                              data: {
                                ...staged.data,
                                time_column: event.target.value || null,
                              },
                            })
                          }
                        />
                      </Field>
                    ) : null}
                    {matchesFilter("Sample rate", query) ? (
                      <Field label="Sample rate" unit="Hz">
                        <input
                          aria-label="Sample rate"
                          type="number"
                          min={0}
                          value={staged.data.sample_rate ?? ""}
                          placeholder="from time column"
                          onChange={(event) =>
                            setStaged({
                              ...staged,
                              data: {
                                ...staged.data,
                                sample_rate: numberValue(event.target.value),
                              },
                            })
                          }
                        />
                      </Field>
                    ) : null}
                    {matchesFilter("Remove mean", query) ? (
                      <Field label="Remove mean">
                        <Toggle
                          checked={staged.data.remove_mean}
                          onChange={(checked) =>
                            setStaged({
                              ...staged,
                              data: { ...staged.data, remove_mean: checked },
                            })
                          }
                          label="Remove mean"
                        />
                      </Field>
                    ) : null}
                  </>
                ) : null}
              </>
            ) : null}
          </Card>
        ))}
      </div>

      {dirty ? (
        <footer className="inspector-footer" role="status">
          <span>
            <span className="inspector-dirty-dot" aria-hidden="true" /> Unapplied
            DSP edits
          </span>
          <button
            type="button"
            className="secondary-button"
            onClick={() => setStaged(applied)}
          >
            Revert
          </button>
          <button type="button" className="primary-button" onClick={apply}>
            Apply &amp; re-analyse
          </button>
        </footer>
      ) : null}
    </div>
  );
}

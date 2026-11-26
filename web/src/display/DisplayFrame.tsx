import { useState, type ReactNode } from "react";
import { Banner } from "../ui/Banner";
import { EmptyState } from "../ui/EmptyState";
import { ProgressBar } from "../ui/ProgressBar";
import { fileName } from "../launcher/recent";
import "./display.css";

export type DisplayKind =
  | "spec"
  | "order"
  | "line"
  | "plot"
  | "recorder"
  | "console";

export interface DisplayProgressView {
  /** 0..=1; null = indeterminate. */
  value?: number | null;
  framesDone: number;
  framesTotal: number | null;
  etaSeconds?: number | null;
  label?: string;
}

export type DisplayState =
  | { kind: "ready" }
  | {
      kind: "empty";
      recentFiles?: string[];
      onBrowse?: () => void;
      onOpenPath?: (path: string) => void;
      onOpenRecent?: (path: string) => void;
    }
  | { kind: "loading"; progress: DisplayProgressView; onCancel?: () => void }
  | {
      kind: "error";
      message: string;
      onRetry?: () => void;
      onChooseFile?: () => void;
    };

export interface DisplayFrameProps {
  displayId?: number;
  title: string;
  kind?: DisplayKind;
  parentLabel?: string;
  state: DisplayState;
  stale?: boolean;
  staleLabel?: string;
  actions?: ReactNode;
  footerHints?: ReactNode;
  footerRight?: ReactNode;
  hintsEnabled?: boolean;
  children?: ReactNode;
}

function progressLabel(progress: DisplayProgressView): string {
  if (progress.label) {
    return progress.label;
  }
  return progress.framesTotal === null
    ? `Analysing… ${progress.framesDone} frames`
    : `Analysing ${progress.framesDone}/${progress.framesTotal} frames`;
}

function progressValue(progress: DisplayProgressView): number | null {
  if (progress.value !== undefined) {
    return progress.value;
  }
  return progress.framesTotal && progress.framesTotal > 0
    ? progress.framesDone / progress.framesTotal
    : null;
}

function EmptyBody({
  recentFiles = [],
  onBrowse,
  onOpenPath,
  onOpenRecent,
}: {
  recentFiles?: string[];
  onBrowse?: () => void;
  onOpenPath?: (path: string) => void;
  onOpenRecent?: (path: string) => void;
}) {
  const [manual, setManual] = useState(false);
  const [path, setPath] = useState("");
  const visibleRecent = recentFiles.slice(0, 5);

  const openTyped = () => {
    const value = path.trim();
    if (value) {
      onOpenPath?.(value);
    }
  };

  return (
    <div className="display-empty" data-testid="display-empty">
      <EmptyState
        icon="▤"
        title="No file loaded"
        description="Choose an audio file or data table to analyse."
        actions={
          <>
            <button
              type="button"
              className="primary-button"
              onClick={onBrowse}
              disabled={!onBrowse}
            >
              Browse…
            </button>
            <button
              type="button"
              className="text-button"
              onClick={() => setManual((value) => !value)}
            >
              Enter a path
            </button>
          </>
        }
        hint="Drop a file anywhere on the window, or pick a recent file below."
      />

      {manual ? (
        <form
          className="display-manual"
          data-testid="display-manual"
          onSubmit={(event) => {
            event.preventDefault();
            openTyped();
          }}
        >
          <input
            aria-label="File path"
            placeholder="/path/to/audio.wav"
            value={path}
            onChange={(event) => setPath(event.target.value)}
          />
          <button type="submit" className="secondary-button">
            Open
          </button>
        </form>
      ) : null}

      {visibleRecent.length > 0 ? (
        <div className="display-recent" data-testid="display-recent">
          <span className="display-recent-title">Recent files</span>
          <ul>
            {visibleRecent.map((recent) => (
              <li key={recent}>
                <button
                  type="button"
                  className="display-recent-item"
                  title={recent}
                  onClick={() => onOpenRecent?.(recent)}
                >
                  {fileName(recent)}
                </button>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

/**
 * Mandatory three-row display frame: 36 px header, flexible body, 24 px
 * footer. Empty/loading/error/stale states are shared by every display.
 */
export function DisplayFrame({
  displayId,
  title,
  kind = "spec",
  parentLabel,
  state,
  stale = false,
  staleLabel = "Updating from parent…",
  actions,
  footerHints,
  footerRight,
  hintsEnabled = true,
  children,
}: DisplayFrameProps) {
  const [interacted, setInteracted] = useState(false);
  const showHints = hintsEnabled && !interacted && state.kind === "ready";

  return (
    <section
      className={`display-frame display-${kind}`}
      data-testid="display-frame"
      data-display-id={displayId}
      data-ui-id={`display.${kind}`}
    >
      <header className="display-header" data-ui-id={`display.${kind}.header`}>
        <span className={`display-kind-dot kind-${kind}`} aria-hidden="true" />
        <div className="display-title">
          <strong title={title}>{title}</strong>
          {parentLabel ? <span className="display-parent">{parentLabel}</span> : null}
        </div>
        <span className={`display-state state-${state.kind}`} aria-label={state.kind} />
        <div className="display-actions" data-ui-id={`display.${kind}.actions`}>
          {actions}
        </div>
      </header>

      <div
        className="display-body"
        data-testid="display-body"
        data-ui-id={`display.${kind}.body`}
        onPointerDown={() => {
          // Empty/loading/error actions are not plot interactions; only a
          // ready display should dismiss its plot hints.
          if (state.kind === "ready" && !interacted) {
            setInteracted(true);
          }
        }}
      >
        {stale ? (
          <div className="display-stale" role="status">
            <span className="display-stale-shimmer" aria-hidden="true" />
            <span className="display-stale-label">{staleLabel}</span>
          </div>
        ) : null}

        {state.kind === "ready" ? (
          children ?? <div className="display-placeholder">Ready.</div>
        ) : null}

        {state.kind === "empty" ? (
          <EmptyBody
            recentFiles={state.recentFiles}
            onBrowse={state.onBrowse}
            onOpenPath={state.onOpenPath}
            onOpenRecent={state.onOpenRecent}
          />
        ) : null}

        {state.kind === "loading" ? (
          <div className="display-loading">
            <div className="display-skeleton" aria-hidden="true">
              <span />
              <span />
              <span />
              <span />
            </div>
            <div className="display-progress">
              <ProgressBar
                value={progressValue(state.progress)}
                label={progressLabel(state.progress)}
                etaSeconds={state.progress.etaSeconds}
                onCancel={state.onCancel}
              />
            </div>
          </div>
        ) : null}

        {state.kind === "error" ? (
          <div className="display-error">
            <Banner
              tone="error"
              actions={
                <>
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={state.onRetry}
                    disabled={!state.onRetry}
                  >
                    Try again
                  </button>
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={state.onChooseFile}
                    disabled={!state.onChooseFile}
                  >
                    Choose another file
                  </button>
                </>
              }
            >
              {state.message}
            </Banner>
          </div>
        ) : null}
      </div>

      <footer className="display-footer" data-ui-id={`display.${kind}.footer`}>
        <span className="display-hints">{showHints ? footerHints : null}</span>
        <span className="display-footer-right">{footerRight}</span>
      </footer>
    </section>
  );
}

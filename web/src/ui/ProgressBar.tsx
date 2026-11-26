import { IconButton } from "./IconButton";

/** Human ETA for progress UI; null when unknown. */
export function formatEta(seconds: number | null | undefined): string | null {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds)) {
    return null;
  }
  if (seconds < 1) {
    return "<1 s left";
  }
  if (seconds < 60) {
    return `${Math.round(seconds)} s left`;
  }
  const minutes = Math.floor(seconds / 60);
  const rest = Math.round(seconds % 60);
  return `${minutes}:${rest.toString().padStart(2, "0")} left`;
}

export interface ProgressBarProps {
  /** 0..=1; null/undefined = indeterminate. */
  value?: number | null;
  label?: string;
  etaSeconds?: number | null;
  onCancel?: () => void;
  className?: string;
}

/** Determinate or indeterminate progress bar with % / ETA and Cancel. */
export function ProgressBar({
  value,
  label = "Progress",
  etaSeconds,
  onCancel,
  className,
}: ProgressBarProps) {
  const determinate = typeof value === "number" && Number.isFinite(value);
  const fraction = determinate ? Math.min(1, Math.max(0, value)) : null;
  const percent = fraction === null ? null : Math.round(fraction * 100);
  const eta = formatEta(etaSeconds);

  return (
    <div className={`progress${className ? ` ${className}` : ""}`}>
      <div className="progress-head">
        <span className="progress-label">{label}</span>
        <span className="progress-value" aria-hidden="true">
          {percent === null ? "working" : `${percent}%`}
          {eta ? <span className="progress-eta"> · {eta}</span> : null}
        </span>
      </div>
      <div
        className={`progress-track${determinate ? "" : " indeterminate"}`}
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        aria-valuetext={percent === null ? "Working" : `${percent}%`}
      >
        <div
          className="progress-fill"
          style={determinate ? { width: `${percent}%` } : undefined}
        />
      </div>
      {onCancel ? (
        <IconButton label="Cancel" className="progress-cancel" onClick={onCancel}>
          ×
        </IconButton>
      ) : null}
    </div>
  );
}

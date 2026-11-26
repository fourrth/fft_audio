import {
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { INFERNO_CSS_GRADIENT } from "./colormap";
import { dbFromFraction, fractionFromDb } from "./geometry";

export interface ColorBarProps {
  minDb: number;
  maxDb: number;
  /**
   * Stable dB domain shown behind the selected range. Defaults to the current
   * min/max, which puts the handles at the track edges.
   */
  domainMinDb?: number;
  domainMaxDb?: number;
  /** Called with the new (min, max); a handle keeps at least a 1 dB gap. */
  onChange: (minDb: number, maxDb: number) => void;
  /** Called once when a pointer drag ends or on each keyboard adjustment. */
  onCommit?: (minDb: number, maxDb: number) => void;
  /** Restore the default color range. */
  onReset?: () => void;
}

/**
 * 58 px interactive color bar. The top handle is the maximum dB, the bottom
 * handle the minimum; both are draggable pointers and keyboard-adjustable.
 */
export function ColorBar({
  minDb,
  maxDb,
  domainMinDb = minDb,
  domainMaxDb = maxDb,
  onChange,
  onCommit,
  onReset,
}: ColorBarProps) {
  const trackRef = useRef<HTMLDivElement>(null);
  const latest = useRef({ minDb, maxDb });
  const rangeDrag = useRef<{
    startY: number;
    minDb: number;
    maxDb: number;
  } | null>(null);
  const [drag, setDrag] = useState<"min" | "max" | "range" | null>(null);
  const [live, setLive] = useState<string | null>(null);

  // A stable domain keeps both handles away from the edges so they can be
  // dragged outward as well as inward. Guard against bad/inverted settings.
  const domainLo = Math.min(domainMinDb, minDb);
  const domainHi = Math.max(domainMaxDb, maxDb);
  const domainValid =
    Number.isFinite(domainLo) &&
    Number.isFinite(domainHi) &&
    domainHi > domainLo;
  const safeDomainMin = domainValid ? domainLo : minDb;
  const safeDomainMax = domainValid ? domainHi : Math.max(maxDb, minDb + 1);

  // Keep the latest values for the pointer-up commit. While dragging, local
  // updates win over props, which may not have committed yet.
  if (!drag) {
    latest.current = { minDb, maxDb };
  }

  const apply = (
    which: "min" | "max",
    db: number,
    commit: boolean,
  ): { minDb: number; maxDb: number } => {
    const next =
      which === "max"
        ? {
            minDb,
            maxDb: Math.min(
              safeDomainMax,
              Math.max(minDb + 1, db),
            ),
          }
        : {
            minDb: Math.max(
              safeDomainMin,
              Math.min(maxDb - 1, db),
            ),
            maxDb,
          };
    latest.current = next;
    onChange(next.minDb, next.maxDb);
    if (commit) {
      onCommit?.(next.minDb, next.maxDb);
    }
    return next;
  };

  useEffect(() => {
    if (!drag) {
      return;
    }
    const move = (event: PointerEvent) => {
      const rect = trackRef.current?.getBoundingClientRect();
      if (!rect || rect.height <= 0) {
        return;
      }
      if (drag === "range") {
        const start = rangeDrag.current;
        if (!start) {
          return;
        }
        const delta =
          ((start.startY - event.clientY) / rect.height) *
          (safeDomainMax - safeDomainMin);
        const span = start.maxDb - start.minDb;
        let nextMin = start.minDb + delta;
        let nextMax = start.maxDb + delta;
        if (nextMin < safeDomainMin) {
          nextMin = safeDomainMin;
          nextMax = safeDomainMin + span;
        }
        if (nextMax > safeDomainMax) {
          nextMax = safeDomainMax;
          nextMin = safeDomainMax - span;
        }
        const next = { minDb: nextMin, maxDb: nextMax };
        latest.current = next;
        onChange(next.minDb, next.maxDb);
        setLive(
          `${next.minDb.toFixed(1)}–${next.maxDb.toFixed(1)} dB`,
        );
        return;
      }
      const fraction = (event.clientY - rect.top) / rect.height;
      const db = dbFromFraction(fraction, safeDomainMin, safeDomainMax);
      setLive(`${db.toFixed(1)} dB`);
      apply(drag, db, false);
    };
    const up = () => {
      setDrag(null);
      setLive(null);
      rangeDrag.current = null;
      onCommit?.(latest.current.minDb, latest.current.maxDb);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
  }, [
    drag,
    maxDb,
    minDb,
    onCommit,
    onChange,
    safeDomainMax,
    safeDomainMin,
  ]);

  const handleKey = (which: "min" | "max", delta: number) => {
    apply(which, (which === "max" ? maxDb : minDb) + delta, true);
  };

  const shiftRange = (delta: number) => {
    const span = maxDb - minDb;
    let nextMin = minDb + delta;
    let nextMax = maxDb + delta;
    if (nextMin < safeDomainMin) {
      nextMin = safeDomainMin;
      nextMax = safeDomainMin + span;
    }
    if (nextMax > safeDomainMax) {
      nextMax = safeDomainMax;
      nextMin = safeDomainMax - span;
    }
    if (nextMin === minDb && nextMax === maxDb) {
      return;
    }
    latest.current = { minDb: nextMin, maxDb: nextMax };
    onChange(nextMin, nextMax);
    onCommit?.(nextMin, nextMax);
  };

  const beginRangeDrag = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (event.pointerType === "mouse" && event.button !== 0) {
      return;
    }
    event.preventDefault();
    rangeDrag.current = { startY: event.clientY, minDb, maxDb };
    latest.current = { minDb, maxDb };
    setDrag("range");
    setLive(`${minDb.toFixed(1)}–${maxDb.toFixed(1)} dB`);
  };

  const maxFraction = fractionFromDb(maxDb, safeDomainMin, safeDomainMax);
  const minFraction = fractionFromDb(minDb, safeDomainMin, safeDomainMax);
  const selectionTop = Math.min(maxFraction, minFraction);
  const selectionHeight = Math.abs(minFraction - maxFraction);
  const gap = maxDb - minDb;

  return (
    <div
      className="colorbar"
      data-testid="colorbar"
      aria-label="color range"
    >
      <div className="colorbar-top">
        <span
          className="colorbar-value colorbar-value-max"
          data-testid="colorbar-max-value"
        >
          {Math.round(maxDb)}
        </span>
        <button
          type="button"
          className="colorbar-reset"
          aria-label="Reset color range"
          title="Reset color range"
          disabled={!onReset}
          onClick={onReset}
        >
          ↺
        </button>
      </div>
      <div className="colorbar-track" ref={trackRef}>
        <div
          className="colorbar-gradient"
          style={{ background: INFERNO_CSS_GRADIENT }}
        />
        <div
          className="colorbar-outside colorbar-outside-top"
          style={{ height: `${selectionTop * 100}%` }}
          aria-hidden="true"
        />
        <div
          className="colorbar-outside colorbar-outside-bottom"
          style={{ top: `${(selectionTop + selectionHeight) * 100}%` }}
          aria-hidden="true"
        />
        <button
          type="button"
          className="colorbar-selection"
          aria-label="Move color range"
          title="Drag to move the color range"
          style={{
            top: `${selectionTop * 100}%`,
            height: `${selectionHeight * 100}%`,
          }}
          onPointerDown={beginRangeDrag}
          onKeyDown={(event) => {
            if (event.key === "ArrowUp") shiftRange(1);
            if (event.key === "ArrowDown") shiftRange(-1);
          }}
        />
        <button
          type="button"
          role="slider"
          className="colorbar-handle handle-max"
          aria-label="Maximum dB"
          aria-valuemin={minDb + 1}
          aria-valuemax={safeDomainMax}
          aria-valuenow={Math.round(maxDb)}
          aria-valuetext={`${maxDb.toFixed(1)} dB`}
          title={`${maxDb.toFixed(1)} dB`}
          style={{ top: `${maxFraction * 100}%` }}
          onPointerDown={(event) => {
            event.preventDefault();
            setDrag("max");
            setLive(`${maxDb.toFixed(1)} dB`);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowUp") handleKey("max", 1);
            if (event.key === "ArrowDown") handleKey("max", -1);
          }}
        >
          <span className="colorbar-handle-grip" aria-hidden="true" />
        </button>
        <button
          type="button"
          role="slider"
          className="colorbar-handle handle-min"
          aria-label="Minimum dB"
          aria-valuemin={safeDomainMin}
          aria-valuemax={maxDb - 1}
          aria-valuenow={Math.round(minDb)}
          aria-valuetext={`${minDb.toFixed(1)} dB`}
          title={`${minDb.toFixed(1)} dB`}
          style={{ top: `${minFraction * 100}%` }}
          onPointerDown={(event) => {
            event.preventDefault();
            setDrag("min");
            setLive(`${minDb.toFixed(1)} dB`);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowUp") handleKey("min", 1);
            if (event.key === "ArrowDown") handleKey("min", -1);
          }}
        >
          <span className="colorbar-handle-grip" aria-hidden="true" />
        </button>
      </div>
      <span
        className="colorbar-value colorbar-value-min"
        data-testid="colorbar-min-value"
      >
        {Math.round(minDb)}
      </span>
      <span className="colorbar-unit">dB</span>
      <span className="colorbar-live" role="status">
        {live ?? `${gap.toFixed(0)} dB range`}
      </span>
    </div>
  );
}

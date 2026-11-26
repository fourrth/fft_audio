import { useEffect, useRef, useState, type RefObject } from "react";
import {
  snapLineEndpoint,
  stripLineSnapHistory,
  toggleLineOrientation,
  type LineAnchor,
  type LineSpec,
} from "../cuts/model";
import {
  LOG_MIN_HZ,
  freqAtFraction,
  freqFraction,
  freqToY,
  panView,
  timeToX,
  viewFromRegion,
  zoomView,
  type FrequencyBounds,
  type Viewport,
} from "./geometry";
import { heatmapPixels } from "./heatmap";
import {
  type SpectrogramLineCut,
  type SpectrogramOrderGuide,
} from "./overlays";
import { frequencyTicks, formatHz, formatTime, niceTimeTicks } from "./ticks";
import { dbAt, type DecodedGrid } from "./types";

export interface SpectrogramCanvasProps {
  grid: DecodedGrid;
  view: Viewport;
  minDb: number;
  maxDb: number;
  domainMinDb?: number;
  domainMaxDb?: number;
  minHz?: number;
  maxHz?: number;
  /** Draw the tick-aligned grid lines; axis labels stay visible either way. */
  showGrid?: boolean;
  /** Fit the view; called by double-clicking the plot background. */
  onResetView?: () => void;
  onViewChange: (view: Viewport) => void;
  orders?: readonly number[];
  rpm?: number | null;
  onReadoutChange?: (text: string | null) => void;
  lineCuts?: SpectrogramLineCut[];
  orderGuides?: SpectrogramOrderGuide[];
  onUpdateLineCut?: (
    panelId: string,
    lineIndex: number,
    line: LineSpec,
  ) => void;
}

interface HostSize {
  width: number;
  height: number;
}

const INITIAL_HOST_SIZE: HostSize = { width: 800, height: 420 };

function requestFrame(callback: () => void): number {
  if (
    typeof window !== "undefined" &&
    typeof window.requestAnimationFrame === "function"
  ) {
    return window.requestAnimationFrame(callback);
  }
  return globalThis.setTimeout(callback, 16);
}

function cancelFrame(handle: number): void {
  if (
    typeof window !== "undefined" &&
    typeof window.cancelAnimationFrame === "function"
  ) {
    window.cancelAnimationFrame(handle);
    return;
  }
  globalThis.clearTimeout(handle);
}

/**
 * Observe the real host box. Dockview can move/resize panels without changing
 * React state, so resize/layout notifications also schedule a fresh measure.
 */
function useHostSize(ref: RefObject<HTMLDivElement | null>): HostSize {
  const [size, setSize] = useState<HostSize>(INITIAL_HOST_SIZE);
  useEffect(() => {
    const element = ref.current;
    if (!element) {
      return;
    }
    let frame = 0;
    let disposed = false;

    const measure = (attempt = 0) => {
      if (disposed) {
        return;
      }
      const width = element.clientWidth;
      const height = element.clientHeight;
      if (width > 1 && height > 1) {
        // A new object forces the draw effect even when the size is unchanged
        // (e.g. the panel was hidden and re-shown by a layout change).
        setSize({ width, height });
        return;
      }
      // Ignore transient 0×1 boxes, but retry for a few frames.
      if (attempt < 4) {
        frame = requestFrame(() => measure(attempt + 1));
      }
    };
    const schedule = () => {
      cancelFrame(frame);
      frame = requestFrame(() => measure());
    };
    const onLayout = () => {
      cancelFrame(frame);
      // Wait for Dockview to finish its own layout pass.
      frame = requestFrame(() => {
        frame = requestFrame(() => measure());
      });
    };

    measure();
    let observer: ResizeObserver | null = null;
    if (typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(schedule);
      observer.observe(element);
    }
    window.addEventListener("resize", schedule);
    window.addEventListener("fft:layout-change", onLayout);
    return () => {
      disposed = true;
      cancelFrame(frame);
      observer?.disconnect();
      window.removeEventListener("resize", schedule);
      window.removeEventListener("fft:layout-change", onLayout);
    };
  }, [ref]);
  return size;
}

interface Readout {
  x: number;
  y: number;
  time: number;
  hz: number;
  db: number | null;
}

interface LineOverlayDrag {
  kind: "body" | "start" | "end";
  panelId: string;
  lineIndex: number;
  line: LineSpec;
  pointerId: number;
  startTime: number;
  startFreq: number;
  snap: boolean;
  snapDeg: number;
}

interface SelectionRect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

type SpectrogramGesture =
  | {
      mode: "pan";
      startX: number;
      startY: number;
      baseView: Viewport;
    }
  | {
      mode: "region";
      startX: number;
      startY: number;
      baseView: Viewport;
    };

export function SpectrogramCanvas({
  grid,
  view,
  minDb,
  maxDb,
  domainMinDb = minDb,
  domainMaxDb = maxDb,
  minHz = 0,
  maxHz = grid.meta.nyquist,
  showGrid = true,
  onResetView,
  onViewChange,
  orders = [],
  rpm,
  onReadoutChange,
  lineCuts = [],
  orderGuides = [],
  onUpdateLineCut,
}: SpectrogramCanvasProps) {
  const bounds: FrequencyBounds = {
    minHz: Math.max(0, Number.isFinite(minHz) ? minHz : 0),
    maxHz: Math.max(1, Number.isFinite(maxHz) ? maxHz : grid.meta.nyquist),
  };
  const hostRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const overlayRef = useRef<SVGSVGElement>(null);
  const size = useHostSize(hostRef);
  const width = size.width;
  const height = size.height;
  const [readout, setReadout] = useState<Readout | null>(null);
  const gesture = useRef<SpectrogramGesture | null>(null);
  const [selectionRect, setSelectionRect] = useState<SelectionRect | null>(null);
  const pointer = useRef<{ clientX: number; clientY: number } | null>(null);
  const frame = useRef<number | undefined>(undefined);
  const [selectedLine, setSelectedLine] = useState<{
    panelId: string;
    lineIndex: number;
  } | null>(null);
  const lineDrag = useRef<LineOverlayDrag | null>(null);
  const viewRef = useRef(view);
  viewRef.current = view;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") {
        return;
      }
      gesture.current = null;
      pointer.current = null;
      setSelectionRect(null);
      if (frame.current !== undefined) {
        cancelFrame(frame.current);
        frame.current = undefined;
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  useEffect(
    () => () => {
      if (frame.current !== undefined) {
        cancelFrame(frame.current);
        frame.current = undefined;
      }
    },
    [],
  );

  useEffect(() => {
    const host = hostRef.current;
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!host || !canvas || !ctx) {
      return;
    }
    // Read the live box: a layout change may have happened since the last
    // observer callback. Fall back to the last good measure for 0×1 boxes.
    const liveWidth = host.clientWidth;
    const liveHeight = host.clientHeight;
    const drawWidth = Math.max(
      1,
      Math.round(liveWidth > 1 ? liveWidth : size.width),
    );
    const drawHeight = Math.max(
      1,
      Math.round(liveHeight > 1 ? liveHeight : size.height),
    );
    canvas.width = drawWidth;
    canvas.height = drawHeight;
    const pixels = heatmapPixels(
      grid,
      view,
      drawWidth,
      drawHeight,
      minDb,
      maxDb,
      domainMinDb,
      domainMaxDb,
    );
    const image = ctx.createImageData(drawWidth, drawHeight);
    image.data.set(pixels);
    ctx.putImageData(image, 0, 0);
  }, [domainMaxDb, domainMinDb, grid, maxDb, minDb, size, view]);

  const point = (event: React.PointerEvent<HTMLCanvasElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const x = Math.min(Math.max(event.clientX - rect.left, 0), rect.width);
    const y = Math.min(Math.max(event.clientY - rect.top, 0), rect.height);
    return { rect, x, y };
  };

  const readAt = (x: number, y: number, rect: DOMRect) => {
    const current = viewRef.current;
    const time =
      current.t0 + (x / Math.max(1, rect.width)) * (current.t1 - current.t0);
    const hz = freqAtFraction(1 - y / Math.max(1, rect.height), current);
    const db = dbAt(grid, time, hz);
    setReadout({ x, y, time, hz, db });
    onReadoutChange?.(
      `t ${time.toFixed(3)} s · f ${hz.toFixed(1)} Hz · ${
        db === null ? "—" : `${db.toFixed(1)} dB`
      }`,
    );
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) {
      return;
    }
    const onWheel = (event: WheelEvent) => {
      // React's synthetic wheel listener is passive in some browsers, so use
      // a native non-passive listener.
      event.preventDefault();
      const rect = canvas.getBoundingClientRect();
      if (!(rect.width > 0) || !(rect.height > 0)) {
        return;
      }
      const current = viewRef.current;
      const x = Math.min(Math.max(event.clientX - rect.left, 0), rect.width);
      const y = Math.min(Math.max(event.clientY - rect.top, 0), rect.height);
      const unit =
        event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? rect.height : 1;
      let deltaX = event.deltaX * unit;
      let deltaY = event.deltaY * unit;

      if (event.ctrlKey || event.metaKey) {
        const anchorT =
          current.t0 + (x / rect.width) * (current.t1 - current.t0);
        const anchorF = freqAtFraction(1 - y / rect.height, current);
        const factor = deltaY < 0 ? 0.8 : 1.25;
        const next = zoomView(
          current,
          factor,
          anchorT,
          anchorF,
          grid.meta.durationS,
          grid.meta.nyquist,
          current.logFreq,
          bounds,
        );
        // Compose rapid wheel ticks before React has committed the last one.
        viewRef.current = next;
        onViewChange(next);
        return;
      }

      // Shift+wheel pans time; plain wheel pans frequency. Some browsers
      // report Shift+wheel as deltaX instead of deltaY.
      if (event.shiftKey) {
        if (deltaX === 0) {
          deltaX = deltaY;
          deltaY = 0;
        }
      } else {
        deltaX = 0;
      }
      if (deltaX === 0 && deltaY === 0) {
        return;
      }
      const spanT = current.t1 - current.t0;
      const spanF = current.f1 - current.f0;
      const dt = (deltaX / rect.width) * spanT;
      const df = -(deltaY / rect.height) * spanF;
      const next = panView(
        current,
        dt,
        df,
        grid.meta.durationS,
        grid.meta.nyquist,
        bounds,
      );
      viewRef.current = next;
      onViewChange(next);
    };
    canvas.addEventListener("wheel", onWheel, { passive: false });
    return () => canvas.removeEventListener("wheel", onWheel);
  }, [
    bounds.maxHz,
    bounds.minHz,
    grid.meta.durationS,
    grid.meta.nyquist,
    onViewChange,
  ]);

  const processPointer = () => {
    frame.current = undefined;
    const latest = pointer.current;
    const canvas = canvasRef.current;
    if (!latest || !canvas) {
      return;
    }
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      return;
    }
    const x = Math.min(Math.max(latest.clientX - rect.left, 0), rect.width);
    const y = Math.min(Math.max(latest.clientY - rect.top, 0), rect.height);
    const active = gesture.current;
    if (active?.mode === "region") {
      const nextRect: SelectionRect = {
        x0: Math.min(active.startX, x),
        y0: Math.min(active.startY, y),
        x1: Math.max(active.startX, x),
        y1: Math.max(active.startY, y),
      };
      setSelectionRect(nextRect);
      return;
    }
    if (active?.mode === "pan") {
      const dt =
        -((x - active.startX) / rect.width) *
        (active.baseView.t1 - active.baseView.t0);
      const df =
        ((y - active.startY) / rect.height) *
        (active.baseView.f1 - active.baseView.f0);
      const next = panView(
        active.baseView,
        dt,
        df,
        grid.meta.durationS,
        grid.meta.nyquist,
        bounds,
      );
      viewRef.current = next;
      onViewChange(next);
      return;
    }
    readAt(x, y, rect);
  };

  const schedulePointer = () => {
    if (frame.current !== undefined) {
      return;
    }
    frame.current = requestFrame(processPointer);
  };

  const finishGesture = (
    canvas: HTMLCanvasElement,
    clientX: number,
    clientY: number,
  ) => {
    pointer.current = { clientX, clientY };
    if (frame.current !== undefined) {
      cancelFrame(frame.current);
      frame.current = undefined;
    }
    processPointer();
    const active = gesture.current;
    const rect = canvas.getBoundingClientRect();
    if (active && rect.width > 0 && rect.height > 0) {
      const x = Math.min(Math.max(clientX - rect.left, 0), rect.width);
      const y = Math.min(Math.max(clientY - rect.top, 0), rect.height);
      if (active.mode === "region") {
        const next = viewFromRegion(
          active.baseView,
          { x: active.startX, y: active.startY },
          { x, y },
          rect.width,
          rect.height,
          grid.meta.durationS,
          grid.meta.nyquist,
          undefined,
          bounds,
        );
        if (next) {
          viewRef.current = next;
          onViewChange(next);
        }
      } else {
        // Refresh once at the end of a pan instead of on every pointermove.
        readAt(x, y, rect);
      }
    }
    pointer.current = null;
    gesture.current = null;
    setSelectionRect(null);
  };

  const beginLineDrag = (
    event: React.PointerEvent<SVGElement>,
    cut: SpectrogramLineCut,
    kind: "body" | "start" | "end",
  ) => {
    if (event.pointerType === "mouse" && event.button !== 0) {
      return;
    }
    const rect = overlayRef.current?.getBoundingClientRect();
    if (!rect || rect.width <= 0 || rect.height <= 0) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    setSelectedLine({ panelId: cut.panelId, lineIndex: cut.lineIndex });
    const fractionX = Math.min(
      Math.max((event.clientX - rect.left) / rect.width, 0),
      1,
    );
    const fractionY = Math.min(
      Math.max((event.clientY - rect.top) / rect.height, 0),
      1,
    );
    lineDrag.current = {
      kind,
      panelId: cut.panelId,
      lineIndex: cut.lineIndex,
      line: cut.line,
      pointerId: event.pointerId,
      startTime: view.t0 + fractionX * (view.t1 - view.t0),
      startFreq: freqAtFraction(1 - fractionY, view),
      snap: cut.snap,
      snapDeg: cut.snapDeg,
    };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };

  const moveLineDrag = (event: React.PointerEvent<SVGSVGElement>) => {
    const active = lineDrag.current;
    const rect = overlayRef.current?.getBoundingClientRect();
    if (!active || !rect || !onUpdateLineCut) {
      return;
    }
    if (rect.width <= 0 || rect.height <= 0) {
      return;
    }
    event.preventDefault();
    const fractionX = Math.min(
      Math.max((event.clientX - rect.left) / rect.width, 0),
      1,
    );
    const fractionY = Math.min(
      Math.max((event.clientY - rect.top) / rect.height, 0),
      1,
    );
    let time = view.t0 + fractionX * (view.t1 - view.t0);
    let hz = freqAtFraction(1 - fractionY, view);
    const duration = grid.meta.durationS;
    const top = grid.meta.nyquist;
    // Lines are stored in Hz and may sit at DC. A log view already stops the
    // pointer at its axis floor (LOG_MIN_HZ); a linear view reaches 0 Hz.
    const floor = 0;
    const clampT = (value: number) =>
      Math.min(duration, Math.max(0, value));
    const clampF = (value: number) => Math.min(top, Math.max(floor, value));

    if (active.kind !== "body" && active.snap) {
      const fixedTime = active.kind === "start" ? active.line.t1 : active.line.t0;
      const fixedHz = active.kind === "start" ? active.line.f1 : active.line.f0;
      const snapped = snapLineEndpoint({
        fixedX: timeToX(fixedTime, view, width),
        fixedY: freqToY(fixedHz, view, height),
        x: timeToX(time, view, width),
        y: freqToY(hz, view, height),
        time,
        hz,
        fixedTime,
        fixedHz,
        snapDeg: active.snapDeg,
      });
      time = snapped.time;
      hz = snapped.hz;
    }

    let next: LineSpec;
    const baseLine = stripLineSnapHistory(active.line);
    if (active.kind === "start" || active.kind === "end") {
      next =
        active.kind === "start"
          ? { ...baseLine, t0: clampT(time), f0: clampF(hz) }
          : { ...baseLine, t1: clampT(time), f1: clampF(hz) };
    } else {
      const dt = time - active.startTime;
      const df = hz - active.startFreq;
      const tLo = Math.min(baseLine.t0, baseLine.t1);
      const tHi = Math.max(baseLine.t0, baseLine.t1);
      const tSpan = Math.max(0, tHi - tLo);
      const targetT0 = Math.min(
        Math.max(tLo + dt, 0),
        Math.max(0, duration - tSpan),
      );
      const shiftT = targetT0 - tLo;
      const fLo = Math.min(baseLine.f0, baseLine.f1);
      const fHi = Math.max(baseLine.f0, baseLine.f1);
      const fSpan = Math.max(0, fHi - fLo);
      const targetF0 = Math.min(
        Math.max(fLo + df, floor),
        Math.max(floor, top - fSpan),
      );
      const shiftF = targetF0 - fLo;
      next = {
        ...baseLine,
        t0: baseLine.t0 + shiftT,
        t1: baseLine.t1 + shiftT,
        f0: baseLine.f0 + shiftF,
        f1: baseLine.f1 + shiftF,
      };
    }
    onUpdateLineCut(active.panelId, active.lineIndex, next);
  };

  const toggleLineOrientationAtClick = (
    event: React.MouseEvent<SVGGElement>,
    cut: SpectrogramLineCut,
  ) => {
    event.preventDefault();
    event.stopPropagation();
    if (!onUpdateLineCut) {
      return;
    }
    let anchor: LineAnchor = "start";
    const rect = overlayRef.current?.getBoundingClientRect();
    if (rect && rect.width > 0 && rect.height > 0) {
      const x = event.clientX - rect.left;
      const y = event.clientY - rect.top;
      const startDistance = Math.hypot(
        x - timeToX(cut.line.t0, view, width),
        y - freqToY(cut.line.f0, view, height),
      );
      const endDistance = Math.hypot(
        x - timeToX(cut.line.t1, view, width),
        y - freqToY(cut.line.f1, view, height),
      );
      anchor = startDistance <= endDistance ? "start" : "end";
    }
    const next = toggleLineOrientation(cut.line, anchor, {
      minHz: 0,
      maxHz: grid.meta.nyquist,
      durationS: grid.meta.durationS,
    });
    onUpdateLineCut(cut.panelId, cut.lineIndex, next);
  };

  const nearest = (() => {
    if (!readout || !rpm || rpm <= 0 || orders.length === 0) {
      return null;
    }
    const actual = (readout.hz * 60) / rpm;
    return orders.reduce((best, order) =>
      Math.abs(actual - order) < Math.abs(actual - best) ? order : best,
    );
  })();

  const overscan =
    view.t0 < 0 ||
    view.t1 > grid.meta.durationS ||
    view.f1 > grid.meta.nyquist ||
    (view.logFreq ? view.f0 < LOG_MIN_HZ : view.f0 < 0);
  const dataX0 = Math.max(0, Math.min(width, timeToX(0, view, width)));
  const dataX1 = Math.max(
    0,
    Math.min(width, timeToX(grid.meta.durationS, view, width)),
  );
  const dataY0 = Math.max(
    0,
    Math.min(height, freqToY(grid.meta.nyquist, view, height)),
  );
  const dataY1 = Math.max(
    0,
    Math.min(height, freqToY(0, view, height)),
  );
  const dataWidth = Math.max(0, dataX1 - dataX0);
  const dataHeight = Math.max(0, dataY1 - dataY0);
  const freqTickValues = frequencyTicks(
    view,
    Math.max(2, Math.round(height / 48)),
  );
  const timeTickValues = niceTimeTicks(
    view.t0,
    view.t1,
    Math.max(2, Math.round(width / 90)),
  );
  const timeSpan = view.t1 - view.t0;

  return (
    <div className="plot-surface">
      <div
        className="axis-gutter axis-freq"
        data-testid="frequency-axis"
        aria-hidden="true"
      >
        <span className="axis-unit">Hz</span>
        {freqTickValues.map((hz) => (
          <span
            key={`f-${hz}`}
            className="axis-tick axis-tick-freq"
            style={{
              bottom: `${Math.min(98, Math.max(2, freqFraction(hz, view) * 100))}%`,
            }}
          >
            {formatHz(hz)}
          </span>
        ))}
      </div>
      <div
        className="plot-host"
        ref={hostRef}
        data-testid="spectrogram-plot"
        data-overscan={overscan ? "true" : "false"}
        onDoubleClick={(event) => {
          const target =
            event.target instanceof Element
              ? event.target.closest(
                  ".spectrogram-line-cut, .spectrogram-line-hit, " +
                    ".spectrogram-line-handle, .spectrogram-order-guide",
                )
              : null;
          if (!target) {
            onResetView?.();
          }
        }}
      >
      <canvas
        ref={canvasRef}
        className="plot-canvas"
        onPointerMove={(event) => {
          pointer.current = {
            clientX: event.clientX,
            clientY: event.clientY,
          };
          schedulePointer();
        }}
        onPointerDown={(event) => {
          if (event.pointerType === "mouse" && event.button === 2) {
            return;
          }
          if (
            event.pointerType === "mouse" &&
            event.button !== 0 &&
            event.button !== 1
          ) {
            return;
          }
          event.preventDefault();
          setReadout(null);
          onReadoutChange?.(null);
          const { rect, x, y } = point(event);
          if (rect.width <= 0 || rect.height <= 0) {
            return;
          }
          const pan =
            event.pointerType !== "mouse" ||
            event.button === 1 ||
            event.shiftKey;
          gesture.current = {
            mode: pan ? "pan" : "region",
            startX: x,
            startY: y,
            baseView: viewRef.current,
          };
          pointer.current = {
            clientX: event.clientX,
            clientY: event.clientY,
          };
          if (pan) {
            setSelectionRect(null);
          } else {
            const initial: SelectionRect = { x0: x, y0: y, x1: x, y1: y };
            setSelectionRect(initial);
          }
          event.currentTarget.setPointerCapture?.(event.pointerId);
        }}
        onPointerUp={(event) => {
          const canvas = event.currentTarget;
          finishGesture(canvas, event.clientX, event.clientY);
          if (canvas.hasPointerCapture?.(event.pointerId)) {
            canvas.releasePointerCapture?.(event.pointerId);
          }
        }}
        onPointerCancel={() => {
          gesture.current = null;
          pointer.current = null;
          setSelectionRect(null);
          if (frame.current !== undefined) {
            cancelFrame(frame.current);
            frame.current = undefined;
          }
        }}
        onLostPointerCapture={() => {
          if (gesture.current) {
            gesture.current = null;
            pointer.current = null;
            setSelectionRect(null);
          }
        }}
        onPointerLeave={() => {
          if (!gesture.current) {
            pointer.current = null;
            if (frame.current !== undefined) {
              cancelFrame(frame.current);
              frame.current = undefined;
            }
          }
          setReadout(null);
          onReadoutChange?.(null);
        }}
      />
      <svg
        ref={overlayRef}
        className="spectrogram-overlays"
        data-testid="spectrogram-overlays"
        viewBox={`0 0 ${Math.max(1, width)} ${Math.max(1, height)}`}
        preserveAspectRatio="none"
        onPointerMove={moveLineDrag}
        onPointerUp={() => {
          lineDrag.current = null;
        }}
        onPointerCancel={() => {
          lineDrag.current = null;
        }}
        onLostPointerCapture={() => {
          lineDrag.current = null;
        }}
      >
        {showGrid ? (
          <>
            {freqTickValues.map((hz) => {
              const y = freqToY(hz, view, height);
              return (
                <line
                  key={`grid-f-${hz}`}
                  className="spectrogram-grid-line"
                  x1={0}
                  x2={width}
                  y1={y}
                  y2={y}
                  vectorEffect="non-scaling-stroke"
                />
              );
            })}
            {timeTickValues.map((time) => {
              const x = timeToX(time, view, width);
              return (
                <line
                  key={`grid-t-${time}`}
                  className="spectrogram-grid-line"
                  x1={x}
                  x2={x}
                  y1={0}
                  y2={height}
                  vectorEffect="non-scaling-stroke"
                />
              );
            })}
          </>
        ) : null}
        {overscan && dataWidth > 0.5 && dataHeight > 0.5 ? (
          <rect
            className="spectrogram-data-frame"
            data-testid="spectrogram-data-frame"
            x={dataX0}
            y={dataY0}
            width={dataWidth}
            height={dataHeight}
            fill="none"
            stroke="#000"
            strokeWidth={1}
            vectorEffect="non-scaling-stroke"
            pointerEvents="none"
          />
        ) : null}
        {orderGuides.map((guide) => {
          if (guide.points.length < 2) {
            return null;
          }
          const points = guide.points
            .map(
              (point) =>
                `${timeToX(point.timeS, view, width)},${freqToY(
                  point.freqHz,
                  view,
                  height,
                )}`,
            )
            .join(" ");
          return (
            <polyline
              key={guide.id}
              className="spectrogram-order-guide"
              data-panel-id={guide.panelId}
              data-order={guide.order}
              points={points}
              fill="none"
              stroke={guide.color}
              strokeDasharray="6 4"
              strokeWidth={1.5}
              vectorEffect="non-scaling-stroke"
            />
          );
        })}
        {lineCuts.map((cut) => {
          const selected =
            selectedLine?.panelId === cut.panelId &&
            selectedLine.lineIndex === cut.lineIndex;
          const x1 = timeToX(cut.line.t0, view, width);
          const x2 = timeToX(cut.line.t1, view, width);
          const y1 = freqToY(cut.line.f0, view, height);
          const y2 = freqToY(cut.line.f1, view, height);
          return (
            <g
              key={`${cut.panelId}:${cut.lineIndex}`}
              data-line-key={`${cut.panelId}:${cut.lineIndex}`}
              data-line-panel-id={cut.panelId}
              data-line-index={cut.lineIndex}
              onDoubleClick={(event) =>
                toggleLineOrientationAtClick(event, cut)
              }
            >
              <line
                className="spectrogram-line-cut"
                x1={x1}
                x2={x2}
                y1={y1}
                y2={y2}
                stroke={cut.color}
                strokeWidth={selected ? 3 : 2}
                vectorEffect="non-scaling-stroke"
                pointerEvents="stroke"
                style={{ cursor: "move" }}
                onPointerDown={(event) => beginLineDrag(event, cut, "body")}
              />
              <line
                className="spectrogram-line-hit"
                x1={x1}
                x2={x2}
                y1={y1}
                y2={y2}
                stroke="transparent"
                strokeWidth={12}
                vectorEffect="non-scaling-stroke"
                pointerEvents="stroke"
                style={{ cursor: "move" }}
                onPointerDown={(event) => beginLineDrag(event, cut, "body")}
              />
              {selected ? (
                <>
                  <circle
                    className="spectrogram-line-handle"
                    data-handle="start"
                    cx={x1}
                    cy={y1}
                    r={6}
                    fill={cut.color}
                    stroke="#0f1217"
                    strokeWidth={2}
                    vectorEffect="non-scaling-stroke"
                    pointerEvents="all"
                    style={{ cursor: "grab" }}
                    onPointerDown={(event) =>
                      beginLineDrag(event, cut, "start")
                    }
                  />
                  <circle
                    className="spectrogram-line-handle"
                    data-handle="end"
                    cx={x2}
                    cy={y2}
                    r={6}
                    fill={cut.color}
                    stroke="#0f1217"
                    strokeWidth={2}
                    vectorEffect="non-scaling-stroke"
                    pointerEvents="all"
                    style={{ cursor: "grab" }}
                    onPointerDown={(event) =>
                      beginLineDrag(event, cut, "end")
                    }
                  />
                </>
              ) : null}
            </g>
          );
        })}
      </svg>
      {selectionRect ? (
        <div
          className="spectrogram-region"
          data-testid="spectrogram-region"
          style={{
            left: selectionRect.x0,
            top: selectionRect.y0,
            width: Math.max(0, selectionRect.x1 - selectionRect.x0),
            height: Math.max(0, selectionRect.y1 - selectionRect.y0),
          }}
        />
      ) : null}
      {readout ? (
        <>
          <span className="crosshair crosshair-x" style={{ top: readout.y }} />
          <span className="crosshair crosshair-y" style={{ left: readout.x }} />
          <div
            className="readout-card"
            style={{
              left: Math.min(Math.max(readout.x + 12, 8), Math.max(8, width - 190)),
              top: Math.min(Math.max(readout.y + 12, 8), height - 76),
            }}
            data-testid="readout"
          >
            <span>{readout.time.toFixed(3)} s</span>
            <span>{readout.hz.toFixed(1)} Hz</span>
            <span>{readout.db === null ? "—" : `${readout.db.toFixed(1)} dB`}</span>
            <span>{nearest === null ? "order —" : `order ${nearest.toFixed(2)}`}</span>
          </div>
        </>
      ) : null}
      </div>
      <div
        className="axis-gutter axis-time"
        data-testid="time-axis"
        aria-hidden="true"
      >
        {timeTickValues.map((time) => (
          <span
            key={`t-${time}`}
            className="axis-tick axis-tick-time"
            style={{
              left: `${
                timeSpan > 0
                  ? Math.min(
                      98,
                      Math.max(2, ((time - view.t0) / timeSpan) * 100),
                    )
                  : 0
              }%`,
            }}
          >
            {formatTime(time)}
          </span>
        ))}
      </div>
      <div className="axis-corner" aria-hidden="true">
        s
      </div>
    </div>
  );
}

import { useEffect, useRef, useState, type RefObject } from "react";
import {
  fitView,
  minimapRect,
  panView,
  viewFromMinimap,
  type FrequencyBounds,
  type Viewport,
} from "./geometry";
import { heatmapPixels } from "./heatmap";
import type { DecodedGrid } from "./types";

export interface MiniMapProps {
  grid: DecodedGrid;
  view: Viewport;
  minDb: number;
  maxDb: number;
  domainMinDb?: number;
  domainMaxDb?: number;
  minHz?: number;
  maxHz?: number;
  visible?: boolean;
  onViewChange: (view: Viewport) => void;
  onInteraction?: () => void;
}

type MapDrag =
  | { mode: "box"; startX: number; startView: Viewport }
  | { mode: "recenter" };

const HEIGHT = 36;

function useWidth(ref: RefObject<HTMLDivElement | null>): number {
  const [width, setWidth] = useState(800);
  useEffect(() => {
    const element = ref.current;
    if (!element) {
      return;
    }
    const update = () => setWidth(element.clientWidth || 800);
    update();
    if (typeof ResizeObserver !== "undefined") {
      const observer = new ResizeObserver(update);
      observer.observe(element);
      return () => observer.disconnect();
    }
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, [ref]);
  return width;
}

/** 36 px overview: whole file with a rectangle for the current view. */
export function MiniMap({
  grid,
  view,
  minDb,
  maxDb,
  domainMinDb = minDb,
  domainMaxDb = maxDb,
  minHz = 0,
  maxHz = grid.meta.nyquist,
  visible = true,
  onViewChange,
  onInteraction,
}: MiniMapProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const width = useWidth(hostRef);
  const [dragging, setDragging] = useState(false);
  const drag = useRef<MapDrag | null>(null);
  const bounds: FrequencyBounds = {
    minHz: Math.max(0, Number.isFinite(minHz) ? minHz : 0),
    maxHz: Math.max(1, Number.isFinite(maxHz) ? maxHz : grid.meta.nyquist),
  };

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) {
      return;
    }
    canvas.width = Math.max(1, Math.round(width));
    canvas.height = HEIGHT;
    const full = fitView(
      grid.meta.durationS,
      bounds.maxHz,
      view.logFreq,
      bounds.minHz,
    );
    const pixels = heatmapPixels(
      grid,
      full,
      canvas.width,
      canvas.height,
      minDb,
      maxDb,
      domainMinDb,
      domainMaxDb,
    );
    const image = ctx.createImageData(canvas.width, canvas.height);
    image.data.set(pixels);
    ctx.putImageData(image, 0, 0);
  }, [
    bounds.maxHz,
    bounds.minHz,
    domainMaxDb,
    domainMinDb,
    grid,
    maxDb,
    minDb,
    view.logFreq,
    width,
  ]);

  useEffect(() => {
    if (!dragging) {
      return;
    }
    const move = (event: PointerEvent) => {
      const rect = hostRef.current?.getBoundingClientRect();
      if (!rect || rect.width <= 0 || rect.height <= 0) {
        return;
      }
      onInteraction?.();
      const active = drag.current;
      if (active?.mode === "box") {
        const delta =
          ((event.clientX - active.startX) / rect.width) * grid.meta.durationS;
        onViewChange(
          panView(
            active.startView,
            delta,
            0,
            grid.meta.durationS,
            grid.meta.nyquist,
            bounds,
          ),
        );
        return;
      }
      const x = Math.min(Math.max((event.clientX - rect.left) / rect.width, 0), 1);
      const y = Math.min(Math.max((event.clientY - rect.top) / rect.height, 0), 1);
      onViewChange(
        viewFromMinimap(
          x,
          y,
          view,
          grid.meta.durationS,
          grid.meta.nyquist,
          bounds,
        ),
      );
    };
    const up = () => {
      drag.current = null;
      setDragging(false);
      onInteraction?.();
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
    bounds.maxHz,
    bounds.minHz,
    dragging,
    grid.meta.durationS,
    grid.meta.nyquist,
    onInteraction,
    onViewChange,
    view,
  ]);

  const full = fitView(
    grid.meta.durationS,
    bounds.maxHz,
    view.logFreq,
    bounds.minHz,
  );
  const rect = minimapRect(
    view,
    grid.meta.durationS,
    grid.meta.nyquist,
    width,
    HEIGHT,
    bounds,
  );

  const recenterFromPointer = (
    event: React.PointerEvent<HTMLDivElement>,
  ) => {
    if (event.pointerType === "mouse" && event.button !== 0) {
      return;
    }
    const box = event.currentTarget.getBoundingClientRect();
    if (box.width <= 0 || box.height <= 0) {
      return;
    }
    const x = Math.min(Math.max((event.clientX - box.left) / box.width, 0), 1);
    const y = Math.min(Math.max((event.clientY - box.top) / box.height, 0), 1);
    drag.current = { mode: "recenter" };
    setDragging(true);
    onInteraction?.();
    onViewChange(
      viewFromMinimap(
        x,
        y,
        view,
        grid.meta.durationS,
        grid.meta.nyquist,
        bounds,
      ),
    );
  };

  const beginBoxDrag = (event: React.PointerEvent<HTMLSpanElement>) => {
    if (event.pointerType === "mouse" && event.button !== 0) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    drag.current = { mode: "box", startX: event.clientX, startView: view };
    setDragging(true);
    onInteraction?.();
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };

  return (
    <div
      className="minimap"
      ref={hostRef}
      data-testid="minimap"
      data-visible={visible ? "true" : "false"}
      onPointerDown={recenterFromPointer}
    >
      <canvas ref={canvasRef} />
      <span
        className="minimap-view"
        style={{
          left: rect.x,
          top: rect.y,
          width: rect.w,
          height: rect.h,
        }}
        aria-hidden="true"
        onPointerDown={beginBoxDrag}
      />
      <span className="minimap-label">
        {view.t0.toFixed(2)}–{view.t1.toFixed(2)} s ·{" "}
        {view.f0.toFixed(0)}–{view.f1.toFixed(0)} Hz
      </span>
      <button
        type="button"
        className="minimap-nudge"
        aria-label="Reset minimap view"
        onPointerDown={(event) => event.stopPropagation()}
        onClick={() => {
          onInteraction?.();
          onViewChange(full);
        }}
      >
        ⤢
      </button>
    </div>
  );
}

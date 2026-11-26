import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type RefObject,
} from "react";
import { Banner } from "../ui/Banner";
import { IconButton } from "../ui/IconButton";
import { triggerDownload } from "../transport/download";
import { useToast } from "../ui/toast";
import "./cuts.css";

export interface PlotSeries {
  id: string;
  label: string;
  color: string;
  points: Array<{ x: number; y: number }>;
  hidden?: boolean;
}

export interface CutPlotProps {
  series: PlotSeries[];
  kind?: "order" | "line" | "plot";
  axisLabel: string;
  yLabel?: string;
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onToggleVisibility: (id: string) => void;
  onContextMenu?: (id: string, x: number, y: number) => void;
  onPlotContextMenu?: (x: number, y: number) => void;
  onMoveHandle?: (
    id: string,
    endpoint: "start" | "end",
    x: number,
    y: number,
  ) => void;
  warning?: string | null;
  onDismissWarning?: () => void;
  /** Draw the tick-aligned grid lines; axis ticks stay visible either way. */
  showGrid?: boolean;
  panelId?: string;
  onSavePng?: (dataUrl: string, filename: string) => Promise<string | null>;
}

const MARGIN = { left: 52, right: 18, top: 18, bottom: 30 };
const INITIAL_SIZE = { width: 800, height: 300 };

/**
 * Track the SVG's rendered box so the plot drawing fills its panel instead of
 * letterboxing a fixed-size drawing inside it.
 */
function usePlotSize(ref: RefObject<SVGSVGElement | null>) {
  const [size, setSize] = useState(INITIAL_SIZE);
  useEffect(() => {
    const element = ref.current;
    if (!element) {
      return;
    }
    const update = () => {
      const width = element.clientWidth;
      const height = element.clientHeight;
      if (width < 2 || height < 2) {
        return;
      }
      setSize((current) =>
        current.width === width && current.height === height
          ? current
          : { width, height },
      );
    };
    update();
    if (typeof ResizeObserver !== "undefined") {
      const observer = new ResizeObserver(update);
      observer.observe(element);
      return () => observer.disconnect();
    }
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, [ref]);
  return size;
}

/**
 * Map a client point into the plot's viewBox coordinates.
 *
 * The viewBox is measured from the rendered SVG, so one unit is one CSS pixel
 * once a measurement lands. Right after a resize the browser can still scale
 * and centre the old viewBox (`xMidYMid meet`, letterboxed); this removes that
 * transform instead of assuming the rect ratios match the viewBox.
 */
export function viewBoxPoint(
  rect: { left: number; top: number; width: number; height: number },
  view: { width: number; height: number },
  clientX: number,
  clientY: number,
): { x: number; y: number } | null {
  if (
    !(rect.width > 0) ||
    !(rect.height > 0) ||
    !(view.width > 0) ||
    !(view.height > 0)
  ) {
    return null;
  }
  const scale = Math.min(rect.width / view.width, rect.height / view.height);
  const offsetX = rect.left + (rect.width - view.width * scale) / 2;
  const offsetY = rect.top + (rect.height - view.height * scale) / 2;
  return {
    x: (clientX - offsetX) / scale,
    y: (clientY - offsetY) / scale,
  };
}

interface PlotViewport {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
}

interface RegionSelection {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

interface RegionDrag {
  startX: number;
  startY: number;
  startClientX: number;
  startClientY: number;
}

function niceTick(value: number): string {
  const abs = Math.abs(value);
  if (abs >= 1000) return value.toFixed(0);
  if (abs >= 100) return value.toFixed(0);
  if (abs >= 10) return value.toFixed(1);
  return value.toFixed(2);
}

const SVG_NS = "http://www.w3.org/2000/svg";
const EXPORT_STYLE_PROPERTIES = [
  "fill",
  "stroke",
  "stroke-width",
  "stroke-dasharray",
  "stroke-linecap",
  "stroke-linejoin",
  "opacity",
  "font-family",
  "font-size",
  "font-weight",
  "font-style",
  "letter-spacing",
  "text-anchor",
  "paint-order",
  "visibility",
];

/** Copy the page's computed paint/font styles onto a detached SVG clone. */
function inlineComputedSvgStyles(
  source: SVGSVGElement,
  clone: SVGSVGElement,
): void {
  const originals = [source, ...source.querySelectorAll<SVGElement>("*")];
  const copies = [clone, ...clone.querySelectorAll<SVGElement>("*")];
  originals.forEach((original, index) => {
    const copy = copies[index];
    if (!copy) {
      return;
    }
    const computed = getComputedStyle(original);
    for (const property of EXPORT_STYLE_PROPERTIES) {
      const value = computed.getPropertyValue(property);
      if (value) {
        copy.style.setProperty(property, value);
      }
    }
  });
}

function loadSvgImage(markup: string): Promise<HTMLImageElement> {
  const url = URL.createObjectURL(
    new Blob([markup], { type: "image/svg+xml;charset=utf-8" }),
  );
  return new Promise((resolve, reject) => {
    const image = new Image();
    image.onload = () => {
      URL.revokeObjectURL(url);
      resolve(image);
    };
    image.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("could not rasterize the plot"));
    };
    image.src = url;
  });
}

function fitText(
  context: CanvasRenderingContext2D,
  text: string,
  maxWidth: number,
): string {
  if (context.measureText(text).width <= maxWidth) {
    return text;
  }
  let value = text;
  while (value.length > 1 && context.measureText(`${value}…`).width > maxWidth) {
    value = value.slice(0, -1);
  }
  return `${value}…`;
}

function dataUrlToBlob(dataUrl: string): Blob {
  const comma = dataUrl.indexOf(",");
  const payload = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
  const binary = atob(payload);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return new Blob([bytes], { type: "image/png" });
}


/** SVG curve plot shared by Order Cuts and Line Cuts. */
export function CutPlot({
  series,
  kind,
  axisLabel,
  yLabel = "dB",
  selectedId,
  onSelect,
  onToggleVisibility,
  onContextMenu,
  onPlotContextMenu,
  onMoveHandle,
  warning,
  onDismissWarning,
  showGrid = true,
  panelId,
  onSavePng,
}: CutPlotProps) {
  const svgRef = useRef<SVGSVGElement>(null);
  const clipId = `cut-plot-clip-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const size = usePlotSize(svgRef);
  const viewW = Math.max(MARGIN.left + MARGIN.right + 2, size.width);
  const viewH = Math.max(MARGIN.top + MARGIN.bottom + 2, size.height);
  const plotW = viewW - MARGIN.left - MARGIN.right;
  const plotH = viewH - MARGIN.top - MARGIN.bottom;
  const { pushToast } = useToast();
  const [legendOpen, setLegendOpen] = useState(true);
  const [hovered, setHovered] = useState<string | null>(null);
  const [drag, setDrag] = useState<{ id: string; end: "start" | "end" } | null>(
    null,
  );
  const [viewport, setViewport] = useState<PlotViewport | null>(null);
  const [region, setRegion] = useState<RegionSelection | null>(null);
  const regionDrag = useRef<RegionDrag | null>(null);

  const visible = series.filter((s) => !s.hidden && s.points.length > 0);
  const xs = visible.flatMap((s) => s.points.map((p) => p.x));
  const ys = visible.flatMap((s) => s.points.map((p) => p.y));
  let autoMinX = xs.length > 0 ? Math.min(...xs) : 0;
  let autoMaxX = xs.length > 0 ? Math.max(...xs) : 1;
  let autoMinY = ys.length > 0 ? Math.min(...ys) : -100;
  let autoMaxY = ys.length > 0 ? Math.max(...ys) : 0;
  if (!(autoMaxX > autoMinX)) {
    autoMinX -= 1;
    autoMaxX += 1;
  }
  if (!(autoMaxY > autoMinY)) {
    autoMinY -= 1;
    autoMaxY += 1;
  }
  const padX = (autoMaxX - autoMinX) * 0.03;
  const padY = (autoMaxY - autoMinY) * 0.08;
  autoMinX -= padX;
  autoMaxX += padX;
  autoMinY -= padY;
  autoMaxY += padY;

  const bounds: PlotViewport = viewport ?? {
    minX: autoMinX,
    maxX: autoMaxX,
    minY: autoMinY,
    maxY: autoMaxY,
  };
  const { minX, maxX, minY, maxY } = bounds;

  const sx = (x: number) =>
    MARGIN.left + ((x - minX) / (maxX - minX)) * plotW;
  const sy = (y: number) =>
    MARGIN.top + (1 - (y - minY) / (maxY - minY)) * plotH;
  const ix = (px: number) => minX + ((px - MARGIN.left) / plotW) * (maxX - minX);
  const iy = (py: number) =>
    minY + (1 - (py - MARGIN.top) / plotH) * (maxY - minY);

  useEffect(() => {
    if (!drag) {
      return;
    }
    const move = (event: PointerEvent) => {
      const svg = svgRef.current;
      if (!svg || !onMoveHandle) {
        return;
      }
      const point = viewBoxPoint(
        svg.getBoundingClientRect(),
        { width: viewW, height: viewH },
        event.clientX,
        event.clientY,
      );
      if (!point) {
        return;
      }
      onMoveHandle(drag.id, drag.end, ix(point.x), iy(point.y));
    };
    const up = () => setDrag(null);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  });

  const regionPoint = (event: { clientX: number; clientY: number }) => {
    const rect = svgRef.current?.getBoundingClientRect();
    const point = rect
      ? viewBoxPoint(rect, { width: viewW, height: viewH }, event.clientX, event.clientY)
      : null;
    if (!point) {
      return null;
    }
    const x = Math.min(Math.max(point.x, MARGIN.left), MARGIN.left + plotW);
    const y = Math.min(Math.max(point.y, MARGIN.top), MARGIN.top + plotH);
    return { x, y };
  };

  const beginRegion = (event: React.PointerEvent<SVGRectElement>) => {
    if (event.pointerType === "mouse" && event.button !== 0) {
      return;
    }
    const point = regionPoint(event);
    if (!point) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    regionDrag.current = {
      startX: point.x,
      startY: point.y,
      startClientX: event.clientX,
      startClientY: event.clientY,
    };
    setRegion({ x0: point.x, y0: point.y, x1: point.x, y1: point.y });
    event.currentTarget.setPointerCapture?.(event.pointerId);
  };

  const moveRegion = (event: React.PointerEvent<SVGSVGElement>) => {
    const active = regionDrag.current;
    if (!active) {
      return;
    }
    const point = regionPoint(event);
    if (!point) {
      return;
    }
    setRegion({
      x0: Math.min(active.startX, point.x),
      y0: Math.min(active.startY, point.y),
      x1: Math.max(active.startX, point.x),
      y1: Math.max(active.startY, point.y),
    });
  };

  const finishRegion = (event: React.PointerEvent<SVGSVGElement>) => {
    const active = regionDrag.current;
    if (!active) {
      return;
    }
    const point = regionPoint(event);
    regionDrag.current = null;
    if (!point) {
      setRegion(null);
      return;
    }
    const movedX = Math.abs(event.clientX - active.startClientX);
    const movedY = Math.abs(event.clientY - active.startClientY);
    const x0 = Math.min(active.startX, point.x);
    const x1 = Math.max(active.startX, point.x);
    const y0 = Math.min(active.startY, point.y);
    const y1 = Math.max(active.startY, point.y);
    if (movedX >= 4 && movedY >= 4 && x1 > x0 && y1 > y0) {
      const next = {
        minX: ix(x0),
        maxX: ix(x1),
        minY: iy(y1),
        maxY: iy(y0),
      };
      if (next.maxX > next.minX && next.maxY > next.minY) {
        setViewport(next);
      }
    }
    setRegion(null);
  };

  const cancelRegion = () => {
    regionDrag.current = null;
    setRegion(null);
  };

  const exportPlot = useCallback(async () => {
    const svg = svgRef.current;
    if (!svg) {
      return;
    }
    try {
      const host = svg.closest<HTMLElement>(".cut-plot");
      const hostStyle = host ? getComputedStyle(host) : null;
      const rawBackground = hostStyle?.backgroundColor ?? "";
      const background =
        rawBackground && rawBackground !== "rgba(0, 0, 0, 0)"
          ? rawBackground
          : "#11151b";
      const tick = svg.querySelector<SVGTextElement>(".cut-tick");
      const tickStyle = tick ? getComputedStyle(tick) : null;
      const textColor = tickStyle?.fill || "#cbd5e1";
      const fontFamily = tickStyle?.fontFamily || "monospace";
      const border =
        getComputedStyle(document.documentElement).getPropertyValue(
          "--border-subtle",
        ) || "rgba(255,255,255,0.2)";

      const clone = svg.cloneNode(true) as SVGSVGElement;
      inlineComputedSvgStyles(svg, clone);
      clone
        .querySelectorAll(".cut-region, .cut-region-capture, .cut-handle")
        .forEach((element) => element.remove());
      clone.setAttribute("xmlns", SVG_NS);
      clone.setAttribute("width", String(viewW));
      clone.setAttribute("height", String(viewH));
      const backgroundRect = document.createElementNS(SVG_NS, "rect");
      backgroundRect.setAttribute("x", "0");
      backgroundRect.setAttribute("y", "0");
      backgroundRect.setAttribute("width", String(viewW));
      backgroundRect.setAttribute("height", String(viewH));
      backgroundRect.setAttribute("fill", background);
      clone.insertBefore(backgroundRect, clone.firstChild);
      const image = await loadSvgImage(
        new XMLSerializer().serializeToString(clone),
      );

      const legendWidth = 220;
      const scale = 2;
      const canvas = document.createElement("canvas");
      canvas.width = (viewW + legendWidth) * scale;
      canvas.height = viewH * scale;
      const context = canvas.getContext("2d");
      if (!context) {
        throw new Error("could not create the export canvas");
      }
      context.scale(scale, scale);
      context.fillStyle = background;
      context.fillRect(0, 0, viewW + legendWidth, viewH);
      context.drawImage(image, 0, 0, viewW, viewH);

      const legendX = viewW;
      context.fillStyle = background;
      context.fillRect(legendX, 0, legendWidth, viewH);
      context.strokeStyle = border;
      context.lineWidth = 1;
      context.beginPath();
      context.moveTo(legendX + 0.5, 0);
      context.lineTo(legendX + 0.5, viewH);
      context.stroke();
      context.fillStyle = textColor;
      context.font = `600 13px ${fontFamily}`;
      context.fillText("Legend", legendX + 14, 24);
      context.font = `11px ${fontFamily}`;
      context.textBaseline = "middle";
      let rowY = 46;
      for (const item of series) {
        if (rowY > viewH - 12) {
          break;
        }
        context.globalAlpha = item.hidden ? 0.42 : 1;
        context.fillStyle = item.color;
        context.fillRect(legendX + 14, rowY - 5, 10, 10);
        context.fillStyle = textColor;
        context.fillText(
          fitText(context, item.label, legendWidth - 46),
          legendX + 32,
          rowY,
        );
        rowY += 20;
      }
      if (series.length === 0) {
        context.globalAlpha = 0.7;
        context.fillStyle = textColor;
        context.fillText("No cuts yet", legendX + 14, 48);
      }
      context.globalAlpha = 1;

      const dataUrl = canvas.toDataURL("image/png");
      const filename =
        kind === "order"
          ? "order-cuts.png"
          : kind === "line"
            ? "line-cuts.png"
            : kind === "plot"
              ? "plot.png"
              : "cut-plot.png";
      if (onSavePng) {
        const saved = await onSavePng(dataUrl, filename);
        if (saved) {
          pushToast({ tone: "success", message: `Exported ${filename}` });
        }
      } else {
        triggerDownload(dataUrlToBlob(dataUrl), filename);
      }
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `Export failed: ${
          reason instanceof Error ? reason.message : String(reason)
        }`,
      });
    }
  }, [kind, onSavePng, pushToast, series, viewW, viewH]);

  useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ panelId?: string }>).detail;
      if (detail?.panelId && panelId && detail.panelId !== panelId) {
        return;
      }
      void exportPlot();
    };
    window.addEventListener("fft:export-image", handler);
    return () => window.removeEventListener("fft:export-image", handler);
  }, [exportPlot, panelId]);

  const resetView = useCallback(() => setViewport(null), []);

  useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ panelId?: string }>).detail;
      if (detail?.panelId && panelId && detail.panelId !== panelId) {
        return;
      }
      resetView();
    };
    window.addEventListener("fft:reset-plot-view", handler);
    return () => window.removeEventListener("fft:reset-plot-view", handler);
  }, [panelId, resetView]);

  const xTicks = [0, 1, 2, 3, 4].map(
    (i) => minX + ((maxX - minX) * i) / 4,
  );
  const yTicks = [0, 1, 2, 3].map(
    (i) => maxY - ((maxY - minY) * i) / 3,
  );

  return (
    <div
      className="cut-plot"
      data-testid="cut-plot"
      data-cut-kind={kind}
      data-x-min={minX}
      data-x-max={maxX}
      data-y-min={minY}
      data-y-max={maxY}
      data-zoomed={viewport ? "true" : "false"}
    >
      {warning ? (
        <div className="cut-warning">
          <Banner tone="warn" onDismiss={onDismissWarning}>
            {warning}
          </Banner>
        </div>
      ) : null}

      <div className="cut-legend">
        <button
          type="button"
          className="cut-legend-toggle"
          aria-expanded={legendOpen}
          onClick={() => setLegendOpen((value) => !value)}
        >
          <span aria-hidden="true">▤</span> Legend
          <span className="cut-legend-count">{series.length}</span>
        </button>
        {legendOpen ? (
          <ul className="cut-legend-list">
            {series.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  className={`cut-legend-item${item.hidden ? " hidden" : ""}${
                    hovered === item.id ? " hovered" : ""
                  }`}
                  aria-pressed={!item.hidden}
                  title={`Toggle ${item.label}`}
                  onPointerEnter={() => setHovered(item.id)}
                  onPointerLeave={() => setHovered(null)}
                  onClick={() => onToggleVisibility(item.id)}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    onSelect(item.id);
                    if (onContextMenu) {
                      onContextMenu(item.id, event.clientX, event.clientY);
                    } else {
                      onPlotContextMenu?.(event.clientX, event.clientY);
                    }
                  }}
                >
                  <span
                    className="cut-swatch"
                    style={{ background: item.color }}
                    aria-hidden="true"
                  />
                  <span>{item.label}</span>
                </button>
              </li>
            ))}
            {series.length === 0 ? (
              <li className="cut-legend-empty">No cuts yet</li>
            ) : null}
          </ul>
        ) : null}
      </div>

      <svg
        ref={svgRef}
        className="cut-svg"
        viewBox={`0 0 ${viewW} ${viewH}`}
        preserveAspectRatio="xMidYMid meet"
        role="img"
        aria-label={`${axisLabel} cut plot`}
        onContextMenu={(event) => {
          event.preventDefault();
          onPlotContextMenu?.(event.clientX, event.clientY);
        }}
        onDoubleClick={(event) => {
          // Curves and endpoint handles own their double-click gestures.
          const target = event.target instanceof Element ? event.target : null;
          if (target?.closest(".cut-line, .cut-line-hit, .cut-handle")) {
            return;
          }
          resetView();
        }}
        onPointerMove={moveRegion}
        onPointerUp={finishRegion}
        onPointerCancel={cancelRegion}
        onClick={() => {
          onSelect(null);
        }}
      >
        <defs>
          {/* Curves are clipped to the axes area so a zoomed trace never paints
              over the tick margins or the panel chrome. */}
          <clipPath id={clipId}>
            <rect
              x={MARGIN.left}
              y={MARGIN.top}
              width={plotW}
              height={plotH}
            />
          </clipPath>
        </defs>
        {xTicks.map((tick) => (
          <g key={`x-${tick}`}>
            {showGrid ? (
              <line
                x1={sx(tick)}
                x2={sx(tick)}
                y1={MARGIN.top}
                y2={MARGIN.top + plotH}
                className="cut-grid"
              />
            ) : null}
            <text
              x={sx(tick)}
              y={viewH - 9}
              textAnchor="middle"
              className="cut-tick"
            >
              {niceTick(tick)}
            </text>
          </g>
        ))}
        {yTicks.map((tick) => (
          <g key={`y-${tick}`}>
            {showGrid ? (
              <line
                x1={MARGIN.left}
                x2={MARGIN.left + plotW}
                y1={sy(tick)}
                y2={sy(tick)}
                className="cut-grid"
              />
            ) : null}
            <text
              x={MARGIN.left - 7}
              y={sy(tick) + 3}
              textAnchor="end"
              className="cut-tick"
            >
              {niceTick(tick)}
            </text>
          </g>
        ))}

        <text
          x={MARGIN.left + plotW / 2}
          y={viewH - 1}
          textAnchor="middle"
          className="cut-axis-label"
        >
          {axisLabel}
        </text>
        <text
          x={12}
          y={MARGIN.top + plotH / 2}
          textAnchor="middle"
          transform={`rotate(-90 12 ${MARGIN.top + plotH / 2})`}
          className="cut-axis-label"
        >
          {yLabel}
        </text>

        <rect
          className="cut-region-capture"
          data-testid="cut-region-capture"
          x={MARGIN.left}
          y={MARGIN.top}
          width={plotW}
          height={plotH}
          fill="transparent"
          pointerEvents="all"
          style={{ cursor: "crosshair" }}
          onPointerDown={beginRegion}
        />
        {region ? (
          <rect
            className="cut-region"
            data-testid="cut-region"
            x={region.x0}
            y={region.y0}
            width={Math.max(0, region.x1 - region.x0)}
            height={Math.max(0, region.y1 - region.y0)}
            pointerEvents="none"
          />
        ) : null}

        <g clipPath={`url(#${clipId})`}>
          {visible.map((item) => {
            const selected = item.id === selectedId;
            const dimmed = hovered !== null && hovered !== item.id;
            const path = item.points
              .map(
                (p, index) => `${index === 0 ? "M" : "L"}${sx(p.x)},${sy(p.y)}`,
              )
              .join(" ");
            return (
              <g
                key={item.id}
                onContextMenu={
                  onContextMenu
                    ? (event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        onSelect(item.id);
                        onContextMenu(item.id, event.clientX, event.clientY);
                      }
                    : undefined
                }
              >
                <path
                  d={path}
                  fill="none"
                  stroke={item.color}
                  strokeWidth={selected ? 3 : 1.7}
                  className={`cut-line${selected ? " selected" : ""}`}
                  opacity={dimmed ? 0.35 : 1}
                  onPointerEnter={() => setHovered(item.id)}
                  onPointerLeave={() => setHovered(null)}
                  onClick={(event) => {
                    event.stopPropagation();
                    onSelect(item.id);
                  }}
                />
                <path
                  d={path}
                  fill="none"
                  stroke="transparent"
                  strokeWidth={10}
                  className="cut-line-hit"
                  style={{ cursor: "pointer" }}
                  onClick={(event) => {
                    event.stopPropagation();
                    onSelect(item.id);
                  }}
                />
                {selected && onMoveHandle && item.points.length > 0 ? (
                  <>
                    <circle
                      cx={sx(item.points[0].x)}
                      cy={sy(item.points[0].y)}
                      r={5}
                      className="cut-handle"
                      style={{ fill: item.color }}
                      onPointerDown={(event) => {
                        event.stopPropagation();
                        setDrag({ id: item.id, end: "start" });
                      }}
                    />
                    <circle
                      cx={sx(item.points[item.points.length - 1].x)}
                      cy={sy(item.points[item.points.length - 1].y)}
                      r={5}
                      className="cut-handle"
                      style={{ fill: item.color }}
                      onPointerDown={(event) => {
                        event.stopPropagation();
                        setDrag({ id: item.id, end: "end" });
                      }}
                    />
                  </>
                ) : null}
              </g>
            );
          })}
        </g>

        {visible.length === 0 ? (
          <text
            x={MARGIN.left + plotW / 2}
            y={MARGIN.top + plotH / 2}
            textAnchor="middle"
            className="cut-empty-label"
          >
            No drawable cuts
          </text>
        ) : null}

      </svg>

      <div className="cut-plot-actions">
        {viewport ? (
          <IconButton label="Reset cut view" onClick={resetView}>
            ⤢
          </IconButton>
        ) : null}
        <IconButton
          label="Hide legend"
          pressed={!legendOpen}
          onClick={() => setLegendOpen((value) => !value)}
        >
          {legendOpen ? "◧" : "▤"}
        </IconButton>
      </div>
    </div>
  );
}

// @vitest-environment jsdom
import type { ComponentProps } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { CutPlot, viewBoxPoint, type PlotSeries } from "./CutPlot";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

// Auto bounds for this series are x [-0.3, 10.3], y [-84.8, -15.2]; the plot
// box inside the 800x300 viewBox is x 52..782, y 18..270.
const SERIES: PlotSeries[] = [
  {
    id: "cut-0",
    label: "line 1",
    color: "#56b4e9",
    points: [
      { x: 0, y: -20 },
      { x: 10, y: -80 },
    ],
  },
];

// A tall panel: 600x480 around an 800x300 viewBox leaves 127.5 px letterboxing
// above and below, so viewBox y=0 starts at client y=147.5 with scale 0.75.
const TALL_RECT = {
  x: 100,
  y: 20,
  left: 100,
  top: 20,
  width: 600,
  height: 480,
  right: 700,
  bottom: 500,
  toJSON: () => ({}),
} as DOMRect;

function plotBounds(container: HTMLElement) {
  const plot = container.querySelector(".cut-plot")!;
  return {
    minX: Number(plot.getAttribute("data-x-min")),
    maxX: Number(plot.getAttribute("data-x-max")),
    minY: Number(plot.getAttribute("data-y-min")),
    maxY: Number(plot.getAttribute("data-y-max")),
    zoomed: plot.getAttribute("data-zoomed"),
  };
}

function renderPlot(props: Partial<ComponentProps<typeof CutPlot>> = {}) {
  const view = render(
    <CutPlot
      series={SERIES}
      axisLabel="Time (s)"
      selectedId={null}
      onSelect={() => {}}
      onToggleVisibility={() => {}}
      {...props}
    />,
  );
  const svg = view.container.querySelector(".cut-svg") as SVGSVGElement;
  vi.spyOn(svg, "getBoundingClientRect").mockReturnValue(TALL_RECT);
  return { ...view, svg };
}

function pointer(
  target: Element | Window,
  type: "pointerdown" | "pointermove" | "pointerup",
  clientX: number,
  clientY: number,
) {
  const init = { clientX, clientY, button: 0 };
  if (type === "pointerdown") {
    fireEvent.pointerDown(target, init);
  } else if (type === "pointermove") {
    fireEvent.pointerMove(target, init);
  } else {
    fireEvent.pointerUp(target, init);
  }
}

/** Drag the plot background from one client point to another. */
function dragRegion(
  svg: SVGSVGElement,
  from: { x: number; y: number },
  to: { x: number; y: number },
) {
  const capture = screen.getByTestId("cut-region-capture");
  pointer(capture, "pointerdown", from.x, from.y);
  pointer(svg, "pointermove", to.x, to.y);
  pointer(svg, "pointerup", to.x, to.y);
}

describe("viewBoxPoint", () => {
  const VIEW = { width: 800, height: 300 };

  it("removes the vertical letterbox of a tall panel", () => {
    expect(viewBoxPoint(TALL_RECT, VIEW, 400, 297.5)).toEqual({
      x: 400,
      y: 200,
    });
    expect(viewBoxPoint(TALL_RECT, VIEW, 175, 207.5)).toEqual({
      x: 100,
      y: 80,
    });
    expect(viewBoxPoint(TALL_RECT, VIEW, 100, 147.5)).toEqual({ x: 0, y: 0 });
    expect(viewBoxPoint(TALL_RECT, VIEW, 700, 372.5)).toEqual({
      x: 800,
      y: 300,
    });
  });

  it("removes the horizontal letterbox of a wide panel", () => {
    const wide = {
      left: 0,
      top: 0,
      width: 1600,
      height: 300,
      right: 1600,
      bottom: 300,
    };
    expect(viewBoxPoint(wide, VIEW, 400, 0)).toEqual({ x: 0, y: 0 });
    expect(viewBoxPoint(wide, VIEW, 1200, 300)).toEqual({ x: 800, y: 300 });
    expect(viewBoxPoint(wide, VIEW, 800, 150)).toEqual({ x: 400, y: 150 });
  });

  it("keeps a measured viewBox at one unit per CSS pixel", () => {
    const measured = {
      left: 30,
      top: 40,
      width: 1200,
      height: 500,
      right: 1230,
      bottom: 540,
    };
    expect(viewBoxPoint(measured, measured, 330, 240)).toEqual({
      x: 300,
      y: 200,
    });
  });

  it("ignores elements without a rendered size", () => {
    const collapsed = { left: 0, top: 0, width: 0, height: 480 };
    expect(viewBoxPoint(collapsed, VIEW, 10, 10)).toBeNull();
  });
});

describe("CutPlot region zoom", () => {
  it("draws the selection box under the cursor and zooms to it", () => {
    const { container, svg } = renderPlot();
    const before = plotBounds(container);

    pointer(screen.getByTestId("cut-region-capture"), "pointerdown", 175, 207.5);
    pointer(svg, "pointermove", 475, 312.5);

    const region = screen.getByTestId("cut-region");
    expect(Number(region.getAttribute("x"))).toBeCloseTo(100, 6);
    expect(Number(region.getAttribute("y"))).toBeCloseTo(80, 6);
    expect(Number(region.getAttribute("width"))).toBeCloseTo(400, 6);
    expect(Number(region.getAttribute("height"))).toBeCloseTo(140, 6);

    pointer(svg, "pointerup", 475, 312.5);

    const zoomed = plotBounds(container);
    expect(zoomed.zoomed).toBe("true");
    expect(zoomed.minX).toBeCloseTo(0.39699, 4);
    expect(zoomed.maxX).toBeCloseTo(6.20521, 4);
    expect(zoomed.minY).toBeCloseTo(-70.99048, 4);
    expect(zoomed.maxY).toBeCloseTo(-32.32381, 4);
    expect(zoomed.minX).toBeGreaterThan(before.minX);
    expect(zoomed.maxX).toBeLessThan(before.maxX);
    expect(zoomed.minY).toBeGreaterThan(before.minY);
    expect(zoomed.maxY).toBeLessThan(before.maxY);
  });

  it("maps endpoint drags through the same letterboxed transform", () => {
    const onMoveHandle = vi.fn();
    const { container } = renderPlot({
      selectedId: "cut-0",
      onMoveHandle,
    });
    const handle = container.querySelector(".cut-handle")!;

    pointer(handle, "pointerdown", 0, 0);
    pointer(window, "pointermove", 175, 207.5);

    const call = onMoveHandle.mock.calls.at(-1);
    expect(call?.[0]).toBe("cut-0");
    expect(call?.[1]).toBe("start");
    expect(call?.[2]).toBeCloseTo(0.39699, 4);
    expect(call?.[3]).toBeCloseTo(-32.32381, 4);
  });

  it("resets the zoom on a background double-click only", () => {
    const { container } = renderPlot();
    const initial = plotBounds(container);

    dragRegion(
      container.querySelector(".cut-svg") as SVGSVGElement,
      { x: 175, y: 207.5 },
      { x: 475, y: 312.5 },
    );
    expect(plotBounds(container).zoomed).toBe("true");

    // A curve owns its double-click, so the zoom stays.
    fireEvent.doubleClick(container.querySelector(".cut-line-hit")!);
    expect(plotBounds(container).zoomed).toBe("true");

    fireEvent.doubleClick(screen.getByTestId("cut-region-capture"));
    const reset = plotBounds(container);
    expect(reset.zoomed).toBe("false");
    expect(reset.minX).toBeCloseTo(initial.minX, 6);
    expect(reset.maxX).toBeCloseTo(initial.maxX, 6);
    expect(reset.minY).toBeCloseTo(initial.minY, 6);
    expect(reset.maxY).toBeCloseTo(initial.maxY, 6);
    expect(screen.queryByRole("button", { name: "Reset cut view" })).toBeNull();
  });

  it("fills the panel and clips curves to the plot area", () => {
    vi.spyOn(Element.prototype, "clientWidth", "get").mockReturnValue(1200);
    vi.spyOn(Element.prototype, "clientHeight", "get").mockReturnValue(500);
    const { container } = renderPlot();
    const svg = container.querySelector(".cut-svg")!;

    // The drawing uses the measured panel, so one unit is one CSS pixel.
    expect(svg.getAttribute("viewBox")).toBe("0 0 1200 500");

    const clip = svg.querySelector("clipPath");
    expect(clip).not.toBeNull();
    const clipRect = clip!.querySelector("rect")!;
    expect(Number(clipRect.getAttribute("x"))).toBe(52);
    expect(Number(clipRect.getAttribute("y"))).toBe(18);
    expect(Number(clipRect.getAttribute("width"))).toBe(1200 - 70);
    expect(Number(clipRect.getAttribute("height"))).toBe(500 - 48);

    // Curves, handles, and hit targets live inside the clipped group.
    const clipped = svg.querySelector(`g[clip-path="url(#${clip!.id})"]`);
    expect(clipped).not.toBeNull();
    expect(clipped!.querySelector(".cut-line")).not.toBeNull();
    expect(clipped!.querySelector(".cut-line-hit")).not.toBeNull();
  });
});

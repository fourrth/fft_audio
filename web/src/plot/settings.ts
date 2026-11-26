import type { PlotLayout } from "./types";

/** Table columns map to lines; shared-x is the default. */
export function readPlotLayout(state: Record<string, unknown>): PlotLayout {
  return state.plotLayout === "pairs" ? "pairs" : "columns";
}

/** Plot displays show grid lines unless their state explicitly hides them. */
export function readShowGrid(state: Record<string, unknown>): boolean {
  return state.showGrid !== false;
}

/** Wire types for the generic Plot display (`plot.load`). */

/**
 * How table columns map onto plotted lines.
 *
 * `columns`: first column = shared x; every remaining numeric column is one
 * line. `pairs`: columns as x/y pairs (`x1,y1,x2,y2,...`); each pair is one
 * line.
 */
export type PlotLayout = "columns" | "pairs";

export interface PlotPoint {
  x: number;
  y: number;
}

export interface PlotSeries {
  label: string;
  points: PlotPoint[];
}

export interface TablePlot {
  xLabel: string;
  series: PlotSeries[];
  rows: number;
  notes: string[];
}

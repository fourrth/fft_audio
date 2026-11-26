// @vitest-environment jsdom
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { MockTransport } from "../transport/mock";
import type { ShellDisplay } from "../shell/session";
import { PlotPanel } from "./PlotPanel";
import type { PlotLayout, TablePlot } from "./types";

afterEach(cleanup);

const DISPLAY: ShellDisplay = {
  id: 3,
  parent: null,
  kind: "plot",
  name: "series.csv",
  state: { path: "/data/series.csv", plotLayout: "columns" },
};

function payload(layout: PlotLayout): TablePlot {
  return layout === "pairs"
    ? {
        xLabel: "t1",
        series: [{ label: "y1", points: [{ x: 0, y: 5 }] }],
        rows: 1,
        notes: ["trailing column 'z' ignored: it has no y partner"],
      }
    : {
        xLabel: "time",
        series: [
          { label: "a", points: [{ x: 0, y: 1 }, { x: 1, y: 2 }] },
          { label: "b", points: [{ x: 0, y: 10 }, { x: 1, y: 20 }] },
        ],
        rows: 2,
        notes: [],
      };
}

describe("PlotPanel", () => {
  it("draws one line per table series with legend labels", async () => {
    const transport = new MockTransport({
      "plot.load": async ({ layout }) => payload(layout),
    });
    const { container } = render(
      <PlotPanel
        display={DISPLAY}
        panelId="panel-3"
        transport={transport}
        palette={["#111111", "#222222"]}
        onStateChange={() => {}}
      />,
    );

    await screen.findByTitle("Toggle a");
    expect(screen.getByTitle("Toggle b")).toBeTruthy();
    expect(container.querySelectorAll(".cut-line")).toHaveLength(2);
    expect(screen.getByText("2 lines · 2 rows")).toBeTruthy();
    expect(container.querySelector(".cut-plot")?.getAttribute("data-x-min")).not.toBe(
      null,
    );
  });

  it("re-reads the table as x/y pairs from the layout control", async () => {
    const calls: PlotLayout[] = [];
    const transport = new MockTransport({
      "plot.load": async ({ layout }) => {
        calls.push(layout);
        return payload(layout);
      },
    });
    function Harness() {
      const [state, setState] = useState<Record<string, unknown>>(
        DISPLAY.state,
      );
      return (
        <PlotPanel
          display={{ ...DISPLAY, state }}
          panelId="panel-3"
          transport={transport}
          palette={[]}
          onStateChange={(patch) =>
            setState((current) => ({ ...current, ...patch }))
          }
        />
      );
    }
    render(<Harness />);

    await screen.findByTitle("Toggle a");
    fireEvent.click(screen.getByRole("radio", { name: "x/y pairs" }));

    await screen.findByTitle("Toggle y1");
    expect(calls).toEqual(["columns", "pairs"]);
    expect(
      screen.getByText(/trailing column 'z' ignored/),
    ).toBeTruthy();
    expect(screen.getByText("1 line · 1 row")).toBeTruthy();
  });

  it("reports unreadable tables with a retry action", async () => {
    const transport = new MockTransport({
      "plot.load": async () => {
        throw new Error(
          "first column 'label' is not numeric; use the x/y pair layout",
        );
      },
    });
    render(
      <PlotPanel
        display={DISPLAY}
        panelId="panel-3"
        transport={transport}
        palette={[]}
        onStateChange={() => {}}
      />,
    );

    await screen.findByText(/first column 'label' is not numeric/);
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });
});

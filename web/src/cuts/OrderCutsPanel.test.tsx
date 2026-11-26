// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ShellDisplay } from "../shell/session";
import { MockTransport } from "../transport/mock";
import type { Cut, CutsPayload } from "./model";
import { OrderCutsPanel } from "./OrderCutsPanel";
import { defaultOrderSettings } from "./settings";

afterEach(cleanup);

const display: ShellDisplay = {
  id: 8,
  parent: 4,
  kind: "order_cuts",
  name: "Spectrogram 4 > Order Cuts",
  state: {},
};

const cuts: Cut[] = [
  {
    kind: { type: "order", value: 1 },
    points: [
      { timeS: 0, rpm: 2000, freqHz: 33.33, db: -20 },
      { timeS: 1, rpm: 2000, freqHz: 33.33, db: -10 },
    ],
  },
  {
    kind: { type: "freq", value: 120 },
    points: [
      { timeS: 0, rpm: 2000, freqHz: 120, db: -30 },
      { timeS: 1, rpm: 2000, freqHz: 120, db: -25 },
    ],
  },
];

const payload: CutsPayload = {
  cuts,
  durationS: 1,
  nyquist: 1000,
  binHz: 1,
  hopS: 0.1,
};

describe("OrderCutsPanel", () => {
  it("computes from its parent, plots cuts and exports", async () => {
    const transport = new MockTransport({
      "orderCuts.compute": () => payload,
    });
    const onExport = vi.fn();
    render(
      <OrderCutsPanel
        display={display}
        parentId={4}
        transport={transport}
        palette={["#56b4e9", "#e69f00"]}
        onStateChange={() => {}}
        onExport={onExport}
      />,
    );

    expect(await screen.findByText("1x")).toBeTruthy();
    expect(screen.getByText("120 Hz")).toBeTruthy();
    expect(transport.calls[0]).toEqual({
      name: "orderCuts.compute",
      args: { parentId: 4, settings: defaultOrderSettings() },
    });
    expect(screen.getByRole("img", { name: "Time (s) cut plot" })).toBeTruthy();

    fireEvent.click(
      screen.getByRole("button", { name: "Export CSV" }),
    );
    expect(onExport).toHaveBeenCalledWith(cuts);
  });

  it("passes a variable RPM profile from display state", async () => {
    const transport = new MockTransport({
      "orderCuts.compute": () => payload,
    });
    const rpm = {
      kind: "points" as const,
      value: [
        { timeS: 0, rpm: 6000 },
        { timeS: 0.5, rpm: 7200 },
      ],
    };
    const onStateChange = vi.fn();
    render(
      <OrderCutsPanel
        display={{ ...display, state: { rpm } }}
        parentId={4}
        transport={transport}
        palette={[]}
        onStateChange={onStateChange}
        onExport={() => {}}
      />,
    );
    await screen.findByText("1x");
    expect(transport.calls[0]).toEqual({
      name: "orderCuts.compute",
      args: { parentId: 4, settings: defaultOrderSettings(), rpm },
    });
    expect(screen.getByRole("img", { name: "RPM cut plot" })).toBeTruthy();
    expect(screen.getByTestId("order-tracking-summary").textContent).toBe(
      "2-point RPM profile",
    );

    fireEvent.click(screen.getByRole("button", { name: "Apply tracking" }));
    const patch = onStateChange.mock.calls[0][0] as {
      rpm: null;
      orderCuts: { rpm: { source: string } };
    };
    expect(patch.rpm).toBeNull();
    expect(patch.orderCuts.rpm.source).toBe("constant");
  });

  it("selects, deletes and edits cuts", async () => {
    const transport = new MockTransport({
      "orderCuts.compute": () => payload,
    });
    const onStateChange = vi.fn();
    const { container } = render(
      <OrderCutsPanel
        display={{ ...display, state: { orderCuts: defaultOrderSettings() } }}
        parentId={4}
        transport={transport}
        palette={[]}
        onStateChange={onStateChange}
        onExport={() => {}}
      />,
    );
    await screen.findByText("1x");

    const line = container.querySelector(".cut-line");
    expect(line).toBeTruthy();
    fireEvent.click(line!);
    expect(
      screen.getByRole("toolbar", { name: "Selected cut" }),
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(onStateChange).toHaveBeenCalled();
    const patch = onStateChange.mock.calls[0][0] as {
      orderCuts: { orders: string };
    };
    expect(patch.orderCuts.orders.split(",")).not.toContain("1");
  });

  it("changes the axis through the segmented control", async () => {
    const transport = new MockTransport({
      "orderCuts.compute": () => payload,
    });
    const onStateChange = vi.fn();
    render(
      <OrderCutsPanel
        display={display}
        parentId={4}
        transport={transport}
        palette={[]}
        onStateChange={onStateChange}
        onExport={() => {}}
      />,
    );
    await screen.findByText("1x");
    fireEvent.click(screen.getByRole("radio", { name: "Time" }));
    expect(onStateChange).toHaveBeenCalledWith({
      orderCuts: { ...defaultOrderSettings(), axis: "time" },
    });
  });

  it("toggles legend visibility", async () => {
    const transport = new MockTransport({
      "orderCuts.compute": () => payload,
    });
    const { container } = render(
      <OrderCutsPanel
        display={display}
        parentId={4}
        transport={transport}
        palette={[]}
        onStateChange={() => {}}
        onExport={() => {}}
      />,
    );
    await screen.findByText("1x");
    const before = container.querySelectorAll(".cut-line").length;
    fireEvent.click(screen.getByRole("button", { name: "1x" }));
    expect(container.querySelectorAll(".cut-line").length).toBe(before - 1);
  });

  it("hides grid lines from display state and persists the toggle", async () => {
    const transport = new MockTransport({
      "orderCuts.compute": () => payload,
    });
    const onStateChange = vi.fn();
    const { container } = render(
      <OrderCutsPanel
        display={{ ...display, state: { showGrid: false } }}
        parentId={4}
        transport={transport}
        palette={[]}
        onStateChange={onStateChange}
        onExport={() => {}}
      />,
    );
    await screen.findByText("1x");

    expect(container.querySelectorAll(".cut-grid")).toHaveLength(0);
    expect(container.querySelectorAll(".cut-tick").length).toBeGreaterThan(0);
    const toggle = screen.getByRole("button", { name: "Grid lines" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(toggle);
    expect(onStateChange).toHaveBeenCalledWith({ showGrid: true });
  });
});

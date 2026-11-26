// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { ShellDisplay } from "../shell/session";
import { MockTransport } from "../transport/mock";
import type { CutsPayload } from "./model";
import { makeLine } from "./model";
import { LineCutsPanel } from "./LineCutsPanel";
import { defaultLineSettings } from "./settings";

afterEach(cleanup);

const lines = [makeLine(0, 50, 1, 150)];
const display: ShellDisplay = {
  id: 9,
  parent: 4,
  kind: "line_cuts",
  name: "Spectrogram 4 > Line Cuts",
  state: { lines, lineCuts: defaultLineSettings() },
};

const payload: CutsPayload = {
  cuts: [
    {
      kind: { type: "line", value: 1 },
      points: [
        { timeS: 0, rpm: 0, freqHz: 50, db: -20 },
        { timeS: 0.5, rpm: 0, freqHz: 100, db: -15 },
        { timeS: 1, rpm: 0, freqHz: 150, db: -25 },
      ],
    },
  ],
  durationS: 1,
  nyquist: 1000,
  binHz: 1,
  hopS: 0.1,
};

describe("LineCutsPanel", () => {
  it("computes line cuts from the parent and edits endpoint numbers", async () => {
    const transport = new MockTransport({
      "lineCuts.compute": () => payload,
    });
    const onStateChange = vi.fn();
    const { container } = render(
      <LineCutsPanel
        display={display}
        parentId={4}
        transport={transport}
        palette={[]}
        onStateChange={onStateChange}
        onExport={() => {}}
      />,
    );

    expect(await screen.findByText("line 1")).toBeTruthy();
    expect(transport.calls[0]).toEqual({
      name: "lineCuts.compute",
      args: { parentId: 4, settings: defaultLineSettings(), lines },
    });

    fireEvent.click(container.querySelector(".cut-line")!);
    fireEvent.click(screen.getByRole("button", { name: "Edit numbers…" }));
    fireEvent.change(screen.getByRole("spinbutton", { name: "t0" }), {
      target: { value: "0.5" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));

    expect(onStateChange).toHaveBeenCalled();
    const patch = onStateChange.mock.calls[0][0] as {
      lines: Array<{ t0: number }>;
    };
    expect(patch.lines[0].t0).toBe(0.5);
  });

  it("deletes and flips the selected line", async () => {
    const transport = new MockTransport({
      "lineCuts.compute": () => payload,
    });
    const onStateChange = vi.fn();
    const { container } = render(
      <LineCutsPanel
        display={display}
        parentId={4}
        transport={transport}
        palette={[]}
        onStateChange={onStateChange}
        onExport={() => {}}
      />,
    );
    await screen.findByText("line 1");
    fireEvent.click(container.querySelector(".cut-line")!);

    fireEvent.click(screen.getByRole("button", { name: "Flip" }));
    let patch = onStateChange.mock.calls.at(-1)?.[0] as {
      lines: Array<{ t0: number; t1: number }>;
    };
    expect(patch.lines[0].t0).toBe(1);
    expect(patch.lines[0].t1).toBe(0);

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    patch = onStateChange.mock.calls.at(-1)?.[0] as {
      lines: Array<{ t0: number; t1: number }>;
    };
    expect(patch.lines).toEqual([]);
  });

  it("toggles grid lines from display state", async () => {
    const transport = new MockTransport({
      "lineCuts.compute": () => payload,
    });
    const onStateChange = vi.fn();
    const { container } = render(
      <LineCutsPanel
        display={{ ...display, state: { ...display.state, showGrid: false } }}
        parentId={4}
        transport={transport}
        palette={[]}
        onStateChange={onStateChange}
        onExport={() => {}}
      />,
    );
    await screen.findByText("line 1");

    expect(container.querySelectorAll(".cut-grid")).toHaveLength(0);
    const toggle = screen.getByRole("button", { name: "Grid lines" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(toggle);
    expect(onStateChange).toHaveBeenCalledWith({ showGrid: true });
  });
});

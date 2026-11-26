// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ColorBar } from "./ColorBar";

afterEach(cleanup);

describe("ColorBar", () => {
  it("renders both dB handles, endpoints and units", () => {
    render(<ColorBar minDb={-80} maxDb={0} onChange={() => {}} />);
    const max = screen.getByRole("slider", { name: "Maximum dB" });
    const min = screen.getByRole("slider", { name: "Minimum dB" });
    expect(max.getAttribute("aria-valuenow")).toBe("0");
    expect(min.getAttribute("aria-valuenow")).toBe("-80");
    expect(screen.getByTestId("colorbar")).toBeTruthy();
    expect(screen.getByTestId("colorbar-max-value").textContent).toBe("0");
    expect(screen.getByTestId("colorbar-min-value").textContent).toBe("-80");
    expect(screen.getByText("dB")).toBeTruthy();
    expect(screen.getByText("80 dB range")).toBeTruthy();
  });

  it("adjusts the max and min bounds with the keyboard and commits each step", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(
      <ColorBar
        minDb={-80}
        maxDb={0}
        domainMinDb={-100}
        domainMaxDb={20}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const max = screen.getByRole("slider", { name: "Maximum dB" });
    const min = screen.getByRole("slider", { name: "Minimum dB" });

    fireEvent.keyDown(max, { key: "ArrowUp" });
    expect(onChange).toHaveBeenLastCalledWith(-80, 1);
    expect(onCommit).toHaveBeenLastCalledWith(-80, 1);

    fireEvent.keyDown(min, { key: "ArrowDown" });
    expect(onChange).toHaveBeenLastCalledWith(-81, 0);
    expect(onCommit).toHaveBeenLastCalledWith(-81, 0);
  });

  it("commits once when a pointer drag ends", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(
      <ColorBar
        minDb={-80}
        maxDb={0}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const track = screen
      .getByTestId("colorbar")
      .querySelector(".colorbar-track") as HTMLElement;
    vi.spyOn(track, "getBoundingClientRect").mockReturnValue({
      top: 0,
      left: 0,
      width: 30,
      height: 200,
      right: 30,
      bottom: 200,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);

    fireEvent.pointerDown(screen.getByRole("slider", { name: "Maximum dB" }));
    fireEvent.pointerMove(window, { clientY: 100 });
    expect(onChange).toHaveBeenLastCalledWith(-80, -40);
    expect(onCommit).not.toHaveBeenCalled();

    fireEvent.pointerUp(window);
    expect(onCommit).toHaveBeenCalledOnce();
    expect(onCommit).toHaveBeenLastCalledWith(-80, -40);
  });

  it("moves both handles in both directions against a stable domain", () => {
    const onChange = vi.fn();
    render(
      <ColorBar
        minDb={-80}
        maxDb={0}
        domainMinDb={-100}
        domainMaxDb={10}
        onChange={onChange}
      />,
    );
    const track = screen
      .getByTestId("colorbar")
      .querySelector(".colorbar-track") as HTMLElement;
    vi.spyOn(track, "getBoundingClientRect").mockReturnValue({
      top: 0,
      left: 0,
      width: 30,
      height: 200,
      right: 30,
      bottom: 200,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);

    const max = screen.getByRole("slider", { name: "Maximum dB" });
    const min = screen.getByRole("slider", { name: "Minimum dB" });
    // Handles sit inside the track rather than pinned to its edges.
    expect(parseFloat(max.style.top)).toBeGreaterThan(0);
    expect(parseFloat(min.style.top)).toBeLessThan(100);

    fireEvent.pointerDown(max);
    fireEvent.pointerMove(window, { clientY: 10 });
    expect(onChange).toHaveBeenLastCalledWith(
      -80,
      expect.closeTo(4.5, 6),
    );
    fireEvent.pointerUp(window);

    fireEvent.pointerDown(max);
    fireEvent.pointerMove(window, { clientY: 60 });
    expect(onChange).toHaveBeenLastCalledWith(
      -80,
      expect.closeTo(-23, 6),
    );
    fireEvent.pointerUp(window);

    fireEvent.pointerDown(min);
    fireEvent.pointerMove(window, { clientY: 100 });
    expect(onChange).toHaveBeenLastCalledWith(
      expect.closeTo(-45, 6),
      0,
    );
    fireEvent.pointerUp(window);

    fireEvent.pointerDown(min);
    fireEvent.pointerMove(window, { clientY: 190 });
    expect(onChange).toHaveBeenLastCalledWith(
      expect.closeTo(-94.5, 6),
      0,
    );
    fireEvent.pointerUp(window);
  });

  it("pans the selected range by dragging inside it", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(
      <ColorBar
        minDb={-80}
        maxDb={-20}
        domainMinDb={-100}
        domainMaxDb={20}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const track = screen
      .getByTestId("colorbar")
      .querySelector(".colorbar-track") as HTMLElement;
    vi.spyOn(track, "getBoundingClientRect").mockReturnValue({
      top: 0,
      left: 0,
      width: 30,
      height: 200,
      right: 30,
      bottom: 200,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);

    const band = screen.getByRole("button", { name: "Move color range" });
    fireEvent.pointerDown(band, { clientY: 100 });
    fireEvent.pointerMove(window, { clientY: 80 });
    expect(onChange).toHaveBeenLastCalledWith(
      expect.closeTo(-68, 6),
      expect.closeTo(-8, 6),
    );
    fireEvent.pointerUp(window);
    expect(onCommit).toHaveBeenLastCalledWith(
      expect.closeTo(-68, 6),
      expect.closeTo(-8, 6),
    );

    fireEvent.pointerDown(band, { clientY: 100 });
    fireEvent.pointerMove(window, { clientY: 120 });
    expect(onChange).toHaveBeenLastCalledWith(
      expect.closeTo(-92, 6),
      expect.closeTo(-32, 6),
    );
    fireEvent.pointerUp(window);
  });

  it("shifts the selected range with the keyboard", () => {
    const onChange = vi.fn();
    const onCommit = vi.fn();
    render(
      <ColorBar
        minDb={-80}
        maxDb={-20}
        domainMinDb={-100}
        domainMaxDb={20}
        onChange={onChange}
        onCommit={onCommit}
      />,
    );
    const band = screen.getByRole("button", { name: "Move color range" });
    fireEvent.keyDown(band, { key: "ArrowUp" });
    expect(onChange).toHaveBeenLastCalledWith(-79, -19);
    expect(onCommit).toHaveBeenLastCalledWith(-79, -19);
    fireEvent.keyDown(band, { key: "ArrowDown" });
    expect(onChange).toHaveBeenLastCalledWith(-81, -21);
  });

  it("resets the color range from the endpoint button", () => {
    const onReset = vi.fn();
    render(
      <ColorBar minDb={-80} maxDb={0} onChange={() => {}} onReset={onReset} />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Reset color range" }));
    expect(onReset).toHaveBeenCalledOnce();
  });
});

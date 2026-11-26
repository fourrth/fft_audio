// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Tooltip } from "./Tooltip";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("Tooltip", () => {
  it("waits two seconds on hover and clears when the pointer leaves", () => {
    vi.useFakeTimers();
    const { container } = render(
      <Tooltip label="Frame length in milliseconds">
        <span>128 ms</span>
      </Tooltip>,
    );
    const anchor = container.querySelector(".tooltip-anchor")!;

    fireEvent.pointerEnter(anchor);
    act(() => vi.advanceTimersByTime(1999));
    expect(screen.queryByRole("tooltip")).toBeNull();

    act(() => vi.advanceTimersByTime(1));
    expect(screen.getByRole("tooltip").textContent).toBe(
      "Frame length in milliseconds",
    );

    fireEvent.pointerLeave(anchor);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("shows immediately on keyboard focus and links the description", () => {
    const { container } = render(
      <Tooltip label="Samples per frame">
        <span>2048</span>
      </Tooltip>,
    );
    const anchor = container.querySelector(".tooltip-anchor") as HTMLElement;
    fireEvent.focus(anchor);

    const tooltip = screen.getByRole("tooltip");
    expect(tooltip.textContent).toBe("Samples per frame");
    expect(anchor.getAttribute("aria-describedby")).toBe(tooltip.id);

    fireEvent.blur(anchor);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });
});

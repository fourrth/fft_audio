// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { DisplayFrame } from "./DisplayFrame";

afterEach(cleanup);

describe("DisplayFrame", () => {
  it("renders the three-row frame and fades hints after the first interaction", () => {
    render(
      <DisplayFrame
        title="Spectrogram 1"
        kind="spec"
        state={{ kind: "ready" }}
        footerHints="Wheel to zoom · F to fit"
      >
        <div>plot</div>
      </DisplayFrame>,
    );

    expect(screen.getByTestId("display-frame")).toBeTruthy();
    expect(screen.getByText("Spectrogram 1")).toBeTruthy();
    expect(screen.getByText("plot")).toBeTruthy();
    expect(screen.getByText("Wheel to zoom · F to fit")).toBeTruthy();

    fireEvent.pointerDown(screen.getByTestId("display-body"));
    expect(screen.queryByText("Wheel to zoom · F to fit")).toBeNull();
  });

  it("respects the hints setting", () => {
    render(
      <DisplayFrame
        title="Spectrogram"
        state={{ kind: "ready" }}
        footerHints="Hidden hint"
        hintsEnabled={false}
      />,
    );
    expect(screen.queryByText("Hidden hint")).toBeNull();
  });

  it("only shows plot hints when the display is ready", () => {
    const { rerender } = render(
      <DisplayFrame
        title="Spectrogram"
        state={{ kind: "empty", recentFiles: [] }}
        footerHints="Wheel pan"
      />,
    );
    expect(screen.queryByText("Wheel pan")).toBeNull();

    rerender(
      <DisplayFrame
        title="Spectrogram"
        state={{ kind: "loading", progress: { framesDone: 0, framesTotal: null } }}
        footerHints="Wheel pan"
      />,
    );
    expect(screen.queryByText("Wheel pan")).toBeNull();

    rerender(
      <DisplayFrame title="Spectrogram" state={{ kind: "ready" }} footerHints="Wheel pan" />,
    );
    expect(screen.getByText("Wheel pan")).toBeTruthy();
  });

  it("renders the empty state with recents, dialog and manual path", () => {
    const onBrowse = vi.fn();
    const onOpenRecent = vi.fn();
    const onOpenPath = vi.fn();
    render(
      <DisplayFrame
        title="Spectrogram"
        state={{
          kind: "empty",
          recentFiles: ["/audio/kick.wav"],
          onBrowse,
          onOpenRecent,
          onOpenPath,
        }}
      />,
    );

    expect(screen.getByText("No file loaded")).toBeTruthy();
    expect(
      screen.getByText("Drop a file anywhere on the window, or pick a recent file below."),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    expect(onBrowse).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "kick.wav" }));
    expect(onOpenRecent).toHaveBeenCalledWith("/audio/kick.wav");

    fireEvent.click(screen.getByRole("button", { name: "Enter a path" }));
    expect(screen.getByTestId("display-manual")).toBeTruthy();
    fireEvent.change(screen.getByRole("textbox", { name: "File path" }), {
      target: { value: "  /tmp/typed.flac  " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    expect(onOpenPath).toHaveBeenCalledWith("/tmp/typed.flac");
  });

  it("caps the empty-state recent list at five", () => {
    const recentFiles = Array.from(
      { length: 8 },
      (_, index) => `/audio/take-${index + 1}.wav`,
    );
    render(
      <DisplayFrame
        title="Spectrogram"
        state={{ kind: "empty", recentFiles }}
      />,
    );

    expect(screen.getByTestId("display-recent")).toBeTruthy();
    expect(screen.getAllByRole("button", { name: /^take-/ })).toHaveLength(5);
    expect(screen.queryByText("take-6.wav")).toBeNull();
  });

  it("renders determinate loading progress and cancels", () => {
    const onCancel = vi.fn();
    render(
      <DisplayFrame
        title="Spectrogram"
        state={{
          kind: "loading",
          progress: {
            framesDone: 10,
            framesTotal: 40,
            etaSeconds: 3,
          },
          onCancel,
        }}
      />,
    );

    expect(screen.getByText("Analysing 10/40 frames")).toBeTruthy();
    const bar = screen.getByRole("progressbar");
    expect(bar.getAttribute("aria-valuenow")).toBe("25");
    expect(screen.getByText(/3 s left/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("renders the error banner with recovery actions", () => {
    const onRetry = vi.fn();
    const onChooseFile = vi.fn();
    render(
      <DisplayFrame
        title="Spectrogram"
        state={{
          kind: "error",
          message: "Failed to decode the file",
          onRetry,
          onChooseFile,
        }}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("Failed to decode");
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Choose another file" }),
    );
    expect(onRetry).toHaveBeenCalledOnce();
    expect(onChooseFile).toHaveBeenCalledOnce();
  });

  it("shows the stale badge and shimmer for child displays", () => {
    render(
      <DisplayFrame
        title="Order Cuts"
        kind="order"
        parentLabel="Spectrogram 1"
        state={{ kind: "ready" }}
        stale
      >
        <div>cuts</div>
      </DisplayFrame>,
    );
    expect(screen.getByText("Spectrogram 1")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain(
      "Updating from parent",
    );
    expect(screen.getByText("cuts")).toBeTruthy();
  });
});

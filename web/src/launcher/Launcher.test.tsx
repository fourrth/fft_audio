// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { Launcher } from "./Launcher";
import { USAGE_TIPS } from "./recent";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const recent = ["/audio/kick.wav", "C:\\audio\\vocal.flac"];

describe("Launcher", () => {
  it("offers only root displays and enables Scripting", () => {
    render(
      <Launcher
        recentFiles={recent}
        scriptingEnabled
        onOpenSpectrogram={() => {}}
        onOpenRecorder={() => {}}
        onOpenScripting={() => {}}
        onOpenFile={() => {}}
      />,
    );

    expect(screen.getByRole("button", { name: /Spectrogram/ })).toBeTruthy();
    const scripting = screen.getByRole("button", { name: /Scripting/ });
    expect((scripting as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByText("Soon")).toBeNull();
    expect(
      screen.queryByRole("button", { name: /Order Cuts/ }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: /Line Cuts/ })).toBeNull();
  });

  it("hides the Scripting tile when scripting is unavailable", () => {
    render(
      <Launcher
        recentFiles={recent}
        scriptingEnabled={false}
        onOpenSpectrogram={() => {}}
        onOpenRecorder={() => {}}
        onOpenScripting={() => {}}
        onOpenFile={() => {}}
      />,
    );

    expect(screen.getByRole("button", { name: /Spectrogram/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Recorder/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Scripting/ })).toBeNull();
    expect(screen.getByText("2/8")).toBeTruthy();
  });

  it("spawns a Spectrogram from its tile", () => {
    const onOpenSpectrogram = vi.fn();
    render(
      <Launcher
        recentFiles={[]}
        scriptingEnabled
        onOpenSpectrogram={onOpenSpectrogram}
        onOpenRecorder={() => {}}
        onOpenScripting={() => {}}
        onOpenFile={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Spectrogram/ }));
    expect(onOpenSpectrogram).toHaveBeenCalledOnce();
  });

  it("spawns the Recorder from its tile", () => {
    const onOpenRecorder = vi.fn();
    render(
      <Launcher
        recentFiles={[]}
        scriptingEnabled
        onOpenSpectrogram={() => {}}
        onOpenRecorder={onOpenRecorder}
        onOpenScripting={() => {}}
        onOpenFile={() => {}}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /Recorder/ }));
    expect(onOpenRecorder).toHaveBeenCalledOnce();
  });

  it("opens a recent file and shows at most eight", () => {
    const onOpenFile = vi.fn();
    render(
      <Launcher
        recentFiles={recent}
        scriptingEnabled
        onOpenSpectrogram={() => {}}
        onOpenRecorder={() => {}}
        onOpenScripting={() => {}}
        onOpenFile={onOpenFile}
      />,
    );
    expect(screen.getByText("2/8")).toBeTruthy();
    expect(screen.getByText("kick.wav")).toBeTruthy();
    expect(screen.getByText("vocal.flac")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /kick\.wav/ }));
    expect(onOpenFile).toHaveBeenCalledWith("/audio/kick.wav");
  });

  it("shows the drop overlay and opens the dropped file", () => {
    const onOpenFile = vi.fn();
    render(
      <Launcher
        recentFiles={[]}
        scriptingEnabled
        onOpenSpectrogram={() => {}}
        onOpenRecorder={() => {}}
        onOpenScripting={() => {}}
        onOpenFile={onOpenFile}
      />,
    );
    const launcher = screen.getByTestId("launcher");
    const dataTransfer = {
      types: ["Files"],
      dropEffect: "none",
      files: [{ name: "drop.wav", path: "/audio/drop.wav" }],
    };

    fireEvent.dragEnter(launcher, { dataTransfer });
    expect(screen.getByTestId("launcher-drop-overlay").textContent).toContain(
      "Drop a file to open in a new Spectrogram",
    );

    fireEvent.drop(launcher, { dataTransfer });
    expect(screen.queryByTestId("launcher-drop-overlay")).toBeNull();
    expect(onOpenFile).toHaveBeenCalledWith("/audio/drop.wav");
  });

  it("rotates tips manually and automatically", () => {
    vi.useFakeTimers();
    render(
      <Launcher
        recentFiles={[]}
        scriptingEnabled
        onOpenSpectrogram={() => {}}
        onOpenRecorder={() => {}}
        onOpenScripting={() => {}}
        onOpenFile={() => {}}
      />,
    );
    expect(screen.getByText(USAGE_TIPS[0])).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Next tip" }));
    expect(screen.getByText(USAGE_TIPS[1])).toBeTruthy();
    expect(screen.getByText(`2/${USAGE_TIPS.length}`)).toBeTruthy();

    act(() => vi.advanceTimersByTime(12_000));
    expect(screen.getByText(USAGE_TIPS[2])).toBeTruthy();
  });
});

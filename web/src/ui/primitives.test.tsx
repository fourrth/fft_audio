// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import {
  Banner,
  Card,
  Chip,
  EmptyState,
  Field,
  IconButton,
  Kbd,
  ProgressBar,
  Segmented,
  Tile,
  Toggle,
  Toolbar,
  formatEta,
} from "./index";

afterEach(cleanup);

describe("shared UI primitives", () => {
  it("collapses and expands a Card", () => {
    render(
      <Card title="Analysis" defaultOpen={false}>
        hidden body
      </Card>,
    );
    const toggle = screen.getByRole("button", { name: /Analysis/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("hidden body")).toBeNull();
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("hidden body")).toBeTruthy();
  });

  it("emits the selected Segmented value", () => {
    const onChange = vi.fn();
    render(
      <Segmented
        label="Frequency axis"
        value="log"
        options={[
          { value: "log", label: "Log" },
          { value: "linear", label: "Linear" },
        ]}
        onChange={onChange}
      />,
    );
    const group = screen.getByRole("radiogroup", { name: "Frequency axis" });
    expect(group).toBeTruthy();
    expect(screen.getByRole("radio", { name: "Log" }).getAttribute("aria-checked")).toBe(
      "true",
    );
    fireEvent.click(screen.getByRole("radio", { name: "Linear" }));
    expect(onChange).toHaveBeenCalledWith("linear");
  });

  it("toggles a switch", () => {
    const onChange = vi.fn();
    render(<Toggle checked={false} onChange={onChange} label="Snap" />);
    const toggle = screen.getByRole("switch", { name: "Snap" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(toggle);
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("shows help on demand and renders a field banner", () => {
    render(
      <Field
        label="Frame"
        help="Samples per FFT frame"
        unit="samples"
        banner={{ tone: "warn", message: "Not applied yet" }}
      >
        <input aria-label="frame value" defaultValue="2048" />
      </Field>,
    );
    expect(screen.queryByRole("note")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Help for Frame" }));
    expect(screen.getByRole("note").textContent).toContain("Samples per FFT");
    expect(screen.getByText("samples")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("Not applied yet");
  });

  it("dismisses a Banner", () => {
    const onDismiss = vi.fn();
    render(
      <Banner tone="error" onDismiss={onDismiss}>
        Failed to open
      </Banner>,
    );
    expect(screen.getByRole("alert").textContent).toContain("Failed to open");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(onDismiss).toHaveBeenCalledOnce();
  });

  it("computes progress, ETA and cancel", () => {
    const onCancel = vi.fn();
    render(
      <ProgressBar
        value={0.25}
        label="Analysing"
        etaSeconds={42}
        onCancel={onCancel}
      />,
    );
    const bar = screen.getByRole("progressbar", { name: "Analysing" });
    expect(bar.getAttribute("aria-valuenow")).toBe("25");
    expect(screen.getByText(/42 s left/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledOnce();
    expect(formatEta(0.4)).toBe("<1 s left");
    expect(formatEta(125)).toBe("2:05 left");
    expect(formatEta(null)).toBeNull();
  });

  it("renders an indeterminate ProgressBar without aria-valuenow", () => {
    render(<ProgressBar label="Loading" />);
    expect(
      screen.getByRole("progressbar").getAttribute("aria-valuenow"),
    ).toBeNull();
  });

  it("supports chips, icon buttons, kbd, tiles and empty states", () => {
    const onClick = vi.fn();
    render(
      <div>
        <Chip tone="warn">Dirty</Chip>
        <IconButton label="Fit view" kbd="F">
          F
        </IconButton>
        <Kbd>Ctrl+K</Kbd>
        <Tile
          title="Spectrogram"
          description="Analyse audio"
          onClick={onClick}
        />
        <EmptyState
          title="No file loaded"
          description="Open an audio file"
          actions={<button type="button">Browse...</button>}
          hint="Drop a file here"
        />
      </div>,
    );
    expect(screen.getByText("Dirty")).toBeTruthy();
    const icon = screen.getByRole("button", { name: "Fit view" });
    expect(icon.getAttribute("title")).toBe("Fit view (F)");
    expect(screen.getByText("Ctrl+K")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Spectrogram/ }));
    expect(onClick).toHaveBeenCalledOnce();
    expect(screen.getByText("No file loaded")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Browse..." })).toBeTruthy();
    expect(screen.getByText("Drop a file here")).toBeTruthy();
  });

  it("opens Toolbar overflow actions", () => {
    render(
      <Toolbar overflow={[<button type="button" key="extra">Extra</button>]}>
        <button type="button">Main</button>
      </Toolbar>,
    );
    expect(screen.getByRole("toolbar")).toBeTruthy();
    expect(screen.queryByText("Extra")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "More actions" }));
    expect(screen.getByRole("menu", { name: "More actions" })).toBeTruthy();
    expect(screen.getByText("Extra")).toBeTruthy();
  });
});

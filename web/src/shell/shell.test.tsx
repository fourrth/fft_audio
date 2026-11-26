import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { CommandRegistry } from "../commands/registry";
import { AppShell } from "./AppShell";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";

const registry = new CommandRegistry([
  {
    id: "tab.new",
    title: "New Spectrogram",
    category: "Tabs",
    shortcut: "Ctrl+T",
    icon: "＋",
    run: () => {},
  },
]);

describe("shell chrome", () => {
  it("renders the 36 px top bar with an accessible menu bar", () => {
    const html = renderToStaticMarkup(
      <TopBar appName="fft_audio" mode="desktop" registry={registry} />,
    );

    expect(html).toContain('data-testid="topbar"');
    expect(html).toContain('role="menubar"');
    expect(html).toContain('aria-label="Main menu"');
    expect(html).toContain('aria-haspopup="menu"');
    expect(html).toContain("File");
    expect(html).not.toContain("View");
    expect(html).not.toContain("Cuts");
    expect(html).not.toContain("Help");
    expect(html).not.toContain("≡");
    expect(html).toContain("fft_audio");
    expect(html).toContain("desktop");
    expect(html).not.toContain("New Spectrogram");
  });

  it("renders the 24 px status bar with status and app metadata", () => {
    const html = renderToStaticMarkup(
      <StatusBar
        status="Session restored"
        mode="desktop"
        version="0.12.0"
        themeName="Dark"
      />,
    );

    expect(html).toContain('data-testid="statusbar"');
    expect(html).toContain("Session restored");
    expect(html).toContain("Dark · desktop · v0.12.0");
    expect(html).toContain('aria-label="Notifications (0)"');
  });

  it("renders the shell skeleton without a transport", () => {
    const html = renderToStaticMarkup(<AppShell transport={null} />);
    expect(html).toContain('class="shell"');
    expect(html).toContain('data-testid="topbar"');
    expect(html).toContain('data-testid="workspace"');
    expect(html).toContain('data-testid="statusbar"');
    expect(html).toContain("preview");
  });
});

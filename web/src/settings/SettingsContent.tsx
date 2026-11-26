import { useEffect, useMemo, useState } from "react";
import type { AppInfo, Transport } from "../transport/types";
import {
  applyAppearance,
  defaultSettings,
  scaleFromPercent,
  scalePercent,
  type AppearanceSettings,
  type Settings,
} from "./model";
import { filterShortcuts, shortcutsByCategory } from "./shortcuts";
import { applyTheme } from "../theme/css";
import type { ResolvedTheme, ThemeEntry } from "../theme/types";
import { Banner } from "../ui/Banner";
import { Card } from "../ui/Card";
import { Chip } from "../ui/Chip";
import { IconButton } from "../ui/IconButton";
import { Toggle } from "../ui/Toggle";
import { useToast } from "../ui/toast";
import "./settings.css";

export type SettingsPage =
  | "general"
  | "appearance"
  | "keyboard"
  | "files"
  | "themes"
  | "about";

const PAGES: Array<{ id: SettingsPage; label: string; icon: string }> = [
  { id: "general", label: "General", icon: "⚙" },
  { id: "appearance", label: "Appearance", icon: "◐" },
  { id: "keyboard", label: "Keyboard", icon: "⌨" },
  { id: "files", label: "Files", icon: "▤" },
  { id: "themes", label: "Themes", icon: "✦" },
  { id: "about", label: "About", icon: "ⓘ" },
];

interface ConfigPaths {
  configDir: string;
  settingsPath: string;
  sessionPath: string;
  themesDir: string;
}

export interface SettingsContentProps {
  transport: Transport | null;
  onClose?: () => void;
}

export function SettingsContent({ transport, onClose }: SettingsContentProps) {
  const [page, setPage] = useState<SettingsPage>("general");
  const [settings, setSettings] = useState<Settings>(defaultSettings());
  const [themes, setThemes] = useState<ThemeEntry[]>([]);
  const [paths, setPaths] = useState<ConfigPaths | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [loading, setLoading] = useState(Boolean(transport));
  const [error, setError] = useState<string | null>(null);
  const [shortcutQuery, setShortcutQuery] = useState("");
  const [newThemeName, setNewThemeName] = useState("");
  const { pushToast } = useToast();

  const loadThemes = async () => {
    if (!transport) {
      return;
    }
    const list = await transport.invoke({ name: "theme.list", args: {} });
    setThemes(list);
  };

  useEffect(() => {
    let cancelled = false;
    if (!transport) {
      setLoading(false);
      return;
    }
    const load = async () => {
      try {
        const [loaded, configPaths, appInfo, themeList] = await Promise.all([
          transport.invoke({ name: "settings.get", args: {} }),
          transport.invoke({ name: "config.paths", args: {} }),
          transport.invoke({ name: "app.info", args: {} }),
          transport.invoke({ name: "theme.list", args: {} }),
        ]);
        if (cancelled) {
          return;
        }
        setSettings(loaded);
        setPaths(configPaths);
        setInfo(appInfo);
        setThemes(themeList);
        applyAppearance(loaded.appearance);
        const theme = await transport.invoke({
          name: "theme.get",
          args: { name: loaded.appearance.theme },
        });
        if (!cancelled) {
          applyTheme(theme);
          applyAppearance(loaded.appearance);
        }
      } catch (reason) {
        if (!cancelled) {
          setError(reason instanceof Error ? reason.message : String(reason));
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    };
    void load();
    return () => {
      cancelled = true;
    };
  }, [transport]);

  const save = async (next: Settings) => {
    setSettings(next);
    if (!transport) {
      return;
    }
    try {
      await transport.invoke({ name: "settings.save", args: { settings: next } });
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `Could not save settings: ${
          reason instanceof Error ? reason.message : String(reason)
        }`,
      });
    }
  };

  const updateAppearance = (patch: Partial<AppearanceSettings>) => {
    const next = {
      ...settings,
      appearance: { ...settings.appearance, ...patch },
    };
    applyAppearance(next.appearance);
    void save(next);
  };

  const chooseTheme = async (id: string) => {
    const next = { ...settings, appearance: { ...settings.appearance, theme: id } };
    setSettings(next);
    if (!transport) {
      return;
    }
    try {
      const theme: ResolvedTheme = await transport.invoke({
        name: "theme.get",
        args: { name: id },
      });
      applyTheme(theme);
      applyAppearance(next.appearance);
      await save(next);
      pushToast({ tone: "success", message: `Theme: ${theme.name}` });
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `Could not apply the theme: ${
          reason instanceof Error ? reason.message : String(reason)
        }`,
      });
    }
  };

  const duplicateTheme = async () => {
    if (!transport) {
      return;
    }
    const current = themes.find((entry) => entry.id === settings.appearance.theme);
    const name = newThemeName.trim() || `${current?.name ?? "Theme"} copy`;
    try {
      const id = await transport.invoke({
        name: "theme.duplicate",
        args: { source: settings.appearance.theme, name },
      });
      await loadThemes();
      await chooseTheme(id);
      setNewThemeName("");
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `Could not duplicate the theme: ${
          reason instanceof Error ? reason.message : String(reason)
        }`,
      });
    }
  };

  const openPath = async (path: string) => {
    if (!transport) {
      return;
    }
    try {
      await transport.invoke({ name: "openPath", args: { path } });
    } catch (reason) {
      pushToast({
        tone: "error",
        message: `Could not open ${path}: ${
          reason instanceof Error ? reason.message : String(reason)
        }`,
      });
    }
  };

  const currentTheme = useMemo(
    () => themes.find((entry) => entry.id === settings.appearance.theme),
    [settings.appearance.theme, themes],
  );
  const shortcuts = filterShortcuts(shortcutQuery);
  const groups = shortcutsByCategory(shortcuts);
  const parseErrors = themes.filter((entry) => entry.parseError);

  return (
    <div className="settings-shell" data-testid="settings" data-ui-id="settings">
      <nav className="settings-rail" aria-label="Settings pages" data-ui-id="settings.rail">
        {PAGES.map((item) => (
          <button
            key={item.id}
            type="button"
            className={`settings-nav${page === item.id ? " active" : ""}`}
            aria-current={page === item.id ? "page" : undefined}
            onClick={() => setPage(item.id)}
          >
            <span aria-hidden="true">{item.icon}</span>
            {item.label}
          </button>
        ))}
      </nav>

      <div className="settings-main">
        <header className="settings-header">
          <div>
            <h1>{PAGES.find((item) => item.id === page)?.label}</h1>
            <p>fft_audio settings · changes are saved automatically</p>
          </div>
          {onClose ? (
            <IconButton label="Close settings" onClick={onClose}>
              ×
            </IconButton>
          ) : null}
        </header>

        {error ? (
          <Banner tone="error" onDismiss={() => setError(null)}>
            {error}
          </Banner>
        ) : null}
        {loading ? <Banner tone="info">Loading settings…</Banner> : null}

        <div className="settings-content">
          {page === "general" ? (
            <>
              <Card title="Start-up">
                <div className="settings-row">
                  <div>
                    <strong>Restore the last session</strong>
                    <p>Reopen displays, layout and recent files on launch.</p>
                  </div>
                  <Toggle
                    label="Restore the last session"
                    checked={settings.session.reopen_last}
                    onChange={(checked) =>
                      void save({
                        ...settings,
                        session: { reopen_last: checked },
                      })
                    }
                  />
                </div>
              </Card>
              <Card title="Defaults">
                <p className="settings-muted">
                  New spectrograms use the defaults saved from the Inspector
                  (“Use as default for new spectrograms”).
                </p>
                <div className="settings-chips">
                  <Chip>frame {settings.analysis.frame}</Chip>
                  <Chip>range {settings.display.range_db} dB</Chip>
                  <Chip>{settings.appearance.theme} theme</Chip>
                </div>
              </Card>
            </>
          ) : null}

          {page === "appearance" ? (
            <>
              <Card title="Scale and motion">
                <label className="settings-field">
                  <span>
                    UI scale <strong>{scalePercent(settings.appearance.ui_scale)}%</strong>
                  </span>
                  <input
                    aria-label="UI scale"
                    type="range"
                    min={75}
                    max={200}
                    step={5}
                    value={scalePercent(settings.appearance.ui_scale)}
                    onChange={(event) =>
                      updateAppearance({
                        ui_scale: scaleFromPercent(Number(event.target.value)),
                      })
                    }
                  />
                </label>
                <div className="settings-row">
                  <div>
                    <strong>Reduce motion</strong>
                    <p>Set transitions to 0 ms and stop pulses/shimmer.</p>
                  </div>
                  <Toggle
                    label="Reduce motion"
                    checked={settings.appearance.reduce_motion}
                    onChange={(checked) => updateAppearance({ reduce_motion: checked })}
                  />
                </div>
                <div className="settings-row">
                  <div>
                    <strong>High contrast</strong>
                    <p>Strengthen borders and secondary text.</p>
                  </div>
                  <Toggle
                    label="High contrast"
                    checked={settings.appearance.high_contrast}
                    onChange={(checked) => updateAppearance({ high_contrast: checked })}
                  />
                </div>
              </Card>

              <Card title="Theme">
                <div className="theme-grid" role="radiogroup" aria-label="Theme">
                  {themes.length === 0 ? (
                    <p className="settings-muted">No themes loaded in preview mode.</p>
                  ) : null}
                  {themes.map((entry) => (
                    <button
                      key={entry.id}
                      type="button"
                      role="radio"
                      aria-checked={settings.appearance.theme === entry.id}
                      className={`theme-tile${
                        settings.appearance.theme === entry.id ? " selected" : ""
                      }`}
                      onClick={() => void chooseTheme(entry.id)}
                      title={entry.name}
                    >
                      <span
                        className="theme-swatch"
                        style={{
                          background: entry.preview?.bg ?? "var(--bg-canvas)",
                          borderColor: entry.preview?.panel ?? "var(--border-subtle)",
                        }}
                      >
                        <span
                          style={{
                            background: entry.preview?.card ?? "var(--bg-card)",
                            color: entry.preview?.text ?? "var(--text-primary)",
                          }}
                        >
                          Aa
                        </span>
                        <span
                          className="theme-accent"
                          style={{ background: entry.preview?.accent ?? "var(--accent-primary)" }}
                        />
                      </span>
                      <span className="theme-name">{entry.name}</span>
                      {entry.builtin ? <Chip>built-in</Chip> : null}
                    </button>
                  ))}
                </div>
              </Card>
            </>
          ) : null}

          {page === "keyboard" ? (
            <Card title="Shortcuts">
              <input
                className="settings-search"
                type="search"
                aria-label="Filter shortcuts"
                placeholder="Filter shortcuts…"
                value={shortcutQuery}
                onChange={(event) => setShortcutQuery(event.target.value)}
              />
              {groups.map((group) => (
                <div key={group.category} className="shortcut-group">
                  <h3>{group.category}</h3>
                  <table>
                    <tbody>
                      {group.rows.map((row) => (
                        <tr key={row.id}>
                          <td>{row.title}</td>
                          <td>
                            <kbd>{row.keys}</kbd>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              ))}
              {shortcuts.length === 0 ? (
                <p className="settings-muted">No shortcut matches that filter.</p>
              ) : null}
            </Card>
          ) : null}

          {page === "files" ? (
            <>
              <Card title="Config and session files">
                <p className="settings-muted">
                  Paths use the platform config directory. Desktop builds can
                  open the file or folder with the system handler; the browser
                  build keeps these values in local storage.
                </p>
                {paths ? (
                  <div className="path-list">
                    {[
                      ["Config folder", paths.configDir],
                      ["Settings", paths.settingsPath],
                      ["Session", paths.sessionPath],
                      ["Themes", paths.themesDir],
                    ].map(([label, path]) => (
                      <div key={label} className="path-row">
                        <div>
                          <strong>{label}</strong>
                          <code>{path}</code>
                        </div>
                        {info?.mode === "desktop" ? (
                          <button
                            type="button"
                            className="secondary-button"
                            onClick={() => void openPath(path)}
                          >
                            Open
                          </button>
                        ) : null}
                      </div>
                    ))}
                  </div>
                ) : (
                  <p className="settings-muted">Paths are available in the desktop app.</p>
                )}
              </Card>
              <Card title="Recent files">
                <p className="settings-muted">
                  Up to 8 recent files are stored in session.json and shown on
                  the Launcher.
                </p>
              </Card>
            </>
          ) : null}

          {page === "themes" ? (
            <>
              {parseErrors.length > 0 ? (
                <Banner tone="error">
                  {parseErrors.length} theme file(s) have parse errors:{" "}
                  {parseErrors.map((entry) => entry.id).join(", ")}
                </Banner>
              ) : null}
              <Card title="Installed themes">
                <div className="settings-row">
                  <div>
                    <strong>{currentTheme?.name ?? settings.appearance.theme}</strong>
                    <p>
                      {currentTheme?.builtin ? "Built-in theme" : "User theme"} ·{" "}
                      {themes.length} available
                    </p>
                  </div>
                  <button
                    type="button"
                    className="secondary-button"
                    onClick={() => void loadThemes()}
                  >
                    Reload
                  </button>
                </div>
                <ul className="theme-list">
                  {themes.map((entry) => (
                    <li key={entry.id}>
                      <button
                        type="button"
                        className={`theme-list-item${
                          settings.appearance.theme === entry.id ? " selected" : ""
                        }`}
                        onClick={() => void chooseTheme(entry.id)}
                      >
                        <span>{entry.name}</span>
                        <code>{entry.id}.toml</code>
                        {entry.parseError ? <Chip tone="error">parse error</Chip> : null}
                      </button>
                    </li>
                  ))}
                </ul>
              </Card>
              <Card title="Duplicate current as new theme">
                <p className="settings-muted">
                  Writes a standalone TOML copy into the themes folder.
                </p>
                <div className="settings-inline">
                  <input
                    aria-label="New theme name"
                    placeholder={`${currentTheme?.name ?? "Theme"} copy`}
                    value={newThemeName}
                    onChange={(event) => setNewThemeName(event.target.value)}
                  />
                  <button
                    type="button"
                    className="primary-button"
                    onClick={() => void duplicateTheme()}
                    disabled={!transport || settings.appearance.theme.length === 0}
                  >
                    Duplicate current as new theme
                  </button>
                </div>
              </Card>
            </>
          ) : null}

          {page === "about" ? (
            <Card title={`About ${info?.name ?? "fft_audio"}`}>
              <div className="about-grid">
                <div>
                  <strong>Version</strong>
                  <span>{info?.version ?? "0.12.0"}</span>
                </div>
                <div>
                  <strong>Mode</strong>
                  <span>{info?.mode ?? "preview"}</span>
                </div>
                <div>
                  <strong>Displays</strong>
                  <span>
                    {info ? (info.capabilities.displays ? "Enabled" : "Disabled") : "—"}
                  </span>
                </div>
                <div>
                  <strong>Scripting UI</strong>
                  <span>
                    {info
                      ? (info.capabilities.scripting ?? "Not included in this build")
                      : "—"}
                  </span>
                </div>
                <div>
                  <strong>Table formats</strong>
                  <span>
                    {info?.capabilities.tableFormats.join(", ") || "—"}
                  </span>
                </div>
                <div>
                  <strong>UI</strong>
                  <span>Tauri 2 · React · Vite</span>
                </div>
                <div>
                  <strong>Docking</strong>
                  <span>Dockview (tauri-dockview)</span>
                </div>
                <div>
                  <strong>Core</strong>
                  <span>Rust · FFT (realfft) · Symphonia · csv/JSON · optional Parquet/Arrow</span>
                </div>
                <div>
                  <strong>Transport</strong>
                  <span>Tauri IPC (desktop) · HTTP/WS (headless)</span>
                </div>
              </div>
            </Card>
          ) : null}
        </div>
      </div>
    </div>
  );
}

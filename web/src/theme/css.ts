import type { ResolvedTheme, StatusName } from "./types";

/** Minimal element surface needed for theme injection (DOM or a test fake). */
export interface StyleTarget {
  style: {
    setProperty(name: string, value: string): void;
  };
  setAttribute?(name: string, value: string): void;
}

const STATUS_NAMES: StatusName[] = ["info", "success", "warn", "error"];

/**
 * Turn a resolved theme into CSS custom properties.
 *
 * colors are emitted as `--<token>`, the palette as `--plot-palette-1..8`
 * (plus a joined `--plot-palette`), and status tones as
 * `--status-<name>-{bg,fg}`.
 */
export function themeToCssVars(theme: ResolvedTheme): Record<string, string> {
  const vars: Record<string, string> = {};

  for (const [key, value] of Object.entries(theme.colors)) {
    const name = key.startsWith("--") ? key : `--${key}`;
    vars[name] = value;
  }

  theme.plotPalette.forEach((color, index) => {
    vars[`--plot-palette-${index + 1}`] = color;
  });
  if (theme.plotPalette.length > 0) {
    vars["--plot-palette"] = theme.plotPalette.join(", ");
  }

  for (const name of STATUS_NAMES) {
    const colors = theme.status[name];
    if (!colors) {
      continue;
    }
    vars[`--status-${name}-bg`] = colors.bg;
    vars[`--status-${name}-fg`] = colors.fg;
  }

  return vars;
}

/** Inject the theme's CSS variables and appearance attribute at runtime. */
export function applyTheme(
  theme: ResolvedTheme,
  target?: StyleTarget,
): ResolvedTheme {
  const element =
    target ??
    (typeof document === "undefined"
      ? undefined
      : (document.documentElement as unknown as StyleTarget));
  if (!element) {
    throw new Error("applyTheme needs a DOM element (or an explicit target)");
  }

  for (const [name, value] of Object.entries(themeToCssVars(theme))) {
    element.style.setProperty(name, value);
  }
  element.setAttribute?.("data-theme", theme.appearance);
  return theme;
}

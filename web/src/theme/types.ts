export type Appearance = "dark" | "light";

export type StatusName = "info" | "success" | "warn" | "error";

export interface StatusColors {
  bg: string;
  fg: string;
}

/** The resolved shape returned by the `theme.get` command (Rust camelCase). */
export interface ResolvedTheme {
  name: string;
  appearance: Appearance;
  colors: Record<string, string>;
  plotPalette: string[];
  status: Record<StatusName, StatusColors>;
}

export interface ThemePreview {
  bg: string;
  panel: string;
  card: string;
  text: string;
  accent: string;
}

/** One entry in the theme picker. */
export interface ThemeEntry {
  id: string;
  name: string;
  builtin: boolean;
  appearance: Appearance | null;
  preview: ThemePreview | null;
  parseError: string | null;
}

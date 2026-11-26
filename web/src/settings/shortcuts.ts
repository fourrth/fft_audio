export interface ShortcutRow {
  id: string;
  title: string;
  keys: string;
  category: "Global" | "Display" | "Tabs" | "Help";
}

/** Default shortcuts from the product specification. */
export const DEFAULT_SHORTCUTS: ShortcutRow[] = [
  { id: "new-tab", title: "New tab", keys: "Ctrl+T", category: "Tabs" },
  { id: "close-tab", title: "Close tab", keys: "Ctrl+W", category: "Tabs" },
  { id: "next-tab", title: "Next tab", keys: "Ctrl+Tab", category: "Tabs" },
  {
    id: "prev-tab",
    title: "Previous tab",
    keys: "Ctrl+Shift+Tab",
    category: "Tabs",
  },
  { id: "open", title: "Open file", keys: "Ctrl+O", category: "Global" },
  { id: "palette", title: "Command palette", keys: "Ctrl+K", category: "Global" },
  { id: "settings", title: "Settings", keys: "Ctrl+,", category: "Global" },
  { id: "inspector", title: "Toggle inspector", keys: "P", category: "Display" },
  { id: "fit", title: "Fit view", keys: "F", category: "Display" },
  { id: "maximize", title: "Maximize / restore", keys: "M", category: "Display" },
  { id: "escape", title: "Cancel / close popup", keys: "Esc", category: "Global" },
  { id: "shortcuts", title: "Keyboard shortcuts", keys: "F1", category: "Help" },
];

export function filterShortcuts(query: string): ShortcutRow[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) {
    return DEFAULT_SHORTCUTS;
  }
  return DEFAULT_SHORTCUTS.filter(
    (row) =>
      row.title.toLowerCase().includes(q) ||
      row.keys.toLowerCase().includes(q) ||
      row.category.toLowerCase().includes(q),
  );
}

export function shortcutsByCategory(
  rows: ShortcutRow[],
): Array<{ category: string; rows: ShortcutRow[] }> {
  const categories = ["Global", "Tabs", "Display", "Help"];
  return categories
    .map((category) => ({
      category,
      rows: rows.filter((row) => row.category === category),
    }))
    .filter((group) => group.rows.length > 0);
}

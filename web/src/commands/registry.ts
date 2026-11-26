/** Central command registry: menus, palette, tooltips and shortcuts use this. */

export interface UiCommand {
  id: string;
  title: string;
  category: string;
  icon?: string;
  shortcut?: string;
  keywords?: string[];
  enabled?: () => boolean;
  run: () => void | Promise<void>;
}

export class CommandRegistry {
  private readonly commands: UiCommand[];

  constructor(commands: UiCommand[]) {
    this.commands = commands;
  }

  all(): UiCommand[] {
    return this.commands;
  }

  available(): UiCommand[] {
    return this.commands.filter((command) => command.enabled?.() ?? true);
  }

  get(id: string): UiCommand | undefined {
    return this.commands.find((command) => command.id === id);
  }

  search(query: string): UiCommand[] {
    const q = query.trim();
    if (q.length === 0) {
      return this.available();
    }
    return this.available()
      .map((command) => ({
        command,
        score: fuzzyScore(q, `${command.title} ${command.category} ${command.keywords?.join(" ") ?? ""}`),
      }))
      .filter((entry) => entry.score > 0)
      .sort((a, b) => b.score - a.score)
      .map((entry) => entry.command);
  }

  /** Run a command if present and enabled; returns whether it ran. */
  async run(id: string): Promise<boolean> {
    const command = this.get(id);
    if (!command || !(command.enabled?.() ?? true)) {
      return false;
    }
    await command.run();
    return true;
  }
}

/** Subsequence fuzzy score; 0 means no match. */
export function fuzzyScore(query: string, text: string): number {
  const q = query.toLowerCase().replace(/\s+/g, "");
  const t = text.toLowerCase();
  if (q.length === 0) {
    return 1;
  }
  let index = 0;
  let score = 0;
  let streak = 0;
  for (const ch of t) {
    if (index < q.length && ch === q[index]) {
      index += 1;
      streak += 1;
      score += 2 + streak;
    } else if (index > 0) {
      streak = 0;
    }
  }
  if (index !== q.length) {
    return 0;
  }
  return score + 1;
}

export interface KeyEventLike {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  shiftKey?: boolean;
  altKey?: boolean;
}

const KEY_ALIASES: Record<string, string> = {
  esc: "escape",
  escape: "escape",
  comma: ",",
  del: "delete",
};

function normalizeKey(key: string): string {
  const lower = key.toLowerCase();
  return KEY_ALIASES[lower] ?? (lower.length === 1 ? lower : lower);
}

/** Does a keyboard event match a shortcut such as `Ctrl+Shift+Tab` or `P`? */
export function shortcutMatches(event: KeyEventLike, shortcut: string): boolean {
  const parts = shortcut.split("+").map((part) => part.trim()).filter(Boolean);
  const key = parts.pop();
  if (!key) {
    return false;
  }
  const wantsCtrl = parts.some((part) => part.toLowerCase() === "ctrl" || part.toLowerCase() === "cmd");
  const wantsShift = parts.some((part) => part.toLowerCase() === "shift");
  const wantsAlt = parts.some((part) => part.toLowerCase() === "alt");
  const hasCtrl = Boolean(event.ctrlKey || event.metaKey);
  if (hasCtrl !== wantsCtrl || Boolean(event.shiftKey) !== wantsShift || Boolean(event.altKey) !== wantsAlt) {
    return false;
  }
  return normalizeKey(event.key) === normalizeKey(key);
}

export function shortcutFromId(commands: UiCommand[], id: string): UiCommand | undefined {
  return commands.find((command) => command.id === id);
}

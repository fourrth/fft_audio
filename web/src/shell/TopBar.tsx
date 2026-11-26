import type { AppMode } from "../transport/types";
import type { CommandRegistry } from "../commands/registry";
import {
  MenuBar,
  type MenuBarMenu,
  type MenuItemSpec,
  type MenuSubmenu,
} from "../ui";

export interface TopBarProps {
  appName: string;
  mode: AppMode | "preview";
  /** The single source of truth for the menu bar. */
  registry: CommandRegistry;
}

function commandItem(
  registry: CommandRegistry,
  id: string,
  uiPrefix: string,
): MenuItemSpec | null {
  const command = registry.get(id);
  if (!command) {
    return null;
  }
  return {
    kind: "item",
    id,
    label: command.title,
    icon: command.icon,
    shortcut: command.shortcut,
    disabled: !(command.enabled?.() ?? true),
    uiId: `${uiPrefix}.${id}`,
    onSelect: () => {
      void registry.run(id);
    },
  };
}

function commandItems(
  registry: CommandRegistry,
  ids: string[],
  uiPrefix: string,
): MenuItemSpec[] {
  return ids
    .map((id) => commandItem(registry, id, uiPrefix))
    .filter((item): item is MenuItemSpec => item !== null);
}

function separator(id: string): MenuItemSpec {
  return { kind: "separator", id };
}

function submenu(
  id: string,
  label: string,
  items: MenuItemSpec[],
  uiPrefix: string,
): MenuSubmenu {
  return {
    kind: "submenu",
    id,
    label,
    items,
    disabled: items.length === 0,
    uiId: `${uiPrefix}.${id}`,
  };
}

function recentItems(
  registry: CommandRegistry,
  uiPrefix: string,
): MenuItemSpec[] {
  return registry
    .all()
    .filter((command) => command.category === "Recent")
    .map((command) => commandItem(registry, command.id, uiPrefix))
    .filter((item): item is MenuItemSpec => item !== null);
}

function presetItems(
  registry: CommandRegistry,
  uiPrefix: string,
): MenuItemSpec[] {
  return registry
    .all()
    .filter(
      (command) =>
        command.category === "Layout" && command.id.startsWith("layout.preset."),
    )
    .map((command) => commandItem(registry, command.id, uiPrefix))
    .filter((item): item is MenuItemSpec => item !== null);
}

function hasEnabledItem(items: MenuItemSpec[]): boolean {
  return items.some((entry) => {
    if (entry.kind === "item") {
      return !entry.disabled;
    }
    if (entry.kind === "submenu") {
      return !entry.disabled && hasEnabledItem(entry.items);
    }
    return false;
  });
}

function buildMenus(registry: CommandRegistry): MenuBarMenu[] {
  const menus: MenuBarMenu[] = [
    {
      id: "file",
      label: "File",
      uiId: "shell.menu.file",
      items: [
        ...commandItems(
          registry,
          ["tab.newLauncher", "tab.new"],
          "shell.menu.file",
        ),
        separator("file.sep-input"),
        ...commandItems(registry, ["file.open"], "shell.menu.file"),
        ...commandItems(
          registry,
          ["file.newPlot", "file.newPlotPairs"],
          "shell.menu.file",
        ),
        ...commandItems(registry, ["file.exportImage"], "shell.menu.file"),
        submenu(
          "file.recent",
          "Recent",
          recentItems(registry, "shell.menu.file"),
          "shell.menu.file",
        ),
        separator("file.sep-close"),
        ...commandItems(registry, ["tab.close"], "shell.menu.file"),
      ],
    },
    {
      id: "view",
      label: "View",
      uiId: "shell.menu.view",
      items: [
        ...commandItems(
          registry,
          ["view.palette", "settings.open"],
          "shell.menu.view",
        ),
        separator("view.sep-panels"),
        ...commandItems(
          registry,
          ["inspector.toggle", "display.fit", "view.maximize"],
          "shell.menu.view",
        ),
        separator("view.sep-tabs"),
        ...commandItems(
          registry,
          ["tab.next", "tab.prev"],
          "shell.menu.view",
        ),
      ],
    },
    {
      id: "layout",
      label: "Layout",
      uiId: "shell.menu.layout",
      items: [
        ...commandItems(
          registry,
          [
            "layout.reset",
            "layout.analysis",
            "layout.cuts",
            "layout.focus",
          ],
          "shell.menu.layout",
        ),
        separator("layout.sep"),
        ...commandItems(registry, ["layout.savePreset"], "shell.menu.layout"),
        submenu(
          "layout.presets",
          "Saved presets",
          presetItems(registry, "shell.menu.layout"),
          "shell.menu.layout",
        ),
      ],
    },
    {
      id: "cuts",
      label: "Cuts",
      uiId: "shell.menu.cuts",
      items: [
        ...commandItems(
          registry,
          ["cuts.order", "cuts.line"],
          "shell.menu.cuts",
        ),
      ],
    },
    {
      id: "help",
      label: "Help",
      uiId: "shell.menu.help",
      items: commandItems(
        registry,
        ["help.shortcuts", "help.about"],
        "shell.menu.help",
      ),
    },
  ];
  return menus.filter((menu) => hasEnabledItem(menu.items));
}

/** 36 px global bar with the conventional File/View/Layout/Cuts/Help menus. */
export function TopBar({ appName, mode, registry }: TopBarProps) {
  return (
    <header className="topbar" data-testid="topbar" data-ui-id="shell.topbar">
      <div className="topbar-left">
        <span className="app-title" data-ui-id="shell.app-title">
          {appName}
        </span>
        <MenuBar menus={buildMenus(registry)} />
      </div>
      <div className="topbar-right">
        <span
          className={`mode-chip mode-${mode}`}
          data-ui-id="shell.mode"
        >
          {mode}
        </span>
      </div>
    </header>
  );
}

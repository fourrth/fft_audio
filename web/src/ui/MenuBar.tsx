import {
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { MenuPopover, type MenuItemSpec } from "./Menu";

export interface MenuBarMenu {
  id: string;
  label: string;
  items: MenuItemSpec[];
  uiId?: string;
}

export interface MenuBarProps {
  menus: MenuBarMenu[];
}

/** Conventional top-level menu bar with keyboard switching and submenus. */
export function MenuBar({ menus }: MenuBarProps) {
  const [openId, setOpenId] = useState<string | null>(null);
  const [position, setPosition] = useState<{ x: number; y: number } | null>(
    null,
  );
  const buttons = useRef<Record<string, HTMLButtonElement | null>>({});

  const openMenu = (id: string) => {
    const button = buttons.current[id];
    if (!button) {
      return;
    }
    const rect = button.getBoundingClientRect();
    setPosition({ x: rect.left, y: rect.bottom + 2 });
    setOpenId(id);
  };

  const close = () => {
    setOpenId(null);
    setPosition(null);
  };

  const handleButtonKey = (
    event: ReactKeyboardEvent<HTMLButtonElement>,
    menu: MenuBarMenu,
  ) => {
    if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (openId === menu.id) {
        close();
      } else {
        openMenu(menu.id);
      }
      return;
    }
    if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
      event.preventDefault();
      const index = menus.findIndex((entry) => entry.id === menu.id);
      const next =
        menus[
          (index + (event.key === "ArrowRight" ? 1 : -1) + menus.length) %
            menus.length
        ];
      if (openId) {
        openMenu(next.id);
      } else {
        buttons.current[next.id]?.focus();
      }
    }
    if (event.key === "Escape") {
      close();
    }
  };

  return (
    <nav
      className="menubar"
      role="menubar"
      aria-label="Main menu"
      data-ui-id="shell.menubar"
    >
      {menus.map((menu) => (
        <div className="menubar-entry" key={menu.id}>
          <button
            ref={(button) => {
              buttons.current[menu.id] = button;
            }}
            type="button"
            role="menuitem"
            className="menubar-button"
            aria-haspopup="menu"
            aria-expanded={openId === menu.id}
            data-ui-id={menu.uiId}
            onPointerDown={(event) => event.stopPropagation()}
            onClick={() => {
              if (openId === menu.id) {
                close();
              } else {
                openMenu(menu.id);
              }
            }}
            onKeyDown={(event) => handleButtonKey(event, menu)}
          >
            {menu.label}
          </button>
          {openId === menu.id && position ? (
            <MenuPopover
              key={menu.id}
              x={position.x}
              y={position.y}
              label={menu.label}
              items={menu.items}
              onClose={close}
              className="menubar-popover"
            />
          ) : null}
        </div>
      ))}
    </nav>
  );
}

export type { MenuItemSpec };

import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { ViewportMenu } from "./ViewportMenu";

export interface MenuItem {
  kind: "item";
  id: string;
  label: string;
  icon?: ReactNode;
  shortcut?: string;
  disabled?: boolean;
  onSelect: () => void;
  uiId?: string;
}

export interface MenuSeparator {
  kind: "separator";
  id: string;
}

export interface MenuSubmenu {
  kind: "submenu";
  id: string;
  label: string;
  icon?: ReactNode;
  items: MenuItemSpec[];
  disabled?: boolean;
  uiId?: string;
}

export type MenuItemSpec = MenuItem | MenuSeparator | MenuSubmenu;

export interface MenuPopoverProps {
  x: number;
  y: number;
  label: string;
  items: MenuItemSpec[];
  onClose: () => void;
  className?: string;
}

interface MenuEntriesProps {
  items: MenuItemSpec[];
  onClose: () => void;
  autoFocus?: boolean;
  onBack?: () => void;
}

function menuButtonText(item: MenuItem | MenuSubmenu): string {
  return item.label;
}

function MenuEntries({
  items,
  onClose,
  autoFocus = false,
  onBack,
}: MenuEntriesProps) {
  const buttonRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const [openSubmenu, setOpenSubmenu] = useState<string | null>(null);

  const buttons = () =>
    buttonRefs.current.filter(
      (button): button is HTMLButtonElement =>
        button !== null && !button.disabled,
    );

  const focusAt = (index: number) => {
    const enabled = buttons();
    if (enabled.length === 0) {
      return;
    }
    const normalised = ((index % enabled.length) + enabled.length) % enabled.length;
    enabled[normalised]?.focus();
  };

  const focusSibling = (direction: 1 | -1) => {
    const enabled = buttons();
    const current = enabled.indexOf(document.activeElement as HTMLButtonElement);
    if (current < 0) {
      focusAt(0);
      return;
    }
    focusAt(current + direction);
  };

  useEffect(() => {
    if (autoFocus) {
      focusAt(0);
    }
    // The first render is the menu opening; item changes should not steal focus.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoFocus]);

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.defaultPrevented) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onClose();
      return;
    }
    if (event.key === "ArrowLeft" && onBack) {
      event.preventDefault();
      event.stopPropagation();
      onBack();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      focusSibling(event.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      focusAt(event.key === "Home" ? 0 : buttons().length - 1);
    }
  };

  return (
    <div className="ui-menu-entries" onKeyDown={handleKeyDown}>
      {items.map((entry, index) => {
        if (entry.kind === "separator") {
          return (
            <div
              key={entry.id}
              className="ui-menu-separator"
              role="separator"
            />
          );
        }
        if (entry.kind === "submenu") {
          return (
            <SubmenuEntry
              key={entry.id}
              entry={entry}
              open={openSubmenu === entry.id}
              onOpen={() => setOpenSubmenu(entry.id)}
              onClose={() => setOpenSubmenu(null)}
              onCloseAll={onClose}
              buttonRef={(button) => {
                buttonRefs.current[index] = button;
              }}
            />
          );
        }
        return (
          <div
            className="ui-menu-entry"
            key={entry.id}
            onPointerEnter={() => setOpenSubmenu(null)}
          >
            <button
              ref={(button) => {
                buttonRefs.current[index] = button;
              }}
              type="button"
              role="menuitem"
              className="ui-menu-item"
              aria-label={entry.label}
              disabled={entry.disabled}
              aria-disabled={entry.disabled || undefined}
              data-ui-id={entry.uiId}
              title={entry.shortcut ? `${entry.label} (${entry.shortcut})` : entry.label}
              onClick={() => {
                if (entry.disabled) {
                  return;
                }
                entry.onSelect();
                onClose();
              }}
            >
              <span className="ui-menu-label">
                {entry.icon ? (
                  <span className="ui-menu-icon" aria-hidden="true">
                    {entry.icon}
                  </span>
                ) : null}
                {menuButtonText(entry)}
              </span>
              {entry.shortcut ? (
                <kbd className="ui-menu-shortcut">{entry.shortcut}</kbd>
              ) : null}
            </button>
          </div>
        );
      })}
    </div>
  );
}

function SubmenuEntry({
  entry,
  open,
  onOpen,
  onClose,
  onCloseAll,
  buttonRef,
}: {
  entry: MenuSubmenu;
  open: boolean;
  onOpen: () => void;
  onClose: () => void;
  onCloseAll: () => void;
  buttonRef: (button: HTMLButtonElement | null) => void;
}) {
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(
    null,
  );

  const openSubmenu = () => {
    if (entry.disabled) {
      return;
    }
    const rect = triggerRef.current?.getBoundingClientRect();
    if (!rect) {
      return;
    }
    setPosition({ left: rect.right - 2, top: rect.top - 4 });
    onOpen();
  };

  useEffect(() => {
    if (!open) {
      setPosition(null);
    }
  }, [open]);

  return (
    <div
      className="ui-menu-entry ui-submenu-entry"
      onPointerEnter={openSubmenu}
    >
      <button
        ref={(button) => {
          buttonRef(button);
          triggerRef.current = button;
        }}
        type="button"
        role="menuitem"
        className="ui-menu-item"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={entry.label}
        disabled={entry.disabled}
        aria-disabled={entry.disabled || undefined}
        data-ui-id={entry.uiId}
        onClick={() => {
          if (!open) {
            openSubmenu();
          }
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowRight") {
            event.preventDefault();
            event.stopPropagation();
            openSubmenu();
          }
        }}
      >
        <span className="ui-menu-label">
          {entry.icon ? (
            <span className="ui-menu-icon" aria-hidden="true">
              {entry.icon}
            </span>
          ) : null}
          {entry.label}
        </span>
        <span className="ui-submenu-arrow" aria-hidden="true">
          ›
        </span>
      </button>
      {open && position ? (
        <div
          className="ui-submenu"
          role="menu"
          aria-label={entry.label}
          style={{ left: position.left, top: position.top }}
        >
          <MenuEntries
            items={entry.items}
            onClose={onCloseAll}
            autoFocus
            onBack={() => {
              onClose();
              requestAnimationFrame(() => triggerRef.current?.focus());
            }}
          />
        </div>
      ) : null}
    </div>
  );
}

/** Fixed, viewport-clamped popover menu with one nesting level of submenus. */
export function MenuPopover({
  x,
  y,
  label,
  items,
  onClose,
  className,
}: MenuPopoverProps) {
  return (
    <ViewportMenu
      x={x}
      y={y}
      onClose={onClose}
      className={`ui-menu${className ? ` ${className}` : ""}`}
      role="menu"
      aria-label={label}
    >
      <MenuEntries items={items} onClose={onClose} autoFocus />
    </ViewportMenu>
  );
}

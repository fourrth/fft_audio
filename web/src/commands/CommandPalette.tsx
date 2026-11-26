import { useEffect, useMemo, useRef, useState } from "react";
import { CommandRegistry, type UiCommand } from "./registry";
import "./commands.css";

export interface CommandPaletteProps {
  registry: CommandRegistry;
  onRun: (id: string) => void;
  onClose: () => void;
}

export function CommandPalette({ registry, onRun, onClose }: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const commands = useMemo(() => registry.search(query), [query, registry]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    setIndex(0);
  }, [query]);

  const run = (command: UiCommand | undefined) => {
    if (!command) {
      return;
    }
    onClose();
    onRun(command.id);
  };

  return (
    <div className="modal-backdrop" onPointerDown={onClose}>
      <div
        className="command-palette"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <input
          ref={inputRef}
          aria-label="Search commands"
          placeholder="Type a command…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              onClose();
            } else if (event.key === "ArrowDown") {
              event.preventDefault();
              setIndex((value) => Math.min(commands.length - 1, value + 1));
            } else if (event.key === "ArrowUp") {
              event.preventDefault();
              setIndex((value) => Math.max(0, value - 1));
            } else if (event.key === "Enter") {
              event.preventDefault();
              run(commands[index]);
            }
          }}
        />
        <ul className="command-results" role="listbox" aria-label="Commands">
          {commands.map((command, row) => (
            <li key={command.id} role="option" aria-selected={row === index}>
              <button
                type="button"
                className={`command-row${row === index ? " active" : ""}`}
                onPointerEnter={() => setIndex(row)}
                onClick={() => run(command)}
              >
                <span className="command-icon" aria-hidden="true">
                  {command.icon ?? "›"}
                </span>
                <span className="command-title">
                  {command.title}
                  <span className="command-category">{command.category}</span>
                </span>
                {command.shortcut ? <kbd>{command.shortcut}</kbd> : null}
              </button>
            </li>
          ))}
          {commands.length === 0 ? (
            <li className="command-empty">No matching commands</li>
          ) : null}
        </ul>
        <footer className="command-hint">
          ↑↓ navigate · Enter run · Esc close
        </footer>
      </div>
    </div>
  );
}

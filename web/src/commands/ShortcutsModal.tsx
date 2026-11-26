import { useState } from "react";
import { DEFAULT_SHORTCUTS, filterShortcuts } from "../settings/shortcuts";
import { IconButton } from "../ui/IconButton";
import "./commands.css";

export interface ShortcutsModalProps {
  onClose: () => void;
}

export function ShortcutsModal({ onClose }: ShortcutsModalProps) {
  const [query, setQuery] = useState("");
  const rows = filterShortcuts(query);
  return (
    <div className="modal-backdrop" onPointerDown={onClose}>
      <div
        className="shortcuts-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Keyboard shortcuts"
        onPointerDown={(event) => event.stopPropagation()}
      >
        <header>
          <h2>Keyboard shortcuts</h2>
          <IconButton label="Close shortcuts" onClick={onClose}>
            ×
          </IconButton>
        </header>
        <input
          type="search"
          aria-label="Filter shortcuts"
          placeholder="Filter shortcuts…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <div className="shortcuts-table">
          {rows.map((row) => (
            <div className="shortcut-row" key={row.id}>
              <span>{row.title}</span>
              <kbd>{row.keys}</kbd>
            </div>
          ))}
          {rows.length === 0 ? (
            <p>No shortcut matches that filter.</p>
          ) : null}
        </div>
        <footer>{DEFAULT_SHORTCUTS.length} default shortcuts · F1 to close</footer>
      </div>
    </div>
  );
}

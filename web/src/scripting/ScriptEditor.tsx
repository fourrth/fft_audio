import type { ChangeEvent, KeyboardEvent } from "react";

export interface ScriptEditorProps {
  value: string;
  dirty: boolean;
  busy: boolean;
  onChange: (value: string) => void;
  onSave: () => void;
}

/**
 * Controlled, transport-free script editor. Owns only text editing and the
 * Ctrl/Cmd+S shortcut; reading, writing and running live in the container.
 */
export function ScriptEditor({
  value,
  dirty,
  busy,
  onChange,
  onSave,
}: ScriptEditorProps) {
  const handleChange = (event: ChangeEvent<HTMLTextAreaElement>) => {
    onChange(event.target.value);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (
      (event.ctrlKey || event.metaKey) &&
      !event.altKey &&
      event.key.toLowerCase() === "s"
    ) {
      event.preventDefault();
      if (dirty && !busy) {
        onSave();
      }
    }
  };

  return (
    <div className="script-editor" aria-busy={busy}>
      <div className="script-editor-status">
        {busy ? (
          <span className="script-muted" role="status">
            Working…
          </span>
        ) : dirty ? (
          <span className="script-dirty" data-testid="scripting-editor-dirty">
            Unsaved changes
          </span>
        ) : (
          <span className="script-muted">Up to date</span>
        )}
      </div>
      <textarea
        aria-label="Script text"
        className="script-editor-text"
        value={value}
        onChange={handleChange}
        onKeyDown={handleKeyDown}
        readOnly={busy}
        spellCheck={false}
        placeholder="Open a script or load an example to start editing."
      />
    </div>
  );
}

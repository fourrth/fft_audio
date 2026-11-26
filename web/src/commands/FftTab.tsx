import { useContext, useState } from "react";
import type { IDockviewPanelHeaderProps } from "dockview-react";
import type { ShellDisplay } from "../shell/session";
import { ViewportMenu } from "../ui/ViewportMenu";
import { TabActionsContext } from "./tabActions";
import "./commands.css";

/** Custom Dockview tab: inline rename, middle-click close, context menu. */
export function FftTab(props: IDockviewPanelHeaderProps) {
  const actions = useContext(TabActionsContext);
  const panel = props.containerApi.getPanel(props.api.id);
  const display = (panel?.params as { display?: ShellDisplay } | undefined)
    ?.display;
  const title = props.api.title ?? panel?.title ?? display?.name ?? "Tab";
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(title);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  const commit = () => {
    const name = draft.trim();
    if (name && actions) {
      actions.rename(props.api.id, name);
    }
    setEditing(false);
  };

  const displayId = display?.id;

  return (
    <div
      className="fft-tab"
      title={title}
      data-ui-id="tab"
      data-panel-id={props.api.id}
      onDoubleClick={() => {
        if (!actions) return;
        setDraft(title);
        setEditing(true);
      }}
      onAuxClick={(event) => {
        if (event.button === 1) {
          event.preventDefault();
          actions?.requestClose(props.api.id);
        }
      }}
      onContextMenu={(event) => {
        if (!actions) return;
        event.preventDefault();
        setMenu({ x: event.clientX, y: event.clientY });
      }}
    >
      {editing ? (
        <input
          className="fft-tab-input"
          aria-label="Rename tab"
          autoFocus
          value={draft}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              commit();
            } else if (event.key === "Escape") {
              setEditing(false);
            }
          }}
        />
      ) : (
        <span className="tab-title">{title}</span>
      )}
      <button
        type="button"
        className="fft-tab-close"
        aria-label={`Close ${title}`}
        data-ui-id="tab.close"
        onClick={(event) => {
          event.stopPropagation();
          actions?.requestClose(props.api.id);
        }}
      >
        ×
      </button>

      {menu && actions ? (
        <ViewportMenu
          className="tab-context-menu"
          role="menu"
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
        >
          <button
            type="button"
            role="menuitem"
            disabled={displayId === undefined}
            onClick={() => {
              setMenu(null);
              if (displayId !== undefined) {
                actions.toggleInspector(displayId);
              }
            }}
          >
            Inspector
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMenu(null);
              actions.duplicate(props.api.id);
            }}
          >
            Duplicate
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMenu(null);
              setDraft(title);
              setEditing(true);
            }}
          >
            Rename
          </button>
          <div className="context-separator" />
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setMenu(null);
              actions.requestClose(props.api.id);
            }}
          >
            Close
          </button>
        </ViewportMenu>
      ) : null}
    </div>
  );
}

import type { ReactNode } from "react";
import { ViewportMenu } from "./ViewportMenu";

export interface PlotContextMenuProps {
  x: number;
  y: number;
  label: string;
  onClose: () => void;
  /** Show Copy read-out when provided; disabled when null. */
  readout?: string | null;
  onResetView?: () => void;
  onExportImage: () => void;
  /** Display-specific actions rendered after the standard separator. */
  children?: ReactNode;
}

/**
 * Shared plot context menu. Every plot gets the same core actions, then its
 * screen-specific commands; keep new cross-plot actions here.
 */
export function PlotContextMenu({
  x,
  y,
  label,
  onClose,
  readout,
  onResetView,
  onExportImage,
  children,
}: PlotContextMenuProps) {
  return (
    <ViewportMenu
      className="plot-context-menu"
      role="menu"
      aria-label={label}
      x={x}
      y={y}
      onClose={onClose}
      onContextMenu={(event) => event.preventDefault()}
    >
      {onResetView ? (
        <button
          type="button"
          role="menuitem"
          onClick={() => {
            onClose();
            onResetView();
          }}
        >
          Reset view
        </button>
      ) : null}
      {readout !== undefined ? (
        <button
          type="button"
          role="menuitem"
          disabled={!readout}
          onClick={() => {
            if (readout) {
              void navigator.clipboard?.writeText(readout);
            }
            onClose();
          }}
        >
          Copy read-out
        </button>
      ) : null}
      <button
        type="button"
        role="menuitem"
        onClick={() => {
          onClose();
          onExportImage();
        }}
      >
        Export image
      </button>
      {children ? (
        <>
          <div className="context-separator" />
          {children}
        </>
      ) : null}
    </ViewportMenu>
  );
}

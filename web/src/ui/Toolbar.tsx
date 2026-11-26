import { useState, type ReactNode } from "react";
import { IconButton } from "./IconButton";

export interface ToolbarProps {
  children: ReactNode;
  /** Actions that collapse into the `...` menu on narrow layouts. */
  overflow?: ReactNode[];
  label?: string;
  className?: string;
}

/** Inline action row with an overflow menu. */
export function Toolbar({
  children,
  overflow = [],
  label = "Toolbar",
  className,
}: ToolbarProps) {
  const [open, setOpen] = useState(false);
  return (
    <div className={`toolbar${className ? ` ${className}` : ""}`} role="toolbar" aria-label={label}>
      <div className="toolbar-main">{children}</div>
      {overflow.length > 0 ? (
        <div className="toolbar-overflow">
          <IconButton label="More actions" pressed={open} onClick={() => setOpen((v) => !v)}>
            …
          </IconButton>
          {open ? (
            <div className="toolbar-menu" role="menu" aria-label="More actions">
              {overflow.map((item, index) => (
                <div className="toolbar-menu-item" role="none" key={index}>
                  {item}
                </div>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

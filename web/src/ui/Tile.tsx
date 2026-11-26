import type { MouseEvent, ReactNode } from "react";
import { Chip } from "./Chip";

export interface TileProps {
  icon?: ReactNode;
  title: string;
  description: string;
  badge?: string;
  kind?:
    | "spec"
    | "order"
    | "line"
    | "recorder"
    | "launcher"
    | "settings";
  disabled?: boolean;
  onClick?: () => void;
  onContextMenu?: (event: MouseEvent<HTMLButtonElement>) => void;
}

/** Large launcher tile (root displays only). */
export function Tile({
  icon,
  title,
  description,
  badge,
  kind = "spec",
  disabled,
  onClick,
  onContextMenu,
}: TileProps) {
  return (
    <button
      type="button"
      className={`tile tile-${kind}`}
      disabled={disabled}
      aria-disabled={disabled}
      onClick={onClick}
      onContextMenu={onContextMenu}
    >
      <span className="tile-icon" aria-hidden="true">
        {icon ?? "▤"}
      </span>
      <span className="tile-text">
        <span className="tile-title">{title}</span>
        <span className="tile-description">{description}</span>
      </span>
      {badge ? <Chip tone="neutral">{badge}</Chip> : null}
    </button>
  );
}

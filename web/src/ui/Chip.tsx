import type { ReactNode } from "react";

export type ChipTone =
  | "neutral"
  | "info"
  | "success"
  | "warn"
  | "error"
  | "spec"
  | "order"
  | "line"
  | "launcher";

export interface ChipProps {
  children: ReactNode;
  tone?: ChipTone;
  title?: string;
  active?: boolean;
  onClick?: () => void;
}

/** Small status/kind label; a button when clickable. */
export function Chip({ children, tone = "neutral", title, active, onClick }: ChipProps) {
  const className = `chip chip-${tone}${active ? " active" : ""}`;
  if (onClick) {
    return (
      <button
        type="button"
        className={className}
        title={title}
        aria-pressed={active}
        onClick={onClick}
      >
        {children}
      </button>
    );
  }
  return (
    <span className={className} title={title}>
      {children}
    </span>
  );
}

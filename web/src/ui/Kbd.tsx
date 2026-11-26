import type { ReactNode } from "react";

export interface KbdProps {
  children: ReactNode;
  className?: string;
}

/** Monospace keycap hint. */
export function Kbd({ children, className }: KbdProps) {
  return <kbd className={`kbd${className ? ` ${className}` : ""}`}>{children}</kbd>;
}

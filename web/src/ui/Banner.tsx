import type { ReactNode } from "react";
import { IconButton } from "./IconButton";

export type BannerTone = "info" | "success" | "warn" | "error";

const ICONS: Record<BannerTone, string> = {
  info: "i",
  success: "✓",
  warn: "!",
  error: "×",
};

export interface BannerProps {
  tone: BannerTone;
  children: ReactNode;
  actions?: ReactNode;
  onDismiss?: () => void;
}

/** Inline status banner with icon, text and optional actions/dismiss. */
export function Banner({ tone, children, actions, onDismiss }: BannerProps) {
  return (
    <div
      className={`banner banner-${tone}`}
      role={tone === "error" ? "alert" : "status"}
    >
      <span className="banner-icon" aria-hidden="true">
        {ICONS[tone]}
      </span>
      <div className="banner-text">{children}</div>
      {actions ? <div className="banner-actions">{actions}</div> : null}
      {onDismiss ? (
        <IconButton label="Dismiss" className="banner-dismiss" onClick={onDismiss}>
          ×
        </IconButton>
      ) : null}
    </div>
  );
}

import type { ButtonHTMLAttributes, ReactNode } from "react";

export interface IconButtonProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "aria-label"> {
  label: string;
  children: ReactNode;
  /** Shortcut hint shown on the right and in the tooltip. */
  kbd?: string | null;
  pressed?: boolean;
}

/** 28×28 minimum icon button with tooltip and keyboard hint. */
export function IconButton({
  label,
  children,
  kbd,
  pressed,
  className,
  title,
  type = "button",
  ...rest
}: IconButtonProps) {
  const tooltip = title ?? (kbd ? `${label} (${kbd})` : label);
  return (
    <button
      {...rest}
      type={type}
      className={`icon-button${className ? ` ${className}` : ""}`}
      aria-label={label}
      aria-pressed={pressed}
      title={tooltip}
    >
      <span className="icon-button-glyph" aria-hidden="true">
        {children}
      </span>
      {kbd ? <kbd className="icon-button-kbd">{kbd}</kbd> : null}
    </button>
  );
}

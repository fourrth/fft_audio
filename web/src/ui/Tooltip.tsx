import {
  useEffect,
  useId,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

export interface TooltipProps {
  /** Explanation shown after the delay. */
  label: string;
  children: ReactNode;
  /** Hover delay in milliseconds; keyboard focus shows immediately. */
  delayMs?: number;
  className?: string;
}

interface Position {
  top: number;
  left: number;
}

/** Delayed, keyboard-accessible tooltip anchored to a wrapper element. */
export function Tooltip({
  label,
  children,
  delayMs = 2000,
  className,
}: TooltipProps) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<Position | null>(null);
  const anchorRef = useRef<HTMLSpanElement>(null);
  const timer = useRef<number | undefined>(undefined);
  const tooltipId = useId();

  const clearTimer = () => {
    if (timer.current !== undefined) {
      window.clearTimeout(timer.current);
      timer.current = undefined;
    }
  };

  const hide = () => {
    clearTimer();
    setOpen(false);
  };

  const show = () => {
    const rect = anchorRef.current?.getBoundingClientRect();
    if (!rect) {
      return;
    }
    const halfWidth = 130;
    const left = Math.min(
      Math.max(rect.left + rect.width / 2, halfWidth + 8),
      Math.max(halfWidth + 8, window.innerWidth - halfWidth - 8),
    );
    setPosition({ top: rect.bottom + 6, left });
    setOpen(true);
  };

  const schedule = (delay: number) => {
    clearTimer();
    if (delay <= 0) {
      show();
      return;
    }
    timer.current = window.setTimeout(show, delay);
  };

  useEffect(() => {
    if (!open) {
      return;
    }
    const onScrollOrResize = () => hide();
    window.addEventListener("scroll", onScrollOrResize, true);
    window.addEventListener("resize", onScrollOrResize);
    return () => {
      window.removeEventListener("scroll", onScrollOrResize, true);
      window.removeEventListener("resize", onScrollOrResize);
    };
  }, [open]);

  useEffect(() => () => clearTimer(), []);

  return (
    <span
      ref={anchorRef}
      className={`tooltip-anchor${className ? ` ${className}` : ""}`}
      tabIndex={0}
      aria-describedby={open ? tooltipId : undefined}
      onPointerEnter={() => schedule(delayMs)}
      onPointerLeave={hide}
      onPointerDown={hide}
      onFocus={() => schedule(0)}
      onBlur={hide}
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          hide();
        }
      }}
    >
      {children}
      {open && position && typeof document !== "undefined"
        ? createPortal(
            <span
              id={tooltipId}
              role="tooltip"
              className="tooltip-popover"
              style={{ top: position.top, left: position.left }}
            >
              {label}
            </span>,
            document.body,
          )
        : null}
    </span>
  );
}

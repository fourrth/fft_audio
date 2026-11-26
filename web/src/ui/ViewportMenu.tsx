import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type HTMLAttributes,
  type ReactNode,
} from "react";
import { clampMenuPosition } from "./menuPosition";

export interface ViewportMenuProps
  extends Omit<HTMLAttributes<HTMLDivElement>, "style"> {
  x: number;
  y: number;
  onClose?: () => void;
  children: ReactNode;
}

/**
 * Fixed-position menu that measures itself and stays inside the visual
 * viewport. Re-clamps when the viewport or any scroll container moves.
 */
export function ViewportMenu({
  x,
  y,
  onClose,
  children,
  className,
  ...rest
}: ViewportMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ left: x, top: y });

  const reposition = useCallback(() => {
    const node = ref.current;
    if (!node) {
      return;
    }
    const rect = node.getBoundingClientRect();
    const viewportWidth =
      document.documentElement.clientWidth || window.innerWidth;
    const viewportHeight =
      document.documentElement.clientHeight || window.innerHeight;
    const next = clampMenuPosition({
      x,
      y,
      width: rect.width,
      height: rect.height,
      viewportWidth,
      viewportHeight,
    });
    setPosition((current) =>
      current.left === next.left && current.top === next.top ? current : next,
    );
  }, [x, y]);

  useLayoutEffect(() => {
    reposition();
  }, [reposition]);

  useEffect(() => {
    window.addEventListener("resize", reposition);
    window.addEventListener("scroll", reposition, true);
    return () => {
      window.removeEventListener("resize", reposition);
      window.removeEventListener("scroll", reposition, true);
    };
  }, [reposition]);

  useEffect(() => {
    if (!onClose) {
      return;
    }
    const onPointerDown = (event: PointerEvent) => {
      if (!ref.current?.contains(event.target as Node)) {
        onClose();
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [onClose]);

  return (
    <div
      {...rest}
      ref={ref}
      className={className}
      style={{ left: position.left, top: position.top }}
    >
      {children}
    </div>
  );
}

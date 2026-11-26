export interface MenuPositionInput {
  x: number;
  y: number;
  width: number;
  height: number;
  viewportWidth: number;
  viewportHeight: number;
  margin?: number;
}

export interface MenuPosition {
  left: number;
  top: number;
}

/**
 * Place a fixed-position menu at an anchor, flipping left/up when it would
 * overflow and keeping a safe margin on all sides. Menus larger than the
 * viewport are pinned to the top/left margin.
 */
export function clampMenuPosition({
  x,
  y,
  width,
  height,
  viewportWidth,
  viewportHeight,
  margin = 8,
}: MenuPositionInput): MenuPosition {
  const safeMargin = Math.max(0, margin);
  const maxLeft = Math.max(safeMargin, viewportWidth - safeMargin - width);
  const maxTop = Math.max(safeMargin, viewportHeight - safeMargin - height);

  let left = x;
  let top = y;
  if (left + width > viewportWidth - safeMargin) {
    left -= width;
  }
  if (top + height > viewportHeight - safeMargin) {
    top -= height;
  }

  return {
    left: Math.min(Math.max(left, safeMargin), maxLeft),
    top: Math.min(Math.max(top, safeMargin), maxTop),
  };
}

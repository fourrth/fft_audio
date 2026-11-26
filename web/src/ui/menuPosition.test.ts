import { describe, expect, it } from "vitest";
import { clampMenuPosition } from "./menuPosition";

describe("clampMenuPosition", () => {
  it("keeps a menu that fits at the anchor", () => {
    expect(
      clampMenuPosition({
        x: 100,
        y: 120,
        width: 200,
        height: 150,
        viewportWidth: 800,
        viewportHeight: 600,
      }),
    ).toEqual({ left: 100, top: 120 });
  });

  it("flips left and up at the right/bottom edges", () => {
    expect(
      clampMenuPosition({
        x: 700,
        y: 560,
        width: 200,
        height: 100,
        viewportWidth: 800,
        viewportHeight: 600,
      }),
    ).toEqual({ left: 500, top: 460 });
  });

  it("applies the safe margin near the top/left edges", () => {
    expect(
      clampMenuPosition({
        x: 3,
        y: 2,
        width: 50,
        height: 40,
        viewportWidth: 300,
        viewportHeight: 200,
      }),
    ).toEqual({ left: 8, top: 8 });

    expect(
      clampMenuPosition({
        x: 12,
        y: 12,
        width: 50,
        height: 40,
        viewportWidth: 300,
        viewportHeight: 200,
        margin: 20,
      }),
    ).toEqual({ left: 20, top: 20 });
  });

  it("clamps to the margin when flipping would leave the viewport", () => {
    expect(
      clampMenuPosition({
        x: 50,
        y: 40,
        width: 100,
        height: 80,
        viewportWidth: 120,
        viewportHeight: 100,
      }),
    ).toEqual({ left: 8, top: 8 });
  });

  it("pins oversized menus to the top/left margin", () => {
    expect(
      clampMenuPosition({
        x: 400,
        y: 300,
        width: 1000,
        height: 900,
        viewportWidth: 600,
        viewportHeight: 400,
      }),
    ).toEqual({ left: 8, top: 8 });
  });
});

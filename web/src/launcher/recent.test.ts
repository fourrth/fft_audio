import { describe, expect, it } from "vitest";
import {
  LAUNCHER_TILES,
  MAX_RECENT_FILES,
  USAGE_TIPS,
  addRecentFile,
  fileName,
  folderOf,
  isSpawnableTile,
  launcherTiles,
  nextTipIndex,
  normalizeRecentFiles,
  tipAt,
} from "./recent";

describe("launcher state", () => {
  it("offers only root displays and never Order/Line Cuts", () => {
    const ids = LAUNCHER_TILES.map((tile) => tile.id);
    expect(ids).toEqual(["spectrogram", "recorder", "scripting"]);
    expect(ids).not.toContain("order_cuts");
    expect(ids).not.toContain("line_cuts");

    const spawnable = LAUNCHER_TILES.filter(isSpawnableTile);
    expect(spawnable.map((tile) => tile.id)).toEqual([
      "spectrogram",
      "recorder",
      "scripting",
    ]);
    expect(LAUNCHER_TILES[1].soon).toBeUndefined();
    expect(LAUNCHER_TILES[1].root).toBe(true);
  });

  it("hides Scripting unless the build offers the scripting UI", () => {
    expect(launcherTiles(true).map((tile) => tile.id)).toEqual([
      "spectrogram",
      "recorder",
      "scripting",
    ]);
    expect(launcherTiles(false).map((tile) => tile.id)).toEqual([
      "spectrogram",
      "recorder",
    ]);
    // The exported catalogue is shared state: the helper must not mutate it.
    expect(LAUNCHER_TILES.map((tile) => tile.id)).toEqual([
      "spectrogram",
      "recorder",
      "scripting",
    ]);
  });

  it("keeps at most eight unique recent files, newest first", () => {
    let recent: string[] = [];
    for (let i = 0; i < 10; i += 1) {
      recent = addRecentFile(recent, `/audio/file-${i}.wav`);
    }
    expect(recent).toHaveLength(MAX_RECENT_FILES);
    expect(recent[0]).toBe("/audio/file-9.wav");
    expect(recent).not.toContain("/audio/file-0.wav");
    expect(recent).not.toContain("/audio/file-1.wav");

    recent = addRecentFile(recent, "/audio/file-5.wav");
    expect(recent[0]).toBe("/audio/file-5.wav");
    expect(recent.filter((path) => path === "/audio/file-5.wav")).toHaveLength(1);
    expect(recent).toHaveLength(MAX_RECENT_FILES);
  });

  it("normalizes damaged recent lists", () => {
    expect(normalizeRecentFiles(null)).toEqual([]);
    expect(
      normalizeRecentFiles(["a.wav", "", " a.wav ", 3, "b.wav"]),
    ).toEqual(["a.wav", "b.wav"]);
    expect(normalizeRecentFiles(Array.from({ length: 20 }, (_, i) => `${i}.wav`))).toHaveLength(8);
  });

  it("rotates tips deterministically", () => {
    expect(USAGE_TIPS.length).toBeGreaterThan(1);
    expect(tipAt(0)).toBe(USAGE_TIPS[0]);
    expect(nextTipIndex(0)).toBe(1);
    expect(nextTipIndex(USAGE_TIPS.length - 1)).toBe(0);
    expect(tipAt(USAGE_TIPS.length)).toBe(USAGE_TIPS[0]);
  });

  it("extracts display names and folders from either separator style", () => {
    expect(fileName("/tmp/song.wav")).toBe("song.wav");
    expect(fileName("C:\\audio\\song.flac")).toBe("song.flac");
    expect(folderOf("/tmp/song.wav")).toBe("/tmp");
    expect(folderOf("C:\\audio\\song.flac")).toBe("C:\\audio");
    expect(folderOf("song.wav")).toBe("");
  });
});

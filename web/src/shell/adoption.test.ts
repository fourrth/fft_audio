import { describe, expect, it } from "vitest";
import type { DisplayInfo } from "../transport/types";
import type { ShellDisplay } from "./session";
import {
  SCRIPTED_DISPLAY_ID_BASE,
  closedDisplayTreeIds,
  closedPanelIds,
  displayAdoptionOrder,
  isBackendOwnedDisplay,
  mergeDisplayInfo,
  nextUserDisplayId,
  recreatedUserDisplay,
  restoreDisposition,
  restoreDropIds,
  sanitizeNextId,
  sessionDisplays,
  shouldAttachAnalysis,
  upsertDisplay,
} from "./adoption";

function backendInfo(overrides: Partial<DisplayInfo> = {}): DisplayInfo {
  return {
    id: SCRIPTED_DISPLAY_ID_BASE,
    parent: null,
    kind: "spectrogram",
    name: "tone.wav",
    state: { analysisId: 4, sourceKind: "backend", scripted: true },
    visible: true,
    ephemeral: false,
    ...overrides,
  };
}

const userDisplay: ShellDisplay = {
  id: 2,
  parent: null,
  kind: "spectrogram",
  name: "tone.wav",
  state: {
    path: "/audio/tone.wav",
    analysisSettings: { frame: 2048 },
    displaySettings: { log_freq: false, max_hz: 4000, range_db: 60 },
    showGrid: true,
    lines: [{ t0: 0, f0: 10, t1: 1, f1: 10, logF: false }],
  },
};

describe("display adoption", () => {
  it("orders a startup snapshot parent-first", () => {
    const root = backendInfo({ id: 3, parent: null });
    const child = backendInfo({
      id: 5,
      parent: 10,
      kind: "order_cuts",
    });
    const parent = backendInfo({ id: 10, parent: null });

    expect(displayAdoptionOrder([child, parent, root]).map((info) => info.id)).toEqual([
      3, 10, 5,
    ]);
  });

  it("adds once and updates the same panel for duplicate events", () => {
    const info = backendInfo();
    const first = upsertDisplay(undefined, info);
    expect(first.isNew).toBe(true);
    expect(first.panelId).toBe(`panel-${SCRIPTED_DISPLAY_ID_BASE}`);
    expect(first.display.id).toBe(SCRIPTED_DISPLAY_ID_BASE);

    const updated = upsertDisplay(first.display, {
      ...info,
      name: "renamed",
      parent: 7,
      state: { analysisId: 4, zoom: 2 },
    });
    expect(updated.isNew).toBe(false);
    expect(updated.panelId).toBe(first.panelId);
    expect(updated.display.id).toBe(first.display.id);
    expect(updated.display.name).toBe("renamed");
    expect(updated.display.parent).toBe(7);
    expect(updated.display.state.zoom).toBe(2);
  });

  it("merges registry state while preserving local path and settings", () => {
    const merged = mergeDisplayInfo(
      userDisplay,
      backendInfo({
        id: userDisplay.id,
        name: "renamed",
        state: {
          analysisId: 9,
          signalId: 3,
          sourceKind: "user",
          scripted: false,
          settings: { log_freq: true, max_hz: null, range_db: 80 },
          showGrid: false,
          zoom: 2,
        },
      }),
    );

    expect(merged.name).toBe("renamed");
    expect(merged.state.analysisId).toBe(9);
    expect(merged.state.signalId).toBe(3);
    expect(merged.state.zoom).toBe(2);
    expect(merged.state.path).toBe("/audio/tone.wav");
    expect(merged.state.analysisSettings).toEqual({ frame: 2048 });
    expect(merged.state.displaySettings).toEqual({
      log_freq: false,
      max_hz: 4000,
      range_db: 60,
    });
    expect(merged.state.lines).toEqual(userDisplay.state.lines);
    expect(merged.state.showGrid).toBe(true);
  });

  it("expands close events to the root and all children without duplicates", () => {
    expect(closedPanelIds(4, [5, 6, 5])).toEqual([
      "panel-4",
      "panel-5",
      "panel-6",
    ]);
  });

  it("removes local descendants when a remote parent closes without listing them", () => {
    const displays: ShellDisplay[] = [
      { ...userDisplay, id: SCRIPTED_DISPLAY_ID_BASE, parent: null },
      { ...userDisplay, id: 8, parent: SCRIPTED_DISPLAY_ID_BASE },
      { ...userDisplay, id: 9, parent: 8 },
      { ...userDisplay, id: 10, parent: null },
    ];
    expect(
      closedDisplayTreeIds(SCRIPTED_DISPLAY_ID_BASE, [], displays),
    ).toEqual([SCRIPTED_DISPLAY_ID_BASE, 8, 9]);
    expect(
      closedDisplayTreeIds(SCRIPTED_DISPLAY_ID_BASE, [7], displays),
    ).toEqual([SCRIPTED_DISPLAY_ID_BASE, 7, 8, 9]);
  });

  it("keeps the id counter below the reserved range", () => {
    expect(SCRIPTED_DISPLAY_ID_BASE).toBe(1_000_000);
    expect(nextUserDisplayId(4, [1, 2, 3])).toBe(4);
    expect(nextUserDisplayId(4, [1, 2, 3, 4])).toBe(5);
    expect(
      nextUserDisplayId(SCRIPTED_DISPLAY_ID_BASE, [1, 2, 3]),
    ).toBe(4);
    expect(sanitizeNextId(SCRIPTED_DISPLAY_ID_BASE + 10, [2, 3])).toBeLessThan(
      SCRIPTED_DISPLAY_ID_BASE,
    );
    expect(sanitizeNextId(1, [2, 3, SCRIPTED_DISPLAY_ID_BASE])).toBe(4);
  });

  it("excludes ephemeral displays from session saves", () => {
    const ephemeral: ShellDisplay = {
      id: 3,
      parent: null,
      kind: "spectrogram",
      name: "in-memory",
      state: { ephemeral: true, analysisId: 8 },
    };
    expect(sessionDisplays([userDisplay, ephemeral])).toEqual([userDisplay]);
  });

  it("selects the attach path for backend-owned and pathless analyses", () => {
    expect(isBackendOwnedDisplay({ id: SCRIPTED_DISPLAY_ID_BASE, state: {} })).toBe(
      true,
    );
    expect(
      isBackendOwnedDisplay({ id: 4, state: { sourceKind: "backend" } }),
    ).toBe(true);
    expect(isBackendOwnedDisplay({ id: 4, state: { scripted: true } })).toBe(true);
    expect(
      shouldAttachAnalysis({ id: 4, state: { analysisId: 9 } }),
    ).toBe(true);
    expect(
      shouldAttachAnalysis({ id: 4, state: { analysisId: 9, path: "/a.wav" } }),
    ).toBe(false);
    expect(shouldAttachAnalysis({ id: 4, state: { path: "/a.wav" } })).toBe(false);
  });

  it("drops unrestorable backend displays and recreates file-backed ones", () => {
    const backend = backendInfo({ id: 4, state: { sourceKind: "backend" } });
    const fileBacked = backendInfo({
      id: 5,
      state: { sourceKind: "backend", scripted: true, path: "/audio/a.wav" },
    });
    const reservedNoPath = backendInfo({ id: SCRIPTED_DISPLAY_ID_BASE });
    const launcher: ShellDisplay = {
      id: 1,
      parent: null,
      kind: "launcher",
      name: "Launcher",
      state: {},
    };

    expect(restoreDisposition(userDisplay)).toBe("keep");
    expect(restoreDisposition(launcher)).toBe("keep");
    expect(
      restoreDisposition({
        id: 3,
        kind: "spectrogram",
        state: { ephemeral: true, path: "/audio/a.wav" },
      }),
    ).toBe("drop");
    expect(restoreDisposition(backend)).toBe("drop");
    expect(restoreDisposition(fileBacked)).toBe("recreate");
    expect(restoreDisposition(reservedNoPath)).toBe("drop");
    expect(
      restoreDisposition({
        id: 2,
        kind: "spectrogram",
        state: { analysisId: 4 },
      }),
    ).toBe("drop");

    const recreated = recreatedUserDisplay(fileBacked, 3);
    expect(recreated.id).toBe(3);
    expect(recreated.parent).toBeNull();
    expect(recreated.state.path).toBe("/audio/a.wav");
    expect(recreated.state.sourceKind).toBe("user");
    expect(recreated.state.scripted).toBe(false);
    expect(recreated.state.analysisId).toBeUndefined();
  });

  it("drops local children of a dropped backend parent and keeps children of a recreated parent", () => {
    const droppedParent = backendInfo({
      id: SCRIPTED_DISPLAY_ID_BASE,
      state: { sourceKind: "backend", scripted: true },
    });
    const droppedChild: ShellDisplay = {
      id: 8,
      parent: droppedParent.id,
      kind: "order_cuts",
      name: "remote > Order Cuts",
      state: {},
    };
    expect(restoreDropIds([droppedParent, droppedChild])).toEqual(
      new Set([droppedParent.id, droppedChild.id]),
    );

    const recreatedParent = backendInfo({
      id: SCRIPTED_DISPLAY_ID_BASE + 1,
      state: { sourceKind: "backend", scripted: true, path: "/audio/a.wav" },
    });
    const keptChild: ShellDisplay = {
      id: 9,
      parent: recreatedParent.id,
      kind: "line_cuts",
      name: "a.wav > Line Cuts",
      state: { lines: [] },
    };
    expect(restoreDropIds([recreatedParent, keptChild])).toEqual(new Set());
  });
});

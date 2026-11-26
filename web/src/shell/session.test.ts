import { describe, expect, it } from "vitest";
import {
  SESSION_VERSION,
  createSession,
  displayFromPanel,
  emptySession,
  parseSession,
  type ShellDisplay,
} from "./session";

const launcher: ShellDisplay = {
  id: 1,
  parent: null,
  kind: "launcher",
  name: "Launcher",
  state: {},
};

describe("session document", () => {
  it("creates a versioned document with an opaque layout", () => {
    const session = createSession({ grid: { root: null } }, [launcher], 2);
    expect(session).toEqual({
      version: SESSION_VERSION,
      nextId: 2,
      displays: [launcher],
      recentFiles: [],
      layoutPresets: {},
      layout: { grid: { root: null } },
    });
    expect(JSON.parse(JSON.stringify(session))).toEqual(session);
  });

  it("starts empty and clamps nextId", () => {
    expect(emptySession()).toEqual({
      version: SESSION_VERSION,
      nextId: 1,
      displays: [],
      recentFiles: [],
      layoutPresets: {},
      layout: null,
    });
    expect(createSession(null, [], 0).nextId).toBe(1);
    expect(createSession(null, [], 3.9).nextId).toBe(3);
  });

  it("round-trips a saved document with recent files", () => {
    const saved = createSession(
      { panels: { "panel-1": {} } },
      [launcher],
      7,
      ["/audio/a.wav", "/audio/b.wav"],
    );
    expect(saved.recentFiles).toEqual(["/audio/a.wav", "/audio/b.wav"]);
    expect(parseSession(JSON.parse(JSON.stringify(saved)))).toEqual(saved);
  });

  it("rejects malformed documents", () => {
    expect(parseSession(null)).toBeNull();
    expect(parseSession({})).toBeNull();
    expect(
      parseSession({ version: 1, nextId: 1, displays: [{}], layout: null }),
    ).toBeNull();
    expect(
      parseSession({ version: "1", nextId: 1, displays: [], layout: null }),
    ).toBeNull();
  });

  it("normalizes a partial layout and missing nextId early", () => {
    const parsed = parseSession({
      version: 1,
      nextId: 0,
      displays: [launcher],
    });
    expect(parsed?.nextId).toBe(1);
    expect(parsed?.recentFiles).toEqual([]);
    expect(parsed?.layoutPresets).toEqual({});
    expect(parsed?.layout).toBeNull();
  });

  it("reads displays from Dockview panel params", () => {
    expect(displayFromPanel({ params: { display: launcher } }, 9)).toEqual(
      launcher,
    );
    const inferred = displayFromPanel({ title: "Spectrogram 2" }, 9);
    expect(inferred.id).toBe(9);
    expect(inferred.kind).toBe("spectrogram");
    expect(inferred.name).toBe("Spectrogram 2");
  });
});

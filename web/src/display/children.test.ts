import { describe, expect, it } from "vitest";
import { canSpawnChild, childLabel, makeChildDisplay } from "./children";
import type { ShellDisplay } from "../shell/session";

const parent: ShellDisplay = {
  id: 4,
  parent: null,
  kind: "spectrogram",
  name: "Spectrogram 4",
  state: {},
};

const scripting: ShellDisplay = {
  id: 8,
  parent: null,
  kind: "scripting",
  name: "Scripting",
  state: {},
};

describe("child displays", () => {
  it("only Spectrograms may spawn cuts", () => {
    expect(canSpawnChild("spectrogram", "order_cuts")).toBe(true);
    expect(canSpawnChild("spectrogram", "line_cuts")).toBe(true);
    expect(canSpawnChild("launcher", "order_cuts")).toBe(false);
    expect(canSpawnChild("order_cuts", "line_cuts")).toBe(false);
    expect(canSpawnChild("line_cuts", "order_cuts")).toBe(false);
  });

  it("only Scripting may spawn a console", () => {
    expect(canSpawnChild("scripting", "script_console")).toBe(true);
    expect(canSpawnChild("spectrogram", "script_console")).toBe(false);
    expect(canSpawnChild("launcher", "script_console")).toBe(false);
    expect(canSpawnChild("script_console", "script_console")).toBe(false);
    expect(canSpawnChild("scripting", "line_cuts")).toBe(false);
  });

  it("links Order/Line Cuts to the parent and names them parent > child", () => {
    const order = makeChildDisplay(parent, "order_cuts", 5);
    expect(order).toEqual({
      id: 5,
      parent: 4,
      kind: "order_cuts",
      name: "Spectrogram 4 > Order Cuts",
      state: {},
    });
    expect(childLabel("order_cuts")).toBe("Order Cuts");
    expect(childLabel("line_cuts")).toBe("Line Cuts");
    const line = makeChildDisplay(parent, "line_cuts", 6);
    expect(line.parent).toBe(parent.id);
    expect(line.kind).toBe("line_cuts");
  });

  it("links the console to the Scripting parent", () => {
    const consoleDisplay = makeChildDisplay(scripting, "script_console", 9);
    expect(consoleDisplay).toEqual({
      id: 9,
      parent: 8,
      kind: "script_console",
      name: "Scripting > Console",
      state: {},
    });
    expect(childLabel("script_console")).toBe("Console");
  });

  it("marks children of an ephemeral parent as ephemeral", () => {
    const child = makeChildDisplay(
      { ...parent, state: { ephemeral: true } },
      "line_cuts",
      6,
    );
    expect(child.state.ephemeral).toBe(true);
    const consoleDisplay = makeChildDisplay(
      { ...scripting, state: { ephemeral: true } },
      "script_console",
      9,
    );
    expect(consoleDisplay.state.ephemeral).toBe(true);
  });
});

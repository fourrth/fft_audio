import { describe, expect, expectTypeOf, it } from "vitest";
import { MockTransport } from "./mock";
import type { Command, EventMap, Transport } from "./types";

describe("MockTransport", () => {
  it("invokes a typed command and returns its typed result", async () => {
    const transport = new MockTransport({
      "app.info": () => ({
        name: "fft_audio",
        version: "0.1.0",
        mode: "headless",
        capabilities: {
          displays: true,
          scripting: null,
          tableFormats: ["csv", "json", "jsonl"],
        },
      }),
    });
    const command: Command<"app.info"> = { name: "app.info", args: {} };

    const result = await transport.invoke(command);

    expectTypeOf(result.mode).toEqualTypeOf<
      "desktop" | "headless" | "browser"
    >();
    expect(result).toEqual({
      name: "fft_audio",
      version: "0.1.0",
      mode: "headless",
      capabilities: {
        displays: true,
        scripting: null,
        tableFormats: ["csv", "json", "jsonl"],
      },
    });
    expect(transport.mode).toBe("mock");
  });

  it("records every invocation in call history", async () => {
    const transport = new MockTransport();
    transport.setHandler("app.ping", ({ nonce }) => ({
      nonce: nonce ?? null,
      at: 1_700_000_000_000,
    }));

    await transport.invoke({ name: "app.ping", args: { nonce: "abc" } });
    await transport.invoke({ name: "app.ping", args: {} });

    expect(transport.calls).toEqual([
      { name: "app.ping", args: { nonce: "abc" } },
      { name: "app.ping", args: {} },
    ]);
  });

  it("rejects with a clear error when no handler is registered", async () => {
    const transport = new MockTransport();

    await expect(
      transport.invoke({ name: "app.info", args: {} }),
    ).rejects.toThrow(
      'MockTransport: no handler registered for command "app.info"',
    );
    expect(transport.calls).toHaveLength(1);
  });

  it("awaits async handlers", async () => {
    const transport = new MockTransport({
      "spectrogram.open": async ({ path }) => {
        await Promise.resolve();
        return { displayId: path.length, analysisId: 11 };
      },
    });

    await expect(
      transport.invoke({
        name: "spectrogram.open",
        args: { displayId: 7, path: "a.wav" },
      }),
    ).resolves.toEqual({ displayId: 5, analysisId: 11 });
  });

  it("delivers event payloads and supports unsubscribe", () => {
    const transport = new MockTransport();
    const contract: Transport = transport;
    const notifications: EventMap["notification"][] = [];
    const otherListener = (payload: EventMap["notification"]) => {
      notifications.push(payload);
    };
    const unsubscribe = contract.on("notification", otherListener);

    transport.emit("notification", { level: "success", message: "opened" });
    unsubscribe();
    transport.emit("notification", { level: "error", message: "ignored" });

    expect(notifications).toEqual([
      { level: "success", message: "opened" },
    ]);
  });

  it("delivers a progress to done event sequence", async () => {
    const transport = new MockTransport({
      "spectrogram.open": () => ({ displayId: 42 }),
    });
    const events: string[] = [];

    transport.on("analysis.progress", (payload) => {
      events.push(
        `progress:${payload.displayId}:${payload.framesDone}/${payload.framesTotal}`,
      );
    });
    transport.on("analysis.done", (payload) => {
      events.push(`done:${payload.displayId}`);
    });

    const { displayId } = await transport.invoke({
      name: "spectrogram.open",
      args: { displayId: 42, path: "clip.wav" },
    });
    transport.emit("analysis.progress", {
      displayId,
      framesDone: 0,
      framesTotal: 100,
      etaSeconds: 2,
    });
    transport.emit("analysis.progress", {
      displayId,
      framesDone: 100,
      framesTotal: 100,
      etaSeconds: 0,
    });
    transport.emit("analysis.done", { displayId });

    expect(events).toEqual([
      "progress:42:0/100",
      "progress:42:100/100",
      "done:42",
    ]);
  });
});

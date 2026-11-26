// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { createHttpTransport } from "./http";

describe("headless file picker", () => {
  afterEach(() => vi.restoreAllMocks());

  it("resolves a dismissed picker to null instead of leaving it pending", async () => {
    const click = vi
      .spyOn(HTMLInputElement.prototype, "click")
      .mockImplementation(function (this: HTMLInputElement) {
        queueMicrotask(() => {
          this.dispatchEvent(new Event("cancel"));
        });
      });
    const transport = createHttpTransport({
      baseUrl: "http://127.0.0.1:9",
      fetchImpl: vi.fn() as unknown as typeof fetch,
    });

    await expect(
      transport.invoke({ name: "dialog.openAudio", args: {} }),
    ).resolves.toBeNull();
    expect(click).toHaveBeenCalledTimes(1);
    transport.close?.();
  });
});

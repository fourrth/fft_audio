import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test, type APIRequestContext, type Page } from "@playwright/test";
import { defaultSettings } from "../../src/settings/model";
import { emptySession } from "../../src/shell/session";

const fixture = fileURLToPath(
  new URL("../fixtures/tone.wav", import.meta.url),
);
const seriesFixture = fileURLToPath(
  new URL("../fixtures/series.csv", import.meta.url),
);
const pairsFixture = fileURLToPath(
  new URL("../fixtures/pairs.csv", import.meta.url),
);

async function resetServer(request: APIRequestContext) {
  // The startup registry sync adopts anything left in the backend, so each
  // test must begin with an empty display registry.
  const displays = await request.get("/api/displays");
  if (displays.ok()) {
    for (const display of (await displays.json()) as Array<{ id: number }>) {
      await request.post("/api/displays/close", { data: { id: display.id } });
    }
  }

  const settings = await request.put("/api/settings", {
    data: defaultSettings(),
  });
  expect(settings.ok()).toBeTruthy();
  const session = await request.put("/api/session", { data: emptySession() });
  expect(session.ok()).toBeTruthy();
}

function trackErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      errors.push(message.text());
    }
  });
  page.on("pageerror", (error) => errors.push(error.message));
  return errors;
}

async function openLauncher(page: Page) {
  await page.goto("/");
  await expect(page.getByTestId("launcher")).toBeVisible();
}


async function addChildTab(page: Page, name: "Order Cuts" | "Line Cuts") {
  await page.getByRole("menuitem", { name: "Cuts" }).click();
  await page
    .getByRole("menu", { name: "Cuts" })
    .getByRole("menuitem", { name })
    .click();
}

async function uploadTone(page: Page) {
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Browse…" }).click();
  const fileChooser = await chooser;
  await fileChooser.setFiles(fixture);
  await expect(page.getByTestId("spectrogram-plot")).toBeVisible({
    timeout: 30_000,
  });
}

async function openPlotFromFileMenu(
  page: Page,
  file: string,
  item: "New Plot…" | "New Plot (x/y pairs)…",
) {
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("menuitem", { name: "File" }).click();
  await page
    .getByRole("menu", { name: "File" })
    .getByRole("menuitem", { name: item, exact: true })
    .click();
  await (await chooser).setFiles(file);
}

async function addLineHere(page: Page) {
  const canvas = page.locator(".plot-canvas");
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();
  if (!box) {
    return;
  }
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, {
    button: "right",
  });
  const add = page.getByRole("menuitem", { name: "Add line here" });
  await expect(add).toBeEnabled();
  await add.click();
}

interface StoredLine {
  t0: number;
  t1: number;
  f0: number;
  f1: number;
}

async function firstLineCutLine(
  request: APIRequestContext,
): Promise<StoredLine | null> {
  const session = await request.get("/api/session");
  const saved = (await session.json()) as {
    displays?: Array<{ kind?: string; state?: { lines?: StoredLine[] } }>;
  };
  return (
    saved.displays?.find((display) => display.kind === "line_cuts")?.state
      ?.lines?.[0] ?? null
  );
}

function lineSignature(line: StoredLine): string {
  return `${line.t0}:${line.t1}:${line.f0}:${line.f1}`;
}

async function cutBounds(page: Page): Promise<{
  xMin: number;
  xMax: number;
  yMin: number;
  yMax: number;
}> {
  return page.getByTestId("cut-plot").first().evaluate((element) => ({
    xMin: Number(element.getAttribute("data-x-min")),
    xMax: Number(element.getAttribute("data-x-max")),
    yMin: Number(element.getAttribute("data-y-min")),
    yMax: Number(element.getAttribute("data-y-max")),
  }));
}

async function cutRegionBackground(
  page: Page,
): Promise<{ x: number; y: number; width: number; height: number } | null> {
  return page.evaluate(() => {
    const svg = document.querySelector(".cut-svg");
    if (!svg) {
      return null;
    }
    const rect = svg.getBoundingClientRect();
    for (let fy = 0.15; fy <= 0.65; fy += 0.05) {
      for (let fx = 0.12; fx <= 0.65; fx += 0.05) {
        const x = rect.left + rect.width * fx;
        const y = rect.top + rect.height * fy;
        const element = document.elementFromPoint(x, y);
        if (
          element instanceof SVGElement &&
          element.classList.contains("cut-region-capture")
        ) {
          return { x, y, width: rect.width, height: rect.height };
        }
      }
    }
    return null;
  });
}

async function cutRegionBox(
  page: Page,
): Promise<{ x: number; y: number; width: number; height: number }> {
  return page.getByTestId("cut-region-capture").evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return {
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
    };
  });
}

/** Bounds a drag between two client points should zoom the plot to. */
function expectedCutBounds(
  before: { xMin: number; xMax: number; yMin: number; yMax: number },
  box: { x: number; y: number; width: number; height: number },
  from: { x: number; y: number },
  to: { x: number; y: number },
) {
  const xSpan = before.xMax - before.xMin;
  const ySpan = before.yMax - before.yMin;
  const fx = (clientX: number) => (clientX - box.x) / box.width;
  const fy = (clientY: number) => (clientY - box.y) / box.height;
  return {
    xMin: before.xMin + fx(Math.min(from.x, to.x)) * xSpan,
    xMax: before.xMin + fx(Math.max(from.x, to.x)) * xSpan,
    yMin: before.yMax - fy(Math.max(from.y, to.y)) * ySpan,
    yMax: before.yMax - fy(Math.min(from.y, to.y)) * ySpan,
  };
}

function minimapRange(text: string | null): {
  start: number;
  end: number;
  span: number;
} {
  const numbers = text?.match(/-?[0-9]+(?:\.[0-9]+)?/g);
  if (!numbers || numbers.length < 2) {
    throw new Error(`could not parse minimap label: ${text}`);
  }
  const start = Number(numbers[0]);
  const end = Number(numbers[1]);
  return { start, end, span: end - start };
}

test.beforeEach(async ({ request }) => {
  await resetServer(request);
});

test("launcher root rules, themes, contrast and reduced motion", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);

  await expect(page.getByRole("button", { name: /Spectrogram/ })).toBeVisible();
  // The headless WebUI has no scripting UI capability, so the tile is absent.
  await expect(page.getByRole("button", { name: /Scripting/ })).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /Order Cuts/ }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /Line Cuts/ }),
  ).toHaveCount(0);
  await expect(page.getByText("Soon")).toHaveCount(0);

  const dark = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue("--bg-canvas")
      .trim(),
  );
  expect(dark).toBe("#0f1217");
  const defaultAnim = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue("--anim-ms")
      .trim(),
  );
  expect(parseFloat(defaultAnim)).toBeLessThanOrEqual(150);

  await page.keyboard.press("Control+,");
  await expect(page.getByTestId("settings")).toBeVisible();
  await page.getByRole("button", { name: /Appearance/ }).click();
  await page.getByRole("radio", { name: /Light/ }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  const light = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue("--bg-canvas")
      .trim(),
  );
  expect(light).toBe("#eef2f6");

  await page.getByRole("switch", { name: "High contrast" }).click();
  await expect(page.locator("html")).toHaveAttribute(
    "data-high-contrast",
    "true",
  );
  const border = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue("--border-subtle")
      .trim(),
  );
  expect(border).toBe("#475569");

  await page.getByRole("switch", { name: "Reduce motion" }).click();
  await expect(page.locator("html")).toHaveAttribute(
    "data-reduce-motion",
    "true",
  );
  const anim = await page.evaluate(() =>
    getComputedStyle(document.documentElement)
      .getPropertyValue("--anim-ms")
      .trim(),
  );
  expect(["0ms", "0s"]).toContain(anim);

  const scale = page.getByRole("slider", { name: "UI scale" });
  await scale.focus();
  await scale.press("End");
  await expect
    .poll(() =>
      page.evaluate(() =>
        getComputedStyle(document.documentElement)
          .getPropertyValue("--ui-scale")
          .trim(),
      ),
    )
    .toBe("2");

  await page.screenshot({ path: "test-results/settings-appearance.png" });
  expect(errors, errors.join("\n")).toEqual([]);
});

test("spectrogram, pointer read-out, inspector, cuts and session restore", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  await expect(page.getByTestId("colorbar")).toBeVisible();
  await expect(page.getByTestId("minimap")).toBeVisible();
  const canvas = page.locator(".plot-canvas");
  await canvas.hover({ position: { x: 220, y: 160 } });
  await expect(page.getByTestId("readout")).toBeVisible();
  await expect(page.getByTestId("readout")).toContainText("Hz");

  const maxHandle = page.getByRole("slider", { name: "Maximum dB" });
  const before = await maxHandle.getAttribute("aria-valuenow");
  await maxHandle.focus();
  await page.keyboard.press("ArrowDown");
  await expect
    .poll(() => maxHandle.getAttribute("aria-valuenow"))
    .not.toBe(before);

  const afterKeyboard = await maxHandle.getAttribute("aria-valuenow");
  const box = await maxHandle.boundingBox();
  expect(box).not.toBeNull();
  if (box) {
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2, box.y + 40, { steps: 5 });
    await page.mouse.up();
    await expect
      .poll(() => maxHandle.getAttribute("aria-valuenow"))
      .not.toBe(afterKeyboard);
  }

  await page.getByRole("button", { name: "Toggle inspector" }).click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  const frame = page.getByRole("spinbutton", { name: "Frame" });
  await frame.fill("1024");
  await expect(page.locator(".inspector-footer")).toContainText(
    "Unapplied DSP edits",
  );
  await page.getByRole("button", { name: "Revert" }).click();
  await expect(page.locator(".inspector-footer")).toHaveCount(0);
  await frame.fill("1024");
  await page.getByRole("button", { name: "Apply & re-analyse" }).click();
  await expect(page.locator(".inspector-footer")).toHaveCount(0);

  await page
    .locator(".fft-tab")
    .filter({ hasText: "tone" })
    .filter({ hasNotText: "Inspector" })
    .filter({ hasNotText: ">" })
    .first()
    .click();
  await addChildTab(page, "Order Cuts");
  const orderPlot = page.locator('.cut-plot[data-cut-kind="order"]').first();
  await expect(orderPlot).toBeVisible({ timeout: 30_000 });
  await orderPlot.locator(".cut-line").first().dispatchEvent("click");
  await expect(
    page.getByRole("toolbar", { name: "Selected cut" }),
  ).toBeVisible();

  await page
    .locator(".fft-tab")
    .filter({ hasText: "tone" })
    .filter({ hasNotText: "Inspector" })
    .filter({ hasNotText: ">" })
    .first()
    .click();
  await addLineHere(page);
  const linePlot = page.locator('.cut-plot[data-cut-kind="line"]').first();
  await expect(linePlot).toBeVisible({ timeout: 30_000 });
  await expect(linePlot.locator(".cut-line").first()).toBeVisible({
    timeout: 30_000,
  });
  await linePlot.locator(".cut-line").first().dispatchEvent("click");
  await expect(
    page.getByRole("toolbar", { name: "Selected line" }),
  ).toBeVisible();

  const groups = await page.locator(".dv-groupview").count();
  expect(groups).toBeGreaterThan(1);

  await page.waitForTimeout(700);
  const session = await request.get("/api/session");
  const saved = (await session.json()) as { displays: unknown[] };
  expect(saved.displays.length).toBeGreaterThanOrEqual(3);

  await page.reload();
  await expect(page.getByTestId("spectrogram-plot")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.locator(".fft-tab")).toHaveCount(4);
  // `display.open` adopts the backend analysis name (a file stem).
  await expect(page.locator(".fft-tab", { hasText: "tone" })).toHaveCount(4);
  await expect(
    page.locator(".fft-tab", { hasText: "Order Cuts" }),
  ).toHaveCount(1);
  await expect(
    page.locator(".fft-tab", { hasText: "Line Cuts" }),
  ).toHaveCount(1);
  await page.screenshot({ path: "test-results/spectrogram-cuts.png" });
  expect(errors, errors.join("\n")).toEqual([]);
});

test("adopts backend displays and closes them without duplicate panels", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.keyboard.press("Control+t");
  await uploadTone(page);
  // New Spectrogram replaces the active Launcher tab.
  await expect(
    page.locator(".fft-tab", { hasText: "Launcher" }),
  ).toHaveCount(0);

  const open = await request.post("/api/spectrogram/open", {
    data: { displayId: 700, path: fixture },
  });
  expect(open.ok()).toBeTruthy();
  const opened = (await open.json()) as {
    displayId: number;
    analysisId: number;
  };
  expect(opened.displayId).toBe(700);
  expect(typeof opened.analysisId).toBe("number");
  await expect
    .poll(async () => (await request.get("/api/spectrogram/700")).ok(), {
      timeout: 30_000,
    })
    .toBe(true);

  const created = await request.post("/api/displays/spectrogram", {
    data: { analysisId: opened.analysisId },
  });
  expect(created.ok()).toBeTruthy();
  const backend = (await created.json()) as { id: number; name: string };
  expect(backend.id).toBeGreaterThanOrEqual(1_000_000);

  // Both registry displays are adopted exactly once.
  await expect(page.locator(".fft-tab")).toHaveCount(3);
  await expect(
    page.locator(".fft-tab", { hasText: "Launcher" }),
  ).toHaveCount(0);
  // The two new tabs were appended in event order: user id 700, then backend.
  await page.locator(".fft-tab").nth(1).click();
  await expect(page.locator('[data-display-id="700"]')).toHaveCount(1);
  await page.locator(".fft-tab").nth(2).click();
  await expect(
    page.locator(`[data-display-id="${backend.id}"]`),
  ).toHaveCount(1);
  await expect(
    page.locator(
      `[data-display-id="${backend.id}"] .display-state.state-ready`,
    ),
  ).toBeVisible({ timeout: 30_000 });

  await page.keyboard.press("Control+w");
  await expect(
    page.locator(`[data-display-id="${backend.id}"]`),
  ).toHaveCount(0);
  await expect(
    page.locator(".fft-tab", { hasText: "Launcher" }),
  ).toHaveCount(0);
  await expect(page.locator(".fft-tab")).toHaveCount(2);

  await expect
    .poll(async () => {
      const response = await request.get("/api/displays");
      const displays = (await response.json()) as Array<{ id: number }>;
      return displays.some((display) => display.id === backend.id);
    })
    .toBe(false);

  // Attaching fetches the payload before a shared analysis is materialized;
  // those 404s are expected until the `analysis.done` refresh.
  const unexpected = errors.filter(
    (error) =>
      !error.includes(
        "Failed to load resource: the server responded with a status of 404",
      ),
  );
  expect(unexpected, unexpected.join("\n")).toEqual([]);
});

test("adopts backend displays created before the UI connects", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);

  const open = await request.post("/api/spectrogram/open", {
    data: { displayId: 703, path: fixture },
  });
  expect(open.ok()).toBeTruthy();
  const opened = (await open.json()) as { analysisId: number };
  await expect
    .poll(async () => (await request.get("/api/spectrogram/703")).ok(), {
      timeout: 30_000,
    })
    .toBe(true);

  const created = await request.post("/api/displays/spectrogram", {
    data: { analysisId: opened.analysisId },
  });
  expect(created.ok()).toBeTruthy();
  const backend = (await created.json()) as { id: number };
  expect(backend.id).toBeGreaterThanOrEqual(1_000_000);

  // Leave only the backend record; its display.open was emitted before this
  // window existed, so startup must reconcile GET /api/displays.
  await request.post("/api/displays/close", { data: { id: 703 } });

  await page.goto("/");
  await expect(
    page.locator(`[data-display-id="${backend.id}"]`),
  ).toHaveCount(1);
  await page.locator(".fft-tab").last().click();
  await expect(
    page.locator(
      `[data-display-id="${backend.id}"] .display-state.state-ready`,
    ),
  ).toBeVisible({ timeout: 30_000 });

  // A reload must adopt the same registry record once, not duplicate it.
  await page.reload();
  await expect(
    page.locator(`[data-display-id="${backend.id}"]`),
  ).toHaveCount(1);

  const unexpected = errors.filter(
    (error) =>
      !error.includes(
        "Failed to load resource: the server responded with a status of 404",
      ),
  );
  expect(unexpected, unexpected.join("\n")).toEqual([]);
});

test("remote close removes local child cuts under a backend display", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);

  const open = await request.post("/api/spectrogram/open", {
    data: { displayId: 701, path: fixture },
  });
  expect(open.ok()).toBeTruthy();
  const opened = (await open.json()) as { analysisId: number };
  await expect
    .poll(async () => (await request.get("/api/spectrogram/701")).ok(), {
      timeout: 30_000,
    })
    .toBe(true);

  const created = await request.post("/api/displays/spectrogram", {
    data: { analysisId: opened.analysisId },
  });
  expect(created.ok()).toBeTruthy();
  const backend = (await created.json()) as { id: number };
  await expect(page.locator(`[data-display-id="${backend.id}"]`)).toHaveCount(1);

  // Spawn a purely local child panel under the backend-owned display. The
  // backend registry never sees this child, so its close event cannot list it.
  await page.locator(".fft-tab").last().click();
  await addChildTab(page, "Order Cuts");
  await expect(page.getByTestId("cut-plot").first()).toBeVisible({
    timeout: 30_000,
  });
  await expect(
    page.locator(".fft-tab", { hasText: "Order Cuts" }),
  ).toHaveCount(1);

  const closed = await request.post("/api/displays/close", {
    data: { id: backend.id },
  });
  expect(closed.ok()).toBeTruthy();

  await expect(page.locator(`[data-display-id="${backend.id}"]`)).toHaveCount(0);
  await expect(page.locator(".fft-tab", { hasText: "Order Cuts" })).toHaveCount(
    0,
  );
  await expect(page.locator('[data-display-id="701"]')).toHaveCount(1);
  await expect(
    page.locator(".fft-tab", { hasText: "Launcher" }),
  ).toHaveCount(1);

  const unexpected = errors.filter(
    (error) =>
      !error.includes(
        "Failed to load resource: the server responded with a status of 404",
      ),
  );
  expect(unexpected, unexpected.join("\n")).toEqual([]);
});

test("closing child cuts redraws the expanded spectrogram", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  // First child.
  await addChildTab(page, "Order Cuts");
  await expect(page.getByTestId("cut-plot").first()).toBeVisible({
    timeout: 30_000,
  });

  // Re-activate the Spectrogram and spawn a second Order Cuts child.
  await page.locator(".fft-tab").first().click();
  await addChildTab(page, "Order Cuts");
  await expect(
    page.locator(".fft-tab", { hasText: "Order Cuts" }),
  ).toHaveCount(2);

  for (let index = 0; index < 2; index += 1) {
    await page
      .locator(".fft-tab", { hasText: "Order Cuts" })
      .first()
      .locator(".fft-tab-close")
      .click();
  }
  await expect(page.locator(".fft-tab", { hasText: "Order Cuts" })).toHaveCount(
    0,
  );

  const plot = page.getByTestId("spectrogram-plot");
  const canvas = page.locator(".plot-canvas");
  await expect(plot).toBeVisible();
  await expect(canvas).toBeVisible();

  await page.waitForTimeout(250);
  await expect
    .poll(
      () =>
        canvas.evaluate((element) => {
          const target = element as HTMLCanvasElement;
          const context = target.getContext("2d");
          if (!context || target.width < 2 || target.height < 2) {
            return 0;
          }
          const data = context.getImageData(
            0,
            0,
            target.width,
            target.height,
          ).data;
          let total = 0;
          let count = 0;
          for (let y = 0; y < target.height; y += 4) {
            for (let x = 0; x < target.width; x += 4) {
              const index = (y * target.width + x) * 4;
              total += data[index] + data[index + 1] + data[index + 2];
              count += 1;
            }
          }
          return count > 0 ? total / count : 0;
        }),
      { timeout: 10_000 },
    )
    .toBeGreaterThan(8);

  await page.screenshot({ path: "test-results/spectrogram-after-close.png" });
  expect(errors, errors.join("\n")).toEqual([]);
});

test("inspector color floor and ceiling update the color bar", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await page.getByRole("button", { name: "Toggle inspector" }).click();
  await expect(page.getByTestId("inspector")).toBeVisible();

  await page.getByRole("spinbutton", { name: "color floor" }).fill("-45");
  await expect(page.getByTestId("colorbar-min-value")).toHaveText("-45");

  await page.getByRole("spinbutton", { name: "color ceiling" }).fill("-15");
  await expect(page.getByTestId("colorbar-max-value")).toHaveText("-15");
  await expect(page.getByTestId("colorbar-min-value")).toHaveText("-45");

  expect(errors, errors.join("\n")).toEqual([]);
});

test("empty spectrogram shows five recents and the path field without scrolling", async ({
  page,
  request,
}) => {
  const recentFiles = Array.from(
    { length: 6 },
    (_, index) => `/audio/recent-${index + 1}.wav`,
  );
  await request.put("/api/session", {
    data: { ...emptySession(), recentFiles },
  });

  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-empty")).toBeVisible();

  const recent = page.getByTestId("display-recent");
  await expect(recent).toBeVisible();
  await expect(recent.getByRole("button")).toHaveCount(5);
  await expect(recent.getByText("recent-6.wav")).toHaveCount(0);

  const metrics = await page.getByTestId("display-empty").evaluate((element) => ({
    scrollHeight: element.scrollHeight,
    clientHeight: element.clientHeight,
  }));
  expect(metrics.scrollHeight).toBeLessThanOrEqual(metrics.clientHeight + 1);

  await page.getByRole("button", { name: "Enter a path" }).click();
  await expect(page.getByTestId("display-manual")).toBeInViewport();
  await expect(
    page.getByTestId("display-manual").getByRole("textbox", { name: "File path" }),
  ).toBeVisible();
  await expect(page.getByText(/Wheel pan/)).toHaveCount(0);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("plot hints appear only after a spectrogram is loaded", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-empty")).toBeVisible();
  await expect(page.getByText(/Wheel pan/)).toHaveCount(0);

  await uploadTone(page);
  await expect(page.getByText(/Wheel pan/)).toBeVisible();

  expect(errors, errors.join("\n")).toEqual([]);
});

test("spectrogram time axis spans the plot column", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const timeAxis = page.getByTestId("time-axis");
  const plot = page.getByTestId("spectrogram-plot");
  await expect(timeAxis).toBeVisible();
  await expect(timeAxis.locator(".axis-tick-time").first()).toBeVisible();
  const axisBox = await timeAxis.boundingBox();
  const plotBox = await plot.boundingBox();
  expect(axisBox).not.toBeNull();
  expect(plotBox).not.toBeNull();
  if (axisBox && plotBox) {
    expect(axisBox.width).toBeGreaterThan(plotBox.width * 0.8);
    expect(Math.abs(axisBox.x - plotBox.x)).toBeLessThan(2);
    expect(axisBox.y).toBeGreaterThanOrEqual(
      plotBox.y + plotBox.height - 2,
    );
  }

  expect(errors, errors.join("\n")).toEqual([]);
});

test("spectrogram wheel pans, modifier zooms, and drag pans", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const label = page.locator(".minimap-label");
  const scales = page.getByTestId("spectrogram-scales");
  const initial = minimapRange(await label.textContent());
  expect(initial.span).toBeGreaterThan(0);

  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect
    .poll(async () => minimapRange(await label.textContent()).span)
    .toBeLessThan(initial.span * 0.95);
  await page.getByRole("button", { name: "Fit view" }).click();
  await expect
    .poll(async () =>
      Math.abs(minimapRange(await label.textContent()).span - initial.span),
    )
    .toBeLessThan(0.05);

  const canvas = page.locator(".plot-canvas");
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();
  if (!box) {
    return;
  }
  const centerX = box.x + box.width * 0.5;
  const centerY = box.y + box.height * 0.5;

  // Fit pins the view to the frequency bounds; zoom in before panning.
  await page.getByRole("button", { name: "Zoom in" }).click();
  const zoomedForPan = minimapRange(await label.textContent());
  const beforePan = await scales.textContent();
  await page.mouse.move(centerX, centerY);
  await page.mouse.wheel(0, 180);
  await expect
    .poll(async () => (await scales.textContent()) !== beforePan)
    .toBe(true);
  const panned = minimapRange(await label.textContent());
  expect(Math.abs(panned.span - zoomedForPan.span)).toBeLessThan(0.05);

  await page.keyboard.down("Shift");
  await page.mouse.wheel(0, 240);
  await page.keyboard.up("Shift");
  await expect
    .poll(async () =>
      Math.abs(minimapRange(await label.textContent()).start - panned.start),
    )
    .toBeGreaterThan(0.01);

  await page.getByRole("button", { name: "Fit view" }).click();
  const beforeZoom = minimapRange(await label.textContent());
  await page.mouse.move(centerX, centerY);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -400);
  await page.keyboard.up("Control");
  await expect
    .poll(async () => minimapRange(await label.textContent()).span)
    .toBeLessThan(beforeZoom.span * 0.9);

  const zoomed = minimapRange(await label.textContent());
  await page.mouse.move(centerX, centerY);
  await page.keyboard.down("Shift");
  await page.mouse.down();
  await page.mouse.move(centerX + Math.min(120, box.width * 0.2), centerY, {
    steps: 8,
  });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  await expect
    .poll(async () =>
      Math.abs(minimapRange(await label.textContent()).start - zoomed.start),
    )
    .toBeGreaterThan(0.01);

  await page.getByRole("button", { name: "Fit view" }).click();
  await expect
    .poll(async () => {
      const current = minimapRange(await label.textContent());
      return (
        Math.abs(current.start - initial.start) < 0.02 &&
        Math.abs(current.end - initial.end) < 0.02
      );
    })
    .toBe(true);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("spectrogram region drag zooms into the selected rectangle", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const label = page.locator(".minimap-label");
  const initial = minimapRange(await label.textContent());
  const canvas = page.locator(".plot-canvas");
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();
  if (!box) {
    return;
  }

  const startX = box.x + box.width * 0.3;
  const startY = box.y + box.height * 0.3;
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.mouse.move(
    box.x + box.width * 0.7,
    box.y + box.height * 0.7,
    { steps: 10 },
  );
  await expect(page.getByTestId("spectrogram-region")).toBeVisible();
  await page.mouse.up();
  await expect(page.getByTestId("spectrogram-region")).toHaveCount(0);

  await expect
    .poll(async () => minimapRange(await label.textContent()).span)
    .toBeLessThan(initial.span * 0.65);
  const selected = minimapRange(await label.textContent());
  expect(selected.start).toBeGreaterThan(
    initial.start + initial.span * 0.2,
  );
  expect(selected.end).toBeLessThan(initial.start + initial.span * 0.8);

  await page.getByRole("button", { name: "Fit view" }).click();
  await expect
    .poll(async () =>
      Math.abs(minimapRange(await label.textContent()).span - initial.span),
    )
    .toBeLessThan(0.01);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("order cuts region drag zooms and reset restores bounds", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await addChildTab(page, "Order Cuts");
  await expect(page.getByTestId("order-tracking")).toBeVisible({
    timeout: 30_000,
  });

  const before = await cutBounds(page);
  const start = await cutRegionBackground(page);
  expect(start).not.toBeNull();
  if (!start) {
    return;
  }
  await page.mouse.move(start.x, start.y);
  await page.mouse.down();
  await page.mouse.move(
    start.x + start.width * 0.22,
    start.y + start.height * 0.25,
    { steps: 8 },
  );
  await expect(page.getByTestId("cut-region")).toBeVisible();
  await page.mouse.up();
  await expect(page.getByTestId("cut-region")).toHaveCount(0);

  const zoomed = await cutBounds(page);
  expect(zoomed.xMax - zoomed.xMin).toBeLessThan(
    (before.xMax - before.xMin) * 0.9,
  );
  expect(zoomed.yMax - zoomed.yMin).toBeLessThan(
    (before.yMax - before.yMin) * 0.9,
  );
  const reset = page.getByRole("button", { name: "Reset cut view" });
  await expect(reset).toBeVisible();
  await reset.click();
  await expect
    .poll(async () => {
      const current = await cutBounds(page);
      return (
        Math.abs(current.xMin - before.xMin) +
        Math.abs(current.xMax - before.xMax) +
        Math.abs(current.yMin - before.yMin) +
        Math.abs(current.yMax - before.yMax)
      );
    })
    .toBeLessThan(1e-6);
  await expect(reset).toHaveCount(0);

  await page.locator(".cut-line").first().dispatchEvent("click");
  await expect(
    page.getByRole("toolbar", { name: "Selected cut" }),
  ).toBeVisible();
  expect(errors, errors.join("\n")).toEqual([]);
});

test("line cuts region drag zooms and resets", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await addChildTab(page, "Line Cuts");
  await expect(page.locator(".cut-svg")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("cut-region-capture")).toHaveCount(1);

  // The drawing uses the measured panel box, so it fills the window instead of
  // letterboxing a fixed-size viewBox; curves are clipped to the axes area.
  const plotLayout = await page.locator(".cut-svg").evaluate((element) => {
    const svg = element as SVGSVGElement;
    const rect = svg.getBoundingClientRect();
    const clipId = svg.querySelector("clipPath")?.getAttribute("id") ?? "";
    return {
      scaleX: rect.width / svg.viewBox.baseVal.width,
      scaleY: rect.height / svg.viewBox.baseVal.height,
      clipped:
        clipId.length > 0 &&
        Boolean(svg.querySelector(`g[clip-path="url(#${clipId})"]`)),
    };
  });
  expect(plotLayout.scaleX).toBeCloseTo(1, 1);
  expect(plotLayout.scaleY).toBeCloseTo(1, 1);
  expect(plotLayout.clipped).toBe(true);

  const before = await cutBounds(page);
  const box = await cutRegionBox(page);
  const start = await cutRegionBackground(page);
  expect(start).not.toBeNull();
  if (!start) {
    return;
  }
  const end = {
    x: Math.min(box.x + box.width - 3, start.x + box.width * 0.25),
    y: Math.min(box.y + box.height - 3, start.y + box.height * 0.3),
  };
  const dragCutRegion = async () => {
    await page.mouse.move(start.x, start.y);
    await page.mouse.down();
    await page.mouse.move(end.x, end.y, { steps: 8 });
    await expect(page.getByTestId("cut-region")).toBeVisible();
    await page.mouse.up();
    await expect(page.getByTestId("cut-region")).toHaveCount(0);
  };
  const waitForAutoBounds = () =>
    expect
      .poll(async () => {
        const current = await cutBounds(page);
        return (
          Math.abs(current.xMin - before.xMin) +
          Math.abs(current.xMax - before.xMax) +
          Math.abs(current.yMin - before.yMin) +
          Math.abs(current.yMax - before.yMax)
        );
      })
      .toBeLessThan(1e-6);

  await dragCutRegion();
  const zoomed = await cutBounds(page);
  expect(zoomed.xMax - zoomed.xMin).toBeLessThan(
    (before.xMax - before.xMin) * 0.9,
  );
  // The zoom follows the cursor: the new view is the dragged box, even when the
  // SVG letterboxes its fixed-aspect viewBox inside a differently shaped panel.
  const expected = expectedCutBounds(before, box, start, end);
  const xTolerance = (before.xMax - before.xMin) * 0.02;
  const yTolerance = (before.yMax - before.yMin) * 0.02;
  expect(Math.abs(zoomed.xMin - expected.xMin)).toBeLessThan(xTolerance);
  expect(Math.abs(zoomed.xMax - expected.xMax)).toBeLessThan(xTolerance);
  expect(Math.abs(zoomed.yMin - expected.yMin)).toBeLessThan(yTolerance);
  expect(Math.abs(zoomed.yMax - expected.yMax)).toBeLessThan(yTolerance);

  const reset = page.getByRole("button", { name: "Reset cut view" });
  await expect(reset).toBeVisible();
  await reset.click();
  await waitForAutoBounds();
  await expect(reset).toHaveCount(0);

  // A background double-click resets the view as it does on the Spectrogram.
  await dragCutRegion();
  await expect(reset).toBeVisible();
  const background = await cutRegionBackground(page);
  expect(background).not.toBeNull();
  if (!background) {
    return;
  }
  await page.mouse.dblclick(background.x, background.y);
  await waitForAutoBounds();
  await expect(reset).toHaveCount(0);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("new plot opens a csv table and switches to x/y pairs", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  // Plots are opened from the File menu, not offered as launcher tiles.
  await expect(
    page.getByRole("button", { name: "Plot", exact: true }),
  ).toHaveCount(0);
  await openPlotFromFileMenu(page, seriesFixture, "New Plot…");

  const frame = page.locator(".display-frame").filter({ hasText: "series.csv" });
  await expect(frame.locator(".cut-plot")).toBeVisible({ timeout: 30_000 });
  await expect(frame.locator(".cut-line")).toHaveCount(2);
  await expect(frame.getByTitle("Toggle a")).toBeVisible();
  await expect(frame.getByTitle("Toggle b")).toBeVisible();
  await expect(frame.getByText("2 lines · 3 rows")).toBeVisible();

  await frame.getByRole("radio", { name: "x/y pairs" }).click();
  await expect(frame.locator(".cut-line")).toHaveCount(1);
  await expect(
    frame.getByText(/trailing column 'b' ignored/),
  ).toBeVisible();

  expect(errors, errors.join("\n")).toEqual([]);
});

test("new x/y pair plot reads paired columns and exports an image", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await openPlotFromFileMenu(page, pairsFixture, "New Plot (x/y pairs)…");

  const frame = page.locator(".display-frame").filter({ hasText: "pairs.csv" });
  await expect(frame.locator(".cut-plot")).toBeVisible({ timeout: 30_000 });
  await expect(frame.locator(".cut-line")).toHaveCount(2);
  await expect(frame.getByTitle("Toggle y1")).toBeVisible();
  await expect(frame.getByTitle("Toggle y2")).toBeVisible();
  await expect(frame.getByText("2 lines · 3 rows")).toBeVisible();

  const download = page.waitForEvent("download");
  await frame.getByRole("button", { name: "Export image" }).click();
  expect((await download).suggestedFilename()).toBe("plot.png");

  expect(errors, errors.join("\n")).toEqual([]);
});

test("records the microphone and opens the take in a spectrogram", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Recorder/ }).click();
  await expect(page.locator(".recorder-panel")).toBeVisible();

  await page.getByRole("button", { name: "Start recording" }).click();
  await expect(
    page.getByRole("button", { name: "Stop recording" }),
  ).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(1200);
  await page.getByRole("button", { name: "Stop recording" }).click();

  // The Recorder tab is replaced by the Spectrogram for the saved take.
  await expect(page.getByTestId("spectrogram-plot")).toBeVisible({
    timeout: 30_000,
  });
  await expect(
    page.locator(".display-frame .display-title strong"),
  ).toContainText(/recording-/);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("line cuts plot context menu offers the common plot actions", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await addLineHere(page);
  await expect(page.locator(".cut-svg")).toBeVisible({ timeout: 30_000 });

  const region = await cutRegionBackground(page);
  expect(region).not.toBeNull();
  if (!region) {
    return;
  }
  await page.mouse.click(region.x, region.y, { button: "right" });
  const menu = page.getByRole("menu", { name: "Plot actions" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Export image" })).toBeVisible();

  const download = page.waitForEvent("download");
  await menu.getByRole("menuitem", { name: "Export image" }).click();
  expect((await download).suggestedFilename()).toBe("line-cuts.png");

  const lineFrame = page
    .locator(".display-frame")
    .filter({ has: page.locator('.cut-plot[data-cut-kind="line"]') });
  await lineFrame.getByRole("button", { name: "Toggle inspector" }).click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Inspector line axis" }),
  ).toBeVisible();

  expect(errors, errors.join("\n")).toEqual([]);
});

test("plot context menu stays inside the viewport", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const canvas = page.locator(".plot-canvas");
  const box = await canvas.boundingBox();
  const viewport = page.viewportSize();
  expect(box).not.toBeNull();
  expect(viewport).not.toBeNull();
  if (!box || !viewport) {
    return;
  }
  const menu = page.locator(".plot-context-menu");

  await canvas.click({
    button: "right",
    position: { x: box.width - 8, y: box.height - 8 },
  });
  await expect(menu).toBeVisible();
  const bottomRight = await menu.boundingBox();
  expect(bottomRight).not.toBeNull();
  if (bottomRight) {
    expect(bottomRight.x).toBeGreaterThanOrEqual(0);
    expect(bottomRight.y).toBeGreaterThanOrEqual(0);
    expect(bottomRight.x + bottomRight.width).toBeLessThanOrEqual(
      viewport.width,
    );
    expect(bottomRight.y + bottomRight.height).toBeLessThanOrEqual(
      viewport.height,
    );
  }

  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  await canvas.click({ button: "right", position: { x: 8, y: 8 } });
  await expect(menu).toBeVisible();
  const topLeft = await menu.boundingBox();
  expect(topLeft).not.toBeNull();
  if (topLeft) {
    expect(topLeft.x).toBeGreaterThanOrEqual(0);
    expect(topLeft.y).toBeGreaterThanOrEqual(0);
  }
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("closing all tabs restores a Launcher and Undo restores the layout", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  await addChildTab(page, "Order Cuts");
  await expect(page.getByTestId("cut-plot").first()).toBeVisible({
    timeout: 30_000,
  });

  await page.locator(".fft-tab").first().locator(".fft-tab-close").click();
  const confirm = page.getByRole("dialog", { name: "Close displays" });
  await expect(confirm).toBeVisible();
  await confirm.getByRole("button", { name: "Close all" }).click();

  await expect(page.getByTestId("launcher")).toBeVisible();
  await expect(page.locator(".fft-tab")).toHaveCount(1);
  await expect(page.locator(".fft-tab")).toContainText("Launcher");

  await page.locator(".fft-tab").first().locator(".fft-tab-close").click();
  await expect(page.getByTestId("launcher")).toBeVisible();
  await expect(page.locator(".fft-tab")).toHaveCount(1);

  await page.getByRole("button", { name: "Undo" }).click();
  await expect(page.getByTestId("spectrogram-plot")).toBeVisible({
    timeout: 30_000,
  });
  await expect(
    page.locator(".fft-tab", { hasText: "Order Cuts" }),
  ).toHaveCount(1);
  await expect(page.locator(".fft-tab")).toHaveCount(2);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("plot context Add line here seeds a Line Cuts child", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const canvas = page.locator(".plot-canvas");
  const box = await canvas.boundingBox();
  expect(box).not.toBeNull();
  if (!box) {
    return;
  }
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, {
    button: "right",
  });
  await expect(page.locator(".plot-context-menu")).toBeVisible();
  const addLine = page.getByRole("menuitem", { name: "Add line here" });
  await expect(addLine).toBeEnabled();
  await addLine.click();

  await expect(
    page.locator(".fft-tab", { hasText: "Line Cuts" }),
  ).toHaveCount(1);
  const linePlot = page.getByTestId("cut-plot").first();
  await expect(linePlot.locator(".cut-line").first()).toBeVisible({
    timeout: 30_000,
  });
  await expect(linePlot.getByText("line 1")).toBeVisible();

  await expect
    .poll(async () => {
      const session = await request.get("/api/session");
      const saved = (await session.json()) as {
        displays?: Array<{ kind?: string; state?: { lines?: unknown[] } }>;
      };
      return (
        saved.displays?.find((display) => display.kind === "line_cuts")?.state
          ?.lines?.length ?? 0
      );
    })
    .toBe(1);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("Add line here reuses one Line Cuts child", async ({ page, request }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  await addLineHere(page);
  const lineTabs = page.locator(".fft-tab", { hasText: "Line Cuts" });
  await expect(lineTabs).toHaveCount(1);
  const plot = page.getByTestId("cut-plot").first();
  await expect(plot.getByText("line 1")).toBeVisible();

  await addLineHere(page);
  await expect(lineTabs).toHaveCount(1);
  await expect(plot.getByText("line 2")).toBeVisible();
  await expect(plot.locator(".cut-line")).toHaveCount(2);

  await expect
    .poll(async () => {
      const session = await request.get("/api/session");
      const saved = (await session.json()) as {
        displays?: Array<{ kind?: string; state?: { lines?: unknown[] } }>;
      };
      const cuts = saved.displays?.find(
        (display) => display.kind === "line_cuts",
      );
      return cuts?.state?.lines?.length ?? 0;
    })
    .toBe(2);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("Move line cut to another Line Cuts window", async ({ page, request }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  await addLineHere(page);
  await addLineHere(page);
  const lineTabs = page.locator(".fft-tab", { hasText: "Line Cuts" });
  await expect(lineTabs).toHaveCount(1);

  await page.locator(".fft-tab").first().click();
  await addChildTab(page, "Line Cuts");
  await expect(lineTabs).toHaveCount(2);

  await lineTabs.first().click();
  const sourcePlot = page
    .getByTestId("cut-plot")
    .filter({ hasText: "line 2" });
  await expect(sourcePlot.getByText("line 1")).toBeVisible();
  await sourcePlot
    .locator(".cut-legend-item")
    .filter({ hasText: "line 1" })
    .click({ button: "right" });

  const moveMenu = page.getByRole("menu", { name: "Line cut actions" });
  await expect(moveMenu).toBeVisible();
  await expect(moveMenu.getByText("Move to another window")).toBeVisible();
  const targetName = (
    await lineTabs.nth(1).locator(".tab-title").innerText()
  ).trim();
  await moveMenu.getByRole("menuitem", { name: targetName }).click();

  await expect
    .poll(async () => {
      const session = await request.get("/api/session");
      const saved = (await session.json()) as {
        displays?: Array<{ kind?: string; state?: { lines?: unknown[] } }>;
      };
      const counts = (saved.displays ?? [])
        .filter((display) => display.kind === "line_cuts")
        .map((display) => display.state?.lines?.length ?? 0)
        .sort((a, b) => a - b);
      return `${counts.length}:${counts.join(",")}`;
    })
    .toBe("2:1,1");

  await lineTabs.first().click();
  await expect(page.locator(".cut-plot:visible .cut-line").first()).toBeVisible();
  await expect(
    page.locator(".cut-plot:visible").getByText("line 1").first(),
  ).toBeVisible();
  await lineTabs.nth(1).click();
  await expect(page.locator(".cut-plot:visible .cut-line").first()).toBeVisible();
  await expect(
    page.locator(".cut-plot:visible").getByText("line 1").first(),
  ).toBeVisible();

  expect(errors, errors.join("\n")).toEqual([]);
});

test("spectrogram overlays mirror Line Cuts colors", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  await addLineHere(page);
  await addLineHere(page);
  await expect(page.locator(".fft-tab", { hasText: "Line Cuts" })).toHaveCount(
    1,
  );
  await expect(page.locator(".spectrogram-line-cut")).toHaveCount(2);
  await expect(page.locator(".cut-swatch")).toHaveCount(2);

  const overlays = await page
    .locator(".spectrogram-line-cut")
    .evaluateAll((nodes) =>
      nodes.map((node) => {
        const line = node as SVGLineElement;
        return {
          stroke: getComputedStyle(line).stroke,
          y1: Number(line.getAttribute("y1")),
          y2: Number(line.getAttribute("y2")),
        };
      }),
    );
  const swatches = await page
    .locator(".cut-swatch")
    .evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).backgroundColor));
  expect(overlays.map((overlay) => overlay.stroke)).toEqual(
    swatches.slice(0, 2),
  );
  for (const overlay of overlays) {
    expect(Math.abs(overlay.y1 - overlay.y2)).toBeLessThan(0.5);
  }

  expect(errors, errors.join("\n")).toEqual([]);
});

test("dragging a spectrogram line endpoint persists the line", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await addLineHere(page);
  await expect(page.locator(".spectrogram-line-cut")).toHaveCount(1);

  await expect
    .poll(async () => (await firstLineCutLine(request))?.f0 ?? null)
    .not.toBeNull();
  const initial = await firstLineCutLine(request);
  expect(initial).not.toBeNull();
  if (!initial) {
    return;
  }

  const midpoint = await page
    .locator(".spectrogram-line-cut")
    .first()
    .evaluate((node) => {
      const line = node as SVGLineElement;
      const svg = line.ownerSVGElement;
      if (!svg) {
        return null;
      }
      const rect = svg.getBoundingClientRect();
      const viewBox = svg.viewBox.baseVal;
      const x1 = Number(line.getAttribute("x1"));
      const x2 = Number(line.getAttribute("x2"));
      const y1 = Number(line.getAttribute("y1"));
      const y2 = Number(line.getAttribute("y2"));
      return {
        x: rect.left + (((x1 + x2) / 2) / viewBox.width) * rect.width,
        y: rect.top + (((y1 + y2) / 2) / viewBox.height) * rect.height,
      };
    });
  expect(midpoint).not.toBeNull();
  if (!midpoint) {
    return;
  }
  await page.mouse.click(midpoint.x, midpoint.y);
  await expect(page.locator(".spectrogram-line-handle")).toHaveCount(2);
  const handle = page.locator(
    '.spectrogram-line-handle[data-handle="end"]',
  );
  const box = await handle.boundingBox();
  expect(box).not.toBeNull();
  if (!box) {
    return;
  }
  const handleX = box.x + box.width / 2 - 2;
  const handleY = box.y + box.height / 2;
  await page.mouse.move(handleX, handleY);
  await page.mouse.down();
  await page.mouse.move(handleX, handleY - 36, { steps: 6 });
  await page.mouse.up();

  await expect
    .poll(async () => {
      const line = await firstLineCutLine(request);
      return line ? lineSignature(line) : "";
    })
    .not.toBe(lineSignature(initial));
  await expect(page.locator(".spectrogram-line-cut")).toHaveCount(1);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("line endpoints reach the log floor and 0 Hz in linear view", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await addLineHere(page);
  await expect(page.locator(".spectrogram-line-cut")).toHaveCount(1);
  await expect
    .poll(async () => (await firstLineCutLine(request))?.f0 ?? null)
    .not.toBeNull();

  // Endpoint snapping is a separate gesture aid; turn it off so this test
  // measures the frequency clamp itself rather than the 2° snap zone.
  const lineFrame = page
    .locator(".display-frame")
    .filter({ has: page.locator('.cut-plot[data-cut-kind="line"]') });
  await lineFrame.getByRole("button", { name: "Toggle inspector" }).click();
  await expect(page.getByTestId("inspector")).toBeVisible();
  const snap = page.getByRole("switch", { name: "Snap line angle" });
  await expect(snap).toHaveAttribute("aria-checked", "true");
  await snap.click();
  await expect(snap).toHaveAttribute("aria-checked", "false");
  await page.getByRole("button", { name: "Close inspector" }).click();
  await expect(page.getByTestId("inspector")).toHaveCount(0);

  const line = page.locator(".spectrogram-line-cut").first();
  const lineBox = await line.boundingBox();
  expect(lineBox).not.toBeNull();
  if (!lineBox) {
    return;
  }
  await page.mouse.click(
    lineBox.x + lineBox.width / 2,
    lineBox.y + lineBox.height / 2,
  );
  await expect(page.locator(".spectrogram-line-handle")).toHaveCount(2);

  const overlay = await page.locator(".spectrogram-overlays").boundingBox();
  expect(overlay).not.toBeNull();
  if (!overlay) {
    return;
  }
  // Pointer capture keeps the drag alive past the plot edge, so overshooting
  // the bottom reaches the view's lowest representable frequency exactly.
  const dragEndPastBottom = async () => {
    const handle = page.locator(
      '.spectrogram-line-handle[data-handle="end"]',
    );
    const box = await handle.boundingBox();
    expect(box).not.toBeNull();
    if (!box) {
      return;
    }
    // The end handle can sit on the plot's right edge; keep the pointer a
    // few pixels inside the SVG hit area.
    const handleX = box.x + box.width / 2 - 3;
    await page.mouse.move(handleX, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(handleX, overlay.y + overlay.height + 150, {
      steps: 8,
    });
    await page.mouse.up();
  };
  const endFrequency = async () =>
    (await firstLineCutLine(request))?.f1 ?? Number.POSITIVE_INFINITY;

  // Log view: the axis stops at its 10 Hz floor (never an arbitrary
  // 0.5 %-of-Nyquist value).
  await dragEndPastBottom();
  await expect.poll(endFrequency).toBeLessThan(20);

  await page.getByRole("radio", { name: "Linear" }).click();
  await dragEndPastBottom();
  await expect.poll(endFrequency).toBeLessThan(1);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("order tracking controls update chips and guide colors", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await addChildTab(page, "Order Cuts");
  await expect(page.getByTestId("order-tracking")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.locator(".cut-swatch")).toHaveCount(3);

  await page.getByLabel("RPM source").selectOption("ramp");
  await page.getByLabel("RPM start").fill("2000");
  await page.getByLabel("RPM end").fill("2400");
  const summary = page.getByTestId("order-tracking-summary");
  await expect(summary).toContainText("2000→2400 RPM");
  await expect(summary).toContainText("order 1 = 33.3–40.0 Hz");
  await page.getByRole("button", { name: "Apply tracking" }).click();

  const chip = page.getByTestId("spectrogram-tracking");
  await expect(chip).toContainText("2000→2400 RPM");
  await expect(chip).toContainText("33.3–40.0 Hz");
  await expect(page.locator(".spectrogram-order-guide")).toHaveCount(3);
  const guideStrokes = await page
    .locator(".spectrogram-order-guide")
    .evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).stroke));
  const swatches = await page
    .locator(".cut-swatch")
    .evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).backgroundColor));
  expect(guideStrokes).toEqual(swatches.slice(0, 3));

  await page.getByLabel("RPM source").selectOption("csv");
  await page.getByLabel("RPM CSV path").fill("/tmp/rpm.csv");
  await expect(summary).toContainText("CSV /tmp/rpm.csv");

  expect(errors, errors.join("\n")).toEqual([]);
});

test("grid line toggles hide plot chrome and persist", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const specGrid = page.locator(".display-spec .spectrogram-grid-line");
  await expect.poll(() => specGrid.count()).toBeGreaterThan(0);
  const specToggle = page
    .locator(".display-spec")
    .getByRole("button", { name: "Grid lines" });
  await expect(specToggle).toHaveAttribute("aria-pressed", "true");
  await specToggle.click();
  await expect(specToggle).toHaveAttribute("aria-pressed", "false");
  await expect(specGrid).toHaveCount(0);
  await expect(page.getByTestId("frequency-axis")).toBeVisible();

  await addChildTab(page, "Order Cuts");
  const cutGrid = page.locator(".cut-grid");
  await expect.poll(() => cutGrid.count()).toBeGreaterThan(0);
  const cutToggle = page
    .locator(".display-order")
    .getByRole("button", { name: "Grid lines" });
  await cutToggle.click();
  await expect(cutGrid).toHaveCount(0);
  await expect(page.locator(".cut-tick").first()).toBeVisible();

  await expect
    .poll(async () => {
      const session = await request.get("/api/session");
      const saved = (await session.json()) as {
        displays?: Array<{
          kind?: string;
          state?: {
            showGrid?: boolean;
            displaySettings?: { show_grid?: boolean };
          };
        }>;
      };
      const spectrogram = saved.displays?.find(
        (display) => display.kind === "spectrogram",
      );
      const orders = saved.displays?.find(
        (display) => display.kind === "order_cuts",
      );
      return `${spectrogram?.state?.displaySettings?.show_grid}:${orders?.state?.showGrid}`;
    })
    .toBe("false:false");

  await page.reload();
  await expect(page.getByTestId("spectrogram-plot")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.locator(".spectrogram-grid-line")).toHaveCount(0);
  await expect(page.locator(".cut-svg")).toHaveCount(1);
  await expect(page.locator(".cut-grid")).toHaveCount(0);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("spectrogram zoom-out reveals the bounded overscan frame", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const host = page.getByTestId("spectrogram-plot");
  const label = page.locator(".minimap-label");
  const frame = page.locator(".spectrogram-data-frame");
  await expect(host).toHaveAttribute("data-overscan", "false");
  await expect(frame).toHaveCount(0);
  const initial = minimapRange(await label.textContent());

  await page.getByRole("button", { name: "Zoom out" }).click();
  await expect(host).toHaveAttribute("data-overscan", "true");
  await expect(frame).toBeVisible();
  const zoomed = minimapRange(await label.textContent());
  expect(zoomed.start).toBeLessThan(0);
  expect(zoomed.span).toBeGreaterThan(initial.span);

  await page.getByRole("button", { name: "Fit view" }).click();
  await expect(host).toHaveAttribute("data-overscan", "false");
  await expect(frame).toHaveCount(0);
  const fitted = minimapRange(await label.textContent());
  expect(Math.abs(fitted.start - initial.start)).toBeLessThan(0.02);
  expect(Math.abs(fitted.end - initial.end)).toBeLessThan(0.02);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("spectrogram scale badges and transient pan-only minimap", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await addChildTab(page, "Order Cuts");
  const tracking = page.getByTestId("spectrogram-tracking");
  await expect(tracking).toBeVisible({ timeout: 30_000 });
  await page.locator(".fft-tab").first().click();

  const scales = page.getByTestId("spectrogram-scales");
  const minimap = page.locator(".minimap");
  await expect(scales).toContainText("Range 80 dB");
  await expect(scales).toContainText("Log frequency");
  await expect(minimap).toHaveAttribute("data-visible", "true");

  await page.getByRole("radio", { name: "Linear" }).click();
  await expect(scales).toContainText("Linear frequency");

  const colorbar = page.locator(".colorbar");
  const scaleBox = await scales.boundingBox();
  const colorbarBox = await colorbar.boundingBox();
  const minimapBox = await minimap.boundingBox();
  const trackingBox = await tracking.boundingBox();
  expect(scaleBox).not.toBeNull();
  expect(colorbarBox).not.toBeNull();
  expect(minimapBox).not.toBeNull();
  expect(trackingBox).not.toBeNull();
  if (scaleBox && colorbarBox && minimapBox && trackingBox) {
    expect(scaleBox.x + scaleBox.width).toBeLessThanOrEqual(
      colorbarBox.x + 0.5,
    );
    expect(minimapBox.x + minimapBox.width).toBeLessThanOrEqual(
      colorbarBox.x + 0.5,
    );
    const separated =
      scaleBox.x >= trackingBox.x + trackingBox.width ||
      trackingBox.x >= scaleBox.x + scaleBox.width ||
      scaleBox.y >= trackingBox.y + trackingBox.height ||
      trackingBox.y >= scaleBox.y + scaleBox.height;
    expect(separated).toBe(true);
  }

  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(minimap).toHaveAttribute("data-visible", "true");
  await expect
    .poll(() => minimap.getAttribute("data-visible"), {
      timeout: 6_000,
      intervals: [200],
    })
    .toBe("false");
  await page.getByRole("button", { name: "Zoom in" }).click();
  await expect(minimap).toHaveAttribute("data-visible", "true");

  const label = minimap.locator(".minimap-label");
  const viewBox = page.locator(".minimap-view");
  const before = minimapRange(await label.textContent());
  const beforeBox = await viewBox.boundingBox();
  const miniBox = await minimap.boundingBox();
  expect(beforeBox).not.toBeNull();
  expect(miniBox).not.toBeNull();
  if (!beforeBox || !miniBox) {
    return;
  }
  const startX = beforeBox.x + beforeBox.width / 2;
  const startY = beforeBox.y + beforeBox.height / 2;
  await page.mouse.move(startX, startY);
  await page.mouse.down();
  await page.mouse.move(startX + miniBox.width * 0.1, startY, { steps: 8 });
  await page.mouse.up();

  await expect
    .poll(async () =>
      Math.abs(minimapRange(await label.textContent()).start - before.start),
    )
    .toBeGreaterThan(0.05);
  const after = minimapRange(await label.textContent());
  expect(Math.abs(after.span - before.span)).toBeLessThan(0.05);
  const afterBox = await viewBox.boundingBox();
  expect(afterBox).not.toBeNull();
  if (afterBox) {
    expect(Math.abs(afterBox.width - beforeBox.width)).toBeLessThanOrEqual(1);
  }

  expect(errors, errors.join("\n")).toEqual([]);
});

test("spectrogram can disable windowing from the inspector", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await page.getByRole("button", { name: "Toggle inspector" }).click();
  await expect(page.getByTestId("inspector")).toBeVisible();

  const windowSelect = page.getByLabel("Window");
  const windowValues = await windowSelect
    .locator("option")
    .evaluateAll((options) =>
      options.map((option) => (option as HTMLOptionElement).value),
    );
  expect(windowValues).toContain("none");
  expect(windowValues).toContain("rectangular");
  await expect(windowSelect.locator('option[value="none"]')).toHaveText("None");
  await expect(
    windowSelect.locator('option[value="rectangular"]'),
  ).toHaveCount(1);

  await windowSelect.selectOption("none");
  await expect(windowSelect).toHaveValue("none");
  await page.getByRole("button", { name: "Apply & re-analyse" }).click();
  await expect(page.getByTestId("spectrogram-plot")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.locator(".display-state.state-ready")).toBeVisible({
    timeout: 30_000,
  });
  await expect(windowSelect).toHaveValue("none");

  expect(errors, errors.join("\n")).toEqual([]);
});

test("inspector scrolls cleanly at constrained height", async ({ page }) => {
  const errors = trackErrors(page);
  await page.setViewportSize({ width: 900, height: 520 });
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await page.getByRole("button", { name: "Toggle inspector" }).click();
  await expect(page.getByTestId("inspector")).toBeVisible();

  const collapsed = page.locator(
    ".inspector-scroll .card-toggle[aria-expanded='false']",
  );
  for (let index = 0; index < 10; index += 1) {
    if ((await collapsed.count()) === 0) {
      break;
    }
    await collapsed.first().click();
  }
  const scroll = page.locator(".inspector-scroll");
  const metrics = await scroll.evaluate((element) => ({
    scrollHeight: element.scrollHeight,
    clientHeight: element.clientHeight,
    overflowY: getComputedStyle(element).overflowY,
  }));
  expect(metrics.scrollHeight).toBeGreaterThan(metrics.clientHeight);
  expect(["auto", "scroll"]).toContain(metrics.overflowY);

  const cards = page.locator(".inspector-scroll .card");
  expect(await cards.first().evaluate((element) => getComputedStyle(element).flexShrink)).toBe(
    "0",
  );
  await scroll.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  const scrollBox = await scroll.boundingBox();
  expect(scrollBox).not.toBeNull();
  if (!scrollBox) {
    return;
  }
  const lastCard = cards.last();
  await expect(lastCard).toBeVisible();
  const lastControl = lastCard
    .locator("input:visible, select:visible, button:visible")
    .last();
  await expect(lastControl).toBeVisible();
  const controlBox = await lastControl.boundingBox();
  expect(controlBox).not.toBeNull();
  if (controlBox) {
    expect(controlBox.y).toBeGreaterThanOrEqual(scrollBox.y - 1);
    expect(controlBox.y + controlBox.height).toBeLessThanOrEqual(
      scrollBox.y + scrollBox.height + 1,
    );
  }
  const ordered = await cards.evaluateAll((nodes) => {
    const rects = nodes.map((node) => node.getBoundingClientRect());
    return rects.every(
      (rect, index) =>
        index === 0 || rect.top >= rects[index - 1].bottom - 0.5,
    );
  });
  expect(ordered).toBe(true);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("add order cut from the order plot context menu", async ({
  page,
  request,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);
  await addChildTab(page, "Order Cuts");
  await expect(page.getByTestId("order-tracking")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.locator(".cut-swatch")).toHaveCount(3);

  const plot = page.locator(".cut-svg");
  await expect(plot).toBeVisible();
  await plot.click({ button: "right", position: { x: 20, y: 150 } });
  const menu = page.getByRole("menu", { name: "Order cut actions" });
  await expect(menu).toBeVisible();
  await menu.getByRole("menuitem", { name: "Add order cut" }).click();
  await page.getByLabel("New order value").fill("5");
  await menu.getByRole("button", { name: "Add" }).click();

  await expect(
    page.locator(".cut-legend-item").filter({ hasText: "5x" }),
  ).toBeVisible();
  await expect(page.locator(".cut-line")).toHaveCount(4);

  await expect
    .poll(async () => {
      const session = await request.get("/api/session");
      const saved = (await session.json()) as {
        displays?: Array<{
          kind?: string;
          state?: { orderCuts?: { orders?: string } };
        }>;
      };
      const orders =
        saved.displays?.find((display) => display.kind === "order_cuts")?.state
          ?.orderCuts?.orders ?? "";
      return orders.split(",").includes("5");
    })
    .toBe(true);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("menu bar opens commands and adds a launcher tab", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);

  await page.getByRole("menuitem", { name: "File" }).click();
  const fileMenu = page.getByRole("menu", { name: "File" });
  await expect(fileMenu).toBeVisible();
  await fileMenu.getByRole("menuitem", { name: "New Launcher" }).click();
  await expect(page.locator(".fft-tab")).toHaveCount(2);

  await page.getByRole("menuitem", { name: "Help" }).click();
  const helpMenu = page.getByRole("menu", { name: "Help" });
  await expect(helpMenu).toBeVisible();
  await helpMenu.getByRole("menuitem", { name: "Keyboard shortcuts" }).click();
  await expect(
    page.getByRole("dialog", { name: "Keyboard shortcuts" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  expect(errors, errors.join("\n")).toEqual([]);
});

test("children stay in one companion group", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  await addChildTab(page, "Order Cuts");
  await expect(page.getByTestId("cut-plot").first()).toBeVisible({
    timeout: 30_000,
  });

  await page.locator(".fft-tab").first().click();
  await addChildTab(page, "Line Cuts");
  await expect(page.locator(".cut-svg")).toBeVisible({ timeout: 30_000 });

  await expect(page.locator(".fft-tab", { hasText: "Order Cuts" })).toHaveCount(
    1,
  );
  await expect(page.locator(".fft-tab", { hasText: "Line Cuts" })).toHaveCount(
    1,
  );
  expect(await page.locator(".dv-groupview").count()).toBe(2);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("add-tab adds a Launcher next to the active tab and appears in every group", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const activeTabs = page.locator(".dv-active-group .fft-tab");
  await expect(activeTabs).toHaveCount(1);
  const tabBox = await activeTabs.first().boundingBox();
  const addBox = await page
    .locator(".dv-active-group")
    .getByRole("button", { name: "Add tab" })
    .boundingBox();
  expect(tabBox).not.toBeNull();
  expect(addBox).not.toBeNull();
  if (tabBox && addBox) {
    expect(addBox.x).toBeGreaterThan(tabBox.x + tabBox.width);
    expect(addBox.width).toBeGreaterThanOrEqual(32);
    expect(addBox.height).toBeGreaterThanOrEqual(tabBox.height - 1);
    expect(
      Math.abs(
        addBox.y + addBox.height / 2 - (tabBox.y + tabBox.height / 2),
      ),
    ).toBeLessThan(2);
  }
  await page
    .locator(".dv-active-group")
    .getByRole("button", { name: "Add tab" })
    .click();
  await expect(activeTabs).toHaveCount(2);
  await expect(activeTabs.nth(1)).toContainText("Launcher");

  await page
    .locator(".fft-tab")
    .filter({ hasText: "tone" })
    .filter({ hasNotText: "Launcher" })
    .first()
    .click();
  await addChildTab(page, "Order Cuts");
  await expect(page.getByTestId("cut-plot").first()).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.locator(".add-tab")).toHaveCount(2);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("quick export saves the visible spectrogram and cut plot", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.getByRole("button", { name: /Spectrogram/ }).click();
  await expect(page.getByTestId("display-frame")).toBeVisible();
  await uploadTone(page);

  const spectrogramDownload = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export image" }).click();
  const image = await spectrogramDownload;
  expect(image.suggestedFilename()).toMatch(/\.png$/);

  const menuDownload = page.waitForEvent("download");
  await page.getByRole("menuitem", { name: "File" }).click();
  await page
    .getByRole("menu", { name: "File" })
    .getByRole("menuitem", { name: "Export image" })
    .click();
  expect((await menuDownload).suggestedFilename()).toMatch(/\.png$/);

  await page.getByRole("menuitem", { name: "Cuts" }).click();
  await page
    .getByRole("menu", { name: "Cuts" })
    .getByRole("menuitem", { name: "Order Cuts" })
    .click();
  const orderPlot = page.locator('.cut-plot[data-cut-kind="order"]').first();
  await expect(orderPlot).toBeVisible({ timeout: 30_000 });
  const plotSize = await orderPlot.evaluate((element) => {
    const view = (element.querySelector(".cut-svg") as SVGSVGElement).viewBox
      .baseVal;
    return { width: view.width, height: view.height };
  });

  const orderFrame = page
    .locator(".display-frame")
    .filter({ has: page.locator('.cut-plot[data-cut-kind="order"]') });
  const cutDownload = page.waitForEvent("download");
  await orderFrame.getByRole("button", { name: "Export image" }).click();
  const cutImage = await cutDownload;
  expect(cutImage.suggestedFilename()).toBe("order-cuts.png");

  const cutMenuDownload = page.waitForEvent("download");
  await page.getByRole("menuitem", { name: "File" }).click();
  await page
    .getByRole("menu", { name: "File" })
    .getByRole("menuitem", { name: "Export image" })
    .click();
  expect((await cutMenuDownload).suggestedFilename()).toBe("order-cuts.png");
  const path = await cutImage.path();
  expect(path).not.toBeNull();
  if (path) {
    const bytes = readFileSync(path);
    // Measured plot area plus the 220 px legend, exported at 2×.
    expect(bytes.readUInt32BE(16)).toBe((plotSize.width + 220) * 2);
    expect(bytes.readUInt32BE(20)).toBe(plotSize.height * 2);
  }

  expect(errors, errors.join("\n")).toEqual([]);
});

test("launcher inspector shows only input and Cuts menu is hidden", async ({
  page,
}) => {
  const errors = trackErrors(page);
  await openLauncher(page);

  await expect(page.getByRole("menuitem", { name: "Cuts" })).toHaveCount(0);
  await page.keyboard.press("p");
  await expect(page.getByTestId("inspector")).toBeVisible();
  await expect(page.getByRole("textbox", { name: "Input file" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Browse…" })).toBeVisible();
  await expect(page.getByRole("spinbutton", { name: "Frame" })).toHaveCount(0);
  await expect(page.getByRole("menuitem", { name: "Cuts" })).toHaveCount(0);

  expect(errors, errors.join("\n")).toEqual([]);
});

test("File > New Spectrogram replaces an active Launcher", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);

  await page.getByRole("menuitem", { name: "File" }).click();
  await page
    .getByRole("menu", { name: "File" })
    .getByRole("menuitem", { name: "New Spectrogram" })
    .click();

  await expect(page.locator(".fft-tab")).toHaveCount(1);
  await expect(page.getByTestId("launcher")).toHaveCount(0);
  await expect(page.getByTestId("display-empty")).toBeVisible();

  expect(errors, errors.join("\n")).toEqual([]);
});

test("command palette, shortcuts modal and tab commands", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);

  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Command palette" });
  await expect(palette).toBeVisible();
  await page
    .getByRole("textbox", { name: "Search commands" })
    .fill("new spectrogram");
  await expect(
    page.getByRole("button", { name: /New Spectrogram/ }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(palette).toHaveCount(0);

  await page.keyboard.press("F1");
  const shortcuts = page.getByRole("dialog", { name: "Keyboard shortcuts" });
  await expect(shortcuts).toBeVisible();
  await page
    .getByRole("searchbox", { name: "Filter shortcuts" })
    .fill("palette");
  await expect(page.getByText("Command palette")).toBeVisible();
  await page.keyboard.press("Escape");

  await page.keyboard.press("Control+t");
  await expect(page.locator(".fft-tab")).toHaveCount(1);
  await page.keyboard.press("Control+t");
  await expect(page.locator(".fft-tab")).toHaveCount(2);
  await page.keyboard.press("Control+w");
  await expect(page.locator(".fft-tab")).toHaveCount(1);

  await page.screenshot({ path: "test-results/palette.png" });
  expect(errors, errors.join("\n")).toEqual([]);
});

test("all settings pages render", async ({ page }) => {
  const errors = trackErrors(page);
  await openLauncher(page);
  await page.keyboard.press("Control+,");
  await expect(page.getByTestId("settings")).toBeVisible();

  for (const pageName of [
    "General",
    "Appearance",
    "Keyboard",
    "Files",
    "Themes",
    "About",
  ]) {
    await page.getByRole("button", { name: new RegExp(pageName) }).click();
    await expect(
      page.getByRole("heading", { name: pageName }),
    ).toBeVisible();
  }

  await page.getByRole("button", { name: /Keyboard/ }).click();
  await page
    .getByRole("searchbox", { name: "Filter shortcuts" })
    .fill("palette");
  await expect(page.getByText("Command palette")).toBeVisible();

  await page.getByRole("button", { name: /Files/ }).click();
  await expect(page.getByText(/settings\.toml/).first()).toBeVisible();
  await page.getByRole("button", { name: /About/ }).click();
  await expect(page.locator(".about-grid").getByText("0.13.0")).toBeVisible();
  await expect(page.getByText("Dockview (tauri-dockview)")).toBeVisible();

  expect(errors, errors.join("\n")).toEqual([]);
});

test("responsive layouts at 850 px and 500 px", async ({ page }) => {
  const errors = trackErrors(page);
  await page.setViewportSize({ width: 850, height: 700 });
  await openLauncher(page);
  const chip = await page
    .locator(".mode-chip")
    .evaluate((element) => getComputedStyle(element).display);
  expect(chip).toBe("none");

  await page.setViewportSize({ width: 500, height: 700 });
  await page.reload();
  await expect(page.getByTestId("launcher")).toBeVisible();
  const direction = await page
    .locator(".launcher-tiles")
    .evaluate((element) => getComputedStyle(element).flexDirection);
  expect(direction).toBe("column");

  await page.keyboard.press("Control+,");
  await expect(page.getByTestId("settings")).toBeVisible();
  const rail = await page
    .locator(".settings-rail")
    .evaluate((element) => getComputedStyle(element).flexDirection);
  expect(rail).toBe("row");
  await page.screenshot({ path: "test-results/responsive-500.png" });
  expect(errors, errors.join("\n")).toEqual([]);
});

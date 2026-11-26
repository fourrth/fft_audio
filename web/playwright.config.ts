import path from "node:path";
import { defineConfig } from "@playwright/test";

const port = Number(process.env.FFT_E2E_PORT ?? 18765);
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 90_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  outputDir: "test-results",
  use: {
    baseURL,
    viewport: { width: 1280, height: 800 },
    actionTimeout: 15_000,
    navigationTimeout: 30_000,
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    // The Recorder test needs a microphone; Chromium supplies a fake device.
    permissions: ["microphone"],
    launchOptions: {
      args: [
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
      ],
    },
  },
  webServer: {
    command: `cargo run -q -p fft_desktop --no-default-features --features http -- --serve --port ${port} --bind 127.0.0.1 --dist web/dist --config-dir web/.playwright-config`,
    cwd: path.resolve(process.cwd(), ".."),
    url: `${baseURL}/api/app/info`,
    reuseExistingServer: false,
    timeout: 180_000,
  },
});

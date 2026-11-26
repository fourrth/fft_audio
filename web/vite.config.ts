import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig(({ mode }) => ({
  // The static browser bundle must also work when served from a sub-path
  // (for example GitHub Pages). Normal desktop/headless builds keep root URLs.
  base: mode === "browser" ? "./" : "/",
  plugins: [react()],
  build: {
    // Dockview + React + the settings/inspector screens form one desktop
    // bundle; ~700 kB is expected. Keep the warning above that.
    chunkSizeWarningLimit: 800,
  },
  test: {
    environment: "node",
    exclude: ["**/node_modules/**", "**/dist/**", "tests/e2e/**"],
  },
}));

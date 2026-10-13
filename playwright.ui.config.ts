import { defineConfig } from "@playwright/test";

// Dev-only screenshot harness for the app UI (`pnpm shots:ui`). NOT part of `pnpm test:e2e`.
//
// Serves the built frontend with `vite preview` on UI_PORT and injects a mock Tauri backend
// (e2e/ui/mock-tauri.ts) into every page, so no kinetra.exe is started or touched. Live thumbnails
// load the real renderer from the fixture output server (e2e/fixture.ts), routed behind
// http://127.0.0.1:7770 inside the browser only. PNGs go to review/kinetra-m3/ (gitignored).
const UI_PORT = 4317;

export default defineConfig({
  testDir: "e2e/ui",
  testMatch: "**/*.spec.ts",
  workers: 1,
  fullyParallel: false,
  timeout: 120_000,
  reporter: [["list"]],
  use: {
    channel: "msedge",
    baseURL: `http://127.0.0.1:${UI_PORT}`,
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
  },
  webServer: {
    command: `pnpm build && pnpm exec vite preview --host 127.0.0.1 --port ${UI_PORT} --strictPort`,
    // The harness shows remote control's UI (a build with it; see svelte.config.js `$remote`).
    env: { KINETRA_REMOTE: "1" },
    url: `http://127.0.0.1:${UI_PORT}`,
    reuseExistingServer: false,
    timeout: 180_000,
  },
});

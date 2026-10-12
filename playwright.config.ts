import { defineConfig } from "@playwright/test";

// End-to-end tests for the renderer against the real Rust server (src-tauri/examples/fixture_server).
// Runs on the installed Microsoft Edge, so no browser download is needed.
export default defineConfig({
  testDir: "e2e",
  // e2e/ui is the dev-only app screenshot harness (pnpm shots:ui), not part of the e2e suite.
  testIgnore: ["ui/**"],
  // One fixture server per worker; the tests drive shared live state, so keep them serial.
  workers: 1,
  fullyParallel: false,
  timeout: 30_000,
  reporter: [["list"]],
  use: {
    channel: "msedge",
    viewport: { width: 1920, height: 1080 },
    deviceScaleFactor: 1,
  },
  expect: {
    toHaveScreenshot: { maxDiffPixelRatio: 0.01, animations: "allow" },
  },
  snapshotPathTemplate: "e2e/__screenshots__/{arg}{ext}",
});

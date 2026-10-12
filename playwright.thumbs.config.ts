import { defineConfig } from "@playwright/test";

// Gallery thumbnails for the built-in presets (`pnpm shots:presets`), NOT part of `pnpm test:e2e`.
// Renders each preset with the real renderer against the fixture output server
// (e2e/thumbs/presets.thumbs.ts) and writes presets/thumbs/<id>.png and, per style look,
// <id>-<style>.webp. Those files are committed; regenerate them only with this script.
export default defineConfig({
  testDir: "e2e/thumbs",
  testMatch: "**/*.thumbs.ts",
  workers: 1,
  fullyParallel: false,
  timeout: 60_000,
  reporter: [["list"]],
  use: {
    channel: "msedge",
    // Laid out on the real 1920×1080 stage, rasterised at a quarter: 480×270 device pixels.
    viewport: { width: 1920, height: 1080 },
    deviceScaleFactor: 0.25,
  },
});

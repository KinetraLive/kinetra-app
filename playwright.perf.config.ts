import base from "./playwright.config";
import { defineConfig } from "@playwright/test";

// Renderer cost measurements (e2e/*.measure.ts). Not part of `pnpm test:e2e`; run with
// `pnpm perf:renderer`.
export default defineConfig({ ...base, testMatch: "**/*.measure.ts" });

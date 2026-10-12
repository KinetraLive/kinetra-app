import base from "./playwright.ui.config";
import { defineConfig } from "@playwright/test";

// App UI cost measurements (e2e/ui/*.measure.ts), on the same built app + mock backend as the
// screenshot harness. Not part of `pnpm test:e2e`; run with `pnpm perf:ui`.
export default defineConfig({ ...base, testMatch: "**/*.measure.ts" });

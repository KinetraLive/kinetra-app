// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

/** KINETRA_REMOTE: "1" | "0"; unset = on for `pnpm dev`, off for builds. */
function remoteBuild() {
  const v = (process.env.KINETRA_REMOTE ?? "").trim();
  if (v === "1") return true;
  if (v === "0") return false;
  if (v) throw new Error(`KINETRA_REMOTE must be "1" or "0", not ${JSON.stringify(v)}`);
  return process.env.npm_lifecycle_event === "dev";
}

/** KINETRA_CLOUD: "1" | "0"; unset = on for `pnpm dev` and tests (vitest), off for builds. */
function cloudBuild() {
  const v = (process.env.KINETRA_CLOUD ?? "").trim();
  if (v === "1") return true;
  if (v === "0") return false;
  if (v) throw new Error(`KINETRA_CLOUD must be "1" or "0", not ${JSON.stringify(v)}`);
  return process.env.npm_lifecycle_event === "dev" || !!process.env.VITEST;
}

/** @type {import('@sveltejs/kit').Config} */
const config = {
  preprocess: vitePreprocess(),
  // No `window.__svelte.v` version stamp: it is a chunk every page loads plus an import in every
  // component chunk, and nothing reads it.
  compilerOptions: { discloseVersion: false },
  kit: {
    adapter: adapter({
      fallback: "index.html",
    }),
    alias: {
      // The CANARY badge exists only in canary builds (KINETRA_CHANNEL=canary at build time, as CI
      // sets it); stable builds resolve to an empty component and carry no canary text or styles
      // (scripts/check-channel-build.mjs checks the output).
      "$channel-badge":
        process.env.KINETRA_CHANNEL === "canary"
          ? "src/lib/components/brand/CanaryBadge.svelte"
          : "src/lib/components/brand/NoBadge.svelte",
      // Remote control is in builds with KINETRA_REMOTE=1 (CI release builds leave it off until
      // kinetra-cloud's relay is live) and in `pnpm dev`; other builds get the empty module, so
      // the switch and its UI aren't in them (scripts/check-remote-build.mjs checks the output).
      // The core decides the same way at its build (src-tauri/build.rs).
      $remote: remoteBuild()
        ? "src/lib/components/remote/build-on.ts"
        : "src/lib/components/remote/build-off.ts",
      // Cloud backup is in builds with KINETRA_CLOUD=1 (CI release builds leave it off until the
      // production API has cloud projects switched on), `pnpm dev` and tests; other builds get the
      // empty module (scripts/check-cloud-build.mjs checks the output). The core decides the same
      // way at its build (src-tauri/build.rs).
      $cloud: cloudBuild()
        ? "src/lib/components/cloud/build-on.ts"
        : "src/lib/components/cloud/build-off.ts",
      // The picker's "In the cloud" card: its own alias, so the shell's `$cloud` stays out of the
      // picker and the card out of the shell.
      "$cloud-picker": cloudBuild()
        ? "src/lib/components/picker/PickerCloud.svelte"
        : "src/lib/components/picker/NoCloud.svelte",
    },
  },
};

export default config;

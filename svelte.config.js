// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";
import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

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
    },
  },
};

export default config;

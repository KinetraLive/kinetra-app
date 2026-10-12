import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import process from "node:process";

const host = process.env.TAURI_DEV_HOST;

// Preload lists without what is already loaded. Every dynamic import carries a list of the chunks
// to preload with it (`__vite__mapDeps([...])` in the importing chunk), and Vite lists the target's
// whole static closure except the importing chunk itself: the Svelte runtime, the preload helper,
// the stores... A chunk only runs once its own static imports have loaded, so those entries are
// no-ops that every route and lazy chunk paid for in bytes (≈0.1–0.35 KB gz per route). The
// bundle is captured first; without it (or for HTML hosts) the list stays as Vite made it.
/** @type {import("vite").Rolldown.OutputBundle | null} */
let outputBundle = null;
/** @type {import("vite").Plugin} */
const captureBundle = {
  name: "kinetra:capture-bundle",
  generateBundle: {
    order: "pre",
    handler(_options, bundle) {
      outputBundle = bundle;
    },
  },
};
/** @type {import("vite").ResolveModulePreloadDependenciesFn} */
function withoutLoaded(_file, deps, { hostId, hostType }) {
  const bundle = outputBundle;
  if (hostType !== "js" || !bundle?.[hostId]) return deps;
  /** @type {Set<string>} */
  const loaded = new Set();
  /** @param {string} f */
  const walk = (f) => {
    if (loaded.has(f)) return;
    loaded.add(f);
    const chunk = bundle[f];
    if (chunk?.type === "chunk") chunk.imports.forEach(walk);
  };
  walk(hostId);
  return deps.filter((d) => !loaded.has(d));
}

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [sveltekit(), captureBundle],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  // Svelte runtime modules only @xyflow/svelte uses (use: actions, element size bindings,
  // MediaQuery) are pure, but svelte declares no `sideEffects`, so Rolldown places them by
  // reachability through the svelte/internal/client and svelte/reactivity barrels: into the
  // shared runtime chunk every page loads. Marked side-effect free, they follow their real
  // importer into the editor's lazy canvas chunk (keeps the picker within 50 KB gz).
  build: {
    // Release bundles ship without source maps.
    sourcemap: false,
    modulePreload: { resolveDependencies: withoutLoaded },
    rolldownOptions: {
      treeshake: {
        moduleSideEffects: (id) =>
          /\/svelte\/src\/(internal\/client\/dom\/elements\/(actions|bindings\/size)|reactivity\/(media-query|reactive-value))\.js$/.test(id)
            ? false
            : undefined,
      },
    },
  },

  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "renderer/src/**/*.test.ts", "scripts/*.test.mjs", ".github/scripts/*.test.mjs"],
  },

  // Svelte must resolve its browser build under vitest so components can mount.
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
}));

import { defineConfig } from "vite";

// Served by the app's axum server: /screen/:id returns dist/index.html and /renderer/* the assets.
export default defineConfig({
  root: import.meta.dirname,
  base: "/renderer/",
  plugins: [
    {
      // Vite wraps every dynamic import (the lazy studio chunk) in a preload helper (~0.6 KB gz)
      // that preloads CSS and modulepreload deps. The renderer has neither (modulePreload: false,
      // no CSS files), so the helper becomes a plain call.
      name: "kinetra:no-preload-helper",
      enforce: "pre",
      load(id) {
        if (id === "\0vite/preload-helper.js") return "export const __vitePreload = (f) => f();";
      },
    },
  ],
  build: {
    outDir: "dist",
    sourcemap: false,
    emptyOutDir: true,
    // OBS ships CEF/Chromium 103+, vMix a recent Chromium; ES2020 runs on both.
    target: "es2020",
    modulePreload: false,
    rollupOptions: {
      // pattern.html: the static test pattern for display outputs (/pattern; inline script only).
      input: { index: import.meta.dirname + "/index.html", pattern: import.meta.dirname + "/pattern.html" },
      output: {
        entryFileNames: "assets/renderer-[hash].js",
        assetFileNames: "assets/[name]-[hash][extname]",
      },
    },
  },
});

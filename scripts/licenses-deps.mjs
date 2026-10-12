// The dependency fingerprint stored in src/lib/licenses/licenses.json (`depsHash`): a hash of the
// declared dependencies that ship (the app's production npm dependencies, the Svelte/SvelteKit
// runtime compiled into the bundle, the @fontsource fonts of the app and the renderer, and the
// normal dependency sections of src-tauri/Cargo.toml). Shared by scripts/licenses.mjs and the
// vitest that fails when the manifests changed without `pnpm licenses`.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/** Dev dependencies whose runtime code is compiled into the app bundle. */
export const BUNDLED_DEV = ["svelte", "@sveltejs/kit"];

const pick = (deps, keep) => Object.fromEntries(Object.entries(deps ?? {}).filter(([n]) => keep(n)).sort(([a], [b]) => a.localeCompare(b)));
const isFont = (n) => n.startsWith("@fontsource/");

/** Normal (not dev/build) dependency lines of a Cargo.toml, trimmed and sorted. */
export function cargoDependencyLines(toml) {
  const out = [];
  let on = false;
  for (const raw of toml.split(/\r?\n/)) {
    const line = raw.trim();
    const header = /^\[([^\]]+)\]$/.exec(line);
    if (header) {
      on = /(^|\.)dependencies$/.test(header[1]) && !/(dev|build)-dependencies$/.test(header[1]);
      if (on && header[1] !== "dependencies") out.push(`[${header[1]}]`);
      continue;
    }
    if (on && line && !line.startsWith("#")) out.push(line);
  }
  return out.sort();
}

/** sha256 (hex) over the declared shipped dependencies of the repo at `root`. */
export function depsFingerprint(root) {
  const app = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  const renderer = JSON.parse(readFileSync(join(root, "renderer", "package.json"), "utf8"));
  const data = {
    app: pick(app.dependencies, () => true),
    bundled: pick(app.devDependencies, (n) => BUNDLED_DEV.includes(n)),
    renderer: pick(renderer.dependencies, () => true),
    fonts: { ...pick(app.devDependencies, isFont), ...pick(renderer.devDependencies, isFont) },
    cargo: cargoDependencyLines(readFileSync(join(root, "src-tauri", "Cargo.toml"), "utf8")),
  };
  return createHash("sha256").update(JSON.stringify(data)).digest("hex");
}

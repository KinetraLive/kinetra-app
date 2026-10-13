// Checks a finished frontend build (SvelteKit's build/) for remote control's switch and UI:
//
//   node scripts/check-remote-build.mjs [build] --remote 0|1
//
// 0 (KINETRA_REMOTE unset or 0, as CI release builds are until kinetra-cloud's relay is live):
// none of the markers may appear; 1: they must (the UI was built in). CI runs it after every
// release build. Remote control comes in only through svelte.config.js's `$remote` alias.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** What only remote control's UI brings into a build: the switch's command, the panel's lock
 *  and Preferences' separate-show button. */
export const MARKERS = ["remote_set", "Lock remote take", "Make this a separate show"];

/** The markers present in `files` ({name, text}), with the first file each appears in. */
export function findMarkers(files) {
  const found = new Map();
  for (const f of files) {
    for (const m of MARKERS) {
      if (!found.has(m) && f.text.includes(m)) found.set(m, f.name);
    }
  }
  return found;
}

/** Problems for `remote` ("0" | "1") given the markers found (empty = fine). */
export function verdict(remote, found) {
  if (remote === "0") return [...found].map(([m, file]) => `build without remote control contains ${JSON.stringify(m)} (${file})`);
  if (remote === "1") return MARKERS.filter((m) => !found.has(m)).map((m) => `build with remote control lacks ${JSON.stringify(m)}`);
  throw new Error(`--remote must be 0 or 1, not ${JSON.stringify(remote)}`);
}

function readBuild(dir) {
  const out = [];
  const walk = (d) => {
    for (const name of readdirSync(d)) {
      const p = join(d, name);
      if (statSync(p).isDirectory()) walk(p);
      else if (/\.(js|css|html)$/.test(name)) out.push({ name: relative(dir, p), text: readFileSync(p, "utf8") });
    }
  };
  walk(dir);
  return out;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const remote = (args[args.indexOf("--remote") + 1] ?? "").trim() || "0";
  const dir = resolve(args.find((a, i) => !a.startsWith("--") && args[i - 1] !== "--remote") ?? "build");
  const files = readBuild(dir);
  if (files.length === 0) {
    console.log(`::error::no build output in ${dir}`);
    process.exit(1);
  }
  const problems = verdict(remote, findMarkers(files));
  for (const p of problems) console.log(`::error::${p}`);
  if (problems.length) process.exit(1);
  console.log(`remote control ${remote === "1" ? "in" : "not in"} the build: OK (${files.length} files checked)`);
}

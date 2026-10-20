// Checks a finished frontend build (SvelteKit's build/) for cloud backup's UI:
//
//   node scripts/check-cloud-build.mjs [build] --cloud 0|1
//
// 0 (KINETRA_CLOUD unset or 0, as CI release builds are until the production API has cloud
// projects switched on): none of the markers may appear; 1: they must (the UI was built in). CI
// runs it after every release build. Cloud backup comes in only through svelte.config.js's
// `$cloud` alias.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** What only cloud backup's UI brings into a build: Back up now's and restore's commands, the
 *  picker's card and the quit question. */
export const MARKERS = [
  "cloud_backup_now",
  "cloud_restore_start",
  "Download a project from your cloud backup",
  "Back up before quitting",
];

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

/** Problems for `cloud` ("0" | "1") given the markers found (empty = fine). */
export function verdict(cloud, found) {
  if (cloud === "0") return [...found].map(([m, file]) => `build without cloud backup contains ${JSON.stringify(m)} (${file})`);
  if (cloud === "1") return MARKERS.filter((m) => !found.has(m)).map((m) => `build with cloud backup lacks ${JSON.stringify(m)}`);
  throw new Error(`--cloud must be 0 or 1, not ${JSON.stringify(cloud)}`);
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
  const cloud = (args[args.indexOf("--cloud") + 1] ?? "").trim() || "0";
  const dir = resolve(args.find((a, i) => !a.startsWith("--") && args[i - 1] !== "--cloud") ?? "build");
  const files = readBuild(dir);
  if (files.length === 0) {
    console.log(`::error::no build output in ${dir}`);
    process.exit(1);
  }
  const problems = verdict(cloud, findMarkers(files));
  for (const p of problems) console.log(`::error::${p}`);
  if (problems.length) process.exit(1);
  console.log(`cloud backup ${cloud === "1" ? "in" : "not in"} the build: OK (${files.length} files checked)`);
}

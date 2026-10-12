// Checks a finished frontend build (SvelteKit's build/) for the CANARY badge:
//
//   node scripts/check-channel-build.mjs [build] --channel stable|canary
//
// stable: none of the badge's markers may appear (no canary text, class or color);
// canary: they must (the badge was built in). CI runs it after every release build.
// The badge comes in only through svelte.config.js's `$channel-badge` alias.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** What only the canary badge brings into a build. */
export const MARKERS = ["Experimental build", "canary-badge", "#f5c400"];

/** The markers present in `files` ({name, text}), with the first file each appears in. */
export function findMarkers(files) {
  const found = new Map();
  for (const f of files) {
    const text = f.text.toLowerCase();
    for (const m of MARKERS) {
      if (!found.has(m) && text.includes(m.toLowerCase())) found.set(m, f.name);
    }
  }
  return found;
}

/** Problems for `channel` given the markers found (empty = fine). */
export function verdict(channel, found) {
  if (channel === "stable") return [...found].map(([m, file]) => `stable build contains ${JSON.stringify(m)} (${file})`);
  if (channel === "canary") return MARKERS.filter((m) => !found.has(m)).map((m) => `canary build lacks ${JSON.stringify(m)}`);
  throw new Error(`Unknown channel ${JSON.stringify(channel)}`);
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
  const channel = args[args.indexOf("--channel") + 1];
  const dir = resolve(args.find((a, i) => !a.startsWith("--") && args[i - 1] !== "--channel") ?? "build");
  const files = readBuild(dir);
  if (files.length === 0) {
    console.log(`::error::no build output in ${dir}`);
    process.exit(1);
  }
  const problems = verdict(channel, findMarkers(files));
  for (const p of problems) console.log(`::error::${p}`);
  if (problems.length) process.exit(1);
  console.log(`${channel} build OK (${files.length} files checked)`);
}

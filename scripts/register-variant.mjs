#!/usr/bin/env node
// Registers a style look of a library preset in the preset's template.json.
//
//   node scripts/register-variant.mjs <presetId> <styleId>   one look
//   node scripts/register-variant.mjs --all                  re-sync every look on disk
//   add --check to change nothing and exit 1 if a template.json is out of date
//
// A look lives in presets/templates/<presetId>/variants/<styleId>/: template.html, template.css and
// vars.json ({ "<variableKey>": "<value>" }: the style's values for the template's variables).
// Registering writes `{ id: styleId, name: <the style's name in presets/library.json>, vars }` into
// the preset's `variants` list (added, or updated in place) and keeps every other entry as it is.
// Order: `classic` first, then the styles in library.json order, then anything else as it was.
// Only the `variants` array is rewritten, in the file's existing format and line endings.
//
// Safe for design agents working in parallel: each template.json is updated under a lock file
// (`template.json.lock`, created exclusively, stale after 30 s), read fresh inside the lock and
// written atomically (temp file + rename, retried while Windows reports the file busy).
//
// It refuses (exit 1) when the style is not in library.json, the preset is not a schema 2 library
// preset, the look has no template.html / template.css, or vars.json is missing, not an object of
// strings, or names a variable the template does not declare. Values themselves are checked by
// renderer/src/presets.test.ts and the Rust preset loader (`cargo test`).
import { closeSync, existsSync, openSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const PRESETS = join(ROOT, "presets");
const TEMPLATES = join(PRESETS, "templates");

const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

class Refused extends Error {}

function library() {
  return JSON.parse(readFileSync(join(PRESETS, "library.json"), "utf8"));
}

/** Runs `fn` holding `<file>.lock` (exclusive create; a lock older than 30 s is stale). */
function withLock(file, fn) {
  const lock = `${file}.lock`;
  const deadline = Date.now() + 20_000;
  for (;;) {
    try {
      closeSync(openSync(lock, "wx"));
      break;
    } catch (e) {
      if (e.code !== "EEXIST") throw e;
      try {
        if (Date.now() - statSync(lock).mtimeMs > 30_000) rmSync(lock, { force: true });
      } catch {
        // gone meanwhile
      }
      if (Date.now() > deadline) throw new Refused(`${lock} is held by another process (delete it if nothing is running)`);
      sleep(25 + Math.random() * 50);
    }
  }
  try {
    return fn();
  } finally {
    rmSync(lock, { force: true });
  }
}

/** Writes through a temp file and a rename, retrying while Windows reports the target busy. */
function writeAtomic(file, text) {
  const tmp = `${file}.${process.pid}.tmp`;
  writeFileSync(tmp, text);
  for (let i = 0; ; i++) {
    try {
      renameSync(tmp, file);
      return;
    } catch (e) {
      if (!["EBUSY", "EPERM", "EACCES"].includes(e.code) || i >= 40) {
        rmSync(tmp, { force: true });
        throw e;
      }
      sleep(50);
    }
  }
}

/** [start, end) of the JSON array that follows `"variants":` at the top level of the file. */
function variantsSpan(text) {
  const m = /\n([ \t]*)"variants"\s*:\s*\[/.exec(text);
  if (!m) return null;
  const start = m.index + m[0].length - 1;
  let depth = 0;
  let inStr = false;
  for (let i = start; i < text.length; i++) {
    const c = text[i];
    if (inStr) {
      if (c === "\\") i++;
      else if (c === '"') inStr = false;
    } else if (c === '"') inStr = true;
    else if (c === "[" || c === "{") depth++;
    else if ((c === "]" || c === "}") && --depth === 0) return { start, end: i + 1, indent: m[1] };
  }
  return null;
}

/** The look's vars.json, checked against the template's variables. */
function readVars(dir, meta, label) {
  const file = join(dir, "vars.json");
  if (!existsSync(file)) throw new Refused(`${label}: no vars.json`);
  let vars;
  try {
    vars = JSON.parse(readFileSync(file, "utf8"));
  } catch (e) {
    throw new Refused(`${label}: vars.json is not valid JSON (${e.message})`);
  }
  if (!vars || typeof vars !== "object" || Array.isArray(vars)) throw new Refused(`${label}: vars.json must be an object`);
  const declared = new Set((meta.variables || []).map((v) => v.key));
  for (const [k, v] of Object.entries(vars)) {
    if (typeof v !== "string") throw new Refused(`${label}: vars.json value of "${k}" must be a string`);
    if (!declared.has(k)) throw new Refused(`${label}: vars.json sets "${k}", which the template does not declare`);
  }
  return vars;
}

/**
 * Adds or updates `style` in preset `id`. Returns "added" | "updated" | "unchanged" (with `check`,
 * nothing is written).
 */
export function register(id, style, { check = false, lib = library() } = {}) {
  const label = `${id}/${style}`;
  const s = lib.styles.find((x) => x.id === style);
  if (!s) throw new Refused(`${label}: "${style}" is not a style in presets/library.json`);
  const entry = lib.presets.find((p) => p.id === id);
  if (!entry || entry.legacy || !entry.kind) throw new Refused(`${label}: "${id}" is not a library preset with a type`);
  const dir = join(TEMPLATES, id, "variants", style);
  for (const f of ["template.html", "template.css"])
    if (!existsSync(join(dir, f)) || !readFileSync(join(dir, f), "utf8").trim()) throw new Refused(`${label}: variants/${style}/${f} is missing or empty`);
  const file = join(TEMPLATES, id, "template.json");
  const run = () => {
    const text = readFileSync(file, "utf8");
    const meta = JSON.parse(text);
    if (meta.schemaVersion !== 2 || !Array.isArray(meta.variants)) throw new Refused(`${label}: template.json is not schema 2 with variants`);
    const vars = readVars(dir, meta, label);
    const next = { id: style, name: s.name, vars };
    const old = meta.variants.find((v) => v.id === style);
    const rank = (v) => {
      if (v.id === "classic") return -1;
      const i = lib.styles.findIndex((x) => x.id === v.id);
      return i < 0 ? lib.styles.length : i;
    };
    const list = meta.variants.map((v) => (v.id === style ? next : v));
    if (!old) list.push(next);
    // Stable: unknown ids keep their relative order after the styles.
    const sorted = list.map((v, i) => [v, i]).sort((a, b) => rank(a[0]) - rank(b[0]) || a[1] - b[1]).map(([v]) => v);
    const span = variantsSpan(text);
    if (!span) throw new Refused(`${label}: no "variants" array found in template.json`);
    const eol = text.includes("\r\n") ? "\r\n" : "\n";
    const body = JSON.stringify(sorted, null, 2).replace(/\n/g, eol + span.indent);
    const out = text.slice(0, span.start) + body + text.slice(span.end);
    // Everything but the variants list is untouched.
    const parsed = JSON.parse(out);
    if (JSON.stringify({ ...parsed, variants: null }) !== JSON.stringify({ ...meta, variants: null })) throw new Error(`${label}: internal error, other keys changed`);
    if (out === text) return "unchanged";
    if (!check) writeAtomic(file, out);
    return old ? "updated" : "added";
  };
  return check ? run() : withLock(file, run);
}

/** Every look on disk: [presetId, styleId] for each library preset × style folder (not classic). */
export function looksOnDisk(lib = library()) {
  const styles = new Set(lib.styles.map((s) => s.id));
  const out = [];
  for (const p of lib.presets) {
    if (p.legacy || !p.kind) continue;
    const dir = join(TEMPLATES, p.id, "variants");
    if (!existsSync(dir)) continue;
    for (const v of readdirSync(dir)) {
      if (v === "classic" || !statSync(join(dir, v)).isDirectory()) continue;
      if (!styles.has(v)) {
        console.warn(`skip ${p.id}/${v}: not a style in library.json`);
        continue;
      }
      out.push([p.id, v]);
    }
  }
  return out;
}

function main(argv) {
  const check = argv.includes("--check");
  const args = argv.filter((a) => a !== "--check");
  const lib = library();
  let jobs;
  if (args.length === 1 && args[0] === "--all") jobs = looksOnDisk(lib);
  else if (args.length === 2 && !args.some((a) => a.startsWith("-"))) jobs = [args];
  else {
    console.error("usage: node scripts/register-variant.mjs <presetId> <styleId> [--check]\n       node scripts/register-variant.mjs --all [--check]");
    return 2;
  }
  let failed = 0;
  let stale = 0;
  for (const [id, style] of jobs) {
    try {
      const r = register(id, style, { check, lib });
      if (r !== "unchanged") stale++;
      console.log(`${id}/${style}: ${check && r !== "unchanged" ? `out of date (would be ${r})` : r}`);
    } catch (e) {
      failed++;
      console.error(e instanceof Refused ? `refused: ${e.message}` : e);
    }
  }
  return failed || (check && stale) ? 1 : 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) process.exitCode = main(process.argv.slice(2));

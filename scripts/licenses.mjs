// `pnpm licenses`: writes src/lib/licenses/licenses.json, the third-party notices shown in
// Preferences > About. Four groups:
//   npm   the app's production dependencies (package.json `dependencies`, the renderer's too) and the
//         Svelte/SvelteKit runtime compiled into the bundle, walked transitively through pnpm's
//         node_modules/.pnpm layout (`dependencies` + installed `optionalDependencies`);
//   rust  crates reachable from the kinetra package over normal dependencies on the host target
//         (`cargo metadata --locked --offline --filter-platform <host>`), proc-macros included;
//   fonts the bundled @fontsource families (app UI and renderer).
//   ndi   the NDI® notices in legal/ndi/ (header license, NDI terms).
// License texts (LICENSE*, LICENCE*, COPYING*) are deduplicated into `texts`. The output is
// deterministic (sorted, no timestamp) and carries `depsHash` (scripts/licenses-deps.mjs), which a
// vitest compares with the manifests. No dependencies.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { BUNDLED_DEV, depsFingerprint } from "./licenses-deps.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(ROOT, "src", "lib", "licenses", "licenses.json");
/** Longest license file kept; longer files are cut with a note (none of today's come close). */
const MAX_TEXT = 64 * 1024;

const texts = [];
const textIndex = new Map();
function textRef(text) {
  const norm = text.replace(/\r\n?/g, "\n").replace(/[ \t]+$/gm, "").trim();
  const t = norm.length > MAX_TEXT ? `${norm.slice(0, MAX_TEXT)}\n…` : norm;
  if (!t) return null;
  let i = textIndex.get(t);
  if (i === undefined) {
    i = texts.length;
    texts.push(t);
    textIndex.set(t, i);
  }
  return i;
}

/** License files directly in `dir`, as `{ name, text }` references, sorted by file name. */
function licenseFiles(dir) {
  let names;
  try {
    names = readdirSync(dir);
  } catch {
    return [];
  }
  const out = [];
  for (const name of names.filter((n) => /^(licen[cs]e|copying)/i.test(n)).sort()) {
    const p = join(dir, name);
    if (!statSync(p).isFile()) continue;
    const text = textRef(readFileSync(p, "utf8"));
    if (text !== null) out.push({ name, text });
  }
  return out;
}

/**
 * Crates that ship no license file but carry the notice in a source file's header comment (MIT asks
 * for the notice to be included): crate name → that file.
 */
const NOTICE_IN_SOURCE = { qrcodegen: "src/lib.rs" };
/** Copyright holders named in the notice when the package's license file doesn't name one. */
const COPYRIGHT = { "kinetra-license": "Rørvik Medieteknikk" };

// The leading block comment of a source file, without the comment markers.
function sourceNotice(dir, file) {
  let src;
  try {
    src = readFileSync(join(dir, file), "utf8");
  } catch {
    return [];
  }
  const m = /^\s*\/\*([\s\S]*?)\*\//.exec(src);
  if (!m) return [];
  const lines = m[1].split(/\r?\n/).map((l) => l.replace(/^\s*\* ?/, ""));
  const text = textRef(lines.join("\n"));
  return text === null ? [] : [{ name: `${file} (notice)`, text }];
}

/** A repository field (string, shorthand or `{ url }`) as an https URL, or undefined. */
function repoUrl(repo) {
  let url = typeof repo === "string" ? repo : repo?.url;
  if (!url) return undefined;
  url = url.trim();
  if (/^[\w.-]+\/[\w.-]+$/.test(url)) url = `https://github.com/${url}`;
  url = url
    .replace(/^github:/, "https://github.com/")
    .replace(/^gitlab:/, "https://gitlab.com/")
    .replace(/^git\+/, "")
    .replace(/^git:\/\//, "https://")
    .replace(/^ssh:\/\/git@/, "https://")
    .replace(/^git@([^:]+):/, "https://$1/")
    .replace(/\.git$/, "")
    .replace(/\/$/, "");
  return /^https?:\/\//.test(url) ? url : undefined;
}

/** The `license` field of a package.json (SPDX string, legacy object or array). */
function npmLicense(pkg) {
  const l = pkg.license ?? pkg.licenses;
  if (typeof l === "string") return l;
  if (Array.isArray(l)) return l.map((x) => (typeof x === "string" ? x : x.type)).join(" OR ");
  if (l && typeof l === "object" && l.type) return l.type;
  return "UNLICENSED";
}

const byName = (a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version, undefined, { numeric: true });

/** Directory of package `name` resolved from the node_modules directory `nm`, or null. */
function resolveIn(nm, name) {
  const p = join(nm, ...name.split("/"));
  return existsSync(join(p, "package.json")) ? realpathSync(p) : null;
}

/** node_modules directory that holds the package at `dir` (pnpm: its siblings are its dependencies). */
function holder(dir, name) {
  let d = dir;
  for (const _ of name.split("/")) d = dirname(d);
  return d;
}

function npmGroup() {
  const app = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
  const renderer = JSON.parse(readFileSync(join(ROOT, "renderer", "package.json"), "utf8"));
  const roots = [
    ...Object.keys(app.dependencies ?? {}).map((n) => [join(ROOT, "node_modules"), n]),
    ...BUNDLED_DEV.filter((n) => app.devDependencies?.[n]).map((n) => [join(ROOT, "node_modules"), n]),
    ...Object.keys(renderer.dependencies ?? {}).map((n) => [join(ROOT, "renderer", "node_modules"), n]),
  ];
  const seen = new Map();
  const queue = [...roots];
  while (queue.length) {
    const [nm, name] = queue.shift();
    const dir = resolveIn(nm, name);
    if (!dir) {
      if (roots.some(([, n]) => n === name)) throw new Error(`${name} is not installed: run pnpm install`);
      continue;
    }
    if (seen.has(dir)) continue;
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const files = licenseFiles(dir);
    seen.set(dir, {
      name: pkg.name,
      version: pkg.version,
      license: npmLicense(pkg),
      ...(repoUrl(pkg.repository) ?? pkg.homepage ? { url: repoUrl(pkg.repository) ?? pkg.homepage } : {}),
      ...(files.length ? { files } : {}),
    });
    const base = holder(dir, pkg.name);
    for (const dep of [...Object.keys(pkg.dependencies ?? {}), ...Object.keys(pkg.optionalDependencies ?? {})]) {
      queue.push([base, dep]);
    }
  }
  return [...seen.values()].sort(byName);
}

function rustGroup() {
  const host = /host: (\S+)/.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))?.[1];
  if (!host) throw new Error("rustc -vV printed no host triple");
  const meta = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked", "--offline", "--filter-platform", host, "--manifest-path", join(ROOT, "src-tauri", "Cargo.toml")],
      { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
    ),
  );
  const pkgs = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const root = meta.resolve.root ?? meta.packages.find((p) => p.name === "kinetra")?.id;
  const seen = new Set([root]);
  const queue = [root];
  while (queue.length) {
    const node = nodes.get(queue.shift());
    for (const d of node?.deps ?? []) {
      if (!d.dep_kinds.some((k) => k.kind === null)) continue;
      if (seen.has(d.pkg)) continue;
      seen.add(d.pkg);
      queue.push(d.pkg);
    }
  }
  seen.delete(root);
  return [...seen]
    .map((id) => {
      const p = pkgs.get(id);
      const dir = dirname(p.manifest_path);
      let files = licenseFiles(dir);
      if (!files.length && NOTICE_IN_SOURCE[p.name]) files = sourceNotice(dir, NOTICE_IN_SOURCE[p.name]);
      const url = repoUrl(p.repository) ?? p.homepage ?? undefined;
      return {
        name: p.name,
        version: p.version,
        license: p.license ?? (p.license_file ? "Custom" : "UNLICENSED"),
        ...(COPYRIGHT[p.name] ? { copyright: COPYRIGHT[p.name] } : {}),
        ...(url ? { url } : {}),
        ...(files.length ? { files } : {}),
      };
    })
    .sort(byName);
}

function fontGroup() {
  const dirs = new Map();
  for (const base of [ROOT, join(ROOT, "renderer")]) {
    const pkg = JSON.parse(readFileSync(join(base, "package.json"), "utf8"));
    for (const name of Object.keys({ ...pkg.dependencies, ...pkg.devDependencies }).filter((n) => n.startsWith("@fontsource/"))) {
      const dir = resolveIn(join(base, "node_modules"), name);
      if (!dir) throw new Error(`${name} is not installed: run pnpm install`);
      dirs.set(dir, name);
    }
  }
  return [...dirs.keys()]
    .map((dir) => {
      const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
      const meta = existsSync(join(dir, "metadata.json")) ? JSON.parse(readFileSync(join(dir, "metadata.json"), "utf8")) : {};
      const files = licenseFiles(dir);
      const url = pkg.homepage ?? repoUrl(pkg.repository);
      return {
        name: meta.family ?? pkg.name,
        version: pkg.version,
        license: npmLicense(pkg),
        package: pkg.name,
        ...(url ? { url } : {}),
        ...(files.length ? { files } : {}),
      };
    })
    .sort(byName);
}

// NDI's notices (legal/ndi/): the MIT license of the SDK headers Kinetra's declarations come from,
// and the NDI terms for the NDI Runtime (not included; the user installs it).
function ndiGroup() {
  const dir = join(ROOT, "legal", "ndi");
  const file = (name, path) => ({ name, text: textRef(readFileSync(join(dir, path), "utf8")) });
  return [
    {
      name: "NDI® SDK headers",
      version: "6.3.0.3",
      license: "MIT",
      copyright: "Vizrt NDI AB",
      url: "https://ndi.video",
      files: [file("LICENSE", "HEADERS-LICENSE.txt")],
    },
    {
      name: "NDI® Runtime (not included)",
      version: "6",
      license: "NDI SDK License Agreement",
      copyright: "NDI, Inc. / Vizrt NDI AB",
      url: "https://ndi.video",
      files: [file("NDI terms", "NOTICE.md")],
    },
  ];
}

const groups = [
  { id: "npm", label: "App (npm)", items: npmGroup() },
  { id: "rust", label: "Core (Rust)", items: rustGroup() },
  { id: "fonts", label: "Fonts", items: fontGroup() },
  { id: "ndi", label: "NDI®", items: ndiGroup() },
];
const out = { schemaVersion: 1, depsHash: depsFingerprint(ROOT), groups, texts };
const json = `${JSON.stringify(out)}\n`;
writeFileSync(OUT, json);
const missing = groups.flatMap((g) => g.items.filter((i) => !i.files).map((i) => `${g.id}:${i.name}`));
console.log(
  `licenses: ${groups.map((g) => `${g.label} ${g.items.length}`).join(", ")}; ${texts.length} texts, ${(json.length / 1024).toFixed(1)} KB → ${OUT}`,
);
if (missing.length) console.log(`  without a license file (${missing.length}): ${missing.join(", ")}`);

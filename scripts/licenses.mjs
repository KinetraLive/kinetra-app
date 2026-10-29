// `pnpm licenses`: writes src/lib/licenses/licenses.json, the third-party notices shown in
// Preferences > About. Four groups:
//   npm   the app's production dependencies (package.json `dependencies`, the renderer's too) and the
//         Svelte/SvelteKit runtime compiled into the bundle, walked transitively through pnpm's
//         node_modules/.pnpm layout (`dependencies` + installed `optionalDependencies`);
//   rust  crates reachable from the kinetra package over normal dependencies on every target we
//         ship (`cargo metadata --locked --offline --filter-platform <target>`, TARGETS), proc-macros
//         included; a crate that ships on some platforms only carries `platforms` (the app shows
//         the running platform's, $lib/licenses `forPlatform`);
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
/**
 * Packages published without a license text: repository URL (as `repoUrl` gives it) → the text
 * kept in legal/licenses/ (its README says where each one comes from). Used only when the package
 * ships no license file of its own.
 */
const KEPT_LICENSES = {
  "https://github.com/madsmtm/objc2": "objc2-MIT.txt",
  "https://github.com/lukeed/polka": "polka-MIT.txt",
  "https://github.com/lukeed/sirv": "sirv-MIT.txt",
  "https://github.com/Rich-Harris/is-reference": "rich-harris-MIT.txt",
  "https://gitlab.com/Rich-Harris/locate-character": "rich-harris-MIT.txt",
  "https://github.com/dropbox/rust-alloc-no-stdlib": "rust-alloc-no-stdlib-BSD-3-Clause.txt",
  "https://github.com/knurling-rs/defmt": "defmt-MIT.txt",
  "https://github.com/servo/stylo": "MPL-2.0.txt",
  "https://github.com/wravery/webview2-rs": "webview2-rs-MIT.txt",
};

/** The kept license text for a package from repository `url`, as license file references. */
function keptLicense(url) {
  const file = url && KEPT_LICENSES[url];
  if (!file) return [];
  const text = textRef(readFileSync(join(ROOT, "legal", "licenses", file), "utf8"));
  return text === null ? [] : [{ name: `${file.replace(/\.txt$/, "")} (kept in Kinetra)`, text }];
}

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
    const url = repoUrl(pkg.repository) ?? pkg.homepage;
    let files = licenseFiles(dir);
    if (!files.length) files = keptLicense(url);
    seen.set(dir, {
      name: pkg.name,
      version: pkg.version,
      license: npmLicense(pkg),
      ...(url ? { url } : {}),
      ...(files.length ? { files } : {}),
    });
    const base = holder(dir, pkg.name);
    for (const dep of [...Object.keys(pkg.dependencies ?? {}), ...Object.keys(pkg.optionalDependencies ?? {})]) {
      queue.push([base, dep]);
    }
  }
  return [...seen.values()].sort(byName);
}

/** The targets we ship, per platform (macOS is a universal build: both architectures). */
const TARGETS = {
  windows: ["x86_64-pc-windows-msvc"],
  macos: ["aarch64-apple-darwin", "x86_64-apple-darwin"],
};

/** Crates reachable from kinetra over normal dependencies on `target`: package id → package. */
function cratesFor(target) {
  const meta = JSON.parse(
    execFileSync(
      "cargo",
      ["metadata", "--format-version", "1", "--locked", "--offline", "--filter-platform", target, "--manifest-path", join(ROOT, "src-tauri", "Cargo.toml")],
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
  return new Map([...seen].map((id) => [id, pkgs.get(id)]));
}

function rustGroup() {
  const platforms = Object.keys(TARGETS);
  /** package id → { package, platforms it ships on } */
  const all = new Map();
  for (const platform of platforms) {
    for (const target of TARGETS[platform]) {
      for (const [id, p] of cratesFor(target)) {
        const e = all.get(id) ?? { p, on: new Set() };
        e.on.add(platform);
        all.set(id, e);
      }
    }
  }
  return [...all.values()]
    .map(({ p, on }) => {
      const dir = dirname(p.manifest_path);
      let files = licenseFiles(dir);
      if (!files.length && NOTICE_IN_SOURCE[p.name]) files = sourceNotice(dir, NOTICE_IN_SOURCE[p.name]);
      const url = repoUrl(p.repository) ?? p.homepage ?? undefined;
      if (!files.length) files = keptLicense(url);
      return {
        name: p.name,
        version: p.version,
        license: p.license ?? (p.license_file ? "Custom" : "UNLICENSED"),
        ...(COPYRIGHT[p.name] ? { copyright: COPYRIGHT[p.name] } : {}),
        ...(url ? { url } : {}),
        ...(files.length ? { files } : {}),
        ...(on.size < platforms.length ? { platforms: platforms.filter((x) => on.has(x)) } : {}),
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
const only = (p) => groups[1].items.filter((i) => i.platforms?.length === 1 && i.platforms[0] === p).length;
console.log(
  `licenses: ${groups.map((g) => `${g.label} ${g.items.length}`).join(", ")} (Windows only ${only("windows")}, macOS only ${only("macos")}); ${texts.length} texts, ${(json.length / 1024).toFixed(1)} KB → ${OUT}`,
);
if (missing.length) console.log(`  without a license file (${missing.length}): ${missing.join(", ")}`);

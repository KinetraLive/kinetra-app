// The app version lives in three files: src-tauri/tauri.conf.json (what the installer and the
// updater use), src-tauri/Cargo.toml (+ Cargo.lock) and package.json. They must agree.
//
//   node scripts/version.mjs check [X.Y.Z]   all three agree (and equal X.Y.Z when given)
//   node scripts/version.mjs set X.Y.Z       writes all three (and Cargo.lock)
//   node scripts/version.mjs get             prints the version
//
// The pure functions are exported for the release scripts (.github/scripts/release.mjs).

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

/** A release version: X.Y.Z, no leading zeros, no prerelease. */
export const RELEASE_VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

/** The `[package]` version of a Cargo.toml. */
export function cargoVersion(toml) {
  const pkg = toml.split(/^\[/m).find((s) => s.startsWith("package]"));
  const m = pkg && /^version\s*=\s*"([^"]+)"/m.exec(pkg);
  if (!m) throw new Error("Cargo.toml has no [package] version");
  return m[1];
}

/** Cargo.toml with its `[package]` version replaced. */
export function setCargoVersion(toml, version) {
  const at = toml.search(/^\[package\]/m);
  if (at < 0) throw new Error("Cargo.toml has no [package] section");
  const end = toml.slice(at + 1).search(/^\[/m);
  const stop = end < 0 ? toml.length : at + 1 + end;
  const section = toml.slice(at, stop).replace(/^version\s*=\s*"[^"]+"/m, `version = "${version}"`);
  return toml.slice(0, at) + section + toml.slice(stop);
}

/** Cargo.lock with the `kinetra` package's version replaced. */
export function setLockVersion(lock, version) {
  return lock.replace(/(\[\[package\]\]\r?\nname = "kinetra"\r?\nversion = )"[^"]+"/, `$1"${version}"`);
}

/** `{ tauri, cargo, npm }` versions → the one version, or an error naming the mismatch. */
export function agreedVersion(v) {
  const values = new Set(Object.values(v));
  if (values.size !== 1) {
    throw new Error(
      `Versions disagree: tauri.conf.json ${v.tauri}, Cargo.toml ${v.cargo}, package.json ${v.npm}. ` +
        "Run pnpm run version:set X.Y.Z.",
    );
  }
  const [version] = values;
  if (!RELEASE_VERSION.test(version)) throw new Error(`The app version ${version} is not X.Y.Z`);
  return version;
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const files = {
  tauri: join(root, "src-tauri", "tauri.conf.json"),
  cargo: join(root, "src-tauri", "Cargo.toml"),
  lock: join(root, "src-tauri", "Cargo.lock"),
  npm: join(root, "package.json"),
};

/** The versions in the repo's three files. */
export function readVersions() {
  return {
    tauri: JSON.parse(readFileSync(files.tauri, "utf8")).version,
    cargo: cargoVersion(readFileSync(files.cargo, "utf8")),
    npm: JSON.parse(readFileSync(files.npm, "utf8")).version,
  };
}

/** Replaces `"version": "…"` (the first one: the top-level key in both JSON files). */
function setJsonVersion(path, version) {
  const text = readFileSync(path, "utf8");
  const next = text.replace(/"version":\s*"[^"]*"/, `"version": "${version}"`);
  if (JSON.parse(next).version !== version) throw new Error(`Could not set the version in ${path}`);
  writeFileSync(path, next);
}

function main([cmd, arg]) {
  if (cmd === "get") {
    console.log(agreedVersion(readVersions()));
  } else if (cmd === "check") {
    const version = agreedVersion(readVersions());
    if (arg !== undefined && arg.replace(/^v/, "") !== version) {
      throw new Error(`The tag says ${arg} but the app version is ${version}`);
    }
    console.log(`version ${version}`);
  } else if (cmd === "set") {
    if (!arg || !RELEASE_VERSION.test(arg)) throw new Error("Usage: version.mjs set X.Y.Z");
    setJsonVersion(files.tauri, arg);
    setJsonVersion(files.npm, arg);
    writeFileSync(files.cargo, setCargoVersion(readFileSync(files.cargo, "utf8"), arg));
    writeFileSync(files.lock, setLockVersion(readFileSync(files.lock, "utf8"), arg));
    console.log(`version ${agreedVersion(readVersions())}`);
  } else {
    throw new Error("Usage: version.mjs check [X.Y.Z] | set X.Y.Z | get");
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    main(process.argv.slice(2));
  } catch (e) {
    console.error(e.message);
    process.exit(1);
  }
}

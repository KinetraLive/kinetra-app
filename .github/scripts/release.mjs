// Release steps for the GitHub workflows (docs/RELEASING.md). The logic is in release-lib.mjs;
// this file reads and writes files and talks to download.kinetra.live.
//
//   version  --channel C [--tag vX.Y.Z | --dry-run] [--run N] → the build's version (GITHUB_OUTPUT)
//   validate --channel C --version V                         → fails unless V is a C version
//   config   --channel C --version V --updater B --sign-windows B --out F
//   collect  --platform P --channel C --version V --bundle-dir D --updater B --out D
//   notes    --channel C --version V [--fallback F] --out F
//   assemble --channel C --version V --commit SHA --in D --notes F --out D [--release]
//   verify   --channel C --version V [--dir D]               → fetches and checks the live files
//   rollback --channel C --version V --record F --out D
//   r2-endpoint                                              → the cleaned R2_ENDPOINT (stdout only)
//
// Never prints environment variables or secrets.

import { createHash } from "node:crypto";
import { appendFileSync, copyFileSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { agreedVersion, readVersions } from "../../scripts/version.mjs";
import * as lib from "./release-lib.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "..", "..");

function args(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const m = /^--([a-z-]+)$/.exec(argv[i]);
    if (!m) throw new Error(`Unexpected argument ${argv[i]}`);
    const next = argv[i + 1];
    if (next === undefined || next.startsWith("--")) out[m[1]] = "true";
    else out[m[1]] = argv[++i];
  }
  return out;
}

function need(a, key) {
  if (a[key] === undefined || a[key] === "") throw new Error(`Missing --${key}`);
  return a[key];
}

const bool = (v) => v === "true";

function output(key, value) {
  console.log(`${key}=${value}`);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${key}=${value}\n`);
}

function warn(message) {
  console.log(`::warning::${message}`);
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function listFiles(dir) {
  const out = [];
  const walk = (d) => {
    for (const e of readdirSync(d, { withFileTypes: true })) {
      const p = join(d, e.name);
      if (e.isDirectory()) walk(p);
      else out.push(relative(dir, p).split(sep).join("/"));
    }
  };
  walk(dir);
  return out.sort();
}

const tauriConf = () => JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));

const commands = {
  version(a) {
    const channel = need(a, "channel");
    const base = agreedVersion(readVersions());
    let version;
    if (channel === "stable" && bool(a["dry-run"]) && !a.tag) {
      version = base;
    } else if (channel === "stable") {
      version = lib.tagVersion(need(a, "tag"));
      if (version !== base) {
        throw new Error(`The tag is v${version} but the app version is ${base} (pnpm run version:set)`);
      }
    } else {
      version = lib.canaryVersion(base, new Date(), need(a, "run"));
    }
    lib.assertChannelVersion(channel, version);
    output("version", version);
  },

  validate(a) {
    lib.assertChannelVersion(need(a, "channel"), need(a, "version"));
  },

  config(a) {
    const channel = need(a, "channel");
    lib.channelOf(channel);
    const overlay =
      channel === "canary"
        ? JSON.parse(readFileSync(join(root, "src-tauri", "tauri.canary.conf.json"), "utf8"))
        : {};
    const config = lib.ciConfig({
      channelOverlay: overlay,
      version: need(a, "version"),
      updater: bool(a.updater),
      signScript: bool(a["sign-windows"]) ? join(here, "sign-windows.mjs").split(sep).join("/") : null,
    });
    writeFileSync(need(a, "out"), JSON.stringify(config, null, 2));
    console.log(`config for ${channel} ${config.version}: updater ${bool(a.updater)}, Windows signing ${bool(a["sign-windows"])}`);
  },

  collect(a) {
    const platform = need(a, "platform");
    const channel = need(a, "channel");
    const version = need(a, "version");
    const dir = need(a, "bundle-dir");
    const out = need(a, "out");
    const updater = bool(a.updater);
    const picked = lib.pickBundles(listFiles(dir), platform, updater);
    const names = lib.fileNames(channel, version);
    const pubkeyId = lib.minisignKeyId(tauriConf().plugins.updater.pubkey);
    mkdirSync(out, { recursive: true });
    const copy = (from, to, versioned) => {
      if (!from) return;
      if (versioned && !from.includes(version)) {
        throw new Error(`${from} is not version ${version}: the config overlay didn't apply`);
      }
      copyFileSync(join(dir, from), join(out, to));
      console.log(`${from} → ${to} (${statSync(join(out, to)).size} bytes)`);
    };
    const copySig = (from, to) => {
      if (!from) return;
      const id = lib.minisignKeyId(readFileSync(join(dir, from), "utf8"));
      if (id !== pubkeyId) {
        throw new Error(`${from} is signed with key ${id}, but the app trusts ${pubkeyId}`);
      }
      copy(from, to, false);
    };
    if (platform === "windows") {
      copy(picked.installer, names.installer, true);
      copySig(picked.installerSig, `${names.installer}.sig`);
    } else {
      copy(picked.dmg, names.dmg, true);
      copy(picked.app, names.app, false);
      copySig(picked.appSig, `${names.app}.sig`);
    }
  },

  notes(a) {
    let changelog = "";
    try {
      changelog = readFileSync(join(root, "CHANGELOG.md"), "utf8");
    } catch {
      // no changelog: the fallback decides
    }
    const fallback = a.fallback ? readFileSync(a.fallback, "utf8") : "";
    const notes = lib.releaseNotes({
      channel: need(a, "channel"),
      version: need(a, "version"),
      changelog,
      fallback,
    });
    writeFileSync(need(a, "out"), notes);
    console.log(`notes: ${notes.length} characters`);
  },

  assemble(a) {
    const channel = need(a, "channel");
    const version = need(a, "version");
    lib.assertChannelVersion(channel, version);
    const input = need(a, "in");
    const out = need(a, "out");
    const release = bool(a.release);
    const names = lib.fileNames(channel, version);
    const have = new Set(listFiles(input));
    const present = (n) => have.has(n);

    if (!present(names.installer)) throw new Error(`Missing the Windows installer ${names.installer}`);
    const macos = present(names.dmg);
    if (!macos) warn("No macOS build: this release has Windows only.");
    const updater = present(`${names.installer}.sig`);
    if (!updater) {
      if (release) throw new Error("No updater signature: TAURI_SIGNING_PRIVATE_KEY is required for a release");
      warn("No updater signature (TAURI_SIGNING_PRIVATE_KEY missing): no update manifest.");
    }
    if (macos && updater && !(present(names.app) && present(`${names.app}.sig`))) {
      throw new Error(`Missing the macOS updater archive ${names.app} or its signature`);
    }

    const files = [...have].filter((n) => Object.values(names).some((b) => n === b || n === `${b}.sig`));
    mkdirSync(join(out, "files"), { recursive: true });
    const info = {};
    for (const n of files) {
      copyFileSync(join(input, n), join(out, "files", n));
      info[n] = { sha256: sha256File(join(out, "files", n)), size: statSync(join(out, "files", n)).size };
    }
    writeFileSync(join(out, "SHA256SUMS"), lib.sha256sums(files.map((name) => ({ name, sha256: info[name].sha256 }))));

    const notes = readFileSync(need(a, "notes"), "utf8");
    const pubDate = new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
    const url = (n) => lib.fileUrl(channel, version, n);
    const sig = (n) => (present(`${n}.sig`) ? readFileSync(join(input, `${n}.sig`), "utf8").trim() : null);
    const download = (n) => ({ name: n, url: url(n), sha256: info[n].sha256, size: info[n].size });

    const commit = need(a, "commit");
    if (!/^[0-9a-f]{7,40}$/.test(commit)) throw new Error("--commit must be a commit hash");
    const latest = lib.latestJson({
      channel,
      version,
      commit,
      pubDate,
      notes,
      files: { windows: download(names.installer), macos: macos ? download(names.dmg) : null },
    });
    writeFileSync(join(out, "latest.json"), JSON.stringify(latest, null, 2));
    if (!updater) {
      console.log(`assembled ${files.length} files (no manifest)`);
      return;
    }
    const manifest = lib.updaterManifest({
      version,
      notes,
      pubDate,
      windows: { url: url(names.installer), signature: sig(names.installer) },
      macos: macos && present(names.app) ? { url: url(names.app), signature: sig(names.app) } : null,
    });
    writeFileSync(join(out, "manifest.json"), JSON.stringify(manifest, null, 2));
    const record = { channel, version, commit, pub_date: pubDate, files: files.sort(), manifest, latest };
    lib.checkReleaseRecord(record, channel, version);
    writeFileSync(join(out, "release.json"), JSON.stringify(record, null, 2));
    writeFileSync(join(out, "plan.tsv"), lib.planTsv(lib.uploadPlan({ channel, version, files: record.files })));
    console.log(`assembled ${files.length} files; platforms: ${Object.keys(manifest.platforms).join(", ")}`);
  },

  async verify(a) {
    const channel = need(a, "channel");
    const version = need(a, "version");
    lib.assertChannelVersion(channel, version);
    const local = a.dir ? readFileSync(join(a.dir, "SHA256SUMS"), "utf8") : null;
    const deadline = Date.now() + 3 * 60_000;
    for (let attempt = 1; ; attempt++) {
      try {
        await verifyOnce(channel, version, local);
        return;
      } catch (e) {
        if (Date.now() > deadline) throw e;
        console.log(`attempt ${attempt}: ${e.message}; retrying in 15 s`);
        await new Promise((r) => setTimeout(r, 15_000));
      }
    }
  },

  rollback(a) {
    const channel = need(a, "channel");
    const version = need(a, "version");
    lib.assertChannelVersion(channel, version);
    const record = JSON.parse(readFileSync(need(a, "record"), "utf8"));
    lib.checkReleaseRecord(record, channel, version);
    const out = need(a, "out");
    mkdirSync(out, { recursive: true });
    writeFileSync(join(out, "manifest.json"), JSON.stringify(record.manifest, null, 2));
    writeFileSync(join(out, "latest.json"), JSON.stringify(record.latest, null, 2));
    writeFileSync(join(out, "plan.tsv"), lib.planTsv(lib.pointerPlan(channel)));
    writeFileSync(
      join(out, "keys.txt"),
      [...record.files, "SHA256SUMS"].map((f) => `releases/${channel}/${version}/${f}`).join("\n") + "\n",
    );
    console.log(`rollback of ${channel} to ${version}: ${record.files.length} files to check`);
  },
};

async function get(url) {
  const res = await fetch(url, { headers: { "cache-control": "no-cache" } });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return res;
}

async function verifyOnce(channel, version, localSums) {
  const base = lib.BASE_URL;
  const manifest = await (await get(`${base}/${lib.channelOf(channel).manifest}`)).json();
  if (manifest.version !== version) throw new Error(`the manifest says ${manifest.version}`);
  const latest = await (await get(`${base}/releases/${channel}/latest.json`)).json();
  if (latest.version !== version) throw new Error(`latest.json says ${latest.version}`);
  const sumsText = await (await get(lib.fileUrl(channel, version, "SHA256SUMS"))).text();
  if (localSums !== null && sumsText !== localSums) throw new Error("SHA256SUMS differs from the built one");
  const sums = lib.parseSha256sums(sumsText);
  const url = manifest.platforms["windows-x86_64"]?.url;
  if (!url) throw new Error("the manifest has no Windows update");
  const name = decodeURIComponent(url.slice(url.lastIndexOf("/") + 1));
  const body = Buffer.from(await (await get(url)).arrayBuffer());
  const hash = createHash("sha256").update(body).digest("hex");
  if (sums.get(name) !== hash) throw new Error(`${name}: checksum ${hash} doesn't match SHA256SUMS`);
  console.log(`verified ${channel} ${version}: manifest, latest.json, ${name} (${body.length} bytes, sha256 ${hash})`);
}

const [cmd, ...rest] = process.argv.slice(2);
if (cmd === "r2-endpoint") {
  // Used as $(…): stdout carries only the URL; notes and errors go to stderr, never the value.
  try {
    const { url, notes } = lib.r2Endpoint(process.env.R2_ENDPOINT);
    for (const n of notes) console.error(`note: ${n}`);
    process.stdout.write(url);
    process.exit(0);
  } catch (e) {
    console.error(`error: ${e.message}`);
    process.exit(1);
  }
}
try {
  if (!Object.hasOwn(commands, cmd ?? "")) throw new Error(`Unknown command ${cmd}`);
  await commands[cmd](args(rest));
} catch (e) {
  console.log(`::error::${e.message}`);
  process.exit(1);
}

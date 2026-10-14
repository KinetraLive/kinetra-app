// Pure release logic (docs/RELEASING.md): versions, file names, notes, checksums, the Tauri
// updater manifest, latest.json and the upload plan. No I/O here; release.mjs does that.
// Tests: release-lib.test.mjs (pnpm test).

export const BASE_URL = "https://download.kinetra.live";
export const BUCKET = "kinetra-downloads";

export const CHANNELS = {
  stable: { manifest: "updates/latest.json", fileBase: "Kinetra" },
  canary: { manifest: "updates/latest-canary.json", fileBase: "Kinetra-Canary" },
};

export const IMMUTABLE = "public, max-age=31536000, immutable";
export const SHORT = "public, max-age=60";

const RELEASE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const CANARY = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)-canary\.(\d{8})\.([1-9]\d*)$/;

export function channelOf(name) {
  if (!Object.hasOwn(CHANNELS, name)) throw new Error(`Unknown channel ${JSON.stringify(name)}`);
  return CHANNELS[name];
}

/** True when `version` is a valid version of `channel` (X.Y.Z; X.Y.Z-canary.YYYYMMDD.N). */
export function isChannelVersion(channel, version) {
  channelOf(channel);
  return (channel === "stable" ? RELEASE : CANARY).test(String(version));
}

export function assertChannelVersion(channel, version) {
  if (!isChannelVersion(channel, version)) {
    throw new Error(`${JSON.stringify(version)} is not a ${channel} version`);
  }
}

/** `v1.2.3` → `1.2.3`. */
export function tagVersion(tag) {
  const m = /^v(.+)$/.exec(String(tag));
  if (!m || !RELEASE.test(m[1])) throw new Error(`The tag ${JSON.stringify(tag)} is not vX.Y.Z`);
  return m[1];
}

/** `X.Y.Z-canary.YYYYMMDD.N` (UTC date; N = the workflow run number, always increasing). */
export function canaryVersion(base, date, run) {
  if (!RELEASE.test(base)) throw new Error(`The app version ${base} is not X.Y.Z`);
  const n = Number(run);
  if (!Number.isSafeInteger(n) || n < 1) throw new Error(`Bad run number ${JSON.stringify(run)}`);
  const ymd = date.toISOString().slice(0, 10).replaceAll("-", "");
  return `${base}-canary.${ymd}.${n}`;
}

/** The release files' names: space-free, so URLs need no escaping. */
export function fileNames(channel, version) {
  const b = `${channelOf(channel).fileBase}_${version}`;
  return {
    installer: `${b}_x64-setup.exe`,
    dmg: `${b}_universal.dmg`,
    app: `${b}_universal.app.tar.gz`,
  };
}

export function fileUrl(channel, version, name, base = BASE_URL) {
  return `${base}/releases/${channel}/${version}/${encodeURIComponent(name)}`;
}

/**
 * The body of a `## <heading>` section of CHANGELOG.md (`## 0.2.0`, `## [0.2.0] - 2026-10-11`,
 * `## Unreleased`), trimmed; "" when the section is missing or empty.
 */
export function changelogSection(text, heading) {
  const lines = String(text).split(/\r?\n/);
  const esc = heading.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const start = new RegExp(`^##\\s+\\[?${esc}\\]?(\\s|$)`, "i");
  const i = lines.findIndex((l) => start.test(l));
  if (i < 0) return "";
  const rest = lines.slice(i + 1);
  const end = rest.findIndex((l) => /^##\s/.test(l));
  return (end < 0 ? rest : rest.slice(0, end)).join("\n").trim();
}

const MAX_NOTES = 4000;

/**
 * Release notes. Stable: the version's CHANGELOG section, else the tag message (required).
 * Canary: the Unreleased section, else the commit subjects.
 */
export function releaseNotes({ channel, version, changelog, fallback }) {
  const section =
    channel === "stable"
      ? changelogSection(changelog, version)
      : changelogSection(changelog, "Unreleased");
  let notes = section || String(fallback ?? "").trim();
  if (!notes && channel === "stable") {
    throw new Error(`No notes: add a "## ${version}" section to CHANGELOG.md or annotate the tag`);
  }
  if (notes.length > MAX_NOTES) notes = `${notes.slice(0, MAX_NOTES - 1).trimEnd()}…`;
  return notes;
}

/**
 * What's wrong with the next canary's notes, or null: "empty" (no Unreleased text, so the commit
 * subjects would be used) or "unchanged" (the same notes as the published canary).
 */
export function canaryNotesIssue(changelog, previousNotes) {
  if (!changelogSection(changelog, "Unreleased")) return "empty";
  const norm = (s) => String(s ?? "").replace(/\s+/g, " ").trim();
  const next = releaseNotes({ channel: "canary", version: "", changelog, fallback: "" });
  return norm(next) === norm(previousNotes) ? "unchanged" : null;
}

/** RFC 7396 JSON merge patch (what `tauri build --config` does with an overlay). */
export function mergePatch(target, patch) {
  if (patch === null || typeof patch !== "object" || Array.isArray(patch)) return patch;
  const out =
    target !== null && typeof target === "object" && !Array.isArray(target) ? { ...target } : {};
  for (const [k, v] of Object.entries(patch)) {
    if (v === null) delete out[k];
    else out[k] = mergePatch(out[k], v);
  }
  return out;
}

/**
 * The config overlay CI passes to `tauri build --config`: the channel overlay (canary), the
 * build's version, updater artifacts when the signing key is there, and the Windows sign hook
 * when Azure Artifact Signing is set up (the hook reads its settings from the environment, so
 * none of them is written into the file).
 */
export function ciConfig({ channelOverlay, version, updater, signScript }) {
  let c = mergePatch({}, channelOverlay ?? {});
  c = mergePatch(c, { version, bundle: { createUpdaterArtifacts: Boolean(updater) } });
  if (signScript) {
    if (/\s/.test(signScript)) throw new Error("The sign script path can't contain spaces");
    c = mergePatch(c, { bundle: { windows: { signCommand: `node ${signScript} %1` } } });
  }
  return c;
}

/**
 * Picks a platform's bundles from the files `tauri build` wrote (paths relative to the bundle
 * folder, `/`-separated). Windows: the NSIS `-setup.exe`; macOS: the `.dmg` and the updater
 * `.app.tar.gz`; each updater file with its `.sig` when `updater`.
 */
export function pickBundles(files, platform, updater) {
  const one = (what, test) => {
    const hits = files.filter(test);
    if (hits.length !== 1) {
      throw new Error(`Expected one ${what}, found ${hits.length ? hits.join(", ") : "none"}`);
    }
    return hits[0];
  };
  const sig = (f) => (updater ? one(`${f}.sig`, (p) => p === `${f}.sig`) : null);
  if (platform === "windows") {
    const installer = one("NSIS installer", (p) => p.startsWith("nsis/") && p.endsWith("-setup.exe"));
    return { installer, installerSig: sig(installer) };
  }
  if (platform === "macos") {
    const dmg = one("disk image", (p) => p.startsWith("dmg/") && p.endsWith(".dmg"));
    if (!updater) return { dmg, app: null, appSig: null };
    const app = one("updater archive", (p) => p.startsWith("macos/") && p.endsWith(".app.tar.gz"));
    return { dmg, app, appSig: sig(app) };
  }
  throw new Error(`Unknown platform ${JSON.stringify(platform)}`);
}

/**
 * The key id of a minisign public key or signature as Tauri stores them (base64 of the minisign
 * file: an untrusted comment line, then base64 of algorithm (2 bytes) + key id (8 bytes) + …).
 */
export function minisignKeyId(tauriBase64) {
  const text = Buffer.from(String(tauriBase64).trim(), "base64").toString("utf8");
  const lines = text.split(/\r?\n/);
  if (!/^untrusted comment:/.test(lines[0] ?? "") || !lines[1]) {
    throw new Error("Not a minisign key or signature");
  }
  const raw = Buffer.from(lines[1].trim(), "base64");
  const alg = raw.subarray(0, 2).toString("latin1");
  if (raw.length < 42 || (alg !== "Ed" && alg !== "ED")) {
    throw new Error("Not a minisign key or signature");
  }
  // Stored little-endian; minisign shows it reversed (the "public key: …" comment).
  return Buffer.from(raw.subarray(2, 10)).reverse().toString("hex").toUpperCase();
}

/** `sha256  name` lines, sorted by name (the `sha256sum -c` format). */
export function sha256sums(entries) {
  return (
    [...entries]
      .sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))
      .map((e) => `${e.sha256}  ${e.name}`)
      .join("\n") + "\n"
  );
}

export function parseSha256sums(text) {
  const out = new Map();
  for (const line of String(text).split(/\r?\n/)) {
    if (!line.trim()) continue;
    const m = /^([0-9a-f]{64}) [ *](.+)$/.exec(line);
    if (!m) throw new Error(`Bad SHA256SUMS line: ${line}`);
    out.set(m[2], m[1]);
  }
  return out;
}

/**
 * The Tauri v2 static updater manifest. macOS is one universal archive, listed for both
 * architectures. A platform without a signature is left out.
 */
export function updaterManifest({ version, notes, pubDate, windows, macos }) {
  const platforms = {};
  if (windows?.signature) {
    platforms["windows-x86_64"] = { signature: windows.signature, url: windows.url };
  }
  if (macos?.signature) {
    for (const arch of ["darwin-aarch64", "darwin-x86_64"]) {
      platforms[arch] = { signature: macos.signature, url: macos.url };
    }
  }
  if (!platforms["windows-x86_64"]) throw new Error("The manifest needs the Windows update");
  return { version, notes, pub_date: pubDate, platforms };
}

/** `releases/<channel>/latest.json`: direct installer links for the website. */
export function latestJson({ channel, version, commit, pubDate, notes, files }) {
  const out = { channel, version, commit: commit ?? null, pub_date: pubDate, notes, files: {} };
  for (const platform of ["windows", "macos"]) {
    const f = files[platform];
    if (f) out.files[platform] = { name: f.name, url: f.url, sha256: f.sha256, size: f.size };
  }
  if (!out.files.windows) throw new Error("latest.json needs the Windows installer");
  return out;
}

/**
 * Everything a release uploads, in order: the files (immutable), SHA256SUMS, release.json, then
 * releases/<channel>/latest.json and the updater manifest LAST (60 s), so a client never sees a
 * manifest that points at files not uploaded yet. Entries: {path (local, relative to the
 * assembled folder), key, cacheControl, contentType}.
 */
export function uploadPlan({ channel, version, files }) {
  const dir = `releases/${channel}/${version}`;
  const plan = files.map((name) => ({
    path: `files/${name}`,
    key: `${dir}/${name}`,
    cacheControl: IMMUTABLE,
    contentType: "application/octet-stream",
  }));
  plan.push(
    { path: "SHA256SUMS", key: `${dir}/SHA256SUMS`, cacheControl: IMMUTABLE, contentType: "text/plain; charset=utf-8" },
    { path: "release.json", key: `${dir}/release.json`, cacheControl: IMMUTABLE, contentType: "application/json" },
    ...pointerPlan(channel),
  );
  return plan;
}

/** The two mutable pointers (latest.json, then the manifest), as release and rollback write them. */
export function pointerPlan(channel) {
  return [
    { path: "latest.json", key: `releases/${channel}/latest.json`, cacheControl: SHORT, contentType: "application/json" },
    { path: "manifest.json", key: channelOf(channel).manifest, cacheControl: SHORT, contentType: "application/json" },
  ];
}

/** A plan as tab-separated lines for the shell loop that uploads it. */
export function planTsv(plan) {
  return plan.map((p) => [p.path, p.key, p.cacheControl, p.contentType].join("\t")).join("\n") + "\n";
}

/**
 * Checks a stored release.json before a rollback points the channel at it: the channel and
 * version match, and the manifest and latest.json only link to this release's files.
 */
export function checkReleaseRecord(record, channel, version, base = BASE_URL) {
  if (record?.channel !== channel || record?.version !== version) {
    throw new Error(`release.json is for ${record?.channel} ${record?.version}, not ${channel} ${version}`);
  }
  if (!Array.isArray(record.files) || !record.manifest || !record.latest) {
    throw new Error("release.json is incomplete");
  }
  const prefix = `${base}/releases/${channel}/${version}/`;
  const urls = [
    ...Object.values(record.manifest.platforms ?? {}).map((p) => p.url),
    ...Object.values(record.latest.files ?? {}).map((f) => f.url),
  ];
  if (!record.manifest.platforms?.["windows-x86_64"]) throw new Error("release.json has no Windows update");
  for (const url of urls) {
    if (!url.startsWith(prefix) || !record.files.includes(decodeURIComponent(url.slice(prefix.length)))) {
      throw new Error(`release.json links outside the release: ${url}`);
    }
  }
  if (record.manifest.version !== version || record.latest.version !== version) {
    throw new Error("release.json's manifest is for another version");
  }
}

/**
 * The R2 S3 endpoint from the R2_ENDPOINT secret: `https://<account id>.r2.cloudflarestorage.com`
 * (or a jurisdiction host such as `<id>.eu.r2…`). Tolerates what pasting often adds (surrounding
 * whitespace, a trailing newline, the bucket as a path); anything else is an error that describes
 * the problem without repeating the value.
 */
export function r2Endpoint(raw) {
  const value = String(raw ?? "").trim();
  if (!value) throw new Error("R2_ENDPOINT is empty");
  if (/\s/.test(value)) throw new Error("R2_ENDPOINT has spaces or line breaks inside it");
  let url;
  try {
    url = new URL(value);
  } catch {
    throw new Error("R2_ENDPOINT is not a URL (expected https://<account-id>.r2.cloudflarestorage.com)");
  }
  if (url.protocol !== "https:") throw new Error("R2_ENDPOINT must start with https://");
  if (!/^[0-9a-f]{32}(\.[a-z]+)?\.r2\.cloudflarestorage\.com$/.test(url.hostname)) {
    throw new Error(
      "R2_ENDPOINT's host must be <account-id>.r2.cloudflarestorage.com (the S3 API URL in R2 › Overview, not the public bucket URL)",
    );
  }
  const notes = [];
  if (String(raw) !== value) notes.push("R2_ENDPOINT had surrounding whitespace or a line break; trimmed");
  if (url.pathname !== "/" || url.search || url.username) notes.push("R2_ENDPOINT had a path (the bucket?); using the host only");
  return { url: `https://${url.hostname}`, notes };
}

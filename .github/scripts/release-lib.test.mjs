import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import * as lib from "./release-lib.mjs";

const conf = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
const canary = JSON.parse(readFileSync("src-tauri/tauri.canary.conf.json", "utf8"));

/** A Tauri-style .sig for key id `id` (as minisign shows it). */
function fakeSig(id) {
  const raw = Buffer.concat([Buffer.from("ED"), Buffer.from(id, "hex").reverse(), Buffer.alloc(64, 7)]);
  const text = `untrusted comment: signature from tauri secret key\n${raw.toString("base64")}\ntrusted comment: timestamp:1\tfile:x\nAAAA\n`;
  return Buffer.from(text).toString("base64");
}

describe("versions", () => {
  it("accepts each channel's versions only", () => {
    expect(lib.isChannelVersion("stable", "0.2.0")).toBe(true);
    expect(lib.isChannelVersion("stable", "0.2.0-canary.20261010.3")).toBe(false);
    expect(lib.isChannelVersion("stable", "01.2.0")).toBe(false);
    expect(lib.isChannelVersion("canary", "0.2.0-canary.20261010.3")).toBe(true);
    expect(lib.isChannelVersion("canary", "0.2.0")).toBe(false);
    expect(lib.isChannelVersion("canary", "0.2.0-canary.20261010.0")).toBe(false);
    expect(lib.isChannelVersion("canary", "0.2.0-canary.2026101.3")).toBe(false);
    expect(() => lib.isChannelVersion("beta", "0.2.0")).toThrow(/Unknown channel/);
  });

  it("reads tags and builds canary versions in UTC", () => {
    expect(lib.tagVersion("v1.2.3")).toBe("1.2.3");
    for (const bad of ["1.2.3", "v1.2", "v1.2.3-rc.1", "v1.2.3; rm -rf /"]) {
      expect(() => lib.tagVersion(bad)).toThrow();
    }
    const at = new Date("2026-10-10T23:30:00-02:00"); // already the 11th in UTC
    expect(lib.canaryVersion("0.1.0", at, 42)).toBe("0.1.0-canary.20261011.42");
    expect(() => lib.canaryVersion("0.1.0", at, "0")).toThrow();
    expect(() => lib.canaryVersion("0.1", at, 1)).toThrow();
  });

  it("orders canary builds by date, then run number (semver prerelease rules)", () => {
    const ids = (v) => v.split("-")[1].split(".").slice(1).map(Number);
    const a = ids("0.1.0-canary.20261010.9");
    const b = ids("0.1.0-canary.20261011.10");
    expect(a[0] < b[0] || (a[0] === b[0] && a[1] < b[1])).toBe(true);
  });
});

describe("notes", () => {
  const changelog = "# Changelog\n\n## Unreleased\n- canary thing\n\n## [0.2.0] - 2026-10-11\n- Fixed it\n\n## 0.1.0\nFirst\n";

  it("takes the version's section, else the tag message", () => {
    expect(lib.changelogSection(changelog, "0.2.0")).toBe("- Fixed it");
    expect(lib.changelogSection(changelog, "0.1.0")).toBe("First");
    expect(lib.changelogSection(changelog, "0.1")).toBe("");
    expect(lib.releaseNotes({ channel: "stable", version: "0.2.0", changelog, fallback: "tag" })).toBe("- Fixed it");
    expect(lib.releaseNotes({ channel: "stable", version: "0.3.0", changelog, fallback: "Kinetra 0.3.0\n" })).toBe("Kinetra 0.3.0");
    expect(() => lib.releaseNotes({ channel: "stable", version: "0.3.0", changelog, fallback: " " })).toThrow(/CHANGELOG/);
  });

  it("canary uses Unreleased, else the commits; long notes are cut", () => {
    expect(lib.releaseNotes({ channel: "canary", version: "x", changelog, fallback: "c1" })).toBe("- canary thing");
    expect(lib.releaseNotes({ channel: "canary", version: "x", changelog: "## Unreleased\n", fallback: "c1\nc2" })).toBe("c1\nc2");
    expect(lib.releaseNotes({ channel: "canary", version: "x", changelog: "", fallback: "" })).toBe("");
    const long = lib.releaseNotes({ channel: "canary", version: "x", changelog: "", fallback: "a".repeat(5000) });
    expect(long.length).toBe(4000);
  });
});

describe("config", () => {
  it("merges like tauri --config (RFC 7396)", () => {
    expect(lib.mergePatch({ a: { b: 1, c: 2 }, d: [1] }, { a: { b: null, e: 3 }, d: [2] })).toEqual({ a: { c: 2, e: 3 }, d: [2] });
  });

  it("canary changes the identity, title and icons only", () => {
    expect(canary.identifier).toBe("no.rorvikmedieteknikk.kinetra.canary");
    expect(canary.productName).toBe("Kinetra Canary");
    const merged = lib.mergePatch(conf, canary);
    // Windows are replaced as a whole: the main window must stay the same apart from its title.
    expect(merged.app.windows).toEqual(conf.app.windows.map((w) => ({ ...w, title: "Kinetra Canary" })));
    expect(merged.plugins).toEqual(conf.plugins);
    expect(merged.bundle.icon).toHaveLength(conf.bundle.icon.length);
    for (const icon of merged.bundle.icon) expect(existsSync(`src-tauri/${icon}`), icon).toBe(true);
  });

  it("adds the version, updater artifacts and the sign hook", () => {
    const c = lib.ciConfig({ channelOverlay: canary, version: "0.1.0-canary.20261010.1", updater: true, signScript: "D:/a/x/sign.mjs" });
    expect(c.version).toBe("0.1.0-canary.20261010.1");
    expect(c.identifier).toBe(canary.identifier);
    expect(c.bundle.createUpdaterArtifacts).toBe(true);
    expect(c.bundle.icon).toEqual(canary.bundle.icon);
    expect(c.bundle.windows.signCommand).toBe("node D:/a/x/sign.mjs %1");
    const plain = lib.ciConfig({ channelOverlay: {}, version: "0.2.0", updater: false, signScript: null });
    expect(plain).toEqual({ version: "0.2.0", bundle: { createUpdaterArtifacts: false } });
    expect(() => lib.ciConfig({ version: "1.0.0", signScript: "C:/a b/s.mjs" })).toThrow();
  });
});

describe("artifacts", () => {
  it("picks the bundles and their signatures", () => {
    const win = ["nsis/Kinetra_0.2.0_x64-setup.exe", "nsis/Kinetra_0.2.0_x64-setup.exe.sig", "msi/x.msi"];
    expect(lib.pickBundles(win, "windows", true)).toEqual({ installer: win[0], installerSig: win[1] });
    expect(lib.pickBundles(win.slice(0, 1), "windows", false).installerSig).toBe(null);
    expect(() => lib.pickBundles(win.slice(0, 1), "windows", true)).toThrow(/\.sig/);
    expect(() => lib.pickBundles([], "windows", false)).toThrow(/none/);
    const mac = ["dmg/Kinetra_0.2.0_universal.dmg", "macos/Kinetra.app.tar.gz", "macos/Kinetra.app.tar.gz.sig", "macos/Kinetra.app/Contents/Info.plist"];
    expect(lib.pickBundles(mac, "macos", true)).toEqual({ dmg: mac[0], app: mac[1], appSig: mac[2] });
  });

  it("names files without spaces, per channel", () => {
    expect(lib.fileNames("canary", "0.2.0-canary.20261010.1")).toEqual({
      installer: "Kinetra-Canary_0.2.0-canary.20261010.1_x64-setup.exe",
      dmg: "Kinetra-Canary_0.2.0-canary.20261010.1_universal.dmg",
      app: "Kinetra-Canary_0.2.0-canary.20261010.1_universal.app.tar.gz",
    });
    expect(lib.fileUrl("stable", "0.2.0", "Kinetra_0.2.0_x64-setup.exe")).toBe(
      "https://download.kinetra.live/releases/stable/0.2.0/Kinetra_0.2.0_x64-setup.exe",
    );
  });

  it("reads minisign key ids from the public key and a signature", () => {
    const id = lib.minisignKeyId(conf.plugins.updater.pubkey);
    expect(id).toBe("670E84A21A75AFC8");
    expect(lib.minisignKeyId(fakeSig("C8AF751AA2840E67"))).toBe("C8AF751AA2840E67");
    expect(() => lib.minisignKeyId(Buffer.from("hello").toString("base64"))).toThrow();
  });

  it("writes and parses SHA256SUMS", () => {
    const h = (c) => c.repeat(64);
    const text = lib.sha256sums([{ name: "b.exe", sha256: h("b") }, { name: "a.dmg", sha256: h("a") }]);
    expect(text).toBe(`${h("a")}  a.dmg\n${h("b")}  b.exe\n`);
    expect([...lib.parseSha256sums(text)]).toEqual([["a.dmg", h("a")], ["b.exe", h("b")]]);
    expect(() => lib.parseSha256sums("nope  x")).toThrow();
  });
});

describe("manifests", () => {
  const win = { url: "https://download.kinetra.live/releases/stable/0.2.0/Kinetra_0.2.0_x64-setup.exe", signature: "sigW" };
  const mac = { url: "https://download.kinetra.live/releases/stable/0.2.0/Kinetra_0.2.0_universal.app.tar.gz", signature: "sigM" };

  it("is the Tauri v2 static format; macOS universal serves both architectures", () => {
    const m = lib.updaterManifest({ version: "0.2.0", notes: "n", pubDate: "2026-10-11T12:00:00Z", windows: win, macos: mac });
    expect(m).toEqual({
      version: "0.2.0",
      notes: "n",
      pub_date: "2026-10-11T12:00:00Z",
      platforms: { "windows-x86_64": win, "darwin-aarch64": mac, "darwin-x86_64": mac },
    });
    const winOnly = lib.updaterManifest({ version: "0.2.0", notes: "", pubDate: "x", windows: win, macos: null });
    expect(Object.keys(winOnly.platforms)).toEqual(["windows-x86_64"]);
    expect(() => lib.updaterManifest({ version: "0.2.0", windows: null, macos: mac })).toThrow(/Windows/);
  });

  it("uploads files first and the manifest last", () => {
    const plan = lib.uploadPlan({ channel: "canary", version: "0.2.0-canary.20261010.1", files: ["a.exe", "a.exe.sig"] });
    expect(plan.map((p) => p.key)).toEqual([
      "releases/canary/0.2.0-canary.20261010.1/a.exe",
      "releases/canary/0.2.0-canary.20261010.1/a.exe.sig",
      "releases/canary/0.2.0-canary.20261010.1/SHA256SUMS",
      "releases/canary/0.2.0-canary.20261010.1/release.json",
      "releases/canary/latest.json",
      "updates/latest-canary.json",
    ]);
    expect(plan.slice(0, 4).every((p) => p.cacheControl === lib.IMMUTABLE)).toBe(true);
    expect(plan.slice(4).every((p) => p.cacheControl === "public, max-age=60")).toBe(true);
    expect(lib.planTsv(plan).split("\n")[0]).toBe(
      "files/a.exe\treleases/canary/0.2.0-canary.20261010.1/a.exe\tpublic, max-age=31536000, immutable\tapplication/octet-stream",
    );
    expect(lib.pointerPlan("stable").map((p) => p.key)).toEqual(["releases/stable/latest.json", "updates/latest.json"]);
  });

  it("rolls back only to a consistent release record", () => {
    const manifest = lib.updaterManifest({ version: "0.2.0", notes: "", pubDate: "x", windows: win, macos: null });
    const latest = lib.latestJson({
      channel: "stable",
      version: "0.2.0",
      pubDate: "x",
      notes: "",
      files: { windows: { name: "Kinetra_0.2.0_x64-setup.exe", url: win.url, sha256: "a", size: 1 } },
    });
    const record = { channel: "stable", version: "0.2.0", files: ["Kinetra_0.2.0_x64-setup.exe", "Kinetra_0.2.0_x64-setup.exe.sig"], manifest, latest };
    expect(() => lib.checkReleaseRecord(record, "stable", "0.2.0")).not.toThrow();
    expect(() => lib.checkReleaseRecord(record, "canary", "0.2.0")).toThrow();
    expect(() => lib.checkReleaseRecord(record, "stable", "0.1.0")).toThrow();
    const elsewhere = structuredClone(record);
    elsewhere.manifest.platforms["windows-x86_64"].url = "https://evil.example/x.exe";
    expect(() => lib.checkReleaseRecord(elsewhere, "stable", "0.2.0")).toThrow(/outside/);
    const missing = structuredClone(record);
    missing.files = ["Kinetra_0.2.0_x64-setup.exe.sig"];
    expect(() => lib.checkReleaseRecord(missing, "stable", "0.2.0")).toThrow(/outside/);
  });
});

describe("R2 endpoint", () => {
  const id = "0123456789abcdef0123456789abcdef";
  it("accepts the S3 API URL and cleans what pasting adds", () => {
    expect(lib.r2Endpoint(`https://${id}.r2.cloudflarestorage.com`)).toEqual({ url: `https://${id}.r2.cloudflarestorage.com`, notes: [] });
    const pasted = lib.r2Endpoint(`  https://${id}.r2.cloudflarestorage.com/kinetra-downloads\n`);
    expect(pasted.url).toBe(`https://${id}.r2.cloudflarestorage.com`);
    expect(pasted.notes).toHaveLength(2);
    expect(lib.r2Endpoint(`https://${id}.eu.r2.cloudflarestorage.com/`).url).toBe(`https://${id}.eu.r2.cloudflarestorage.com`);
  });

  it("refuses anything else without repeating the value", () => {
    for (const bad of ["", "\n", `http://${id}.r2.cloudflarestorage.com`, `${id}.r2.cloudflarestorage.com`, "https://pub-abc.r2.dev", "https://download.kinetra.live", `https://${id}.r2.cloud flarestorage.com`]) {
      let message = "";
      try {
        lib.r2Endpoint(bad);
      } catch (e) {
        message = e.message;
      }
      expect(message, JSON.stringify(bad)).toMatch(/^R2_ENDPOINT/);
      if (bad.trim()) expect(message).not.toContain(bad.trim());
    }
  });
});

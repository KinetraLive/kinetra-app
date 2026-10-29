import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { cargoDependencyLines, depsFingerprint } from "../../../scripts/licenses-deps.mjs";
import { TEMPLATE_FONTS } from "../types";
import { filterLicenses, forPlatform, type Licenses } from "./index";

const ROOT = join(__dirname, "..", "..", "..");
const data = JSON.parse(readFileSync(join(__dirname, "licenses.json"), "utf8")) as Licenses;
const item = (group: string, name: string) => data.groups.find((g) => g.id === group)?.items.find((i) => i.name === name);

describe("licenses.json", () => {
  it("is current with the declared dependencies (run `pnpm licenses` after changing them)", () => {
    expect(data.depsHash, "dependencies changed: run `pnpm licenses` and commit src/lib/licenses/licenses.json").toBe(
      depsFingerprint(ROOT),
    );
  });

  it("has the four groups, each non-empty, and every text reference resolves", () => {
    expect(data.schemaVersion).toBe(1);
    expect(data.groups.map((g) => g.id)).toEqual(["npm", "rust", "fonts", "ndi"]);
    for (const g of data.groups) {
      expect(g.items.length).toBeGreaterThan(0);
      for (const i of g.items) {
        expect(i.name && i.version && i.license).toBeTruthy();
        for (const f of i.files ?? []) expect(typeof data.texts[f.text]).toBe("string");
      }
    }
  });

  it("lists Svelte Flow (@xyflow/svelte and @xyflow/system) as MIT with the license text", () => {
    for (const name of ["@xyflow/svelte", "@xyflow/system"]) {
      const it = item("npm", name);
      expect(it?.license).toBe("MIT");
      expect(data.texts[it!.files![0].text]).toMatch(/^MIT License/);
    }
  });

  it("lists kinetra-license as AGPL-3.0-only, by Rørvik Medieteknikk, with the license text", () => {
    const it = item("rust", "kinetra-license");
    expect(it?.license).toBe("AGPL-3.0-only");
    expect(it?.copyright).toBe("Rørvik Medieteknikk");
    expect(data.texts[it!.files![0].text]).toMatch(/^GNU AFFERO GENERAL PUBLIC LICENSE/);
  });

  it("lists the app's runtime, the Rust core and the bundled fonts", () => {
    expect(item("npm", "svelte")).toBeTruthy();
    expect(item("rust", "tauri")).toBeTruthy();
    expect(item("rust", "axum")).toBeTruthy();
    expect(item("fonts", "IBM Plex Sans")).toMatchObject({ license: "OFL-1.1", package: "@fontsource/ibm-plex-sans" });
    expect(item("fonts", "IBM Plex Mono")).toMatchObject({ license: "OFL-1.1", package: "@fontsource/ibm-plex-mono" });
    // NDI: the headers' MIT license and the NDI terms (the runtime isn't bundled).
    expect(item("ndi", "NDI® SDK headers")).toMatchObject({ license: "MIT" });
    const terms = item("ndi", "NDI® Runtime (not included)");
    expect(data.texts[terms!.files![0].text]).toContain("registered trademark of Vizrt NDI AB");
  });

  it("lists the crates of every platform we ship, marking the platform-only ones", () => {
    // Windows only: WebView2 and the windows crate; macOS only: the Keychain and objc2.
    expect(item("rust", "webview2-com")?.platforms).toEqual(["windows"]);
    expect(item("rust", "windows")?.platforms).toEqual(["windows"]);
    expect(item("rust", "security-framework")?.platforms).toEqual(["macos"]);
    expect(item("rust", "objc2-foundation")?.platforms).toEqual(["macos"]);
    // Shared crates carry no mark; only Rust crates differ by platform.
    expect(item("rust", "tauri")?.platforms).toBeUndefined();
    for (const g of data.groups.filter((g) => g.id !== "rust")) {
      for (const i of g.items) expect(i.platforms, `${g.id}:${i.name}`).toBeUndefined();
    }
    for (const g of data.groups) {
      for (const i of g.items) {
        for (const p of i.platforms ?? []) expect(["windows", "macos"]).toContain(p);
      }
    }
  });

  it("gives every item a license text (packages without one: legal/licenses/, KEPT_LICENSES)", () => {
    const without = data.groups.flatMap((g) => g.items.filter((i) => !i.files?.length).map((i) => `${g.id}:${i.name}`));
    expect(without, "add the text to legal/licenses/ and KEPT_LICENSES in scripts/licenses.mjs").toEqual([]);
  });

  it("gives the objc2 crates their MIT text (kept in legal/licenses: the crates ship without one)", () => {
    for (const name of ["objc2", "block2", "objc2-foundation", "objc2-app-kit"]) {
      const it = item("rust", name);
      expect(it?.files?.length, name).toBeGreaterThan(0);
      expect(data.texts[it!.files![0].text], name).toMatch(/Permission is hereby granted, free of charge/);
    }
  });

  it("lists every bundled font (app UI and on-air graphics) as OFL-1.1 with its license text", () => {
    const fonts = data.groups.find((g) => g.id === "fonts")!.items;
    const curated = TEMPLATE_FONTS.map((f) => f.name);
    for (const name of ["IBM Plex Sans", "IBM Plex Mono", ...curated]) {
      const f = fonts.find((i) => i.name === name);
      expect(f, name).toBeTruthy();
      expect(f!.license, name).toBe("OFL-1.1");
      expect(f!.files?.length, name).toBeGreaterThan(0);
      expect(data.texts[f!.files![0].text], name).toMatch(/SIL OPEN FONT LICENSE/i);
    }
    // Nothing else: a font that is no longer bundled is no longer listed.
    expect(fonts.map((f) => f.name).sort()).toEqual([...new Set(["IBM Plex Sans", "IBM Plex Mono", ...curated])].sort());
  });
});

describe("cargoDependencyLines", () => {
  it("keeps normal and target dependency sections, not dev or build ones", () => {
    const toml = [
      "[package]",
      'name = "x"',
      "[build-dependencies]",
      'tauri-build = "2"',
      "[dependencies]",
      'serde = "1"',
      "# comment",
      "[dev-dependencies]",
      'tempfile = "3"',
      "[target.'cfg(windows)'.dependencies]",
      'windows = "0.6"',
    ].join("\r\n");
    expect(cargoDependencyLines(toml)).toEqual(["[target.'cfg(windows)'.dependencies]", 'serde = "1"', 'windows = "0.6"'].sort());
  });
});

describe("forPlatform", () => {
  const names = (p: "windows" | "macos") =>
    new Set(forPlatform(data, p).groups.flatMap((g) => g.items.map((i) => `${g.id}:${i.name}@${i.version}`)));

  it("keeps the shared items and the platform's own, and leaves out the other platform's", () => {
    const win = names("windows");
    const mac = names("macos");
    const rust = (n: string) => [...data.groups[1].items.filter((i) => i.name === n)].map((i) => `rust:${i.name}@${i.version}`);
    for (const k of rust("tauri")) expect(win.has(k) && mac.has(k), k).toBe(true);
    for (const k of rust("webview2-com")) expect([win.has(k), mac.has(k)], k).toEqual([true, false]);
    for (const k of rust("objc2-foundation")) expect([win.has(k), mac.has(k)], k).toEqual([false, true]);
    // Fonts, npm and NDI are on both.
    expect(forPlatform(data, "macos").groups[2].items).toHaveLength(data.groups[2].items.length);
  });

  it("drops nothing but platform-only items", () => {
    const total = data.groups.reduce((n, g) => n + g.items.length, 0);
    const only = (p: "windows" | "macos") =>
      data.groups.flatMap((g) => g.items).filter((i) => i.platforms && !i.platforms.includes(p)).length;
    expect(names("windows").size).toBe(total - only("windows"));
    expect(names("macos").size).toBe(total - only("macos"));
  });
});

describe("filterLicenses", () => {
  it("matches name, package, version and license case-insensitively", () => {
    const names = (q: string) => filterLicenses(data.groups, q).flatMap((g) => g.items.map((i) => i.name));
    expect(names("XYFLOW")).toEqual(["@xyflow/svelte", "@xyflow/system"]);
    expect(names("fontsource/oswald")).toEqual(["Oswald"]);
    expect(names("ofl-1.1")).toHaveLength(data.groups[2].items.length);
    expect(names("  ")).toHaveLength(data.groups.reduce((n, g) => n + g.items.length, 0));
  });
});

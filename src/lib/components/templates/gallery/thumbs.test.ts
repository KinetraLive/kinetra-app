import { describe, expect, it } from "vitest";
import { thumbIndex, thumbUrl, variantThumbUrl } from "./thumbs";

const PRESETS = ["classic-bar", "minimal-line", "slide-in-accent", "news-strap", "corner-bug", "fullscreen-title"];

describe("thumbUrl", () => {
  it.each(PRESETS)("resolves the static thumbnail of %s from presets/thumbs/", async (id) => {
    expect(await thumbUrl(id)).toMatch(new RegExp(`/presets/thumbs/${id}\\.png`));
  });

  it("resolves null for a preset without a thumbnail", async () => {
    expect(await thumbUrl("no-such-preset")).toBeNull();
    expect(await thumbUrl("")).toBeNull();
  });
});

describe("variantThumbUrl", () => {
  it.each(["minimal", "frame", "pill"])("resolves the %s look of lower-third to its WebP", async (style) => {
    expect(await variantThumbUrl("lower-third", style)).toMatch(new RegExp(`/presets/thumbs/lower-third-${style}\\.webp`));
  });

  it("falls back to the preset's own PNG, then null", async () => {
    expect(await variantThumbUrl("corner-bug", "no-such-look")).toMatch(/\/presets\/thumbs\/corner-bug\.png/);
    expect(await variantThumbUrl("lower-third", "no-such-look")).toBeNull();
  });
});

describe("thumbIndex", () => {
  const load = (url: string) => () => Promise.resolve(url);

  it("keys by file name without the extension and prefers WebP over PNG", async () => {
    for (const order of [
      ["/presets/thumbs/a-frame.png", "/presets/thumbs/a-frame.webp"],
      ["/presets/thumbs/a-frame.webp", "/presets/thumbs/a-frame.png"],
    ]) {
      const index = thumbIndex(Object.fromEntries([...order, "/presets/thumbs/a.png"].map((p) => [p, load(p)])));
      expect([...index.keys()].sort()).toEqual(["a", "a-frame"]);
      expect(await index.get("a-frame")!()).toBe("/presets/thumbs/a-frame.webp");
      expect(await index.get("a")!()).toBe("/presets/thumbs/a.png");
    }
  });

  it("ignores other file types", () => {
    expect(thumbIndex({ "/presets/thumbs/a.jpg": load("x") }).size).toBe(0);
  });
});

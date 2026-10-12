// Writes presets/thumbs/<id>.png (480×270, transparent) for the gallery: each preset loaded into
// the fixture server's test screen, every field showing its own label, played in and settled. Each
// style look of a library preset gets presets/thumbs/<id>-<style>.webp, with the preset's sample data:
// every look on disk (variants/<style>/, styles from library.json), registered in template.json.
// Style thumbnails are lossy WebP with alpha (quality WEBP_QUALITY), encoded by the browser itself:
// the PNG screenshot is drawn into a canvas in the page and read back with toBlob("image/webp"), so
// soft shades stay small (the same shots as PNG were up to 84 KB). No encoder dependency.
// One style: pnpm --dir renderer build, then
//   pnpm exec playwright test --config playwright.thumbs.config.ts -g "style <style> "
//
// A full 1920×1080 frame shrunk to 480×270 makes most graphics tiny (a corner bug becomes a speck),
// so each thumbnail is a 16:9 crop around the graphic: the union of the root's descendant boxes,
// padded ~10%, at least 640 px wide, kept inside the frame. The renderer stage is scaled with a transform so
// the crop fills the 1920×1080 viewport, which the config rasterises at a quarter: 480×270.
import { mkdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { expect, presetTest as test } from "../fixture";
import { PRESETS, animationsSettled, labelFields, openScreen, root } from "../preset-helpers";
import { STYLE_LOOKS, sample } from "../style-helpers";

const OUT = join(process.cwd(), "presets", "thumbs");
/** Plan budget for a gallery thumbnail. */
const MAX_BYTES = 40 * 1024;
/** WebP quality of the style thumbnails (canvas.toBlob, 0–1). A thumbnail over MAX_BYTES at this
 *  quality is re-encoded at the next lower one (soft halos and blurs are what cost bytes). */
const WEBP_QUALITY = 0.8;
const WEBP_FALLBACKS = [0.75, 0.7, 0.65];
const FRAME = { w: 1920, h: 1080 };
/** Smallest crop width: a third of the frame, so small graphics are enlarged at most 3×. */
const MIN_CROP_W = 640;
/** Margin around the graphic, as a share of the crop. */
const PAD = 0.1;

interface Box {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

/** The 16:9 crop (x, y, width; height = width × 9/16) around `b`, inside the frame. */
function cropAround(b: Box): { x: number; y: number; w: number } {
  const bw = b.x1 - b.x0;
  const bh = b.y1 - b.y0;
  const fit = Math.max(bw, (bh * 16) / 9);
  const w = Math.min(FRAME.w, Math.max(MIN_CROP_W, fit / (1 - 2 * PAD)));
  const h = (w * 9) / 16;
  const clamp = (v: number, max: number) => Math.min(Math.max(v, 0), max);
  return {
    x: clamp((b.x0 + b.x1) / 2 - w / 2, FRAME.w - w),
    y: clamp((b.y0 + b.y1) / 2 - h / 2, FRAME.h - h),
    w,
  };
}

/** Style looks get their own thumbnail, `<id>-<style>.webp`, rendered with the preset's sample data
 *  (`sample.json`, the values the gallery's hover preview plays). */
const STYLE_SHOTS = STYLE_LOOKS.flatMap(({ style, presets }) => presets.map((id) => ({ id, variant: style, fields: sample(id) })));

const SHOTS = [
  ...PRESETS.map((id) => ({ id: id as string, variant: "", fields: labelFields(id) })),
  ...STYLE_SHOTS,
];

for (const { id, variant, fields } of SHOTS) {
  const name = variant ? `${id}-${variant}` : id;
  test(variant ? `thumbnail style ${variant} ${id}` : `thumbnail ${id}`, async ({ page, presetLive }) => {
    await presetLive.cmd(variant ? `template ${id} ${variant}` : `template ${id}`);
    await presetLive.cmd(`fields ${JSON.stringify(fields)}`);
    const errors = await openScreen(page, presetLive.base);
    await presetLive.cmd("show");
    await expect(root(page)).toHaveAttribute("data-state", "in");
    await animationsSettled(page);
    expect(errors).toEqual([]);

    // Union of every visible descendant box of the template root (inside its shadow root). Style
    // looks lay out in full-frame containers: there only boxes that paint count (text, images,
    // a background or a border).
    const box = await root(page)
      .first()
      .evaluate((el, painted): Box => {
        const b = { x0: Infinity, y0: Infinity, x1: -Infinity, y1: -Infinity };
        const paints = (d: Element) => {
          if (d.localName === "img") return true;
          if ([...d.childNodes].some((n) => n.nodeType === 3 && n.textContent!.trim())) return true;
          const cs = getComputedStyle(d);
          return !/^rgba\(\d+, \d+, \d+, 0\)$|transparent/.test(cs.backgroundColor) || parseFloat(cs.borderTopWidth) > 0 || parseFloat(cs.borderLeftWidth) > 0;
        };
        for (const d of el.querySelectorAll("*")) {
          if (painted && !paints(d)) continue;
          for (const r of d.getClientRects()) {
            if (r.width < 1 || r.height < 1) continue;
            b.x0 = Math.min(b.x0, r.left);
            b.y0 = Math.min(b.y0, r.top);
            b.x1 = Math.max(b.x1, r.right);
            b.y1 = Math.max(b.y1, r.bottom);
          }
        }
        return b;
      }, variant !== "");
    expect(box.x1 - box.x0).toBeGreaterThan(0);
    const crop = cropAround(box);
    console.log(`${name}: graphic ${Math.round(box.x1 - box.x0)}×${Math.round(box.y1 - box.y0)}, crop ${Math.round(crop.w)} wide`);

    // Scale the renderer's stage (the fixed full-viewport element, first child of <body>).
    await page.evaluate(({ x, y, w, frame }) => {
      const s = (document.body.firstElementChild as HTMLElement).style;
      s.transformOrigin = "0 0";
      s.transform = `scale(${frame / w}) translate(${-x}px, ${-y}px)`;
    }, { ...crop, frame: FRAME.w });
    // Two frames so the scaled page is re-rasterised at its new size before the capture.
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

    mkdirSync(OUT, { recursive: true });
    const shot = { omitBackground: true, scale: "device", animations: "allow" } as const;
    let file: string;
    if (variant) {
      // PNG screenshot → canvas in the page → lossy WebP with alpha.
      const png = await page.screenshot(shot);
      const encode = (quality: number) =>
        page.evaluate(
        async ({ src, quality }) => {
          const img = new Image();
          img.src = src;
          await img.decode();
          const canvas = document.createElement("canvas");
          canvas.width = img.naturalWidth;
          canvas.height = img.naturalHeight;
          canvas.getContext("2d")!.drawImage(img, 0, 0);
          const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/webp", quality));
          if (!blob || blob.type !== "image/webp") throw new Error("WebP encoding unavailable");
          const bytes = new Uint8Array(await blob.arrayBuffer());
          let bin = "";
          for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
          return btoa(bin);
        },
        { src: `data:image/png;base64,${png.toString("base64")}`, quality },
      );
      let quality = WEBP_QUALITY;
      let buf = Buffer.from(await encode(quality), "base64");
      for (const q of WEBP_FALLBACKS) {
        if (buf.length <= MAX_BYTES) break;
        quality = q;
        buf = Buffer.from(await encode(q), "base64");
      }
      if (quality !== WEBP_QUALITY) console.log(`${name}: re-encoded at quality ${quality}`);
      expect(buf.subarray(0, 4).toString("latin1")).toBe("RIFF");
      expect(buf.subarray(8, 12).toString("latin1")).toBe("WEBP");
      file = join(OUT, `${name}.webp`);
      writeFileSync(file, buf);
    } else {
      file = join(OUT, `${name}.png`);
      await page.screenshot({ ...shot, path: file });
    }
    const size = statSync(file).size;
    console.log(`${file.slice(OUT.length + 1)}  ${(size / 1024).toFixed(1)} KB`);
    expect(size).toBeLessThanOrEqual(MAX_BYTES);
  });
}

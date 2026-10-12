// "Update to the latest look" on the real server and renderer: a project template copied from a
// preset before the content-sizing work (an older preset version, no design box, a look that draws
// at a fixed place) ignores its layer's box; after `updatetemplate` (the app's
// update_template_from_preset: a pre-update backup, the preset's latest looks) the graphic sits
// inside the box and follows it. Undo (undo_template_update) puts the old look back. Then "Fit box
// to graphic" on a full-screen layer: the design box at its default position, Scale to fit.
//
// No baselines: shots go to review/fit-update/e2e-*.png for a visual check.
//
//   pnpm --dir renderer build
//   pnpm exec playwright test e2e/template-update.spec.ts
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import type { Rect, ScreenLayer, TemplateDoc, TemplateSummary, TemplateUpdated } from "../src/lib/types";
import { expect, styleTest as test, type Fixture } from "./fixture";
import { asFields, openScreen, saveScreen, settle } from "./style-helpers";

const OUT = "review/fit-update";

/** The union of the layer's visible graphic (painted boxes, text, images) and its host's box. */
function graphic(page: Page) {
  return page.evaluate(() => {
    const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot)!;
    const sr = host.shadowRoot!;
    const h = host.getBoundingClientRect();
    let u = { l: 1e9, t: 1e9, r: -1e9, b: -1e9 };
    const alpha = (c: string) => {
      const m = /rgba?\(([^)]+)\)/.exec(c);
      if (!m) return c === "transparent" ? 0 : 1;
      const p = m[1].split(/[ ,/]+/).filter(Boolean);
      return p.length > 3 ? parseFloat(p[3]) : 1;
    };
    for (const e of sr.querySelectorAll<HTMLElement>("*")) {
      if (e.tagName === "STYLE") continue;
      const cs = getComputedStyle(e);
      if (cs.display === "none" || cs.visibility === "hidden") continue;
      const text = [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent!.trim());
      if (!text && alpha(cs.backgroundColor) < 0.05 && e.tagName !== "IMG") continue;
      const r = e.getBoundingClientRect();
      if (r.width < 0.5 || r.height < 0.5) continue;
      u = { l: Math.min(u.l, r.left), t: Math.min(u.t, r.top), r: Math.max(u.r, r.right), b: Math.max(u.b, r.bottom) };
    }
    return { host: { l: h.left, t: h.top, r: h.right, b: h.bottom }, union: u };
  });
}

const inside = (g: Awaited<ReturnType<typeof graphic>>, tol = 1) =>
  g.union.l >= g.host.l - tol && g.union.t >= g.host.t - tol && g.union.r <= g.host.r + tol && g.union.b <= g.host.b + tol;

async function shot(page: Page, name: string, rect: Rect) {
  await page.evaluate((r) => {
    document.getElementById("box")?.remove();
    const d = document.createElement("div");
    d.id = "box";
    d.style.cssText = `position:fixed;left:${r.x}px;top:${r.y}px;width:${r.w}px;height:${r.h}px;outline:2px dashed #00e5ff;pointer-events:none;z-index:99999`;
    document.body.append(d);
  }, rect);
  mkdirSync(OUT, { recursive: true });
  await page.screenshot({ path: join(OUT, `e2e-${name}.png`) });
  await page.evaluate(() => document.getElementById("box")?.remove());
}

/**
 * A template from `preset` as an older Kinetra copied it: preset version 1, no design box, and
 * `look` as the variant's HTML/CSS (written on disk; the watcher picks it up).
 */
async function outdated(live: Fixture, preset: string, variant: string, html: string, css: string): Promise<string> {
  const id = await live.cmd(`addtemplate ${preset}`);
  const dir = join(await live.cmd("dir"), "templates", id);
  const meta = JSON.parse(readFileSync(join(dir, "template.json"), "utf8"));
  meta.preset.version = 1;
  delete meta.design;
  writeFileSync(join(dir, "variants", variant, "template.html"), html);
  writeFileSync(join(dir, "variants", variant, "template.css"), css);
  writeFileSync(join(dir, "template.json"), JSON.stringify(meta, null, 2));
  await expect
    .poll(async () => (JSON.parse(await live.cmd("listtemplates")) as TemplateSummary[]).find((t) => t.id === id)?.presetUpdate, {
      timeout: 10_000,
    })
    .toEqual({ from: 1, to: expect.any(Number) });
  return id;
}

const SCHEDULE = asFields({
  title: "Main stage",
  day: "Thursday",
  clock: "HH:mm",
  items: [
    { title: "Keynote: The live production stack", subtitle: "Ingrid Vale", location: "Main stage", start: "09:30", end: "10:30", new_start: "", label: "Now", _status: "now" },
    { title: "Panel: Remote production at scale", subtitle: "Four guests", location: "Studio B", start: "10:45", end: "11:30", new_start: "10:55", label: "Delayed", _status: "delayed" },
  ],
});

test("an outdated preset template follows its box after the update; undo restores the old look", async ({ page, styleServer: live }) => {
  test.setTimeout(120_000);
  // The old look: a fixed-size panel at a fixed place, whatever the box.
  const id = await outdated(
    live,
    "schedule-board",
    "default",
    '<div class="old"><div class="t">{{title}}</div></div>',
    ".old { position: absolute; left: 1100px; top: 600px; width: 700px; height: 380px; background: #c0392b; } .t { color: #fff; font: 700 40px sans-serif; }",
  );
  const rect: Rect = { x: 406, y: 333, w: 642, h: 379 };
  const l = { id: "sb", name: "Schedule board", kind: "template", templateId: id, rect, fields: SCHEDULE, vars: {} } as ScreenLayer;
  const screen = await saveScreen(live, 1920, 1080, "#3a3d44", [l]);
  await page.setViewportSize({ width: 1920, height: 1080 });
  await openScreen(page, live.base, screen.id);
  await live.cmd(`layershow ${screen.id} sb`);
  await expect(page.locator("[data-state]")).toHaveAttribute("data-state", "in");
  await settle(page);
  const before = await graphic(page);
  expect(inside(before), "the old look ignores the box").toBe(false);
  await shot(page, "schedule-before", rect);

  const r = JSON.parse(await live.cmd(`updatetemplate ${id}`)) as TemplateUpdated;
  expect(r.backup.reason).toBe("pre-update");
  expect(existsSync(join(await live.cmd("dir"), ".backups", r.backup.id))).toBe(true);
  expect(r.doc.design?.w).toBeGreaterThan(0);
  expect((JSON.parse(await live.cmd("listtemplates")) as TemplateSummary[]).find((t) => t.id === id)?.presetUpdate).toBeNull();
  // On air under On take the new look is staged: the take applies it.
  await live.cmd(`layershow ${screen.id} sb`);
  await expect.poll(async () => inside(await graphic(page)), { timeout: 10_000 }).toBe(true);
  await settle(page);
  const after = await graphic(page);
  expect(inside(after), JSON.stringify(after)).toBe(true);
  // It fills a fair part of the box (not a speck in a corner).
  expect(after.union.r - after.union.l).toBeGreaterThan(rect.w * 0.5);
  await shot(page, "schedule-after", rect);

  // The box moves and grows (saved, then the take): the graphic follows.
  const moved: Rect = { x: 100, y: 120, w: 1200, h: 700 };
  await live.cmd(`savescreen ${JSON.stringify({ ...screen, layers: [{ ...l, rect: moved }] })}`);
  await live.cmd(`layershow ${screen.id} sb`);
  await expect.poll(async () => (await graphic(page)).host.r).toBeCloseTo(1300, 0);
  await settle(page);
  const grown = await graphic(page);
  expect(inside(grown), JSON.stringify(grown)).toBe(true);
  expect(grown.union.r - grown.union.l).toBeGreaterThan(after.union.r - after.union.l);
  await shot(page, "schedule-after-moved", moved);

  // Undo: the template as the backup holds it, offered again.
  const back = JSON.parse(await live.cmd(`undotemplateupdate ${id} ${r.backup.id}`)) as TemplateDoc;
  expect(back.preset?.version).toBe(1);
  expect(back.variants[0].css).toContain(".old");
  await live.cmd(`layershow ${screen.id} sb`);
  await expect.poll(async () => inside(await graphic(page)), { timeout: 10_000 }).toBe(false);
});

test("Fit box to graphic: a full-screen layer gets the design box at its default position", async ({ page, styleServer: live }) => {
  test.setTimeout(120_000);
  const id = await live.cmd("addtemplate now-next");
  const doc = JSON.parse(await live.cmd(`gettemplate ${id}`)) as TemplateDoc;
  const d = doc.design!;
  const fields = asFields({ ...JSON.parse(readFileSync(join("presets", "templates", "now-next", "sample.json"), "utf8")) });
  const l = {
    id: "nn",
    name: "Now & Next",
    kind: "template",
    templateId: id,
    rect: { x: 0, y: 0, w: 1920, h: 1080 },
    fields,
    vars: {},
    variant: "inline",
  } as ScreenLayer;
  const screen = await saveScreen(live, 1920, 1080, "#3a3d44", [l]);
  await page.setViewportSize({ width: 1920, height: 1080 });
  await openScreen(page, live.base, screen.id);
  await live.cmd(`layershow ${screen.id} nn`);
  await expect(page.locator("[data-state]")).toHaveAttribute("data-state", "in");

  // What the Inspector's button saves (fitBox: the design box at the design position, fit).
  const fit: Rect = { x: d.x ?? 0, y: d.y ?? 0, w: d.w, h: d.h };
  await live.cmd(`savescreen ${JSON.stringify({ ...screen, layers: [{ ...l, rect: fit, sizing: "fit" }] })}`);
  await live.cmd(`layershow ${screen.id} nn`);
  await expect.poll(async () => {
    const { host } = await graphic(page);
    return Math.round(host.r - host.l);
  }).toBe(d.w);
  await settle(page);
  const g = await graphic(page);
  expect(Math.round(g.host.l)).toBe(fit.x);
  expect(Math.round(g.host.t)).toBe(fit.y);
  expect(inside(g), JSON.stringify(g)).toBe(true);
  await shot(page, "now-next-fit", fit);
});

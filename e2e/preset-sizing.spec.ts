// Content sizing of the built-in presets, in the real renderer against the real output server.
// For every preset (not the sponsor templates, which have their own spec) × every look it ships ×
// {fit, reflow} × {its design box, a small corner box 480×135, a 320×180 box, a full-width bar
// 1920×180} (+ the full screen in reflow): on air and settled, every visible plate, image and
// text sits inside the layer's box, text doesn't overflow its own box (single-line text, data-fit,
// clipped blocks) nor the plate behind it, the root is visible, and the graphic fills a box that
// isn't the full frame (FILL below).
//
// No baselines: a contact sheet per preset goes to review/preset-sizing/<id>.png (rows: looks ×
// {fit, reflow}; columns: the boxes; the dashed line is the layer's box) for a visual review.
//
//   pnpm --dir renderer build
//   pnpm exec playwright test e2e/preset-sizing.spec.ts [-g "<preset> "]
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import type { ContentSizing, Rect, ScreenDoc, ScreenLayer } from "../src/lib/types";
import { expect, styleTest as test, type Fixture } from "./fixture";
import { asFields, openScreen } from "./style-helpers";

const DIR = join(process.cwd(), "presets", "templates");

/** Every built-in preset but the sponsor templates (e2e/sponsors.spec.ts). */
const PRESETS = [
  "lower-third",
  "fullscreen-title",
  "corner-bug",
  "ticker",
  "now-next",
  "countdown",
  "poll",
  "scoreboard",
  "classic-bar",
  "minimal-line",
  "slide-in-accent",
  "news-strap",
  "leaderboard",
  "schedule-board",
  "next-up",
];

interface Meta {
  schemaVersion: number;
  variants?: { id: string }[];
  design?: { w: number; h: number; x?: number; y?: number; sizing: ContentSizing };
}
const meta = (id: string): Meta => JSON.parse(readFileSync(join(DIR, id, "template.json"), "utf8"));

const PHOTO =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 240 240'%3E%3Crect width='240' height='240' fill='%235b6f86'/%3E%3Ccircle cx='120' cy='100' r='44' fill='%23d7b49a'/%3E%3C/svg%3E";

/** Sample data of the presets without a sample.json (the legacy and one-off ones). */
const LEGACY: Record<string, Record<string, unknown>> = {
  "classic-bar": { name: "Alex Morgan", title: "Head of Production, Bright Harbor" },
  "minimal-line": { name: "Alex Morgan", title: "Head of Production, Bright Harbor" },
  "slide-in-accent": { name: "Alex Morgan", title: "Head of Production, Bright Harbor", tag: "Guest" },
  "news-strap": { label: "Breaking", headline: "Storm closes mountain roads in the west", subline: "Ferries cancelled until further notice" },
  leaderboard: {
    title: "Leaderboard",
    rows: [
      { name: "Alex Morgan", team: "Northwind", score: "128" },
      { name: "Sam Carter", team: "Summit", score: "117" },
      { name: "Jordan Lee", team: "Beacon", score: "96" },
      { name: "Alexandra Konstantinopoulou", team: "Polaris", score: "88" },
      { name: "Riley Brooks", team: "Kite & Key", score: "74" },
    ],
  },
  "schedule-board": {
    title: "Main stage",
    day: "Thursday",
    clock: "HH:mm",
    items: [
      { title: "Doors open", subtitle: "Coffee in the foyer", location: "Foyer", start: "09:00", end: "09:30", new_start: "", label: "Done", _status: "previous" },
      { title: "Keynote: The live production stack", subtitle: "Alex Morgan", location: "Main stage", start: "09:30", end: "10:30", new_start: "", label: "Now", _status: "now" },
      { title: "Panel: Remote production at scale", subtitle: "Four guests", location: "Studio B", start: "10:45", end: "11:30", new_start: "10:55", label: "Delayed", _status: "delayed" },
      { title: "Lighting masterclass", subtitle: "", location: "Hall A", start: "11:45", end: "12:30", new_start: "", label: "", _status: "upcoming" },
      { title: "Lunch", subtitle: "", location: "Foyer", start: "12:30", end: "13:30", new_start: "", label: "", _status: "upcoming" },
    ],
  },
  "next-up": {
    label: "Up next",
    status: "next",
    title: "Panel: Remote production at scale",
    subtitle: "With four guests from public broadcasters",
    location: "Studio B",
    countdown: { run: false, at: 0, base: 300000, dir: "down", end: "stop" },
  },
};

function sampleOf(id: string): Record<string, string> {
  if (LEGACY[id]) return asFields(LEGACY[id]);
  const s = JSON.parse(readFileSync(join(DIR, id, "sample.json"), "utf8")) as Record<string, unknown>;
  return asFields({ ...s, ...(s.photo === undefined ? {} : { photo: PHOTO }) });
}

const SCREEN = { w: 1920, h: 1080 };

interface Case {
  sizing: ContentSizing;
  box: string;
  rect: Rect;
}

function cases(m: Meta): Case[] {
  const d = m.design;
  const design: Rect = d
    ? {
        x: Math.round(Math.min(Math.max(d.x ?? (SCREEN.w - d.w) / 2, 0), SCREEN.w - d.w)),
        y: Math.round(Math.min(Math.max(d.y ?? (SCREEN.h - d.h) / 2, 0), SCREEN.h - d.h)),
        w: d.w,
        h: d.h,
      }
    : { x: 0, y: 0, ...SCREEN };
  const boxes: [string, Rect][] = [
    ["design", design],
    ["corner", { x: SCREEN.w - 480 - 40, y: 40, w: 480, h: 135 }],
    ["small", { x: 40, y: 40, w: 320, h: 180 }],
    ["bar", { x: 0, y: SCREEN.h - 180 - 40, w: 1920, h: 180 }],
  ];
  // HUG_ONLY=1: just fit at the design box (sizing the design boxes).
  if (process.env.HUG_ONLY) return [{ sizing: "fit", box: "design", rect: design }];
  if (process.env.BBOX_ONLY) return [{ sizing: "reflow", box: "full", rect: { x: 0, y: 0, ...SCREEN } }];
  return [
    ...boxes.map(([box, rect]) => ({ sizing: "fit" as const, box, rect })),
    ...boxes.map(([box, rect]) => ({ sizing: "reflow" as const, box, rect })),
    { sizing: "reflow", box: "full", rect: { x: 0, y: 0, ...SCREEN } },
  ];
}

/**
 * "Fills the box": the graphic is where the layer box is. Checked for every box but the full frame,
 * and but fit with a full-frame design (that is the full-frame look scaled, as at full frame). The
 * bug it catches: a full-frame preset scaling its whole 1920×1080 frame, safe margins included,
 * into a smaller box, so the graphic covers a fraction of it. Measured on the union of the visible
 * painted parts (plates, images, text glyphs, borders; clipped to what is visible), as fractions
 * of the box per axis. A "backdrop" is a plate covering the whole box (≥ 98% on both axes).
 * - `panel` (boards: their panel is the graphic): the union covers `both` of the box on both axes,
 *   or, for an aspect-locked graphic, `locked` of one axis with its centre within `centered` of the
 *   box centre on the other; on a backdrop the rest also reaches `content` on one axis.
 * - `type` (full-frame cards: a backdrop, or none, and a block of type): the union of everything but
 *   backdrops reaches `content` on at least one axis (the type scales with the box, so a title shrunk
 *   into the middle of its background fails).
 * - `hug` (lower thirds, bugs, straps, chat, poll: plates that hug their text and keep their place
 *   in the box): the union reaches `hug` on at least one axis.
 * Before the fix (reflow): schedule board 480×135 92% × 59%, 320×180 84% × 50% (panel); full-screen
 * title classic 480×135 31% × 29%, bar 10% × 29%, minimal 480×135 33% × 34%; countdown classic
 * 480×135 19% × 49%, 320×180 37% × 50%; next-up bar 14% × 58% (type); leaderboard 480×135
 * 28% × 54% (panel). The hug presets reach 29%–98% on their larger axis (text-sized plates; the
 * smallest is the classic corner bug's "LIVE" in a 320×180 box), so `hug` only catches a graphic
 * lost in its box.
 */
const FILL = { both: 0.85, locked: 0.9, centered: 0.05, content: 0.6, hug: 0.25 };

/**
 * The design box hugs the graphic: in fit at its design box (scale 1), the union of the visible
 * parts covers at least HUG of the box on BOTH axes, for every look with the sample text. Resizing
 * a layer scales it with its design's aspect (the editor keeps it), so a box that hugs at 100 %
 * hugs at every scale and the handles sit on the graphic's edges. Not checked: full-frame designs
 * (fullscreen-title, countdown, next-up: their look is the frame) and HUG_EXEMPT below.
 */
const HUG = 0.9;
/** Graphics whose design box keeps room the sample doesn't use (with why). */
const HUG_EXEMPT: Record<string, string> = {};
type Policy = "panel" | "type" | "hug";
const POLICY: Record<string, Policy> = {
  "schedule-board": "panel",
  leaderboard: "panel",
  "fullscreen-title": "type",
  countdown: "type",
  "next-up": "type",
};

/** Problems of the layer on air: things outside the box, overflowing text, an invisible root. */
function inspect(page: Page, fill: (typeof FILL & { policy: Policy; tight?: number; report?: boolean }) | null) {
  return page.evaluate((fill) => {
    const out: string[] = [];
    let U = { l: 1e9, t: 1e9, r: -1e9, b: -1e9 };
    /** The union of the visible parts but full-box backdrops, and whether there is a backdrop. */
    let C = { l: 1e9, t: 1e9, r: -1e9, b: -1e9 };
    let backdrop = false;
    const host = [...document.querySelectorAll("div")].find((d) => d.shadowRoot);
    if (!host) return ["no layer host"];
    const sr = host.shadowRoot!;
    const hb = host.getBoundingClientRect();
    const root = sr.querySelector<HTMLElement>("[data-state]");
    if (!root) return ["no root"];
    if (root.dataset.state !== "in") out.push(`root is ${root.dataset.state}`);
    if (parseFloat(getComputedStyle(root).opacity) < 0.99) out.push("root not opaque");

    const name = (e: Element) => (e.className && typeof e.className == "string" ? "." + e.className.trim().split(/\s+/)[0] : e.tagName.toLowerCase());
    type Box = { l: number; t: number; r: number; b: number };
    const box = (r: DOMRect): Box => ({ l: r.left, t: r.top, r: r.right, b: r.bottom });
    const cut = (a: Box, c: Box): Box => ({ l: Math.max(a.l, c.l), t: Math.max(a.t, c.t), r: Math.min(a.r, c.r), b: Math.min(a.b, c.b) });
    const empty = (a: Box) => a.r - a.l < 0.5 || a.b - a.t < 0.5;
    const inside = (a: Box, c: Box, tol = 1) => a.l >= c.l - tol && a.t >= c.t - tol && a.r <= c.r + tol && a.b <= c.b + tol;
    const fmt = (a: Box) => `[${Math.round(a.l - hb.left)},${Math.round(a.t - hb.top)} → ${Math.round(a.r - hb.left)},${Math.round(a.b - hb.top)}]`;
    const H = box(hb);

    /** The element's parent in the flat tree, inside the shadow root. */
    const up = (e: Element) => (e.parentElement && sr.contains(e.parentElement) ? e.parentElement : null);
    /** The part of `a` its clipping ancestors leave visible. */
    const clipped = (e: Element, a: Box) => {
      for (let p = up(e); p; p = up(p)) {
        const cs = getComputedStyle(p);
        if (cs.overflowX != "visible" || cs.overflowY != "visible") {
          const pb = box(p.getBoundingClientRect());
          a = cut(a, {
            l: cs.overflowX == "visible" ? -1e9 : pb.l,
            r: cs.overflowX == "visible" ? 1e9 : pb.r,
            t: cs.overflowY == "visible" ? -1e9 : pb.t,
            b: cs.overflowY == "visible" ? 1e9 : pb.b,
          });
        }
      }
      return a;
    };
    const shown = (e: Element) => {
      for (let p: Element | null = e; p; p = up(p)) {
        const cs = getComputedStyle(p);
        if (cs.display == "none" || cs.visibility == "hidden" || parseFloat(cs.opacity) < 0.02) return false;
      }
      return true;
    };
    const alpha = (c: string) => {
      const m = /rgba?\(([^)]+)\)/.exec(c);
      if (!m) return c == "transparent" ? 0 : 1;
      const p = m[1].split(/[ ,/]+/).filter(Boolean);
      return p.length > 3 ? parseFloat(p[3]) : 1;
    };
    const painted = (cs: CSSStyleDeclaration) => alpha(cs.backgroundColor) > 0.05 || cs.backgroundImage != "none";
    const ownText = (e: Element) => [...e.childNodes].some((n) => n.nodeType == 3 && n.textContent!.trim());

    for (const e of sr.querySelectorAll<HTMLElement>("*")) {
      if (e.tagName == "STYLE" || !shown(e)) continue;
      const cs = getComputedStyle(e);
      const text = ownText(e);
      const visual = text || painted(cs) || e.tagName == "IMG" || e.tagName == "svg" || parseFloat(cs.borderTopWidth) + parseFloat(cs.borderLeftWidth) + parseFloat(cs.borderBottomWidth) > 0;
      if (!visual) continue;
      let b = box(e.getBoundingClientRect());
      if (text) {
        // Text: the glyph boxes of its own text nodes.
        const r = document.createRange();
        const parts = [...e.childNodes].filter((n) => n.nodeType == 3 && n.textContent!.trim());
        r.setStartBefore(parts[0]);
        r.setEndAfter(parts[parts.length - 1]);
        const tb = r.getBoundingClientRect();
        if (tb.width > 0) b = box(tb);
      }
      const v = clipped(e, b);
      if (empty(v)) continue;
      U = { l: Math.min(U.l, v.l), t: Math.min(U.t, v.t), r: Math.max(U.r, v.r), b: Math.max(U.b, v.b) };
      if (!text && v.r - v.l >= 0.98 * hb.width && v.b - v.t >= 0.98 * hb.height) backdrop = true;
      else C = { l: Math.min(C.l, v.l), t: Math.min(C.t, v.t), r: Math.max(C.r, v.r), b: Math.max(C.b, v.b) };
      if (!inside(v, H)) out.push(`${name(e)} outside the box: ${fmt(v)}`);
      if (!text) continue;
      // Text overflowing its own box (cut without an ellipsis, or spilling).
      if (cs.display != "inline" && e.clientWidth > 0) {
        const nowrap = cs.whiteSpace == "nowrap" || cs.whiteSpace == "pre";
        const clips = cs.overflowX != "visible";
        const ellipsis = cs.textOverflow == "ellipsis";
        if ((e.dataset.fit !== undefined || nowrap) && !ellipsis && e.scrollWidth > e.clientWidth + 1)
          out.push(`${name(e)} text overflows its width (${e.scrollWidth} > ${e.clientWidth})`);
        if (clips && !nowrap && e.scrollHeight > e.clientHeight + 1 && cs.webkitLineClamp == "none")
          out.push(`${name(e)} text cut at its height (${e.scrollHeight} > ${e.clientHeight})`);
      }
      // Text on a plate: the nearest painted ancestor holds it.
      for (let p = up(e); p; p = up(p)) {
        const ps = getComputedStyle(p);
        if (!painted(ps)) continue;
        const pb = clipped(p, box(p.getBoundingClientRect()));
        if (!inside(v, pb, 2)) out.push(`${name(e)} text outside its plate ${name(p)}: ${fmt(v)} vs ${fmt(pb)}`);
        break;
      }
    }
    if ((window as unknown as { __bbox?: boolean }).__bbox) out.push(`bbox ${fmt(U)} content ${fmt(C)}`);
    if (fill) {
      // How much of the box a union covers per axis, and how far its centre is off the box centre
      // (fractions of the box).
      const cover = (a: Box) => {
        const c = cut(a, H);
        return {
          x: empty(c) ? 0 : (c.r - c.l) / hb.width,
          y: empty(c) ? 0 : (c.b - c.t) / hb.height,
          dx: Math.abs((c.l + c.r) / 2 - (H.l + H.r) / 2) / hb.width,
          dy: Math.abs((c.t + c.b) / 2 - (H.t + H.b) / 2) / hb.height,
        };
      };
      const pct = (n: number) => `${Math.round(n * 100)}%`;
      const g = cover(U);
      const k = cover(C);
      const what = (c: ReturnType<typeof cover>, a: Box) => `${pct(c.x)} × ${pct(c.y)} ${fmt(a)}`;
      if (fill.policy == "panel") {
        const fills =
          (g.x >= fill.both && g.y >= fill.both) ||
          (g.x >= fill.locked && g.dy <= fill.centered) ||
          (g.y >= fill.locked && g.dx <= fill.centered);
        if (!fills) out.push(`graphic doesn't fill the box: ${what(g, U)}`);
        if (backdrop && Math.max(k.x, k.y) < fill.content) out.push(`content doesn't use the box: ${what(k, C)}`);
      } else if (fill.policy == "type") {
        if (Math.max(k.x, k.y) < fill.content) out.push(`content doesn't use the box: ${what(k, C)}`);
      } else if (Math.max(g.x, g.y) < fill.hug) out.push(`graphic doesn't reach the box: ${what(g, U)}`);
      if (fill.tight && (g.x < fill.tight || g.y < fill.tight)) out.push(`design box doesn't hug the graphic: ${what(g, U)}`);
      else if (fill.report) out.push(`hug ${what(g, U)}`);
    }
    return out;
  }, fill);
}

/** Ends the play-in at once: finite animations finished, infinite ones (scrolls) at their start. */
async function settleNow(page: Page) {
  await page.evaluate(async () => {
    await document.fonts.ready;
    const hosts = [...document.querySelectorAll("div")].filter((d) => d.shadowRoot);
    for (const a of hosts.flatMap((h) => h.shadowRoot!.getAnimations())) {
      if (a.effect?.getTiming().iterations === Infinity) {
        a.pause();
        a.currentTime = 0;
      } else a.finish();
    }
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  });
}

/** One screen, one layer, reused by every case of the worker. */
let screen: ScreenDoc | null = null;
const templates = new Map<string, string>();
let opened = "";

const PAD = 6;
const OUT = "review/preset-sizing";
/** Where a case's shot goes (the contact sheet reads them back: a failed test restarts the worker). */
const shotPath = (id: string, look: string, c: Case) => join(OUT, id, `${look}-${c.sizing}-${c.box}.png`);

async function run(page: Page, live: Fixture, id: string, look: string | undefined, c: Case, fields: Record<string, string>) {
  let t = templates.get(id);
  if (!t) templates.set(id, (t = await live.cmd(`addtemplate ${id}`)));
  const l: ScreenLayer = {
    id: "l1",
    name: "Layer",
    kind: "template",
    templateId: t,
    rect: c.rect,
    fields,
    vars: {},
    sizing: c.sizing,
    ...(look ? { variant: look } : {}),
  } as ScreenLayer;
  if (!screen) {
    screen = { id: `ps${Date.now().toString(36)}`, name: "Sizing", width: SCREEN.w, height: SCREEN.h, background: "#3a3f4b", layers: [l], updatedAt: "" };
  }
  await live.cmd("panic");
  await live.cmd(`savescreen ${JSON.stringify({ ...screen, layers: [l] })}`);
  if (opened !== page.url() || !opened) {
    await page.setViewportSize({ width: SCREEN.w, height: SCREEN.h });
    await openScreen(page, live.base, screen.id);
    opened = page.url();
  }
  await live.cmd(`layershow ${screen.id} l1`);
  await expect(page.locator("[data-state]")).toHaveAttribute("data-state", "in");
  await settleNow(page);
  // BBOX=1: report the union of the visible parts at the full screen (to size design boxes).
  await page.evaluate((on) => ((window as unknown as { __bbox?: boolean }).__bbox = on), !!process.env.BBOX && c.box === "full");
  const d = meta(id).design;
  const frame = (w: number, h: number) => w == SCREEN.w && h == SCREEN.h;
  const skip = frame(c.rect.w, c.rect.h) || (c.sizing == "fit" && (!d || frame(d.w, d.h)));
  const tight = c.sizing == "fit" && c.box == "design" && !HUG_EXEMPT[id] ? HUG : undefined;
  // HUG_REPORT=1: also report the coverage of the design boxes that pass (to size them).
  const report = !!process.env.HUG_REPORT && tight !== undefined;
  const problems = await inspect(page, skip ? null : { ...FILL, policy: POLICY[id] ?? "hug", tight, report });
  const r = c.rect;
  const clip = {
    x: Math.max(0, r.x - PAD),
    y: Math.max(0, r.y - PAD),
    width: Math.min(SCREEN.w, r.x + r.w + PAD) - Math.max(0, r.x - PAD),
    height: Math.min(SCREEN.h, r.y + r.h + PAD) - Math.max(0, r.y - PAD),
  };
  mkdirSync(join(OUT, id), { recursive: true });
  await page.screenshot({ clip, path: shotPath(id, look ?? "default", c) });
  return problems;
}

/** Contact sheet: one row per look × sizing, the boxes left to right, scaled down. */
async function sheet(page: Page, id: string, looks: string[]) {
  const list = looks.flatMap((look) =>
    cases(meta(id))
      .filter((c) => existsSync(shotPath(id, look, c)))
      .map((c) => ({ look, sizing: c.sizing, box: c.box, rect: c.rect, png: readFileSync(shotPath(id, look, c)).toString("base64") })),
  );
  const rows = new Map<string, typeof list>();
  for (const s of list) {
    const k = `${s.look} · ${s.sizing}`;
    rows.set(k, [...(rows.get(k) ?? []), s]);
  }
  const H = 150;
  const html = [...rows]
    .map(
      ([k, cells]) =>
        `<div class="row"><div class="k">${k}</div>${cells
          .map((s) => {
            const sc = Math.min(H / (s.rect.h + 2 * PAD), s.box === "bar" || s.box === "full" ? 640 / (s.rect.w + 2 * PAD) : 1);
            return `<figure><img src="data:image/png;base64,${s.png}" style="width:${Math.round((s.rect.w + 2 * PAD) * sc)}px"><figcaption>${s.box} ${s.rect.w}×${s.rect.h}</figcaption></figure>`;
          })
          .join("")}</div>`,
    )
    .join("");
  await page.setViewportSize({ width: 2600, height: 800 });
  await page.setContent(
    `<style>body{margin:12px;background:#15171c;color:#ccd;font:12px system-ui}.row{display:flex;gap:10px;align-items:flex-start;margin-bottom:10px}.k{width:150px;flex:none}figure{margin:0}img{display:block;outline:1px dashed #7dd3fc;outline-offset:-${PAD}px}figcaption{opacity:.6}</style><h3>${id}</h3>${html}`,
  );
  await page.screenshot({ path: join(OUT, `${id}.png`), fullPage: true });
  opened = "";
}

for (const id of PRESETS) {
  const m = meta(id);
  const looks: (string | undefined)[] = m.schemaVersion >= 2 ? (m.variants ?? []).map((v) => v.id) : [undefined];
  test.describe(`${id} `, () => {
    for (const look of looks) {
      test(`${look ?? "default"} in both modes`, async ({ page, styleServer }) => {
        test.setTimeout(120_000);
        const errors: string[] = [];
        page.on("pageerror", (e) => errors.push(e.message));
        opened = "";
        const fields = sampleOf(id);
        const problems: string[] = [];
        for (const c of cases(m)) {
          for (const p of await run(page, styleServer, id, look, c, fields)) problems.push(`${c.sizing} ${c.box}: ${p}`);
          if (process.env.HUG_REPORT) console.log(`HUG ${id} ${look ?? "default"} ${problems.filter((p) => /hug|bbox/.test(p)).join(" | ")}`);
        }
        await styleServer.cmd("panic");
        expect(problems).toEqual([]);
        expect(errors).toEqual([]);
      });
    }
    test("contact sheet", async ({ page }) => {
      await sheet(page, id, looks.map((l) => l ?? "default"));
    });
  });
}

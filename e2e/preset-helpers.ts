// Shared by e2e/presets.spec.ts and the thumbnail script (e2e/thumbs/presets.thumbs.ts).
import type { Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

/** The built-in presets, in gallery order. */
export const PRESETS = [
  "classic-bar",
  "minimal-line",
  "slide-in-accent",
  "news-strap",
  "corner-bug",
  "fullscreen-title",
  "leaderboard",
  "scoreboard",
  "ticker",
  "schedule-board",
  "now-next",
  "next-up",
  "countdown",
  "sponsor-slideshow",
  "sponsor-wall",
] as const;

interface FieldMeta {
  key: string;
  label: string;
  type: string;
  default: string;
  columns?: { key: string; label: string; kind?: "text" | "image" }[];
  format?: string;
}

function fieldsOf(id: string): FieldMeta[] {
  const path = join(process.cwd(), "presets", "templates", id, "template.json");
  return (JSON.parse(readFileSync(path, "utf8")) as { fields: FieldMeta[] }).fields;
}

/** A neutral placeholder picture for image cells (a label is not a URL; an empty logo hides
 *  its sponsor tile or slide). */
export const PLACEHOLDER_IMAGE =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 200 80'%3E%3Crect x='4' y='4' width='192' height='72' rx='12' fill='%238a8f98'/%3E%3C/svg%3E";

/** Sample rows for a list field: three rows of "<column label> <n>" (no invented text); image
 *  cells get the neutral placeholder. */
const sampleRows = (f: FieldMeta) =>
  [1, 2, 3].map((n) =>
    Object.fromEntries((f.columns ?? []).map((c) => [c.key, c.kind === "image" ? PLACEHOLDER_IMAGE : `${c.label} ${n}`])),
  );

/** The html of a preset's default look: `variants/<defaultVariant>/` (schema 2) or the folder root
 *  (schema 1). The fixture's `template <id>` loads that look. */
function htmlOf(id: string): string {
  const dir = join(process.cwd(), "presets", "templates", id);
  const meta = JSON.parse(readFileSync(join(dir, "template.json"), "utf8")) as { schemaVersion: number; defaultVariant?: string };
  const look = meta.schemaVersion >= 2 ? join(dir, "variants", meta.defaultVariant ?? "default") : dir;
  return readFileSync(join(look, "template.html"), "utf8");
}

/**
 * Every field of a preset as the editor previews it: text fields carry their label
 * ({"name":"Name", ...}), lists three sample rows of column labels, timers their default state,
 * clocks their format. No invented text.
 */
export function labelFields(id: string): Record<string, string> {
  return Object.fromEntries(
    fieldsOf(id).map((f) => {
      if (f.type === "list") return [f.key, JSON.stringify(sampleRows(f))];
      if (f.type === "timer") return [f.key, f.default];
      if (f.type === "clock") return [f.key, f.format || "HH:mm:ss"];
      return [f.key, f.label];
    }),
  );
}

/**
 * Text that must be visible once a preset plays in with `labelFields`: labels of fields bound as
 * text (not those bound only into attributes, like a status), and the first list row's text cells
 * (later rows may be hidden by design, e.g. a slideshow shows one slide).
 */
export function labelTexts(id: string): string[] {
  const html = htmlOf(id);
  return fieldsOf(id).flatMap((f) => {
    // Cells of the first row that the look shows as text (not those only in attributes).
    if (f.type === "list")
      return (f.columns ?? [])
        .filter((c) => c.kind !== "image" && new RegExp(`>[^<]*\\{\\{\\s*\\.${c.key}\\s*\\}\\}`).test(html))
        .map((c) => `${c.label} 1`);
    if (f.type === "timer" || f.type === "clock") return [];
    const asText = new RegExp(`>[^<]*\\{\\{\\s*${f.key}\\s*\\}\\}`).test(html);
    return asText ? [f.label] : [];
  });
}

// Playwright's CSS locators pierce open shadow roots, so this finds the template root inside a
// layer's shadow DOM.
export const root = (page: Page) => page.locator("[data-state]");

/** Opens a screen and waits until the renderer has received its snapshot. Collects page errors. */
export async function openScreen(page: Page, base: string, screen = "test") {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${screen}`);
  const socket = await ws;
  await socket.waitForEvent(
    "framereceived",
    (f) => String(f.payload).includes('"t":"snapshot"') || String(f.payload).includes('"t":"error"'),
  );
  return errors;
}

/** Resolves once every animation on the template root (and its subtree) has finished and fonts are in. */
export async function animationsSettled(page: Page) {
  await root(page)
    .first()
    .evaluate(async (el) => {
      // Infinite animations (the ticker's scroll) never finish: only wait for the finite ones.
      const finite = el.getAnimations({ subtree: true }).filter((a) => a.effect?.getTiming().iterations !== Infinity);
      await Promise.all(finite.map((a) => a.finished));
      await document.fonts.ready;
    });
}

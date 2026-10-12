// Shared by e2e/styles.spec.ts and e2e/styles-apply.spec.ts: the library presets, their sample
// data, one-layer screens of a project template, and animation control in the real renderer.
import type { Page } from "@playwright/test";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { ScreenDoc, ScreenLayer } from "../src/lib/types";
import type { Fixture } from "./fixture";

const PRESETS_DIR = join(process.cwd(), "presets");

/** The library presets (one per template type), in gallery order. */
export const LIBRARY: string[] = (
  JSON.parse(readFileSync(join(PRESETS_DIR, "library.json"), "utf8")) as {
    presets: { id: string; legacy?: boolean }[];
  }
).presets
  .filter((p) => !p.legacy)
  .map((p) => p.id);

/** The styles in library order, each with the library presets that have its look on disk
 *  (`presets/templates/<id>/variants/<style>/template.html`). Classic is not a style. */
export const STYLE_LOOKS: { style: string; presets: string[] }[] = (
  JSON.parse(readFileSync(join(PRESETS_DIR, "library.json"), "utf8")) as { styles: { id: string }[] }
).styles
  .map((s) => ({
    style: s.id,
    presets: LIBRARY.filter((id) => existsSync(join(PRESETS_DIR, "templates", id, "variants", s.id, "template.html"))),
  }))
  .filter((s) => s.presets.length > 0);

/** Field values as the fixture takes them: lists and other JSON values as compact JSON strings. */
export function asFields(values: Record<string, unknown>): Record<string, string> {
  return Object.fromEntries(
    Object.entries(values).map(([k, v]) => [k, typeof v === "string" ? v : JSON.stringify(v)]),
  );
}

/** A preset's sample data (`presets/templates/<id>/sample.json`). */
export function sample(id: string): Record<string, string> {
  return asFields(JSON.parse(readFileSync(join(PRESETS_DIR, "templates", id, "sample.json"), "utf8")));
}

export const BG = { light: "#f2f2ee", dark: "#0b0d12" } as const;

let seq = 0;
/** A unique screen id for this run. */
export const screenId = (prefix = "st") => `${prefix}${Date.now().toString(36)}${seq++}`;

export function layer(
  id: string,
  templateId: string,
  w: number,
  h: number,
  fields: Record<string, string>,
  extra: Partial<ScreenLayer> = {},
): ScreenLayer {
  return {
    id,
    name: id,
    kind: "template",
    templateId,
    rect: { x: 0, y: 0, w, h },
    fields,
    vars: {},
    ...extra,
  } as ScreenLayer;
}

/** Saves a screen (new id) with the given layers → the doc. */
export async function saveScreen(
  live: Fixture,
  w: number,
  h: number,
  background: string,
  layers: ScreenLayer[],
): Promise<ScreenDoc> {
  const doc: ScreenDoc = { id: screenId(), name: "Style", width: w, height: h, background, layers, updatedAt: "" };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

/** The template roots in the page (one per layer host); CSS locators pierce open shadow roots. */
export const roots = (page: Page) => page.locator("[data-state]");

/** Opens `/screen/<id>` and waits for the renderer's snapshot. Collects page errors. */
export async function openScreen(page: Page, base: string, id: string) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}`);
  const socket = await ws;
  await socket.waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
  return errors;
}

/**
 * Resolves once every finite animation in every layer has finished and fonts are in; infinite
 * ones (the ticker's scroll) are paused at their start, so the still is deterministic.
 */
export async function settle(page: Page) {
  await page.evaluate(async () => {
    const hosts = [...document.querySelectorAll("div")].filter((d) => d.shadowRoot);
    const all = hosts.flatMap((h) => h.shadowRoot!.getAnimations());
    for (const a of all) {
      if (a.effect?.getTiming().iterations === Infinity) {
        a.pause();
        a.currentTime = 0;
      }
    }
    await Promise.all(all.filter((a) => a.effect?.getTiming().iterations !== Infinity).map((a) => a.finished));
    await document.fonts.ready;
    // The data-fit pass runs at the next frame after a change.
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  });
}

/**
 * Pauses every animation in the page's layers `fraction` of the way through the play-in (the
 * latest end among the finite ones): every animation is set to the same time since the take, so
 * the shot shows the motion order. Returns that time in ms.
 */
export async function freezeMidway(page: Page, fraction = 0.4) {
  return page.evaluate((f) => {
    const hosts = [...document.querySelectorAll("div")].filter((d) => d.shadowRoot);
    const all = hosts.flatMap((h) => h.shadowRoot!.getAnimations());
    const end = Math.max(
      0,
      ...all
        .filter((a) => a.effect?.getTiming().iterations !== Infinity)
        .map((a) => Number(a.effect?.getComputedTiming().endTime ?? 0)),
    );
    const t = Math.round(end * f);
    for (const a of all) {
      a.pause();
      a.currentTime = a.effect?.getTiming().iterations === Infinity ? 0 : t;
    }
    return t;
  }, fraction);
}

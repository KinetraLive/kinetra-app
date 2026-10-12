// Shared by studio.spec.ts and studio.measure.ts (Studio mode, Preview / Program): a screen with
// two layers of the two-look template (variant-helpers.ts: 250 ms in/out animations), Program
// (/screen/<id>) and Preview (/screen/<id>/preview) pages, the studio commands, what the
// renderer's layer hosts look like right now, and a recorder of what happens to them during a
// take or a T-bar (MutationObservers + opacity samples per frame, installed before the command).

import type { BrowserContext, Page, WebSocket } from "@playwright/test";
import type { Fixture } from "./fixture";
import type { LiveLayerView, PreviewView, ScreenDoc, StudioOp, StudioSettings, TakeTransition } from "../src/lib/types";

export const CUT: TakeTransition = { kind: "cut", ms: 100 };

export const settings = (s: Partial<StudioSettings> = {}): StudioSettings => ({
  enabled: true,
  previewAfterTake: "keep",
  transition: CUT,
  ...s,
});

/** `set_studio` (`exit`: take | discard | keepStaged). */
export const studio = (live: Fixture, s: Partial<StudioSettings> = {}, exit?: "take" | "discard" | "keepStaged") =>
  live.cmd(`studio ${JSON.stringify(settings(s))}${exit ? ` ${exit}` : ""}`);

/** Studio mode off whatever the Preview holds (a test's leftovers are discarded). */
export const studioOff = (live: Fixture) => studio(live, { enabled: false }, "discard");

export const op = async (live: Fixture, screen: string, o: StudioOp) =>
  JSON.parse(await live.cmd(`studioop ${screen} ${JSON.stringify(o)}`)) as PreviewView;

export const cue = (live: Fixture, screen: string, layerId: string, visible: boolean | null) =>
  op(live, screen, { op: "cue", layerId, visible });

export const previewOf = async (live: Fixture, screen: string) =>
  JSON.parse(await live.cmd(`preview ${screen}`)) as PreviewView;

export const layerView = async (live: Fixture, screen: string, layer: string) =>
  JSON.parse(await live.cmd(`layerview ${screen} ${layer}`)) as LiveLayerView;

let seq = 0;
/** A screen with two full-screen layers of `templateId`: `a` shows "Alpha", `b` "Bravo". */
export async function twoLayerScreen(live: Fixture, templateId: string): Promise<ScreenDoc> {
  const layer = (id: string, name: string) => ({
    id,
    name,
    kind: "template" as const,
    templateId,
    rect: { x: 0, y: 0, w: 1920, h: 1080 },
    fields: { name, title: "Studio", sponsor: "" },
    vars: {},
  });
  const doc: ScreenDoc = {
    id: `st${Date.now().toString(36)}${seq++}`,
    name: "Studio",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [layer("a", "Alpha"), layer("b", "Bravo")],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

/** Opens a renderer page (Program, or the Preview bus) and waits for its snapshot. */
export async function openBus(page: Page, base: string, id: string, bus: "program" | "preview"): Promise<WebSocket> {
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${id}${bus === "preview" ? "/preview" : ""}`);
  const socket = await ws;
  await socket.waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
  return socket;
}

/** Program and Preview of a screen side by side (two pages of the test's context). */
export async function openBoth(page: Page, context: BrowserContext, base: string, id: string) {
  const pv = await context.newPage();
  await openBus(page, base, id, "program");
  await openBus(pv, base, id, "preview");
  return { prog: page, pv };
}

export interface HostInfo {
  /** The template's `name` text (identifies the layer: "Alpha", "Bravo", or an edited name). */
  name: string;
  title: string;
  look: string;
  state: string;
  /** Computed opacity of the host. */
  opacity: number;
  /** Finite animations running in the shadow root. */
  running: number;
}

/** Every layer host in the page, in DOM order. */
export const hosts = (page: Page) =>
  page.evaluate(() =>
    [...document.querySelectorAll("div")]
      .filter((d) => d.shadowRoot)
      .map((h) => {
        const r = h.shadowRoot!.querySelector("[data-state]");
        return {
          name: r?.querySelector("b")?.textContent ?? "?",
          title: r?.querySelector("i")?.textContent ?? "",
          look: r?.getAttribute("data-look") ?? "?",
          state: r?.getAttribute("data-state") ?? "-",
          opacity: +getComputedStyle(h).opacity,
          running: h.shadowRoot!
            .getAnimations()
            .filter((a) => a.playState === "running" && a.effect?.getComputedTiming().iterations !== Infinity).length,
        };
      }),
  ) as Promise<HostInfo[]>;

/** `name:look:state` per host. */
export const shown = async (page: Page) => (await hosts(page)).map((h) => `${h.name}:${h.look}:${h.state}`);

/** Waits for every finite animation in the page's layers to end. */
export async function settledAll(page: Page) {
  await page.evaluate(() =>
    Promise.all(
      [...document.querySelectorAll("div")]
        .filter((d) => d.shadowRoot)
        .flatMap((h) => h.shadowRoot!.getAnimations().map((a) => a.finished.catch(() => null))),
    ),
  );
}

export interface StEvent {
  /** Epoch ms. */
  t: number;
  kind: "hosts" | "state" | "anim";
  /** hosts: `name:state:running` per host in DOM order. */
  hosts?: string[];
  /** state: the new data-state; anim: the animation name. */
  v?: string | null;
  /** The layer's name text. */
  name?: string | null;
}

export interface StRec {
  max: number;
  events: StEvent[];
  /** Host opacities per animation frame: `[t, {name: opacity}]`. */
  samples: [number, Record<string, number>][];
}

/**
 * Records the page's layer hosts from now on: hosts added/removed under the stage box (with
 * every host's name, data-state and running finite animations at that moment), data-state
 * changes and animation starts in the shadow roots, the most hosts at once, and the host
 * opacities on every animation frame. Works with no host on air (it watches the stage box).
 */
export function startRec(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as { __st: StRec; __stStop?: () => void };
    w.__stStop?.();
    const box = document.querySelector("body > div > div");
    if (!box) throw new Error("no renderer stage");
    const all = () => [...box.children].filter((d) => d.shadowRoot);
    const rec: StRec = { max: all().length, events: [], samples: [] };
    w.__st = rec;
    const now = () => performance.timeOrigin + performance.now();
    const nameOf = (h: Element) => h.shadowRoot!.querySelector("[data-state] b")?.textContent ?? "?";
    const running = (h: Element) =>
      h.shadowRoot!
        .getAnimations()
        .filter((a) => a.playState === "running" && a.effect?.getComputedTiming().iterations !== Infinity).length;
    const observers: MutationObserver[] = [];
    const watched = new WeakSet<Element>();
    const watch = (h: Element) => {
      if (watched.has(h)) return;
      watched.add(h);
      const mo = new MutationObserver((ms) => {
        for (const m of ms) {
          const el = m.target as Element;
          rec.events.push({ t: now(), kind: "state", v: el.getAttribute("data-state"), name: nameOf(h) });
        }
      });
      mo.observe(h.shadowRoot!, { subtree: true, attributes: true, attributeFilter: ["data-state"] });
      observers.push(mo);
      h.shadowRoot!.addEventListener(
        "animationstart",
        (e) => rec.events.push({ t: now(), kind: "anim", v: (e as AnimationEvent).animationName, name: nameOf(h) }),
        true,
      );
    };
    all().forEach(watch);
    const mo = new MutationObserver(() => {
      const hs = all();
      hs.forEach(watch);
      rec.max = Math.max(rec.max, hs.length);
      rec.events.push({
        t: now(),
        kind: "hosts",
        hosts: hs.map((h) => `${nameOf(h)}:${h.shadowRoot!.querySelector("[data-state]")?.getAttribute("data-state") ?? "-"}:${running(h)}`),
      });
    });
    mo.observe(box, { childList: true });
    observers.push(mo);
    let live = true;
    const tick = () => {
      if (!live) return;
      const o: Record<string, number> = {};
      for (const h of all()) o[nameOf(h)] = +getComputedStyle(h).opacity;
      rec.samples.push([now(), o]);
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    w.__stStop = () => {
      live = false;
      observers.forEach((o) => o.disconnect());
    };
  });
}

export const rec = (page: Page) => page.evaluate(() => (window as unknown as { __st: StRec }).__st);

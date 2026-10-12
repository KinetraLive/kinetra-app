// Shared by variants.spec.ts and variants.measure.ts: a template with two looks ("default" with
// name, title and sponsor; "compact" with the name only), a one-layer screen using it, and a
// recorder of what the renderer does to the layer's hosts during a swap.

import type { Page } from "@playwright/test";
import type { Fixture } from "./fixture";
import type { ScreenDoc } from "../src/lib/types";

const CSS = [
  ".lt{position:absolute;left:80px;bottom:80px;display:flex;gap:16px;padding:12px 24px;font:32px sans-serif;color:#fff;background:#123}",
  ".lt[data-state=in]{animation:lt-in 250ms cubic-bezier(.2,0,0,1) both}",
  ".lt[data-state=out]{animation:lt-out 250ms cubic-bezier(.2,0,0,1) both}",
  "@keyframes lt-in{from{opacity:0;transform:translateY(24px)}to{opacity:1;transform:none}}",
  "@keyframes lt-out{from{opacity:1;transform:none}to{opacity:0;transform:translateY(24px)}}",
].join("\n");

export const LOOK = {
  default: {
    html: '<div class="lt" data-look="default"><b>{{name}}</b><i>{{title}}</i><u>{{sponsor}}</u></div>',
    css: CSS,
  },
  compact: {
    html: '<div class="lt" data-look="compact"><b>{{name}}</b></div>',
    css: CSS + "\n.lt[data-look=compact]{font-size:24px;background:#312}",
  },
};

export const FIELDS = [
  { key: "name", label: "Name", type: "text", default: "" },
  { key: "title", label: "Title", type: "text", default: "" },
  { key: "sponsor", label: "Sponsor", type: "text", default: "" },
];

/** A project template with the "default" look plus a "Compact" variant → their ids. */
export async function twoLooks(live: Fixture): Promise<{ template: string; compact: string }> {
  const template = await live.cmd(
    `addtemplatedoc ${JSON.stringify({ name: "Variants", ...LOOK.default, fields: FIELDS })}`,
  );
  const compact = await live.cmd(`addvariant ${template} default Compact`);
  await live.cmd(`savevariant ${template} ${compact} ${JSON.stringify(LOOK.compact)}`);
  return { template, compact };
}

let seq = 0;
/** A screen with one full-screen layer `l1` of `templateId`. */
export async function oneLayer(
  live: Fixture,
  templateId: string,
  fields: Record<string, string> = { name: "Ada Lovelace", title: "Analyst", sponsor: "ACME" },
): Promise<ScreenDoc> {
  const doc: ScreenDoc = {
    id: `var${Date.now().toString(36)}${seq++}`,
    name: "Variants",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      {
        id: "l1",
        name: "Lower third",
        kind: "template",
        templateId,
        rect: { x: 0, y: 0, w: 1920, h: 1080 },
        fields,
        vars: {},
      },
    ],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

/** The look on air: `data-look` of every template root in the page (one per host). */
export const looks = (page: Page) =>
  page.evaluate(() =>
    [...document.querySelectorAll("div")]
      .filter((d) => d.shadowRoot)
      .map((h) => h.shadowRoot!.querySelector("[data-look]")?.getAttribute("data-look") ?? "?"),
  );

export interface RecEvent {
  /** Epoch ms (performance.timeOrigin + now). */
  t: number;
  kind: "hosts" | "state" | "anim" | "tree";
  /** hosts: the template roots, `<look>:<data-state>` per host in DOM order. */
  hosts?: string[];
  /** state: the new data-state of a root; anim: the animation name. */
  v?: string | null;
  /** tree (a shadow root got a new look): finite animations still running on its root. */
  running?: number;
  look?: string | null;
}

export interface Rec {
  max: number;
  events: RecEvent[];
}

/**
 * Starts recording the layer hosts of the page (one layer on air): host additions/removals (with
 * every root's look and data-state at that moment), data-state changes and animation starts in
 * the shadow roots, a host recompiled in place (`tree`), and the most hosts seen at once. Install it before the command.
 */
export function startRecording(page: Page) {
  return page.evaluate(() => {
    const w = window as unknown as { __rec: Rec; __recStop?: () => void };
    w.__recStop?.();
    const all = () => [...document.querySelectorAll("div")].filter((d) => d.shadowRoot);
    const first = all()[0];
    if (!first?.parentElement) throw new Error("no layer host on the page");
    const stage = first.parentElement;
    const rec: Rec = { max: all().length, events: [] };
    w.__rec = rec;
    const now = () => performance.timeOrigin + performance.now();
    const rootOf = (h: Element) => h.shadowRoot!.querySelector("[data-state]");
    const observers: MutationObserver[] = [];
    const watched = new WeakSet<Element>();
    const onAnim = (e: Event) =>
      rec.events.push({
        t: now(),
        kind: "anim",
        v: (e as AnimationEvent).animationName,
        look: (e.target as Element).getAttribute?.("data-look") ?? null,
      });
    const watch = (h: Element) => {
      if (watched.has(h)) return;
      watched.add(h);
      const mo = new MutationObserver((ms) => {
        for (const m of ms) {
          if (m.type === "childList") {
            if (m.target !== h.shadowRoot) continue;
            const r = rootOf(h);
            const running = (r?.getAnimations({ subtree: true }) ?? []).filter(
              (a) => a.playState === "running" && a.effect?.getComputedTiming().iterations !== Infinity,
            ).length;
            rec.events.push({ t: now(), kind: "tree", look: r?.getAttribute("data-look") ?? null, running });
            continue;
          }
          const el = m.target as Element;
          rec.events.push({ t: now(), kind: "state", v: el.getAttribute("data-state"), look: el.getAttribute("data-look") });
        }
      });
      mo.observe(h.shadowRoot!, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-state"] });
      observers.push(mo);
      h.shadowRoot!.addEventListener("animationstart", onAnim, true);
    };
    all().forEach(watch);
    const mo = new MutationObserver(() => {
      const hs = all();
      hs.forEach(watch);
      rec.max = Math.max(rec.max, hs.length);
      rec.events.push({
        t: now(),
        kind: "hosts",
        hosts: hs.map((h) => {
          const r = rootOf(h);
          return `${r?.getAttribute("data-look") ?? "?"}:${r?.getAttribute("data-state") ?? "-"}`;
        }),
      });
    });
    mo.observe(stage, { childList: true });
    observers.push(mo);
    w.__recStop = () => observers.forEach((o) => o.disconnect());
  });
}

export const recording = (page: Page) => page.evaluate(() => (window as unknown as { __rec: Rec }).__rec);

/** Waits for the layer's finite animations to end (the graphic has settled in). */
export async function settled(page: Page) {
  await page.evaluate(() =>
    Promise.all(
      [...document.querySelectorAll("div")]
        .filter((d) => d.shadowRoot)
        .flatMap((h) => h.shadowRoot!.getAnimations().map((a) => a.finished.catch(() => null))),
    ),
  );
}

// Applying a style and the brand kit, against the real server and renderer: `apply_style` with the
// project scope switches every library layer (field data kept, custom templates unchanged), on
// air it follows the live-edit mode (onTake stages it and the take swaps in place without
// replaying the in-animation; immediate swaps at once), undo restores; the brand kit's accent
// and logo reach the renderer, and a layer's own values still win.
// Each test gets its own fixture server: applying to the project touches every screen in it.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, isolatedTest as test, type Fixture } from "./fixture";
import { layer, openScreen, roots, sample, saveScreen, settle } from "./style-helpers";
import { recording, startRecording } from "./variant-helpers";
import type { ApplyStyleResult, LiveLayerView } from "../src/lib/types";

const W = 1920;
const H = 1080;

const view = async (live: Fixture, screen: string, id: string) =>
  JSON.parse(await live.cmd(`layerview ${screen} ${id}`)) as LiveLayerView;

/** A blank-made template, not from a library preset. */
const CUSTOM = {
  name: "Custom",
  html: '<div class="cu"><b>{{name}}</b></div>',
  css: ".cu{position:absolute;left:40px;top:40px;font:32px sans-serif;color:#fff}",
  fields: [{ key: "name", label: "Name", type: "text", default: "" }],
};

/** Frame's fullscreen title has a `.ft__stage`; the classic look does not. */
const frameTitle = (page: Page) => page.locator(".ft__stage");

/** Project with a fullscreen title, a corner bug (both classic by default) and a custom layer. */
async function project(live: Fixture) {
  const title = await live.cmd("addtemplate fullscreen-title");
  const bug = await live.cmd("addtemplate corner-bug");
  const custom = await live.cmd(`addtemplatedoc ${JSON.stringify(CUSTOM)}`);
  const titleFields: Record<string, string> = { ...sample("fullscreen-title"), logo: "" };
  const doc = await saveScreen(live, W, H, "transparent", [
    layer("title", title, W, H, titleFields),
    layer("bug", bug, W, H, sample("corner-bug")),
    layer("custom", custom, W, H, { name: "Custom layer" }),
  ]);
  return { doc, titleFields };
}

test("Frame on the project switches library layers, keeps their data and undoes", async ({ page, own }) => {
  const { doc, titleFields } = await project(own);
  const before = await Promise.all(["title", "bug", "custom"].map((l) => view(own, doc.id, l)));
  expect(before.map((v) => v.variant)).toEqual(["classic", "classic", expect.any(String)]);

  const res = JSON.parse(await own.cmd("applystyle frame project")) as ApplyStyleResult;
  expect(res).toMatchObject({ switched: 2, unchanged: 1, unchangedTemplates: 1 });
  expect(res.undo).toBeTruthy();

  const after = await Promise.all(["title", "bug", "custom"].map((l) => view(own, doc.id, l)));
  expect(after.map((v) => v.variant)).toEqual(["frame", "frame", before[2].variant]);
  after.forEach((v, i) => expect(v.fields).toEqual(before[i].fields));
  expect(after[0].fields).toMatchObject(titleFields);

  const dir = await own.cmd("dir");
  await expect
    .poll(() => JSON.parse(readFileSync(join(dir, "project.json"), "utf8")).settings?.style, { timeout: 3_000 })
    .toBe("frame");

  // The output shows Frame with the layer's data.
  const errors = await openScreen(page, own.base, doc.id);
  await own.cmd(`layershow ${doc.id} title`);
  await expect(frameTitle(page)).toHaveCount(1);
  await expect(page.getByText(titleFields.title)).toBeVisible();
  await own.cmd(`layerhide ${doc.id} title`);
  await expect(roots(page)).toHaveCount(0, { timeout: 4_000 });

  await own.cmd(`undostyle ${res.undo}`);
  const undone = await Promise.all(["title", "bug", "custom"].map((l) => view(own, doc.id, l)));
  expect(undone.map((v) => v.variant)).toEqual(before.map((v) => v.variant));
  await own.cmd(`layershow ${doc.id} title`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await expect(frameTitle(page)).toHaveCount(0);
  await expect(page.getByText(titleFields.title)).toBeVisible();
  // A second undo with the same token has nothing left to undo.
  await expect(own.cmd(`undostyle ${res.undo}`)).rejects.toThrow(/Nothing to undo/);
  expect(errors).toEqual([]);
});

test("on air under onTake, Frame waits for the take, which swaps in place without replaying", async ({ page, own }) => {
  await own.cmd("liveedit onTake");
  const { doc } = await project(own);
  await openScreen(page, own.base, doc.id);
  await own.cmd(`layershow ${doc.id} title`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await settle(page);

  await startRecording(page);
  await own.cmd("applystyle frame project");
  expect(await view(own, doc.id, "title")).toMatchObject({ onAir: true, pending: true, variant: "frame" });
  await page.waitForTimeout(400);
  await expect(frameTitle(page)).toHaveCount(0);
  expect((await recording(page)).events).toEqual([]);

  await own.cmd(`layershow ${doc.id} title`);
  await expect(frameTitle(page)).toHaveCount(1);
  await expect(roots(page)).toHaveCount(1);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await page.waitForTimeout(400);
  const rec = await recording(page);
  expect(rec.max).toBe(1);
  expect(rec.events.filter((e) => e.kind === "state" && e.v === "out")).toEqual([]);
  expect(rec.events.filter((e) => e.kind === "hosts")).toEqual([]);
  // Recompiled on the same host with its in-animation already finished (not replayed).
  const trees = rec.events.filter((e) => e.kind === "tree");
  expect(trees.length).toBeGreaterThan(0);
  // (animationstart still fires for the finished-at-once animations; none is left running.)
  expect(trees.every((e) => e.running === 0)).toBe(true);
  expect(await view(own, doc.id, "title")).toMatchObject({ onAir: true, pending: false, variant: "frame" });
});

test("on air under immediate, Frame swaps at once and undo swaps back", async ({ page, own }) => {
  await own.cmd("liveedit immediate");
  const { doc } = await project(own);
  await openScreen(page, own.base, doc.id);
  await own.cmd(`layershow ${doc.id} title`);
  await expect(roots(page)).toHaveAttribute("data-state", "in");
  await settle(page);

  const res = JSON.parse(await own.cmd("applystyle frame project")) as ApplyStyleResult;
  expect(await view(own, doc.id, "title")).toMatchObject({ onAir: true, pending: false, variant: "frame" });
  await expect(frameTitle(page)).toHaveCount(1);
  await expect(roots(page)).toHaveCount(1, { timeout: 4_000 });
  await expect(roots(page)).toHaveAttribute("data-state", "in");

  await own.cmd(`undostyle ${res.undo}`);
  expect(await view(own, doc.id, "title")).toMatchObject({ onAir: true, pending: false, variant: "classic" });
  await expect(frameTitle(page)).toHaveCount(0, { timeout: 4_000 });
  await expect(roots(page)).toHaveCount(1, { timeout: 4_000 });
  await expect(roots(page)).toHaveAttribute("data-state", "in");
});

test("the brand kit sets the accent and the logo; a layer's own values win", async ({ page, own }) => {
  const asset = await own.cmd(`importasset ${join(process.cwd(), "presets", "thumbs", "classic-bar.png")}`);
  expect(asset).toMatch(/^\/assets\//);
  const lt = await own.cmd("addtemplate lower-third");
  const fields = { ...sample("lower-third"), logo: "" };
  const own2 = { ...sample("lower-third") }; // keeps its sample logo (a data: image)
  const doc = await saveScreen(own, W, H, "transparent", [
    layer("branded", lt, W, H, fields),
    layer("custom", lt, W, H, own2, { vars: { accent: "#00ff00" } }),
  ]);
  await own.cmd(`brand ${JSON.stringify({ accent: "#ff3366", logo: asset })}`);

  const errors = await openScreen(page, own.base, doc.id);
  await own.cmd(`layershow ${doc.id} branded`);
  await own.cmd(`layershow ${doc.id} custom`);
  await expect(roots(page)).toHaveCount(2);

  const hosts = () =>
    page.evaluate(() =>
      [...document.querySelectorAll("div")]
        .filter((d) => d.shadowRoot)
        .sort((a, b) => Number(a.style.zIndex) - Number(b.style.zIndex))
        .map((h) => ({
          accent: h.style.getPropertyValue("--accent").trim(),
          logo: h.shadowRoot!.querySelector(".lt__logo")?.getAttribute("src") ?? null,
        })),
    );
  await expect.poll(hosts).toEqual([
    { accent: "#ff3366", logo: asset },
    { accent: "#00ff00", logo: own2.logo },
  ]);
  // The brand logo is an image that loads from the output server.
  await expect
    .poll(() => page.locator(".lt__logo").first().evaluate((img) => (img as HTMLImageElement).naturalWidth))
    .toBeGreaterThan(0);
  expect(errors).toEqual([]);
});

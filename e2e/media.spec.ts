// Media playback against the real server (fixture_server) in Edge, with the clips from
// e2e/media/make-fixtures.mjs (every frame carries its frame number at 30 fps as a block code,
// read back from the video element with a canvas).
//
// Covers: preload + `ready` before Take (readiness in the layer view), Take plays from the in
// point without a black frame, end actions hold / clear / loop, transport (pause / seek / stop),
// two renderers of the same screen within 2 frames, the preload cap, `?mute=1` previews (muted,
// not counted), H.264 MP4, template video fields, and the media chunk's size.

import type { Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";
import { expect, test, type Fixture } from "./fixture";
import type { MediaLayerConfig, ScreenDoc, ScreenLayer } from "../src/lib/types";

// Unmuted media may start without a user gesture (OBS and vMix run their browsers this way).
test.use({ launchOptions: { args: ["--autoplay-policy=no-user-gesture-required"] } });

const DIR = join(process.cwd(), "e2e/media");
const FPS = 30;
const FRAME_MS = 1000 / FPS;

const assets = new Map<string, string>();
/** Imports a clip once per server → its asset name. */
async function clip(live: Fixture, file: string): Promise<string> {
  let a = assets.get(file);
  if (!a) {
    const url = await live.cmd(`importmedia ${join(DIR, file)}`);
    a = url.replace(/^\/assets\//, "");
    assets.set(file, a);
  }
  return a;
}

let seq = 0;
const RECT = { x: 0, y: 0, w: 1920, h: 1080 };

function mediaLayer(id: string, asset: string, over: Partial<MediaLayerConfig> = {}): ScreenLayer {
  return {
    id,
    name: id,
    kind: "media",
    rect: RECT,
    media: { asset, fit: "fit", inMs: 0, outMs: null, volume: 100, muted: false, endAction: "hold", autoplay: true, ...over },
  } as unknown as ScreenLayer;
}

async function makeScreen(live: Fixture, layers: ScreenLayer[]): Promise<string> {
  const doc: ScreenDoc = {
    id: `md${Date.now().toString(36)}${seq++}`.slice(0, 20),
    name: "Media",
    width: 1920,
    height: 1080,
    background: "#000000",
    layers,
    updatedAt: "",
  };
  return live.cmd(`savescreen ${JSON.stringify(doc)}`);
}

/** Records, in the page, when the snapshot arrived, when each `ready` went out and when the
 * first take (a layer going `in`) arrived. */
async function open(page: Page, base: string, screen: string, query = "") {
  await page.addInitScript(() => {
    const w = window as unknown as { __t: Record<string, number> };
    w.__t = {};
    const send = WebSocket.prototype.send;
    WebSocket.prototype.send = function (d) {
      const m = JSON.parse(String(d));
      if (m.t == "ready") w.__t["ready:" + m.layerId] ??= performance.now();
      return send.call(this, d);
    };
    const desc = Object.getOwnPropertyDescriptor(WebSocket.prototype, "onmessage")!;
    Object.defineProperty(WebSocket.prototype, "onmessage", {
      set(f) {
        desc.set!.call(this, (e: MessageEvent) => {
          const t = String(e.data).slice(0, 20);
          if (t.includes('"snapshot"')) w.__t.snapshot ??= performance.now();
          const d = String(e.data);
          if (d.includes('"t":"state"') && d.includes('"in"')) w.__t.take ??= performance.now();
          return f?.(e);
        });
      },
      get: desc.get,
    });
  });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const ws = page.waitForEvent("websocket");
  await page.goto(`${base}/screen/${screen}${query}`);
  await (await ws).waitForEvent("framereceived", (f) => String(f.payload).includes('"t":"snapshot"'));
  return errors;
}

interface View {
  onAir: boolean;
  media?: { playback: { state: string; at: number; posMs: number }; readiness: Readiness } | null;
  /** Template layers with video / image fields. */
  readiness?: Readiness | null;
}
interface Readiness {
  ready: number;
  total: number;
  error: string | null;
}
const view = async (live: Fixture, s: string, l: string): Promise<View> => JSON.parse(await live.cmd(`layerview ${s} ${l}`));

/** The frame number the layer's video shows now (block code), or -1 when it shows nothing. */
async function frame(page: Page, layer = 0): Promise<number> {
  return page.evaluate((i) => readFrame(i), layer);
}

/** Installs `readFrame(i)` (i-th video on the page) and `presented(i)` in the page. */
async function helpers(page: Page) {
  await page.addInitScript(() => {
    const vids = () =>
      Array.from(document.querySelectorAll("div"))
        .flatMap((d) => Array.from(d.shadowRoot?.querySelectorAll("video") ?? []))
        .filter(Boolean) as HTMLVideoElement[];
    const g = new OffscreenCanvas(480, 270).getContext("2d", { willReadFrequently: true })!;
    const read = (v: HTMLVideoElement) => {
      if (v.readyState < 2) return -1;
      g.clearRect(0, 0, 480, 270);
      g.drawImage(v, 0, 0, 480, 270);
      const grey = g.getImageData(15, 15, 1, 1).data;
      if (Math.abs(grey[0] - 128) > 40) return -1;
      let n = 0;
      for (let i = 0; i < 10; i++) if (g.getImageData(40 + 48 * i, 40, 1, 1).data[0] > 128) n |= 1 << i;
      return n;
    };
    Object.assign(window, {
      vids,
      readFrame: (i: number) => {
        const v = vids()[i];
        return v ? read(v) : -1;
      },
      /** The next presented frame: its media time and the wall-clock time it is displayed. */
      presented: (i: number) =>
        new Promise<{ media: number; wall: number }>((r) =>
          vids()[i].requestVideoFrameCallback((_, md) => r({ media: md.mediaTime * 1000, wall: performance.timeOrigin + md.expectedDisplayTime })),
        ),
    });
  });
}

declare function readFrame(i: number): number;

async function readyOn(live: Fixture, s: string, l: string, n = 1) {
  await expect
    .poll(async () => {
      const v = await view(live, s, l);
      return (v.media?.readiness ?? v.readiness)?.ready;
    }, { timeout: 5000 })
    .toBe(n);
}

test.beforeEach(async ({ page }) => {
  await helpers(page);
});

test("a media layer preloads off air and reports ready before Take", async ({ page, live }) => {
  const a = await clip(live, "clip-vp8.webm");
  const s = await makeScreen(live, [mediaLayer("m", a, { inMs: 1000 })]);
  const errors = await open(page, live.base, s);
  await readyOn(live, s, "m");
  const v = await view(live, s, "m");
  expect(v.onAir).toBe(false);
  expect(v.media!.readiness).toEqual({ ready: 1, total: 1, error: null });
  // Preloaded: the element exists (not in the DOM), with the in point's frame decoded.
  const t = await page.evaluate(() => (window as unknown as { __t: Record<string, number> }).__t);
  const ms = t["ready:m"] - t.snapshot;
  console.log(`preload: ready ${ms.toFixed(0)} ms after the snapshot`);
  expect(ms).toBeLessThan(500);
  expect(errors).toEqual([]);
});

test("Take plays from the in point without a black frame, then at the clip's frame rate", async ({ page, live }) => {
  const a = await clip(live, "clip-vp8.webm");
  const s = await makeScreen(live, [mediaLayer("m", a, { inMs: 1000 })]);
  await open(page, live.base, s);
  await readyOn(live, s, "m");
  // Every animation frame from the moment the take arrives (the layer's `state in`) for 1.8 s:
  // when it was painted and the frame number the video shows (-1 = nothing / black).
  const sampled = page.evaluate(
    () =>
      new Promise<{ t: number; f: number }[]>((r) => {
        const w = window as unknown as { __t: Record<string, number> };
        const out: { t: number; f: number }[] = [];
        const tick = (now: number) => {
          if (w.__t.take !== undefined) {
            out.push({ t: now, f: readFrame(0) });
            if (now - w.__t.take > 1800) return r(out);
          }
          requestAnimationFrame(tick);
        };
        requestAnimationFrame(tick);
      }),
  );
  await live.cmd(`layershow ${s} m`);
  const samples = await sampled;
  const frames = samples.map((x) => x.f);
  console.log(`take: frames ${frames.join(",")}`);
  // 1. No black frame from the take on, and it starts on the in point (frame 30 at 30 fps).
  expect(frames.every((n) => n >= 0)).toBe(true);
  expect(frames[0]).toBeGreaterThanOrEqual(29);
  expect(frames[0]).toBeLessThanOrEqual(31);
  // 2. Once the first frame advances, it plays at the clip's rate (30 fps) over the next second.
  const first = samples.findIndex((x) => x.f > frames[0]);
  expect(first, "the picture never moved").toBeGreaterThan(-1);
  const t0 = samples[first].t;
  let last = first;
  for (let i = first; i < samples.length && samples[i].t <= t0 + 1000; i++) last = i;
  const seconds = (samples[last].t - t0) / 1000;
  expect(seconds, "less than a second of samples after the first advance").toBeGreaterThan(0.9);
  const fps = (samples[last].f - samples[first].f) / seconds;
  console.log(`take: first advance ${(t0 - samples[0].t).toFixed(0)} ms after the take, then ${fps.toFixed(1)} fps`);
  expect(fps).toBeGreaterThan(27);
  expect(fps).toBeLessThan(33);
  expect((await view(live, s, "m")).media!.playback.state).toBe("playing");
});

test("end actions: hold pauses at the out point, clear takes the layer off, loop wraps", async ({ page, live }) => {
  const a = await clip(live, "clip-vp9.webm");
  const s = await makeScreen(live, [
    mediaLayer("hold", a, { outMs: 1000 }),
    mediaLayer("clear", a, { outMs: 1000, endAction: "clear" }),
    mediaLayer("loop", a, { inMs: 500, outMs: 1500, endAction: "loop" }),
  ]);
  await open(page, live.base, s);
  for (const l of ["hold", "clear", "loop"]) await readyOn(live, s, l);
  await live.cmd(`layershow ${s} hold`);
  await live.cmd(`layershow ${s} clear`);
  await live.cmd(`layershow ${s} loop`);
  await page.waitForTimeout(1800);
  // Hold: the last frame stays (frame 30 at 1000 ms) and nothing plays.
  const hold = await page.evaluate(() => {
    const v = (window as unknown as { vids: () => HTMLVideoElement[] }).vids()[0];
    return { paused: v.paused, t: v.currentTime };
  });
  expect(hold.paused).toBe(true);
  expect(hold.t).toBeCloseTo(1, 1);
  expect(await frame(page, 0)).toBeGreaterThanOrEqual(28);
  // Clear: off air at the out point.
  expect((await view(live, s, "clear")).onAir).toBe(false);
  // Loop: still on air, playing inside 500..1500 ms.
  const l = await view(live, s, "loop");
  expect(l.onAir).toBe(true);
  const f = [];
  for (let i = 0; i < 6; i++) {
    f.push(await page.evaluate(() => (window as unknown as { vids: () => HTMLVideoElement[] }).vids().at(-1)!.currentTime));
    await page.waitForTimeout(150);
  }
  console.log(`loop: ${f.map((x) => x.toFixed(2)).join(",")}`);
  expect(f.every((t) => t >= 0.45 && t <= 1.55)).toBe(true);
  await live.cmd(`clearscreen ${s}`);
});

test("transport: pause holds the frame, seek moves it, stop rewinds", async ({ page, live }) => {
  const a = await clip(live, "clip-vp8.webm");
  const s = await makeScreen(live, [mediaLayer("m", a)]);
  await open(page, live.base, s);
  await readyOn(live, s, "m");
  await live.cmd(`layershow ${s} m`);
  await page.waitForTimeout(500);
  await live.cmd(`mediactl ${s} m {"action":"pause"}`);
  await page.waitForTimeout(100);
  const a1 = await frame(page);
  await page.waitForTimeout(400);
  expect(await frame(page)).toBe(a1);
  await live.cmd(`mediactl ${s} m {"action":"seek","ms":2000}`);
  await expect.poll(() => frame(page)).toBe(60);
  await live.cmd(`mediactl ${s} m {"action":"play"}`);
  await expect.poll(() => frame(page)).toBeGreaterThan(62);
  await live.cmd(`mediactl ${s} m {"action":"stop"}`);
  await expect.poll(() => frame(page)).toBe(0);
  await live.cmd(`clearscreen ${s}`);
});

test("two renderers of the same screen stay within 2 frames", async ({ browser, live }) => {
  const a = await clip(live, "clip-vp8.webm");
  const s = await makeScreen(live, [mediaLayer("m", a, { endAction: "loop" })]);
  const ctx = await Promise.all([0, 1].map(() => browser.newContext({ viewport: { width: 960, height: 540 } })));
  const pages = await Promise.all(ctx.map((c) => c.newPage()));
  for (const p of pages) await helpers(p), await open(p, live.base, s);
  await readyOn(live, s, "m", 2);
  await live.cmd(`layershow ${s} m`);
  await pages[0].waitForTimeout(1500);
  // Offset of each renderer = the media time it presents − wall clock: equal offsets = in step.
  const d: number[] = [];
  for (let i = 0; i < 10; i++) {
    const [x, y] = await Promise.all(pages.map((p) => p.evaluate(() => (window as unknown as { presented: (i: number) => Promise<{ media: number; wall: number }> }).presented(0))));
    let e = x.media - x.wall - (y.media - y.wall);
    e = ((((e + 1500) % 3000) + 3000) % 3000) - 1500; // the loop wraps every 3 s
    d.push(Math.abs(e));
    await pages[0].waitForTimeout(200);
  }
  d.sort((p, q) => p - q);
  console.log(`sync: |Δ| median ${d[5].toFixed(1)} ms, max ${d[9].toFixed(1)} ms`);
  expect(d[5]).toBeLessThanOrEqual(2 * FRAME_MS);
  await live.cmd(`clearscreen ${s}`);
  await Promise.all(ctx.map((c) => c.close()));
});

test("at most 4 videos preload per page; the rest load metadata and still report ready", async ({ page, live }) => {
  const a = await clip(live, "clip-vp9.webm");
  const s = await makeScreen(
    live,
    [0, 1, 2, 3, 4].map((i) => mediaLayer("v" + i, a)),
  );
  await open(page, live.base, s);
  for (let i = 0; i < 5; i++) await readyOn(live, s, "v" + i);
  // Off air the hosts are out of the DOM: take them all, then read each video's preload.
  for (let i = 0; i < 5; i++) await live.cmd(`layershow ${s} v${i}`);
  await expect.poll(() => page.evaluate(() => (window as unknown as { vids: () => HTMLVideoElement[] }).vids().length)).toBe(5);
  const p = await page.evaluate(() => (window as unknown as { vids: () => HTMLVideoElement[] }).vids().map((v) => v.preload));
  expect(p.filter((x) => x == "auto")).toHaveLength(4);
  expect(p.filter((x) => x == "metadata")).toHaveLength(1);
  await live.cmd(`clearscreen ${s}`);
});

test("app previews (?mute=1) are muted and never count toward readiness", async ({ page, live }) => {
  const a = await clip(live, "clip-vp8.webm");
  const s = await makeScreen(live, [mediaLayer("m", a)]);
  await open(page, live.base, s, "?mute=1");
  await page.waitForTimeout(800);
  expect((await view(live, s, "m")).media!.readiness.total).toBe(0);
  await live.cmd(`layershow ${s} m`);
  await expect.poll(() => page.evaluate(() => (window as unknown as { vids: () => HTMLVideoElement[] }).vids()[0]?.muted)).toBe(true);
  await live.cmd(`clearscreen ${s}`);
});

test("H.264 MP4 and VP9 with alpha play", async ({ page, live }) => {
  const mp4 = await clip(live, "clip-h264.mp4");
  const alpha = await clip(live, "clip-vp9-alpha.webm");
  const s = await makeScreen(live, [mediaLayer("mp4", mp4, { inMs: 500 }), mediaLayer("alpha", alpha)]);
  await open(page, live.base, s);
  await readyOn(live, s, "mp4");
  await readyOn(live, s, "alpha");
  await live.cmd(`layershow ${s} mp4`);
  await expect.poll(() => frame(page)).toBeGreaterThanOrEqual(15);
  await live.cmd(`layerhide ${s} mp4`);
  await live.cmd(`layershow ${s} alpha`);
  await page.waitForTimeout(400);
  // The clip's transparent corner stays transparent over the black screen; the card is opaque red.
  const px = await page.evaluate(() => {
    const v = (window as unknown as { vids: () => HTMLVideoElement[] }).vids()[0];
    const g = new OffscreenCanvas(480, 270).getContext("2d")!;
    g.drawImage(v, 0, 0, 480, 270);
    return [Array.from(g.getImageData(470, 100, 1, 1).data), Array.from(g.getImageData(240, 160, 1, 1).data)];
  });
  expect(px[0][3]).toBeLessThan(30);
  expect(px[1][0]).toBeGreaterThan(180);
  expect(px[1][3]).toBe(255);
  await live.cmd(`clearscreen ${s}`);
});

test("template video fields preload, report ready and play with the markup's settings", async ({ page, live }) => {
  const a = await clip(live, "clip-vp8.webm");
  const tpl = await live.cmd(
    `addtemplatedoc ${JSON.stringify({
      name: "Video bg",
      html: '<div class="bg"><video src="{{clip}}" autoplay muted loop></video></div>',
      css: ".bg{position:absolute;inset:0}video{width:100%;height:100%}",
      fields: [{ key: "clip", label: "Clip", type: "video", default: "" }],
    })}`,
  );
  const s = await makeScreen(live, [
    { id: "t", name: "T", kind: "template", templateId: tpl, rect: RECT, fields: { clip: `/assets/${a}` }, vars: {} } as unknown as ScreenLayer,
  ]);
  await open(page, live.base, s);
  await readyOn(live, s, "t");
  await live.cmd(`layershow ${s} t`);
  await expect.poll(() => frame(page)).toBeGreaterThan(5);
  const v = await page.evaluate(() => {
    const e = (window as unknown as { vids: () => HTMLVideoElement[] }).vids()[0];
    return { muted: e.muted, loop: e.loop };
  });
  expect(v).toEqual({ muted: true, loop: true });
  await live.cmd(`clearscreen ${s}`);
});

test("renderer sizes: media chunk ≤ 3 KB gz", () => {
  const dist = join(process.cwd(), "renderer/dist/assets");
  const f = readdirSync(dist).find((x) => x.startsWith("media-") && x.endsWith(".js"))!;
  const gz = gzipSync(readFileSync(join(dist, f)), { level: 9 }).length;
  console.log(`media chunk: ${gz} B gz`);
  expect(gz).toBeLessThanOrEqual(3072);
});

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { connect, type SocketLike } from "./connection";
import { DEAD_MS, MediaLayer, PRELOAD_CAP, SEEK_MS, position, rate } from "./media";
import type { ScreenInfo, WireLayer, WireMedia, WirePlayback } from "./protocol";
import { Stage } from "./stage";

const screen: ScreenInfo = { id: "s", name: "S", width: 1920, height: 1080, background: "transparent" };

const media = (over: Partial<WireMedia> = {}): WireMedia => ({
  url: "/assets/clip.webm",
  kind: "video",
  fit: "fit",
  inMs: 0,
  durationMs: 3000,
  volume: 100,
  muted: false,
  endAction: "hold",
  playback: { state: "stopped", at: 0, posMs: 0 },
  ...over,
});

const mlayer = (over: Partial<WireLayer> = {}, m: Partial<WireMedia> = {}): WireLayer => ({
  id: "m1",
  z: 1,
  templateRev: 77,
  html: "",
  css: "",
  fields: {},
  fieldTypes: {},
  state: "out",
  media: media(m),
  ...over,
});

const sent: string[] = [];
class FakeWS implements SocketLike {
  onopen: SocketLike["onopen"] = null;
  onmessage: SocketLike["onmessage"] = null;
  onclose: SocketLike["onclose"] = null;
  onerror: SocketLike["onerror"] = null;
  constructor(readonly url: string) {
    queueMicrotask(() => this.onopen?.(new Event("open")));
  }
  close() {}
  send(d: string) {
    sent.push(d);
  }
}

/** jsdom has no media playback: play/pause/load are stubbed, `paused` follows them. */
let plays: HTMLMediaElement[];
beforeEach(() => {
  plays = [];
  const P = HTMLMediaElement.prototype;
  vi.spyOn(P, "play").mockImplementation(function (this: HTMLMediaElement) {
    plays.push(this);
    Object.defineProperty(this, "paused", { value: false, configurable: true });
    return Promise.resolve();
  });
  vi.spyOn(P, "pause").mockImplementation(function (this: HTMLMediaElement) {
    Object.defineProperty(this, "paused", { value: true, configurable: true });
  });
  vi.spyOn(P, "load").mockImplementation(() => {});
  sent.length = 0;
});

const apps: App[] = [];
afterEach(() => {
  for (const a of apps.splice(0)) for (const id of [...a.layers.keys()]) a.drop(id);
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

async function setup(layers: WireLayer[]) {
  const app = new App(new Stage(document.body));
  apps.push(app);
  app.handle({ t: "snapshot", screen, layers });
  await vi.waitFor(() => expect(app.layers.size).toBe(layers.length));
  return app;
}

const video = (app: App, id = "m1") => app.layers.get(id)!.shadow.querySelector("video")!;

describe("position", () => {
  const p = (state: WirePlayback["state"], at: number, posMs: number): WirePlayback => ({ state, at, posMs });
  it("follows the server clock while playing, waits for a future start", () => {
    const m = media({ inMs: 500 });
    expect(position(m, p("playing", 1000, 500), 1600, 3000)).toBe(1100);
    expect(position(m, p("playing", 1000, 500), 900, 3000)).toBe(500);
    expect(position(m, p("paused", 1000, 700), 5000, 3000)).toBe(700);
    expect(position(m, p("stopped", 1000, 2000), 5000, 3000)).toBe(500);
  });
  it("holds at the out point, or the end", () => {
    expect(position(media({ outMs: 2000 }), p("playing", 0, 0), 2500, 3000)).toBe(2000);
    expect(position(media(), p("playing", 0, 0), 9000, 3000)).toBe(3000);
    expect(position(media({ durationMs: undefined }), p("playing", 0, 0), 9000, Infinity)).toBe(9000);
  });
  it("loops between in and out", () => {
    const m = media({ inMs: 1000, outMs: 2000, endAction: "loop" });
    expect(position(m, p("playing", 0, 1000), 1500, 3000)).toBe(1500);
    expect(position(m, p("playing", 0, 1000), 2250, 3000)).toBe(1250);
    expect(position(media({ endAction: "loop" }), p("playing", 0, 0), 7000, 3000)).toBe(1000);
  });
});

describe("rate", () => {
  it("leaves frame-timing drift alone, nudges only muted elements, seeks beyond SEEK_MS", () => {
    expect(rate(30)).toBe(1);
    expect(rate(-DEAD_MS + 1)).toBe(1);
    expect(rate(60)).toBeCloseTo(0.98);
    expect(rate(-50)).toBeCloseTo(1.02);
    // Audible: never a rate change (it stretches the sound), only a seek when far off.
    expect(rate(60, true)).toBe(1);
    expect(rate(-200, true)).toBe(1);
    expect(rate(SEEK_MS + 1, true)).toBe(0);
    expect(rate(SEEK_MS + 1)).toBe(0);
    expect(rate(-SEEK_MS - 1)).toBe(0);
  });
});

describe("MediaLayer", () => {
  it("a media layer loads the chunk: a video in the inner box, preloaded and fitted", async () => {
    const app = await setup([mlayer({}, { fit: "fill" })]);
    const l = app.layers.get("m1")!;
    expect(l).toBeInstanceOf(MediaLayer);
    const v = video(app);
    expect(v.getAttribute("src")).toBe("/assets/clip.webm");
    expect(v.preload).toBe("auto");
    expect(v.style.objectFit).toBe("cover");
    expect(l.root?.className).toBe("m");
    // Off air: not in the DOM, not playing.
    expect(l.host.isConnected).toBe(false);
    expect(plays).toHaveLength(0);
  });

  it("a plain template snapshot never loads the chunk", async () => {
    const app = new App(new Stage(document.body));
    apps.push(app);
    app.handle({
      t: "snapshot",
      screen,
      layers: [{ ...mlayer(), media: undefined, html: "<div>{{a}}</div>", fields: { a: "x" }, fieldTypes: { a: "text" } }],
    });
    // Synchronous: no chunk to wait for.
    expect(app.layers.get("m1")).toBeDefined();
    expect(app.layers.get("m1")).not.toBeInstanceOf(MediaLayer);
  });

  it("fit, volume and mute follow the config in place (same rev)", async () => {
    const app = await setup([mlayer()]);
    const v = video(app);
    app.handle({ t: "layer", layer: mlayer({}, { fit: "stretch", volume: 40, muted: true }) });
    expect(video(app)).toBe(v);
    expect(v.style.objectFit).toBe("fill");
    expect(v.volume).toBeCloseTo(0.4);
    expect(v.muted).toBe(true);
  });

  it(`preloads at most ${PRELOAD_CAP} videos per page; more load metadata only`, async () => {
    const ls = Array.from({ length: PRELOAD_CAP + 1 }, (_, i) => mlayer({ id: "v" + i, templateRev: i }));
    const app = await setup(ls);
    const pre = ls.map((l) => video(app, l.id).preload);
    expect(pre.filter((p) => p == "auto")).toHaveLength(PRELOAD_CAP);
    expect(pre[PRELOAD_CAP]).toBe("metadata");
    // Removing one frees its slot.
    app.handle({ t: "remove", layerId: "v0" });
    app.handle({ t: "layer", layer: mlayer({ id: "v9", templateRev: 9 }) });
    expect(video(app, "v9").preload).toBe("auto");
  });

  it("reports ready once the in point's frame is decoded, once per socket", async () => {
    connect("ws://x/ws?screen=s", () => {}, { WS: FakeWS });
    await Promise.resolve();
    const app = await setup([mlayer({}, { inMs: 1000 })]);
    const v = video(app);
    expect(v.currentTime).toBe(1);
    Object.defineProperty(v, "readyState", { value: 2, configurable: true });
    v.dispatchEvent(new Event("seeked"));
    v.dispatchEvent(new Event("loadeddata"));
    expect(sent.map((s) => JSON.parse(s))).toEqual([{ t: "ready", layerId: "m1", ok: true }]);
    // The same snapshot again on the same socket: nothing new.
    app.handle({ t: "snapshot", screen, layers: [mlayer({}, { inMs: 1000 })] });
    expect(sent).toHaveLength(1);
    // A reconnect: the next snapshot sends it again.
    connect("ws://x/ws?screen=s", () => {}, { WS: FakeWS });
    await Promise.resolve();
    app.handle({ t: "snapshot", screen, layers: [mlayer({}, { inMs: 1000 })] });
    expect(sent).toHaveLength(2);
  });

  it("reports a load error", async () => {
    connect("ws://x/ws?screen=s", () => {}, { WS: FakeWS });
    await Promise.resolve();
    const app = await setup([mlayer()]);
    const v = video(app);
    Object.defineProperty(v, "error", { value: { code: 4 }, configurable: true });
    v.dispatchEvent(new Event("error"));
    expect(JSON.parse(sent[0])).toEqual({ t: "ready", layerId: "m1", ok: false, error: "Format not supported" });
  });

  it("take: shown paused at the in point, plays at `at`; pause and stop schedule nothing", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const app = new App(new Stage(document.body));
    apps.push(app);
    app.handle({ t: "snapshot", screen, layers: [mlayer({}, { inMs: 500 })] });
    await vi.waitFor(() => expect(app.layers.size).toBe(1));
    const v = video(app);
    app.handle({ t: "state", layerId: "m1", state: "in" });
    expect(app.layers.get("m1")!.host.isConnected).toBe(true);
    expect(v.currentTime).toBe(0.5);
    const at = Date.now() + 120;
    app.handle({ t: "media", layerId: "m1", playback: { state: "playing", at, posMs: 500 } });
    expect(plays).toHaveLength(0);
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(120);
    expect(plays).toEqual([v]);
    // Playing toward the end (hold): one timer at the out point.
    expect(vi.getTimerCount()).toBe(1);
    app.handle({ t: "media", layerId: "m1", playback: { state: "paused", at: Date.now(), posMs: 900 } });
    expect(v.paused).toBe(true);
    expect(v.currentTime).toBeCloseTo(0.9);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("hold: pauses at the out point", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const app = new App(new Stage(document.body));
    apps.push(app);
    app.handle({ t: "snapshot", screen, layers: [mlayer({ state: "in" }, { outMs: 1000, playback: { state: "playing", at: Date.now(), posMs: 0 } })] });
    await vi.waitFor(() => expect(app.layers.size).toBe(1));
    const v = video(app);
    expect(v.paused).toBe(false);
    vi.advanceTimersByTime(1001);
    expect(v.paused).toBe(true);
    expect(v.currentTime).toBe(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("loop over the whole file uses the element's loop; a range loops by seeking at the out point", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    const pb: WirePlayback = { state: "playing", at: Date.now(), posMs: 0 };
    const app = new App(new Stage(document.body));
    apps.push(app);
    app.handle({
      t: "snapshot",
      screen,
      layers: [
        mlayer({ id: "a", state: "in" }, { endAction: "loop", playback: pb }),
        mlayer({ id: "b", state: "in", templateRev: 2 }, { endAction: "loop", inMs: 1000, outMs: 2000, playback: { ...pb, posMs: 1000 } }),
      ],
    });
    await vi.waitFor(() => expect(app.layers.size).toBe(2));
    expect(video(app, "a").loop).toBe(true);
    const b = video(app, "b");
    expect(b.loop).toBe(false);
    expect(b.currentTime).toBe(1);
    // jsdom's currentTime doesn't advance: play it forward by hand, then reach the out point.
    b.currentTime = 2;
    vi.advanceTimersByTime(1001);
    expect(b.currentTime).toBeCloseTo(1.001, 2);
    expect(b.paused).toBe(false);
  });

  it("clear: plays through the out animation, then stops at the in point; cut likewise", async () => {
    const app = await setup([mlayer({ state: "in" }, { inMs: 200, playback: { state: "playing", at: Date.now(), posMs: 200 } })]);
    const v = video(app);
    const l = app.layers.get("m1")!;
    expect(v.paused).toBe(false);
    app.handle({ t: "cut" });
    expect(l.host.isConnected).toBe(false);
    expect(v.paused).toBe(true);
    expect(v.currentTime).toBeCloseTo(0.2);
    // The next take starts paused at the in point until the core's `media` arrives.
    app.handle({ t: "state", layerId: "m1", state: "in" });
    expect(v.paused).toBe(true);
  });

  it("a new asset (new rev) replaces and releases the element", async () => {
    const app = await setup([mlayer()]);
    const v = video(app);
    app.handle({ t: "layer", layer: mlayer({ templateRev: 78 }, { url: "/assets/other.webm" }) });
    expect(video(app)).not.toBe(v);
    expect(v.hasAttribute("src")).toBe(false);
    expect(video(app).getAttribute("src")).toBe("/assets/other.webm");
  });

  it("image and audio media layers", async () => {
    connect("ws://x/ws?screen=s", () => {}, { WS: FakeWS });
    await Promise.resolve();
    const decode = vi.fn(() => Promise.resolve());
    (HTMLImageElement.prototype as unknown as { decode: () => Promise<void> }).decode = decode;
    const app = await setup([
      mlayer({ id: "i" }, { kind: "image", url: "/assets/a.png", fit: "stretch" }),
      mlayer({ id: "a", templateRev: 3 }, { kind: "audio", url: "/assets/a.mp3" }),
    ]);
    const img = app.layers.get("i")!.shadow.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("/assets/a.png");
    expect(img.style.objectFit).toBe("fill");
    await vi.waitFor(() => expect(sent.map((s) => JSON.parse(s).layerId)).toContain("i"));
    expect(app.layers.get("a")!.shadow.querySelector("audio")!.preload).toBe("auto");
  });

  it("template video fields: preloaded, autoplay only on air, ready once loaded", async () => {
    connect("ws://x/ws?screen=s", () => {}, { WS: FakeWS });
    await Promise.resolve();
    const app = await setup([
      {
        ...mlayer(),
        media: undefined,
        html: '<div><video src="{{clip}}" autoplay muted loop></video></div>',
        fields: { clip: "/assets/bg.webm" },
        fieldTypes: { clip: "video" },
      },
    ]);
    const l = app.layers.get("m1")!;
    const v = video(app);
    expect(l).toBeInstanceOf(MediaLayer);
    expect(v.preload).toBe("auto");
    expect(v.autoplay).toBe(false);
    expect(v.loop).toBe(true);
    v.dispatchEvent(new Event("loadeddata"));
    await vi.waitFor(() => expect(sent).toHaveLength(1));
    expect(JSON.parse(sent[0])).toEqual({ t: "ready", layerId: "m1", ok: true });
    expect(plays).toHaveLength(0);
    app.handle({ t: "state", layerId: "m1", state: "in" });
    expect(plays).toEqual([v]);
    // A new file: loaded and reported again.
    app.handle({ t: "fields", layerId: "m1", fields: { clip: "/assets/other.webm" } });
    expect(v.getAttribute("src")).toBe("/assets/other.webm");
    v.dispatchEvent(new Event("loadeddata"));
    await vi.waitFor(() => expect(sent).toHaveLength(2));
  });

  it("the studio chunk creates media layers too", async () => {
    const app = await setup([mlayer()]);
    app.handle({ t: "take", transition: { kind: "cut", ms: 0 }, layers: [mlayer({ state: "in" })], hide: [] });
    await vi.waitFor(() => expect(app.layers.get("m1")!.host.isConnected).toBe(true));
    expect(app.layers.get("m1")).toBeInstanceOf(MediaLayer);
  });
});

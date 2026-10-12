import { flushSync } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LiveThumbs } from "./thumbs.svelte";

// Observers that tests drive by hand.
const obs = vi.hoisted(() => ({
  io: [] as { cb: IntersectionObserverCallback; targets: Element[]; off: boolean }[],
  ro: [] as { cb: ResizeObserverCallback; targets: Element[]; off: boolean }[],
}));
class FakeIO {
  rec: (typeof obs.io)[number];
  constructor(cb: IntersectionObserverCallback) {
    this.rec = { cb, targets: [], off: false };
    obs.io.push(this.rec);
  }
  observe(el: Element) {
    this.rec.targets.push(el);
  }
  unobserve(el: Element) {
    this.rec.targets = this.rec.targets.filter((t) => t !== el);
  }
  disconnect() {
    this.rec.off = true;
  }
}
class FakeRO {
  rec: (typeof obs.ro)[number];
  constructor(cb: ResizeObserverCallback) {
    this.rec = { cb, targets: [], off: false };
    obs.ro.push(this.rec);
  }
  observe(el: Element) {
    this.rec.targets.push(el);
  }
  unobserve(el: Element) {
    this.rec.targets = this.rec.targets.filter((t) => t !== el);
  }
  disconnect() {
    this.rec.off = true;
  }
}
vi.stubGlobal("IntersectionObserver", FakeIO);
vi.stubGlobal("ResizeObserver", FakeRO);

let visibility: DocumentVisibilityState = "visible";
Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });

const ids = ["a", "b", "c", "d", "e", "f", "g", "h"];

function setup(cap?: number) {
  const s = $state({ order: [...ids], running: true, first: null as string | null });
  const thumbs = new LiveThumbs({
    order: () => s.order,
    running: () => s.running,
    cap,
    first: () => s.first,
  });
  const els = new Map<string, HTMLElement>();
  const detach = new Map<string, () => void>();
  for (const id of ids) {
    const el = document.createElement("div");
    els.set(id, el);
    const off = thumbs.track(id)(el);
    if (off) detach.set(id, off);
  }
  return { s, thumbs, els, detach };
}

function intersect(els: Map<string, HTMLElement>, which: string[], isIntersecting = true) {
  const rec = obs.io.at(-1)!;
  rec.cb(
    which.map((id) => ({ target: els.get(id)!, isIntersecting })) as unknown as IntersectionObserverEntry[],
    {} as IntersectionObserver,
  );
  flushSync();
}

beforeEach(() => {
  obs.io.length = 0;
  obs.ro.length = 0;
  visibility = "visible";
});
afterEach(() => vi.clearAllMocks());

describe("LiveThumbs", () => {
  it("caps live thumbnails, in grid order, among the visible ones", () => {
    const { thumbs, els } = setup(3);
    expect(thumbs.live.size).toBe(0);
    intersect(els, ids);
    expect([...thumbs.live]).toEqual(["a", "b", "c"]);
    intersect(els, ["a"], false);
    expect([...thumbs.live]).toEqual(["b", "c", "d"]);
    thumbs.destroy();
  });

  it("defaults to a cap of 6", () => {
    const { thumbs, els } = setup();
    intersect(els, ids);
    expect(thumbs.live.size).toBe(6);
    thumbs.destroy();
  });

  it("`first` counts first when visible, and only then", () => {
    const { s, thumbs, els } = setup(2);
    intersect(els, ["a", "b", "c", "h"]);
    s.first = "h";
    flushSync();
    expect([...thumbs.live]).toEqual(["h", "a"]);
    s.first = "e"; // off screen: no slot
    flushSync();
    expect([...thumbs.live]).toEqual(["a", "b"]);
    s.first = "zzz"; // not in the grid
    flushSync();
    expect([...thumbs.live]).toEqual(["a", "b"]);
    thumbs.destroy();
  });

  it("nothing is live without the server or while the window is hidden; notes say why", () => {
    const { s, thumbs, els } = setup(1);
    intersect(els, ["a", "b"]);
    expect(thumbs.note("a")).toBeNull();
    expect(thumbs.note("b")).toBe("Preview paused");
    expect(thumbs.note("c")).toBeNull(); // off screen
    s.running = false;
    flushSync();
    expect(thumbs.live.size).toBe(0);
    expect(thumbs.note("a")).toBe("Server not running");
    s.running = true;
    visibility = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    flushSync();
    expect(thumbs.live.size).toBe(0);
    visibility = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    flushSync();
    expect([...thumbs.live]).toEqual(["a"]);
    thumbs.destroy();
  });

  it("a removed card stops counting and is unobserved; destroy disconnects", () => {
    const { thumbs, els, detach } = setup(2);
    intersect(els, ["a", "b", "c"]);
    expect([...thumbs.live]).toEqual(["a", "b"]);
    detach.get("a")!();
    flushSync();
    expect([...thumbs.live]).toEqual(["b", "c"]);
    expect(obs.io.at(-1)!.targets).not.toContain(els.get("a"));
    thumbs.destroy();
    expect(obs.io.at(-1)!.off).toBe(true);
    expect(obs.ro.at(-1)!.off).toBe(true);
  });

  it("track returns one stable attachment per id; box sizes come from the ResizeObserver", () => {
    const { thumbs, els } = setup();
    expect(thumbs.track("zz")).toBe(thumbs.track("zz"));
    expect(thumbs.box("a")).toEqual({ w: 0, h: 0 });
    obs.ro.at(-1)!.cb(
      [{ target: els.get("a")!, contentRect: { width: 320, height: 180 } }] as unknown as ResizeObserverEntry[],
      {} as ResizeObserver,
    );
    flushSync();
    expect(thumbs.box("a")).toEqual({ w: 320, h: 180 });
    thumbs.destroy();
  });
});

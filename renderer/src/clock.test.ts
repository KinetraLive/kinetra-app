import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { clockText, setServerNow, timerText } from "./clock";
import { Layer } from "./layer";
import type { ScreenInfo, WireLayer } from "./protocol";
import { Stage } from "./stage";

const T0 = new Date(2026, 9, 4, 12, 34, 56, 300).getTime(); // local 12:34:56.300

const timer = (s: Partial<{ run: boolean; at: number; base: number; dir: "up" | "down"; end: string }>) =>
  JSON.stringify({ run: false, at: 0, base: 0, dir: "up", end: "stop", ...s });

describe("clockText", () => {
  it("formats HH mm ss in local time; other characters are literal; empty = HH:mm:ss", () => {
    expect(clockText("HH:mm:ss", T0)).toBe("12:34:56");
    expect(clockText("", T0)).toBe("12:34:56");
    expect(clockText("HH.mm", T0)).toBe("12.34");
    expect(clockText("[HH h mm]", new Date(2026, 0, 1, 7, 5, 9).getTime())).toBe("[07 h 05]");
  });
});

describe("timerText", () => {
  it("up: base + elapsed while running, base when paused", () => {
    expect(timerText(timer({ run: true, at: T0, base: 5000 }), T0 + 61_999)).toBe("1:06");
    expect(timerText(timer({ run: false, at: T0, base: 5000 }), T0 + 61_999)).toBe("0:05");
  });

  it("down: counts to zero and holds 0:00", () => {
    const s = timer({ run: true, at: T0, base: 10_000, dir: "down" });
    expect(timerText(s, T0)).toBe("0:10");
    expect(timerText(s, T0 + 1)).toBe("0:10"); // 9.999 s left still reads 0:10
    expect(timerText(s, T0 + 1000)).toBe("0:09");
    expect(timerText(s, T0 + 9_999)).toBe("0:01");
    expect(timerText(s, T0 + 10_000)).toBe("0:00");
    expect(timerText(s, T0 + 99_000)).toBe("0:00");
  });

  it("m:ss below an hour, h:mm:ss from an hour", () => {
    expect(timerText(timer({ base: 3_599_999 }), 0)).toBe("59:59");
    expect(timerText(timer({ base: 3_600_000 }), 0)).toBe("1:00:00");
    expect(timerText(timer({ base: 10 * 3_600_000 + 62_000 }), 0)).toBe("10:01:02");
  });

  it("invalid state reads 0:00; a start in the future does not count", () => {
    expect(timerText("nope", T0)).toBe("0:00");
    expect(timerText("", T0)).toBe("0:00");
    expect(timerText(timer({ run: true, at: T0 + 5000, base: 1000 }), T0)).toBe("0:01");
  });
});

// ---------------------------------------------------------------------------------------------

const screen: ScreenInfo = { id: "s", name: "S", width: 1920, height: 1080, background: "transparent" };

function def(over: Partial<WireLayer> = {}): WireLayer {
  return {
    id: "c",
    z: 1,
    templateRev: 1,
    html: '<div class="bug"><span class="clock">{{time}}</span><span class="t">{{game}}</span></div>',
    css: "",
    fields: { time: "HH:mm:ss", game: timer({}) },
    fieldTypes: { time: "clock", game: "timer" },
    state: "out",
    ...over,
  };
}

let created: Layer[] = [];
function layer(d: WireLayer) {
  const l = new Layer(d, document.createElement("div"));
  created.push(l);
  return l;
}
const text = (l: Layer, sel: string) => l.shadow.querySelector(sel)!.textContent;

function setHidden(hidden: boolean) {
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (hidden ? "hidden" : "visible") });
  document.dispatchEvent(new Event("visibilitychange"));
}

beforeEach(() => {
  vi.useFakeTimers({ now: T0 });
});

afterEach(() => {
  for (const l of created) l.dispose();
  created = [];
  delete (document as unknown as { visibilityState?: string }).visibilityState;
  setServerNow(undefined);
  vi.useRealTimers();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

describe("clock scheduler", () => {
  it("no timer at rest: off-air clocks and paused timers never wake the page", () => {
    const l = layer(def());
    expect(text(l, ".clock")).toBe("12:34:56");
    expect(vi.getTimerCount()).toBe(0);
    const l2 = layer(def({ id: "x", state: "in", html: "<p>{{game}}</p>", fieldTypes: { game: "timer" } }));
    expect(l2.state).toBe("in");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("an on-air clock wakes once per second, aligned to the second boundary", () => {
    const l = layer(def());
    l.setState("in");
    expect(vi.getTimerCount()).toBe(1);
    const node = l.shadow.querySelector(".clock")!.firstChild;
    vi.advanceTimersByTime(699);
    expect(text(l, ".clock")).toBe("12:34:56");
    vi.advanceTimersByTime(1); // 12:34:57.000
    expect(text(l, ".clock")).toBe("12:34:57");
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(1000);
    expect(text(l, ".clock")).toBe("12:34:58");
    expect(l.shadow.querySelector(".clock")!.firstChild).toBe(node);
    expect(Date.now() % 1000).toBe(0);
    // Two layers with clocks still share one timer.
    const l2 = layer(def({ id: "y", state: "in" }));
    expect(text(l2, ".clock")).toBe("12:34:58");
    expect(vi.getTimerCount()).toBe(1);
  });

  it("only changed text is written on a tick", async () => {
    const l = layer(def({ html: "<p><b>{{time}}</b><i>{{game}}</i></p>", fields: { time: "HH:mm", game: timer({}) } }));
    l.setState("in");
    const records: MutationRecord[] = [];
    const mo = new MutationObserver((r) => records.push(...r));
    mo.observe(l.shadow, { subtree: true, characterData: true, childList: true, attributes: true });
    vi.advanceTimersByTime(5000); // 12:35:01: HH:mm changed once, the paused timer never
    await Promise.resolve();
    mo.disconnect();
    records.push(...mo.takeRecords());
    expect(text(l, "b")).toBe("12:35");
    expect(records.length).toBe(1);
  });

  it("stops when the layer goes off air and resumes on take", () => {
    const l = layer(def());
    l.setState("in");
    expect(vi.getTimerCount()).toBe(1);
    l.setState("out"); // no durations in jsdom: detached at once
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(10_000);
    expect(text(l, ".clock")).toBe("12:34:56"); // stale while off air
    l.setState("in");
    expect(text(l, ".clock")).toBe("12:35:06"); // refreshed on take
    expect(vi.getTimerCount()).toBe(1);
    l.cut();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("pauses while the page is hidden", () => {
    const l = layer(def());
    l.setState("in");
    setHidden(true);
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(3000);
    expect(text(l, ".clock")).toBe("12:34:56");
    setHidden(false);
    expect(text(l, ".clock")).toBe("12:34:59");
    expect(vi.getTimerCount()).toBe(1);
  });

  it("a running timer ticks; pausing it stops the wake-up; a countdown stops at zero", () => {
    const html = "<p>{{game}}</p>";
    const l = layer(def({ html, fieldTypes: { game: "timer" }, fields: { game: timer({ run: true, at: T0, base: 0 }) } }));
    l.setState("in");
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(700 + 1000 * 64);
    expect(text(l, "p")).toBe("1:04");
    l.patch({ game: timer({ run: false, base: 64_700 }) });
    expect(text(l, "p")).toBe("1:04");
    expect(vi.getTimerCount()).toBe(0);

    l.patch({ game: timer({ run: true, at: Date.now(), base: 2000, dir: "down" }) });
    expect(text(l, "p")).toBe("0:02");
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(3000);
    expect(text(l, "p")).toBe("0:00");
    expect(vi.getTimerCount()).toBe(0);
  });

  it("a dropped layer leaves the scheduler", () => {
    const app = new App(new Stage(document.body));
    app.handle({ t: "snapshot", screen, layers: [def({ state: "in" })] });
    expect(vi.getTimerCount()).toBe(1);
    app.handle({ t: "remove", layerId: "c" });
    expect(vi.getTimerCount()).toBe(0);
  });

  it("hello.now corrects clock skew for clocks and timers", () => {
    const app = new App(new Stage(document.body));
    app.handle({ t: "hello", protocol: 1, now: T0 + 3_600_000 + 5000 }); // server 1 h 5 s ahead
    app.handle({
      t: "snapshot",
      screen,
      layers: [def({ state: "in", fields: { time: "HH:mm:ss", game: timer({ run: true, at: T0 + 3_600_000, base: 0 }) } })],
    });
    const l = app.layers.get("c")!;
    expect(text(l, ".clock")).toBe("13:35:01");
    expect(text(l, ".t")).toBe("0:05");
    // The wake-up aligns to the server's second (offset .300 → 700 ms away).
    vi.advanceTimersByTime(700);
    expect(text(l, ".clock")).toBe("13:35:02");
    // A hello without `now` drops the correction.
    app.handle({ t: "hello", protocol: 1 });
    expect(text(l, ".clock")).toBe("12:34:57");
    app.handle({ t: "remove", layerId: "c" });
  });

  it("a clock inside list rows ticks with the layer", () => {
    const l = layer(
      def({
        html: '<ul><li data-each="l">{{.city}} {{time}}</li></ul>',
        fields: { time: "HH:mm:ss", l: JSON.stringify([{ city: "Oslo" }, { city: "Bergen" }]) },
        fieldTypes: { time: "clock", l: "list" },
      }),
    );
    l.setState("in");
    expect(vi.getTimerCount()).toBe(1);
    vi.advanceTimersByTime(700);
    expect(Array.from(l.shadow.querySelectorAll("li")).map((li) => li.textContent)).toEqual([
      "Oslo 12:34:57",
      "Bergen 12:34:57",
    ]);
  });
});

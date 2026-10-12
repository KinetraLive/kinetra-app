import { afterEach, describe, expect, it, vi } from "vitest";
import { BACKOFF_MAX, BACKOFF_MIN, type SocketLike, connect, parseMsg } from "./connection";
import type { ServerMsg } from "./protocol";
import { screenId, wsUrl } from "./url";

class FakeWS implements SocketLike {
  static all: FakeWS[] = [];
  onopen: SocketLike["onopen"] = null;
  onmessage: SocketLike["onmessage"] = null;
  onclose: SocketLike["onclose"] = null;
  onerror: SocketLike["onerror"] = null;
  constructor(readonly url: string) {
    FakeWS.all.push(this);
  }
  close() {}
  open() {
    this.onopen?.(new Event("open"));
  }
  send(data: string) {
    this.onmessage?.({ data } as MessageEvent);
  }
  drop() {
    this.onclose?.({} as CloseEvent);
  }
}

const last = () => FakeWS.all[FakeWS.all.length - 1];

afterEach(() => {
  FakeWS.all = [];
  vi.useRealTimers();
  vi.restoreAllMocks();
});

/** Drops the current socket and returns the delay until the next one is created. */
function reconnectDelay(): number {
  const n = FakeWS.all.length;
  last().drop();
  let t = 0;
  while (FakeWS.all.length === n) {
    vi.advanceTimersByTime(1);
    t++;
    if (t > 10_000) throw new Error("never reconnected");
  }
  return t;
}

describe("connect", () => {
  it("backs off 250 → ×2 → 5 s and resets after hello", () => {
    vi.useFakeTimers();
    connect("ws://h/ws?screen=a", () => {}, { WS: FakeWS, random: () => 0.5 });
    expect(FakeWS.all.length).toBe(1);
    expect(last().url).toBe("ws://h/ws?screen=a");
    const delays = Array.from({ length: 7 }, reconnectDelay);
    expect(delays).toEqual([250, 500, 1000, 2000, 4000, 5000, 5000]);
    last().open();
    last().send(JSON.stringify({ t: "hello", protocol: 1 }));
    expect(reconnectDelay()).toBe(BACKOFF_MIN);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("jitter stays within ±20%", () => {
    vi.useFakeTimers();
    for (const r of [0, 0.999]) {
      FakeWS.all = [];
      connect("ws://h/ws", () => {}, { WS: FakeWS, random: () => r });
      let base = BACKOFF_MIN;
      for (let i = 0; i < 7; i++) {
        const d = reconnectDelay();
        expect(d).toBeGreaterThanOrEqual(Math.floor(base * 0.8));
        expect(d).toBeLessThanOrEqual(Math.ceil(base * 1.2));
        base = Math.min(base * 2, BACKOFF_MAX);
      }
      last().send(JSON.stringify({ t: "hello", protocol: 1 }));
    }
  });

  it("reports status and passes messages; malformed JSON is ignored", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    const got: ServerMsg[] = [];
    const status: string[] = [];
    connect("ws://h/ws", (m) => got.push(m), { WS: FakeWS, onStatus: (s) => status.push(s) });
    last().open();
    expect(() => last().send("{not json")).not.toThrow();
    last().send("42");
    last().send(JSON.stringify({ t: "clear" }));
    expect(got).toEqual([{ t: "clear" }]);
    expect(status).toEqual(["connecting", "connected"]);
  });

  it("a throwing handler doesn't break the socket", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    let n = 0;
    connect("ws://h/ws", () => {
      n++;
      throw new Error("x");
    }, { WS: FakeWS });
    expect(() => last().send(JSON.stringify({ t: "clear" }))).not.toThrow();
    last().send(JSON.stringify({ t: "clear" }));
    expect(n).toBe(2);
  });

  it("parseMsg rejects non-messages", () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(parseMsg("null")).toBeNull();
    expect(parseMsg("{}")).toBeNull();
    expect(parseMsg("<")).toBeNull();
    expect(parseMsg('{"t":"x"}')).toEqual({ t: "x" });
  });
});

describe("url", () => {
  it("derives the screen id and websocket url", () => {
    expect(screenId("/screen/test")).toBe("test");
    expect(screenId("/screen/a%20b/")).toBe("a b");
    expect(screenId("/screen/%E0%A4%A")).toBe("");
    expect(screenId("/other")).toBe("");
    expect(wsUrl({ protocol: "http:", host: "127.0.0.1:7770" }, "a b")).toBe("ws://127.0.0.1:7770/ws?screen=a%20b");
    expect(wsUrl({ protocol: "https:", host: "x" }, "t")).toBe("wss://x/ws?screen=t");
  });
});

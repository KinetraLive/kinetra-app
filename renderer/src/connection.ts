// WebSocket with reconnect backoff. Timers exist only while disconnected.
import type { ServerMsg } from "./protocol";

export type Status = "connecting" | "connected" | "reconnecting";

export interface SocketLike {
  onopen: ((e: Event) => void) | null;
  onmessage: ((e: MessageEvent) => void) | null;
  onclose: ((e: CloseEvent) => void) | null;
  onerror: ((e: Event) => void) | null;
  close(): void;
  send?(data: string): void;
}

/** The open socket (the last one opened), for the media chunk's `ready` reports. */
export let live: SocketLike | undefined;

export interface ConnectOpts {
  onStatus?: (s: Status) => void;
  /** Socket constructor (tests inject a fake). */
  WS?: new (url: string) => SocketLike;
  random?: () => number;
}

export const BACKOFF_MIN = 250;
export const BACKOFF_MAX = 5000;

/** Parses one frame; returns null for anything that isn't a protocol message. */
export function parseMsg(data: unknown): ServerMsg | null {
  try {
    const m = JSON.parse(String(data));
    if (m && typeof m.t === "string") return m as ServerMsg;
  } catch {
    console.warn("renderer: malformed message ignored");
  }
  return null;
}

/** Opens the socket and keeps it open: reconnects with jittered exponential backoff. */
export function connect(url: string, onMsg: (m: ServerMsg) => void, o: ConnectOpts = {}) {
  const WS = o.WS ?? (WebSocket as unknown as new (url: string) => SocketLike);
  const rnd = o.random ?? Math.random;
  const status = o.onStatus ?? (() => {});
  let delay = BACKOFF_MIN;

  const retry = () => {
    status("reconnecting");
    const d = delay * (0.8 + 0.4 * rnd());
    delay = Math.min(delay * 2, BACKOFF_MAX);
    setTimeout(open, d);
  };

  function open() {
    let ws: SocketLike;
    try {
      ws = new WS(url);
    } catch (e) {
      console.warn("renderer: websocket", e);
      return retry();
    }
    ws.onopen = () => ((live = ws), status("connected"));
    ws.onmessage = (e) => {
      const m = parseMsg(e.data);
      if (!m) return;
      if (m.t === "hello") delay = BACKOFF_MIN;
      try {
        onMsg(m);
      } catch (err) {
        console.warn("renderer:", err);
      }
    };
    ws.onclose = () => {
      ws.onclose = ws.onmessage = ws.onopen = ws.onerror = null;
      retry();
    };
    ws.onerror = () => {};
  }

  status("connecting");
  open();
}

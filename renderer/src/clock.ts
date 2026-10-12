// Clock and timer display, and the one shared wake-up that keeps them current.
import type { FieldType } from "./protocol";

/** Server clock minus renderer clock, from `hello.now`. */
let skew = 0;
/** Corrected time: the server's epoch ms. */
export const now = () => Date.now() + skew;

const p2 = (n: number) => (n < 10 ? "0" : "") + n;

/** Local time of day for `t`; tokens HH mm ss, everything else literal. */
export function clockText(fmt: string, t: number): string {
  const d = new Date(t);
  return (fmt || "HH:mm:ss").replace(/HH|mm|ss/g, (x) =>
    p2(x === "HH" ? d.getHours() : x === "mm" ? d.getMinutes() : d.getSeconds()),
  );
}

interface TimerState {
  run?: unknown;
  at?: unknown;
  base?: unknown;
  dir?: unknown;
}

function parseTimer(v: string): TimerState {
  try {
    const s = JSON.parse(v);
    return s && typeof s === "object" ? s : {};
  } catch {
    return {};
  }
}

const num = (v: unknown) => (typeof v === "number" && isFinite(v) ? v : 0);

/** Milliseconds a timer shows at `t`: up counts from base, down to zero; paused = base. */
export function timerMs(s: TimerState, t: number): number {
  const base = Math.max(0, num(s.base));
  if (s.run !== true) return base;
  const e = Math.max(0, t - num(s.at));
  return s.dir === "down" ? Math.max(0, base - e) : base + e;
}

/** `m:ss` below an hour, `h:mm:ss` from an hour. Up floors, down ceils (0:00 only at zero). */
export function timerText(v: string, t: number): string {
  const s = parseTimer(v);
  const ms = timerMs(s, t);
  const sec = s.dir === "down" ? Math.ceil(ms / 1000) : Math.floor(ms / 1000);
  const h = Math.floor(sec / 3600);
  const m = Math.floor(sec / 60) % 60;
  return h ? `${h}:${p2(m)}:${p2(sec % 60)}` : `${m}:${p2(sec % 60)}`;
}

/** Whether a timer's display changes over time. */
export function timerRuns(v: string, t: number): boolean {
  const s = parseTimer(v);
  return s.run === true && (s.dir !== "down" || timerMs(s, t) > 0);
}

/** The text a binding shows for a field value. */
export function show(type: FieldType | undefined, v: string): string {
  return type === "clock" ? clockText(v, now()) : type === "timer" ? timerText(v, now()) : v;
}

export interface Ticker {
  /** True while on air with a clock or a running timer. */
  needs(): boolean;
  /** Re-renders clock/timer bindings while on air (only changed text is written). */
  tick(): void;
}

const all = new Set<Ticker>();
let timer: ReturnType<typeof setTimeout> | undefined;

function tick() {
  for (const t of all)
    try {
      t.tick();
    } catch (e) {
      console.warn("renderer:", e);
    }
}

/**
 * Starts or stops the wake-up: one setTimeout to the next whole second of the corrected clock,
 * only while some ticker needs it and the page is visible. At rest no timer exists.
 */
export function sync() {
  let run = false;
  if (document.visibilityState !== "hidden") for (const t of all) if ((run = t.needs())) break;
  if (!run) {
    clearTimeout(timer);
    timer = undefined;
  } else if (timer === undefined)
    timer = setTimeout(
      () => {
        timer = undefined;
        tick();
        sync();
      },
      1000 - (((now() % 1000) + 1000) % 1000),
    );
}

/** Registers (`on`) or unregisters a ticker, then re-evaluates the wake-up. */
export function watch(t: Ticker, on: boolean) {
  if (on) all.add(t);
  else all.delete(t);
  sync();
}

/** Applies `hello.now` (absent → no correction) and re-renders every clock and timer. */
export function setServerNow(n: unknown) {
  skew = typeof n === "number" && isFinite(n) ? n - Date.now() : 0;
  tick();
  clearTimeout(timer);
  timer = undefined;
  sync();
}

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "hidden") tick();
  sync();
});

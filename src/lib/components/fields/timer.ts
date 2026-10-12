// `timer` field values (`TimerState` as JSON) and their display. Pure: no Svelte, no DOM.
//
// Shown time: `base` while paused; while running, `base` + (now - at) counting up, or
// `base` - (now - at) counting down, held at 0. Counting up shows whole seconds elapsed (floor);
// counting down shows the seconds left, rounded up, so it reads 00:01 until it reaches zero.

import type { FieldDef, TimerAction, TimerDir, TimerEnd, TimerState } from "$lib/types";

/** Longest start value accepted by Set (99:59:59). */
export const TIMER_MAX_MS = (99 * 3600 + 59 * 60 + 59) * 1000;

export const TIMER_DEFAULT: TimerState = { run: false, at: 0, base: 0, dir: "up", end: "stop" };

const isDir = (v: unknown): v is TimerDir => v === "up" || v === "down";
const isEnd = (v: unknown): v is TimerEnd => v === "stop" || v === "hide";
const num = (v: unknown, d: number) => (typeof v === "number" && Number.isFinite(v) ? v : d);

/** A timer value; anything malformed falls back field by field to a paused count-up at 0. */
export function parseTimer(value: string | undefined): TimerState {
  if (!value) return { ...TIMER_DEFAULT };
  let raw: unknown;
  try {
    raw = JSON.parse(value);
  } catch {
    return { ...TIMER_DEFAULT };
  }
  if (!raw || typeof raw !== "object") return { ...TIMER_DEFAULT };
  const r = raw as Record<string, unknown>;
  return {
    run: r.run === true,
    at: num(r.at, 0),
    base: Math.max(0, num(r.base, 0)),
    dir: isDir(r.dir) ? r.dir : "up",
    end: isEnd(r.end) ? r.end : "stop",
  };
}

export const timerValue = (s: TimerState) =>
  JSON.stringify({ run: s.run, at: s.at, base: s.base, dir: s.dir, end: s.end });

/** Milliseconds the timer shows at `now` (never below 0). */
export function shownMs(s: TimerState, now: number): number {
  if (!s.run) return s.base;
  const elapsed = Math.max(0, now - s.at);
  return s.dir === "up" ? s.base + elapsed : Math.max(0, s.base - elapsed);
}

/** Whole seconds displayed: floor counting up, ceil counting down. */
export function shownSeconds(s: TimerState, now: number): number {
  const ms = shownMs(s, now);
  return s.dir === "down" ? Math.ceil(ms / 1000) : Math.floor(ms / 1000);
}

/** `mm:ss`, or `h:mm:ss` from an hour. */
export function formatSeconds(total: number): string {
  const t = Math.max(0, Math.floor(total));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = t % 60;
  const p = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${p(m)}:${p(s)}` : `${p(m)}:${p(s)}`;
}

export const formatTimer = (s: TimerState, now: number) => formatSeconds(shownSeconds(s, now));

/**
 * Milliseconds until the displayed second changes, or null when it never will (paused, or a
 * countdown at zero). Aligned to the timer's own seconds, so one wake-up per visible change.
 */
export function nextTickIn(s: TimerState, now: number): number | null {
  if (!s.run) return null;
  const ms = shownMs(s, now);
  if (s.dir === "down") {
    if (ms <= 0) return null;
    const rem = ms % 1000;
    return rem === 0 ? 1000 : rem;
  }
  return 1000 - (ms % 1000);
}

/** "90", "1:30", "01:30" or "1:00:00" → ms; null when it doesn't parse or is out of range. */
export function parseDuration(text: string): number | null {
  const t = text.trim();
  if (!/^\d{1,5}(:\d{1,2}){0,2}$/.test(t)) return null;
  const parts = t.split(":").map(Number);
  if (parts.length > 1 && parts.slice(1).some((p) => p > 59)) return null;
  const secs = parts.reduce((acc, p) => acc * 60 + p, 0);
  const ms = secs * 1000;
  return ms <= TIMER_MAX_MS ? ms : null;
}

/**
 * The state after `action` at `now`, computed locally (the template preview, which has no server
 * timer). Mirrors the server: start runs from the shown time, pause holds it, reset returns to the
 * field's default, set holds at `ms`, configure changes direction / end (keeping the shown time).
 */
export function applyTimerAction(s: TimerState, action: TimerAction, now: number, def: FieldDef): TimerState {
  switch (action.action) {
    case "start":
      return s.run ? s : { ...s, run: true, at: now };
    case "pause":
      return s.run ? { ...s, run: false, base: shownMs(s, now) } : s;
    case "reset":
      return parseTimer(def.default);
    case "set":
      return { ...s, run: false, at: now, base: Math.max(0, Math.min(TIMER_MAX_MS, action.ms)) };
    case "configure": {
      const base = shownMs(s, now);
      return { ...s, base, at: now, dir: action.dir, end: action.end };
    }
  }
}

// The chip's words (`chipOf`) and the brief activity line (`ActivityLine`): part of the chip's lazy
// chunk (and Preferences), not Control's route chunk.
//
// The line follows "≤ 1 per 2 s per person, latest wins": a person's next action within 2 s of
// their last shown one waits for the end of that window (only the latest waiting one shows). Each
// line shows for LINE_MS. Timers exist only while a line shows or one waits.

import { on } from "$lib/api";
import type { RemoteActivity, RemoteView } from "$lib/types";

/** How long an activity line shows. */
export const LINE_MS = 4000;
/** Shortest time between two lines of one person. */
export const PER_PERSON_MS = 2000;

export class ActivityLine {
  /** The line showing now. */
  line = $state.raw<RemoteActivity | null>(null);

  #timer: ReturnType<typeof setTimeout> | undefined;
  /** Per person: when their last line showed, and their waiting one. */
  #people = new Map<string, { shown: number; wait?: { a: RemoteActivity; t: ReturnType<typeof setTimeout> } }>();

  /** Follows `remote-activity` until the returned stop. */
  listen(onEach?: (a: RemoteActivity) => void): () => void {
    const off = on("remote-activity", (a) => {
      this.add(a);
      onEach?.(a);
    });
    return () => {
      off();
      clearTimeout(this.#timer);
      for (const p of this.#people.values()) clearTimeout(p.wait?.t);
      this.#people.clear();
      this.line = null;
    };
  }

  /** An accepted remote action: shown now, or when its person's 2 s are up (latest wins). */
  add(a: RemoteActivity, now = Date.now()) {
    const p = this.#people.get(a.name) ?? { shown: 0 };
    this.#people.set(a.name, p);
    const due = p.shown + PER_PERSON_MS;
    if (now >= due && !p.wait) return this.#show(a, p, now);
    if (p.wait) {
      p.wait.a = a;
      return;
    }
    const wait = {
      a,
      t: setTimeout(() => {
        p.wait = undefined;
        this.#show(wait.a, p, Date.now());
      }, due - now),
    };
    p.wait = wait;
  }

  #show(a: RemoteActivity, p: { shown: number }, now: number) {
    p.shown = now;
    this.line = a;
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => (this.line = null), LINE_MS);
  }
}

type Tally = "preview" | "warning" | "idle";

/** The chip's words and tally for a view (null: nothing shows, remote control is off). The chip
 *  shows a remote icon before `text`; `label` is the full wording (screen readers, tooltip). */
export function chipOf(v: RemoteView | null): { text: string; label: string; tally: Tally } | null {
  if (!v?.on) return null;
  const c = (text: string, tally: Tally, label = `Remote control: ${text}`) => ({ text, label, tally });
  switch (v.state) {
    case "live": {
      const n = v.crew.length;
      return c(n === 0 ? "On" : `${n} connected`, "preview");
    }
    case "connecting":
      return c("Connecting", "idle");
    case "reconnecting":
      return c("Reconnecting", "warning");
    case "showInUse":
      return c("Move it here?", "warning");
    case "ended":
      return c("Ended", "idle", "Remote control ended");
    case "stopped":
      return c("Stopped", "warning");
    default:
      return null;
  }
}

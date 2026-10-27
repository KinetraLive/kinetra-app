// Remote control as the UI mirrors it: the view (`remote-control`) and the cue list's position
// (`cue-position`, Control's "Next"). Counted `watch()`: listeners exist only while something
// shows remote control; nothing polls. Kept small: it is in Control's route chunk. The chip's
// words and the activity line live in ./chip.svelte.ts (the chip's lazy chunk).

import { on } from "$lib/api";
import { cuePosition, remoteView } from "$lib/api/remote";
import type { CuePosition, RemoteView } from "$lib/types";

export class RemoteState {
  view = $state.raw<RemoteView | null>(null);
  cues = $state.raw<CuePosition>({ current: null, next: null });
  /** Remote control was just switched on (an event, from any window): the chip opens its panel
   *  once and clears this, so the link is the first thing the owner sees. */
  justOn = $state(false);

  #refs = 0;
  #off: (() => void)[] = [];

  /** What a `remote-control` event does. */
  apply(v: RemoteView) {
    // Off → on as this window saw it (an unknown view isn't "off": a page reload while on).
    if (v.on && this.view && !this.view.on) this.justOn = true;
    if (!v.on) this.justOn = false;
    this.view = v;
  }

  /** Starts mirroring (counted); returns the stop. */
  watch(): () => void {
    if (this.#refs++ === 0) {
      this.#off = [on("remote-control", (v) => this.apply(v)), on("cue-position", (c) => (this.cues = c))];
      // No backend (tests with a partial API mock): nothing to mirror.
      try {
        remoteView().then((v) => (this.view ??= v), () => {});
        cuePosition().then((c) => (this.cues = c), () => {});
      } catch {
        // stays empty
      }
    }
    let done = false;
    return () => {
      if (done) return;
      done = true;
      if (--this.#refs === 0) {
        this.#off.forEach((f) => f());
        this.#off = [];
      }
    };
  }
}

export const remote = new RemoteState();

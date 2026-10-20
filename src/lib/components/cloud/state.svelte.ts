// Cloud backup as the UI mirrors it (`cloud-changed`). Counted `watch()`: the listener exists
// only while something shows cloud backup; nothing polls.

import { on } from "$lib/api";
import { cloudView } from "$lib/api/cloud";
import type { CloudView } from "$lib/types";

export class CloudMirror {
  view = $state.raw<CloudView | null>(null);

  #refs = 0;
  #off: (() => void) | null = null;

  /** Starts mirroring (counted); returns the stop. */
  watch(): () => void {
    if (this.#refs++ === 0) {
      this.#off = on("cloud-changed", (v) => (this.view = v));
      try {
        cloudView().then((v) => (this.view ??= v), () => {});
      } catch {
        // No backend (tests with a partial API mock): nothing to mirror.
      }
    }
    let done = false;
    return () => {
      if (done) return;
      done = true;
      if (--this.#refs === 0) {
        this.#off?.();
        this.#off = null;
      }
    };
  }
}

export const cloud = new CloudMirror();

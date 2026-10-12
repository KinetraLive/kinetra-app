// The project's MIDI mappings (`controls.json`) as the UI sees them: loaded while someone watches
// (Preferences > MIDI & OSC, Control's chips and context menu), kept current by `controls-changed`,
// saved optimistically (rolled back with a toast when the core refuses). Lazy pieces import it;
// Control's eager code (./eager.svelte.ts) watches with a listener that fills its MIDI chips.

import { toAppError } from "$lib/api";
import { getControls, saveControls } from "$lib/api/controls";
import { onControlsChanged } from "$lib/api/midi";
import { toasts } from "$lib/stores/toasts.svelte";
import type { ControlsDoc } from "$lib/types";

export { chips } from "./model";

const EMPTY: ControlsDoc = { mappings: [] };

class ControlsStore {
  /** null until loaded (or without a project). */
  doc = $state.raw<ControlsDoc | null>(null);
  #watchers = 0;
  #listeners = new Set<(d: ControlsDoc) => void>();
  #off: (() => void) | null = null;
  #gen = 0;

  #put(doc: ControlsDoc) {
    this.doc = doc;
    for (const l of this.#listeners) l(doc);
  }

  /** Loads and follows the mappings while at least one caller watches; returns the unwatch.
   * `listener` gets every new document while watched (and the current one at once). */
  watch(listener?: (d: ControlsDoc) => void): () => void {
    if (listener) {
      this.#listeners.add(listener);
      if (this.doc) listener(this.doc);
    }
    if (this.#watchers++ === 0) {
      this.#off = onControlsChanged((d) => {
        this.#gen++;
        this.#put(d);
      });
      void this.load();
    }
    let done = false;
    return () => {
      if (done) return;
      done = true;
      if (listener) this.#listeners.delete(listener);
      if (--this.#watchers === 0) {
        this.#off?.();
        this.#off = null;
      }
    };
  }

  async load(): Promise<void> {
    const gen = ++this.#gen;
    try {
      const d = await getControls();
      if (gen === this.#gen) this.#put(d);
    } catch {
      // No project or no backend: nothing is mapped.
      if (gen === this.#gen && this.doc === null) this.#put(EMPTY);
    }
  }

  /** Saves `next` (shown at once); resolves false after a rollback. */
  async save(next: ControlsDoc): Promise<boolean> {
    const before = this.doc;
    const gen = ++this.#gen;
    this.#put(next);
    try {
      const saved = await saveControls(next);
      if (gen === this.#gen) this.#put(saved ?? next);
      return true;
    } catch (e) {
      if (gen === this.#gen) this.#put(before ?? EMPTY);
      toasts.push({ kind: "error", message: `Couldn't save the MIDI mappings: ${toAppError(e).message}` });
      return false;
    }
  }

  /** Applies `edit` to the current document and saves it. */
  update(edit: (d: ControlsDoc) => ControlsDoc): Promise<boolean> {
    return this.save(edit(this.doc ?? EMPTY));
  }
}

export const controls = new ControlsStore();

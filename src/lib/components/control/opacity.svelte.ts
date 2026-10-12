// Opacity in Control's expanded layer (LayerBody's chunk): the slider shows a local draft at once;
// ~150 ms after the last move the layer's opacity is saved to the screen (`saveScreen`: the core
// applies the live rules, so on air with "On take" it waits for TAKE, and in Studio mode it goes
// to Preview). The saved document replaces Control's copy of the selected screen.

import { toAppError } from "$lib/api";
import { getScreen, saveScreen } from "$lib/api/screens";
import { toasts } from "$lib/stores/toasts.svelte";
import type { ControlCtl } from "./control.svelte";

/** Like Control's field edits (EDIT_DEBOUNCE_MS; not imported: this chunk must not pull
 * control.svelte.ts out of the route's own chunk). */
const DEBOUNCE_MS = 150;

export class LayerOpacity {
  #drafts = $state.raw<ReadonlyMap<string, number>>(new Map());
  #timers = new Map<string, ReturnType<typeof setTimeout>>();
  readonly #ctlOf: () => ControlCtl;

  constructor(ctl: () => ControlCtl) {
    this.#ctlOf = ctl;
  }

  /** 0–100: the draft, else the saved value (absent = 100). */
  value(screenId: string, layerId: string): number {
    const d = this.#drafts.get(`${screenId}/${layerId}`);
    if (d !== undefined) return d;
    const doc = this.#ctlOf().doc;
    return (doc?.id === screenId ? doc.layers.find((l) => l.id === layerId)?.opacity : null) ?? 100;
  }

  set(screenId: string, layerId: string, n: number): void {
    const key = `${screenId}/${layerId}`;
    this.#drafts = new Map(this.#drafts).set(key, n);
    clearTimeout(this.#timers.get(key));
    this.#timers.set(
      key,
      setTimeout(() => void this.#save(key), DEBOUNCE_MS),
    );
  }

  /** Saves what is waiting now (on teardown). */
  async flush(): Promise<void> {
    const keys = [...this.#timers.keys()];
    for (const k of keys) clearTimeout(this.#timers.get(k));
    await Promise.all(keys.map((k) => this.#save(k)));
  }

  async #save(key: string): Promise<void> {
    this.#timers.delete(key);
    const n = this.#drafts.get(key);
    const [screenId, layerId] = key.split("/");
    if (n === undefined || !layerId) return;
    try {
      const { doc } = await getScreen(screenId);
      const next = { ...doc, layers: doc.layers.map((l) => (l.id === layerId ? { ...l, opacity: n } : l)) };
      const saved = await saveScreen(next);
      const ctl = this.#ctlOf();
      if (ctl.doc?.id === screenId) ctl.doc = saved.doc;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't change the opacity: ${toAppError(e).message}` });
    }
    // Keep a newer draft (the slider moved meanwhile).
    if (this.#drafts.get(key) === n && !this.#timers.has(key)) {
      const d = new Map(this.#drafts);
      d.delete(key);
      this.#drafts = d;
    }
  }
}

// Studio mode: each screen's Preview (cues, staged content, the T-bar), mirrored from the backend's
// `preview-changed` event and from `studioOp` replies. Only Control reads it.
//
//   $effect(() => previews.watch());   // subscribes while mounted (ref-counted)
//
// A view older than the one shown (lower `rev`) never replaces it; an optimistic view (a cue
// toggle) is put in place and rolled back only when nothing newer arrived meanwhile.

import { on } from "$lib/api";
import { previewView } from "$lib/api/live";
import type { PreviewView } from "$lib/types";
import { project } from "./project.svelte";

class PreviewStore {
  /** Screen id → its Preview. Replaced wholesale on every change. */
  views = $state.raw<ReadonlyMap<string, PreviewView>>(new Map());

  #watchers = 0;
  #off: (() => void) | null = null;
  #path: string | null = null;

  watch(): () => void {
    const path = project.current?.path ?? null;
    if (path !== this.#path) {
      this.#path = path;
      this.views = new Map();
    }
    if (this.#watchers++ === 0) this.#off = on("preview-changed", (v) => this.put(v));
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (--this.#watchers === 0) {
        this.#off?.();
        this.#off = null;
      }
    };
  }

  /** Puts a view in place unless a newer one (higher `rev`) is shown; `force` for optimistic ones. */
  put(v: PreviewView, force = false): void {
    const cur = this.views.get(v.screenId);
    if (!force && cur && cur.rev > v.rev) return;
    this.views = new Map(this.views).set(v.screenId, v);
  }

  /** Loads the given screens' Previews (once each unless `again`); failures leave them unknown. */
  load(ids: readonly string[], again = false): Promise<void> {
    const want = again ? ids : ids.filter((id) => !this.views.has(id));
    return Promise.all(
      want.map((id) =>
        previewView(id).then(
          (v) => this.put(v),
          () => {},
        ),
      ),
    ).then(() => {});
  }

  /** Forgets every Preview (Studio mode off). */
  clear(): void {
    if (this.views.size > 0) this.views = new Map();
  }

  /** The screen's Preview differs from Program. */
  pending(id: string): boolean {
    return (this.views.get(id)?.cues.length ?? 0) > 0;
  }
}

export const previews = new PreviewStore();

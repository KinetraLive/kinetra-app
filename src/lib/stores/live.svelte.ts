// Live state of the user's screens (what is on air, staged edits, field values), mirrored from
// the backend's `live-changed` event, which fires on any change: commands, auto-out, saves,
// watcher reloads.
//
//   $effect(() => live.watch());   // in any component that shows live state
//
// Ref-counted like the server store: the first watcher loads `listScreens()` and subscribes; the
// last unwatch releases the listener (nothing listens while nothing watches). `screens` keeps its
// last value between watchers so a remount paints at once, then refreshes. Cleared when
// another project is watched.

import { on } from "$lib/api";
import { listScreens } from "$lib/api/live";
import { TEST_SCREEN_ID, type LiveLayerView, type LiveScreenView } from "$lib/types";
import { project } from "./project.svelte";

const isReserved = (id: string) => id === TEST_SCREEN_ID || id.startsWith("__");

class LiveStore {
  /** Screen id → view, in the backend's order (by name). Replaced wholesale on every change. */
  screens = $state.raw<ReadonlyMap<string, LiveScreenView>>(new Map());
  /** True once the first `listScreens()` reply arrived for the current watch. */
  loaded = $state(false);

  #watchers = 0;
  #off: (() => void) | null = null;
  #gen = 0;
  /** Screens updated by events since the last list request; a stale list never overwrites them. */
  #fresh = new Set<string>();
  /** The project the cached `screens` belong to. */
  #path: string | null = null;

  /** Views in display order. */
  list = $derived([...this.screens.values()]);

  watch(): () => void {
    const path = project.current?.path ?? null;
    if (path !== this.#path) {
      // Another project: never paint the previous one's screens.
      this.#path = path;
      this.screens = new Map();
      this.loaded = false;
    }
    if (this.#watchers++ === 0) this.#start();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      if (--this.#watchers === 0) this.#stop();
    };
  }

  /** Reloads the whole list (e.g. after creating or deleting a screen). */
  refresh(): Promise<void> {
    const gen = this.#gen;
    this.#fresh.clear();
    return listScreens().then(
      (all) => {
        if (gen !== this.#gen) return;
        const next = new Map<string, LiveScreenView>();
        for (const v of all) next.set(v.screen.id, this.#fresh.has(v.screen.id) ? (this.screens.get(v.screen.id) ?? v) : v);
        // A screen an event reported after the request went out (just created) is kept.
        for (const id of this.#fresh) {
          const v = this.screens.get(id);
          if (v && !next.has(id)) next.set(id, v);
        }
        this.screens = next;
        this.loaded = true;
      },
      () => {
        // No backend (plain browser) or a transient failure: keep the last known state, but
        // count as loaded so pages show their empty state instead of waiting forever.
        if (gen === this.#gen) this.loaded = true;
      },
    );
  }

  /** Forgets the cached state (tests; the next watch reloads). */
  reset(): void {
    this.#path = null;
    this.screens = new Map();
    this.loaded = false;
    if (this.#watchers > 0) {
      this.#stop();
      this.#start();
    } else this.#gen++;
  }

  /** Puts one screen's view in place (an event, or a command reply). */
  put(view: LiveScreenView): void {
    const next = new Map(this.screens);
    next.set(view.screen.id, view);
    this.screens = next;
  }

  /** Puts one layer's view in place, e.g. from a TAKE reply, before the event arrives. */
  putLayer(screenId: string, layer: LiveLayerView): void {
    const s = this.screens.get(screenId);
    if (!s) return;
    this.put({ ...s, layers: s.layers.map((l) => (l.id === layer.id ? layer : l)) });
  }

  #start() {
    const gen = ++this.#gen;
    const offFiles = on("project-files-changed", (c) => {
      // A screen file added or removed outside Kinetra: the list itself changed.
      if (gen === this.#gen && c.kind === "screen" && c.change !== "invalid") void this.refresh();
    });
    const offLive = on("live-changed", (view) => {
      // Only user screens: never the dev test screen or `__*` system screens (as list_screens).
      if (gen !== this.#gen || isReserved(view.screen.id)) return;
      this.#fresh.add(view.screen.id);
      // A screen the list doesn't know yet (just created) is added at the end until the next list.
      this.put(view);
    });
    this.#off = () => {
      offFiles();
      offLive();
    };
    void this.refresh();
  }

  #stop() {
    this.#gen++;
    this.#off?.();
    this.#off = null;
  }
}

export const live = new LiveStore();

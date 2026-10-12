// Theme state: what the user chose (`pref`) and what is actually shown (`resolved`).
//
//   theme.pref       "system" | "light" | "dark"  (prefs.value.theme, else the localStorage mirror)
//   theme.resolved   "light" | "dark"             (pref with "system" resolved against the OS)
//   theme.toggle()   flip to the opposite of `resolved` (explicit light/dark), persisted to prefs
//   theme.set(pref)  for the Preferences page (System / Light / Dark)
//   theme.init()     call once from the root layout; keeps <html data-theme> + the mirror in sync
//                    with `pref` (including SyncedStore rollbacks). Returns a teardown.
//
// <html data-theme> is set for an explicit choice and removed for "system", so the CSS media query
// decides. The choice is mirrored to localStorage ("kinetra.theme") so app.html can apply it
// before first paint. The OS color-scheme is watched with a single matchMedia listener that is only
// attached while something reactive reads `resolved`/`systemDark`.

import { tick } from "svelte";
import { createSubscriber } from "svelte/reactivity";
import { toAppError } from "$lib/api";
import type { Theme } from "$lib/types";
import { prefs } from "./prefs.svelte";
import { toasts } from "./toasts.svelte";

export const THEME_STORAGE_KEY = "kinetra.theme";

export type ResolvedTheme = "light" | "dark";

export function resolveTheme(pref: Theme, systemDark: boolean): ResolvedTheme {
  if (pref === "system") return systemDark ? "dark" : "light";
  return pref;
}

function isTheme(v: unknown): v is Theme {
  return v === "system" || v === "light" || v === "dark";
}

function readMirror(): Theme | null {
  try {
    const v = localStorage.getItem(THEME_STORAGE_KEY);
    return isTheme(v) ? v : null;
  } catch {
    return null;
  }
}

function writeMirror(pref: Theme) {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, pref);
  } catch {
    // Storage unavailable: the theme still applies for this session.
  }
}

function applyDom(pref: Theme) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (pref === "system") delete root.dataset.theme;
  else if (root.dataset.theme !== pref) root.dataset.theme = pref;
}

function media(query: string): MediaQueryList | null {
  return typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(query)
    : null;
}

function reducedMotion(): boolean {
  return media("(prefers-reduced-motion: reduce)")?.matches ?? false;
}

class ThemeStore {
  #mirror = $state<Theme | null>(readMirror());
  #mql = media("(prefers-color-scheme: dark)");
  #subscribe = createSubscriber((update) => {
    const mql = this.#mql;
    if (!mql) return;
    mql.addEventListener("change", update);
    return () => mql.removeEventListener("change", update);
  });

  /** Whether the OS prefers dark. Reactive while read inside an effect/template. */
  get systemDark(): boolean {
    this.#subscribe();
    return this.#mql?.matches ?? false;
  }

  readonly pref: Theme = $derived(prefs.value?.theme ?? this.#mirror ?? "system");
  readonly resolved: ResolvedTheme = $derived(resolveTheme(this.pref, this.systemDark));

  init(): () => void {
    return $effect.root(() => {
      $effect(() => {
        const p = this.pref;
        applyDom(p);
        writeMirror(p);
      });
    });
  }

  toggle() {
    this.set(this.resolved === "dark" ? "light" : "dark");
  }

  set(next: Theme) {
    const commit = () => {
      applyDom(next);
      writeMirror(next);
      this.#mirror = next;
      const current = prefs.value;
      if (!current || current.theme === next) return;
      prefs.update({ ...current, theme: next }).catch((e: unknown) => {
        // SyncedStore rolled `prefs.value` back; make the DOM and mirror follow it.
        const p = this.pref;
        applyDom(p);
        writeMirror(p);
        this.#mirror = p;
        toasts.push({ kind: "error", message: `Couldn't save theme: ${toAppError(e).message}` });
      });
    };

    const doc = typeof document !== "undefined" ? document : null;
    if (doc && typeof doc.startViewTransition === "function" && !reducedMotion()) {
      doc.startViewTransition(async () => {
        commit();
        await tick();
      });
    } else {
      commit();
    }
  }
}

export const theme = new ThemeStore();

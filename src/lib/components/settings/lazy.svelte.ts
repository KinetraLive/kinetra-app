// Lazy pieces shared by the Preferences sections: a first-view trigger and a confirm-dialog
// controller that loads ConfirmDialog on first use and returns focus to the trigger afterwards.

import { tick } from "svelte";
import { toAppError } from "$lib/api";
import { toasts } from "$lib/stores/toasts.svelte";

/**
 * Calls `cb` once, the first time `node` scrolls near the viewport, then disconnects.
 * Use as an attachment: `{@attach firstVisible(load)}`.
 */
export function firstVisible(cb: () => void) {
  return (node: Element) => {
    if (typeof IntersectionObserver === "undefined") {
      cb();
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          io.disconnect();
          cb();
        }
      },
      { rootMargin: "200px 0px" },
    );
    io.observe(node);
    return () => io.disconnect();
  };
}

type ConfirmDialogType = typeof import("$lib/components/ConfirmDialog.svelte").default;
let confirmModule: Promise<ConfirmDialogType> | undefined;

export function loadConfirmDialog(): Promise<ConfirmDialogType> {
  confirmModule ??= import("$lib/components/ConfirmDialog.svelte").then((m) => m.default);
  confirmModule.catch(() => (confirmModule = undefined));
  return confirmModule;
}

/**
 * One pending confirmation carrying a value of type `T` (what is being confirmed).
 *   {#if c.Dialog && c.value}<c.Dialog ... busy={c.busy} onconfirm={...} oncancel={() => c.close()} />{/if}
 */
export class Confirm<T> {
  Dialog = $state.raw<ConfirmDialogType | null>(null);
  value = $state.raw<T | null>(null);
  busy = $state(false);
  #trigger: HTMLElement | null = null;

  async open(value: T) {
    const active = typeof document !== "undefined" ? document.activeElement : null;
    this.#trigger = active instanceof HTMLElement ? active : null;
    try {
      this.Dialog = await loadConfirmDialog();
      this.value = value;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't open the dialog: ${toAppError(e).message}` });
    }
  }

  close() {
    this.value = null;
    this.busy = false;
    const t = this.#trigger;
    this.#trigger = null;
    if (t) void tick().then(() => t.isConnected && t.focus());
  }

  /** Runs `fn` with the dialog busy; closes afterwards, whatever the outcome. */
  async run(fn: (value: T) => Promise<void>) {
    const v = this.value;
    if (v === null || this.busy) return;
    this.busy = true;
    try {
      await fn(v);
    } finally {
      this.close();
    }
  }
}

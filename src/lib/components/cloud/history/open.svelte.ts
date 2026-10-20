// Opens the version history from anywhere in the main window (the chip, the palette). The dialog
// is its own lazy chunk: nothing loads until the first `show()`. The top bar hosts it.

import type { Component } from "svelte";

export type HistoryComponent = Component<{ open: boolean; onclose: () => void }>;

let mod: Promise<HistoryComponent> | undefined;

/** Loads the dialog (cached; a failed load is retried next time). */
export function loadHistory(): Promise<HistoryComponent> {
  mod ??= import("./HistoryDialog.svelte").then((m) => m.default as HistoryComponent);
  mod.catch(() => (mod = undefined));
  return mod;
}

class HistoryLauncher {
  open = $state(false);
  show = () => {
    this.open = true;
  };
  close = () => {
    this.open = false;
  };
}

export const history = new HistoryLauncher();

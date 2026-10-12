// Opens the "New screen" dialog from anywhere. The dialog is its own lazy chunk: nothing loads
// until the first `show()`.
//
//   const newScreen = new NewScreenLauncher();
//   <Button onclick={newScreen.show}>New screen</Button>
//   {#if newScreen.component}
//     {@const NewScreen = newScreen.component}
//     <NewScreen open={newScreen.open} onclose={newScreen.close} />
//   {/if}
//
// Creating a screen closes the dialog and navigates to its layout editor (`/screens/<id>`).
// Optional `names` (the existing screen names) picks the default "Screen N"; without it the
// dialog asks the backend for the list itself.

import type { Component } from "svelte";
import { toAppError } from "$lib/api";
import { toasts } from "$lib/stores/toasts.svelte";

export type NewScreenProps = {
  open: boolean;
  onclose: () => void;
  names?: readonly string[];
};
export type NewScreenComponent = Component<NewScreenProps>;

let mod: Promise<NewScreenComponent> | undefined;

/** Loads the dialog component (cached; a failed load is retried next time). */
export function loadNewScreenDialog(): Promise<NewScreenComponent> {
  mod ??= import("./NewScreenDialog.svelte").then((m) => m.default as NewScreenComponent);
  mod.catch(() => (mod = undefined));
  return mod;
}

export class NewScreenLauncher {
  /** The loaded component; null until the first `show()`. */
  component = $state.raw<NewScreenComponent | null>(null);
  open = $state(false);

  show = async (): Promise<void> => {
    try {
      this.component ??= await loadNewScreenDialog();
      this.open = true;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't open the dialog: ${toAppError(e).message}` });
    }
  };

  close = (): void => {
    this.open = false;
  };
}

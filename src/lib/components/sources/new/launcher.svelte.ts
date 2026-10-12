// Opens the "New source" dialog from anywhere. The dialog is its own lazy chunk: nothing loads
// until the first `show()`.
//
//   const newSource = new NewSourceLauncher();
//   <Button onclick={newSource.show}>New source</Button>
//   {#if newSource.component}
//     {@const NewSource = newSource.component}
//     <NewSource open={newSource.open} onclose={newSource.close} oncreated={(doc) => select(doc.id)} />
//   {/if}
//
// Creating a source (data table or schedule) closes the dialog and calls `oncreated` with the new document.
// Optional `names` (the existing source names) picks the default "Source N"; without it the
// dialog asks the backend for the list itself.

import type { Component } from "svelte";
import { toAppError } from "$lib/api";
import { toasts } from "$lib/stores/toasts.svelte";
import type { ScheduleDoc, SourceDoc } from "$lib/types";

export type NewSourceProps = {
  open: boolean;
  onclose: () => void;
  oncreated?: (doc: SourceDoc | ScheduleDoc) => void;
  names?: readonly string[];
};
export type NewSourceComponent = Component<NewSourceProps>;

let mod: Promise<NewSourceComponent> | undefined;

/** Loads the dialog component (cached; a failed load is retried next time). */
export function loadNewSourceDialog(): Promise<NewSourceComponent> {
  mod ??= import("./NewSourceDialog.svelte").then((m) => m.default as NewSourceComponent);
  mod.catch(() => (mod = undefined));
  return mod;
}

export class NewSourceLauncher {
  /** The loaded component; null until the first `show()`. */
  component = $state.raw<NewSourceComponent | null>(null);
  open = $state(false);

  show = async (): Promise<void> => {
    try {
      this.component ??= await loadNewSourceDialog();
      this.open = true;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't open the dialog: ${toAppError(e).message}` });
    }
  };

  close = (): void => {
    this.open = false;
  };
}

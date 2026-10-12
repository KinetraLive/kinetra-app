// Opens the linear flow editor ("When … then …") from Control. The editor is its own lazy chunk:
// nothing loads until the first `show()`. One editor at a time; the page renders it:
//
//   {#if ctl.flowEditor.component && ctl.flowEditor.target}
//     {@const Editor = ctl.flowEditor.component}
//     <Editor target={ctl.flowEditor.target} {ctl} onclose={ctl.flowEditor.close} />
//   {/if}

import type { Component } from "svelte";
import { toAppError } from "$lib/api";
import { toasts } from "$lib/stores/toasts.svelte";
import type { ControlCtl } from "$lib/components/control/control.svelte";

/** The recipe a new flow belongs to (its `origin`), and what its trigger and steps aim at. */
export interface EditorOrigin {
  id: string;
  screenId: string;
  layerId: string;
  sourceId: string | null;
}

/**
 * A new cue (Cue trigger, armed) or rule (an automatic trigger, disarmed), optionally owned by a
 * recipe, or an existing flow.
 */
export type EditorTarget =
  | { mode: "new"; kind: "cue" | "rule"; origin?: EditorOrigin }
  | { mode: "edit"; id: string };

export type LinearEditorProps = {
  target: EditorTarget;
  ctl: ControlCtl;
  onclose: () => void;
};
export type LinearEditorComponent = Component<LinearEditorProps>;

let mod: Promise<LinearEditorComponent> | undefined;

/** Loads the editor component (cached; a failed load is retried next time). */
export function loadLinearEditor(): Promise<LinearEditorComponent> {
  mod ??= import("./LinearEditor.svelte").then((m) => m.default as LinearEditorComponent);
  mod.catch(() => (mod = undefined));
  return mod;
}

export class FlowEditorLauncher {
  /** The loaded component; null until the first `show()`. */
  component = $state.raw<LinearEditorComponent | null>(null);
  /** What is open; null when closed. */
  target = $state.raw<EditorTarget | null>(null);

  show = async (target: EditorTarget): Promise<void> => {
    try {
      this.component ??= await loadLinearEditor();
      this.target = target;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't open the flow editor: ${toAppError(e).message}` });
    }
  };

  close = (): void => {
    this.target = null;
  };
}

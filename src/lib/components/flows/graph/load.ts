// Lazy chunks of the flow editor, each loaded on first use (cached; a failed load is retried next
// time): the canvas (the ONLY way @xyflow/svelte is loaded), the palette, the inspector (on the
// first selection) and its form per node category.

import type { Component } from "svelte";
import type { FlowEditorCtl } from "./editor.svelte";
import type { Category } from "./ports";
import type { FlowNode } from "$lib/types";

export type CanvasProps = { ed: FlowEditorCtl };
export type PaletteProps = { ed: FlowEditorCtl };
export type FormProps = { ed: FlowEditorCtl; node: FlowNode };
export type InspectorProps = FormProps;

function cached<P extends Record<string, unknown>>(load: () => Promise<{ default: unknown }>) {
  let mod: Promise<Component<P>> | undefined;
  return (): Promise<Component<P>> => {
    mod ??= load().then((m) => m.default as Component<P>);
    mod.catch(() => (mod = undefined));
    return mod;
  };
}

export const loadCanvas = cached<CanvasProps>(() => import("./canvas/Canvas.svelte"));
export const loadPalette = cached<PaletteProps>(() => import("./Palette.svelte"));
export const loadInspector = cached<InspectorProps>(() => import("./Inspector.svelte"));

const forms: Record<Category, () => Promise<Component<FormProps>>> = {
  trigger: cached<FormProps>(() => import("./inspector/TriggerForm.svelte")),
  data: cached<FormProps>(() => import("./inspector/DataForm.svelte")),
  logic: cached<FormProps>(() => import("./inspector/LogicForm.svelte")),
  action: cached<FormProps>(() => import("./inspector/ActionForm.svelte")),
};

export const loadForm = (c: Category) => forms[c]();

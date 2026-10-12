// The flow editor controller as Svelte context, for the canvas's node and edge components (they are
// created by @xyflow/svelte, so props can't reach them).

import { getContext, setContext } from "svelte";
import type { FlowEditorCtl } from "./editor.svelte";

const KEY = Symbol("flow-editor");

export const provideEditor = (ctl: FlowEditorCtl) => setContext(KEY, ctl);
export const useEditor = () => getContext<FlowEditorCtl>(KEY);

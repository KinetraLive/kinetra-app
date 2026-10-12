// Shared by the inspector forms (one lazy chunk per node category): committing a node's new config
// as one editor edit. Typing into one field merges into one undo step (`group`).

import type { FlowNode, NodeKind } from "$lib/types";
import type { FlowEditorCtl } from "../editor.svelte";

/** Replaces `node`'s config. `field`: keystrokes in the same field are one undo step. */
export function commit<N extends FlowNode>(ed: FlowEditorCtl, node: N, config: N["config"], field?: string): void {
  ed.setKind(node.id, { type: node.type, config } as NodeKind, field ? `${node.id}:${field}` : null);
}

/** A deep copy of a node's config to change and commit (helpers like setScreen mutate). */
export const draftOf = <N extends FlowNode>(node: N): N["config"] => structuredClone(node.config) as N["config"];

/** Seconds as shown in the forms ↔ milliseconds in the config. */
export const toSeconds = (ms: number) => Math.round(ms / 100) / 10;
export const toMs = (s: number) => Math.round(s * 1000);

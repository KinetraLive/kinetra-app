// A cue flow's button text, for the lazy chunks that list cues by name (the palette, Keyboard
// shortcuts). The same test as flows.svelte's cueOf / cueLabel: not imported from there, so that
// module stays inside the Control route's chunk instead of being split out for these chunks.

import type { Flow } from "$lib/types";

/** The text on a cue's button (its Cue trigger's label, else the flow's name); null when the flow
 * has no Cue trigger. */
export function cueText(flow: Pick<Flow, "nodes" | "name">): string | null {
  const cue = flow.nodes.find((n) => n.type === "trigger.cue");
  if (!cue) return null;
  const label = (cue.config as { label?: string | null }).label;
  return label?.trim() || flow.name;
}

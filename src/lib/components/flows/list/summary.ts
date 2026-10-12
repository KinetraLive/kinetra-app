// Each /flows row's trigger summary. A lazy chunk of the list (it pulls in linear.ts and the node
// labels): the page imports it while the list loads and renders names first. Plain TS, unit-tested.

import type { FlowInfo } from "$lib/types";
import { isTriggerType, summary, toLinear, type Lookups } from "../linear";
import { nodeLabel } from "../graph/labels";

/**
 * What sets a flow off, in one line: the linear summary ("When … then …"), else its trigger types
 * and size ("Cue · Layer on/off air · 6 nodes").
 */
export function triggerSummary(f: Pick<FlowInfo, "nodes" | "edges"> & { id?: string }, k: Lookups): string {
  if (toLinear(f)) return summary(f, k);
  const triggers = [...new Set(f.nodes.filter((n) => isTriggerType(n.type)).map((n) => nodeLabel(n.type)))];
  const size = `${f.nodes.length} ${f.nodes.length === 1 ? "node" : "nodes"}`;
  return triggers.length > 0 ? `${triggers.join(" · ")} · ${size}` : `No trigger · ${size}`;
}

// The node palette's model: search over the catalogue, grouped by category. Plain TS.

import { CATEGORIES, NODES, type NodeInfo } from "./nodes";
import { categoryOf, type Category } from "./ports";

/** The drag-and-drop type of a palette item (its value is the node type). */
export const PALETTE_MIME = "application/x-kinetra-node";

export interface PaletteGroup {
  id: Category;
  label: string;
  items: NodeInfo[];
}

/** Every word of `query` appears in the label, the type or the description (any case). */
export function matches(n: NodeInfo, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return true;
  const hay = `${n.label} ${n.type} ${n.description}`.toLowerCase();
  return words.every((w) => hay.includes(w));
}

/** The palette's groups for a search (empty groups left out), in catalogue order. */
export function paletteGroups(query: string): PaletteGroup[] {
  return CATEGORIES.map((c) => ({
    id: c.id,
    label: c.label,
    items: NODES.filter((n) => categoryOf(n.type) === c.id && matches(n, query)),
  })).filter((g) => g.items.length > 0);
}

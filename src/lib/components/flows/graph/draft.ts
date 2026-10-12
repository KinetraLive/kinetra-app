// The flow editor's working graph: pure operations on { nodes, edges } and the session history
// (undo / redo of whole-graph snapshots). No Svelte, no DOM, unit-tested. The editor
// (./editor.svelte.ts) owns one History and sends its current graph to the server (draft autosave,
// validation).
//
// Every operation returns a new graph and leaves untouched nodes and edges as the same objects, so
// the canvas reuses their rendered nodes. An operation that changes nothing returns its input
// (callers skip the history entry and the save).

import { FLOW_EDGES_MAX, FLOW_NODES_MAX, type FlowEdge, type FlowNode, type NodeKind, type PortRef } from "$lib/types";
import { newNodeId } from "../linear";
import { checkConnection, inputOf, outputOf } from "./ports";

export interface Graph {
  nodes: FlowNode[];
  edges: FlowEdge[];
}

export type Pos = { x: number; y: number };

export function addNodes(g: Graph, nodes: readonly FlowNode[]): Graph {
  if (nodes.length === 0) return g;
  return { nodes: [...g.nodes, ...nodes], edges: g.edges };
}

/** Removes nodes (with their wires) and wires. */
export function removeElements(g: Graph, nodeIds: Iterable<string>, edgeIds: Iterable<string> = []): Graph {
  const nodes = new Set(nodeIds);
  const edges = new Set(edgeIds);
  if (nodes.size === 0 && edges.size === 0) return g;
  const keptNodes = g.nodes.filter((n) => !nodes.has(n.id));
  const keptEdges = g.edges.filter((e) => !edges.has(e.id) && !nodes.has(e.from.node) && !nodes.has(e.to.node));
  if (keptNodes.length === g.nodes.length && keptEdges.length === g.edges.length) return g;
  return { nodes: keptNodes, edges: keptEdges };
}

const round = (v: number) => Math.round(v);

/** Sets node positions (rounded to whole pixels); unchanged nodes stay the same objects. */
export function moveNodes(g: Graph, positions: ReadonlyMap<string, Pos>): Graph {
  let changed = false;
  const nodes = g.nodes.map((n) => {
    const p = positions.get(n.id);
    if (!p) return n;
    const x = round(p.x);
    const y = round(p.y);
    if (x === n.position.x && y === n.position.y) return n;
    changed = true;
    return { ...n, position: { x, y } };
  });
  return changed ? { nodes, edges: g.edges } : g;
}

/** Moves nodes by (dx, dy). */
export function nudge(g: Graph, ids: Iterable<string>, dx: number, dy: number): Graph {
  const set = new Set(ids);
  const positions = new Map<string, Pos>();
  for (const n of g.nodes) if (set.has(n.id)) positions.set(n.id, { x: n.position.x + dx, y: n.position.y + dy });
  return moveNodes(g, positions);
}

export type ConnectResult = { ok: true; graph: Graph; edge: FlowEdge } | { ok: false; reason: string };

/** Adds a wire from an output to an input, if `checkConnection` allows it. */
export function connect(g: Graph, from: PortRef, to: PortRef, id: () => string = newNodeId): ConnectResult {
  const c = checkConnection(g, from, to);
  if (!c.ok) return c;
  if (g.edges.length >= FLOW_EDGES_MAX) return { ok: false, reason: `A flow has at most ${FLOW_EDGES_MAX} wires.` };
  const edge: FlowEdge = { id: id(), from: { ...from }, to: { ...to }, kind: c.kind };
  return { ok: true, graph: { nodes: g.nodes, edges: [...g.edges, edge] }, edge };
}

/** Wires whose ports still exist on their nodes (a config change can remove ports). */
export function pruneEdges(g: Graph): Graph {
  const byId = new Map(g.nodes.map((n) => [n.id, n]));
  const edges = g.edges.filter((e) => {
    const a = byId.get(e.from.node);
    const b = byId.get(e.to.node);
    return !!a && !!b && !!outputOf(a, e.from.port) && !!inputOf(b, e.to.port);
  });
  return edges.length === g.edges.length ? g : { nodes: g.nodes, edges };
}

/** Replaces a node's type and config (the id and position stay); drops wires on vanished ports. */
export function setNodeKind(g: Graph, id: string, kind: NodeKind): Graph {
  let changed = false;
  const nodes = g.nodes.map((n) => {
    if (n.id !== id) return n;
    if (stableStringify(kind) === stableStringify({ type: n.type, config: n.config })) return n;
    changed = true;
    return { ...kind, id: n.id, position: n.position } as FlowNode;
  });
  return changed ? pruneEdges({ nodes, edges: g.edges }) : g;
}

export const canAdd = (g: Graph, n = 1) => g.nodes.length + n <= FLOW_NODES_MAX;

/** JSON with object keys sorted: the same graph gives the same text whatever the key order. */
export function stableStringify(v: unknown): string {
  if (v === null || typeof v !== "object") return JSON.stringify(v) ?? "null";
  if (Array.isArray(v)) return `[${v.map(stableStringify).join(",")}]`;
  const o = v as Record<string, unknown>;
  const keys = Object.keys(o)
    .filter((k) => o[k] !== undefined)
    .sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${stableStringify(o[k])}`).join(",")}}`;
}

export const graphKey = (g: Graph) => stableStringify({ nodes: g.nodes, edges: g.edges });

// ---- History ------------------------------------------------------------------------------------

/** Most undo steps kept per editor session. */
export const HISTORY_MAX = 100;

/**
 * Undo / redo over whole-graph snapshots (cheap: unchanged nodes are shared). `push` with a
 * `group` merges into the previous step when that step had the same group, so typing into one
 * field is one undo step, not one per keystroke.
 */
export class History {
  #past: Graph[] = [];
  #future: Graph[] = [];
  #group: string | null = null;
  #current: Graph;

  constructor(
    initial: Graph,
    private readonly cap = HISTORY_MAX,
  ) {
    this.#current = initial;
  }

  get current(): Graph {
    return this.#current;
  }

  get canUndo(): boolean {
    return this.#past.length > 0;
  }

  get canRedo(): boolean {
    return this.#future.length > 0;
  }

  /** Makes `next` current; false when it is the current graph (nothing to record). */
  push(next: Graph, group: string | null = null): boolean {
    if (next === this.#current) return false;
    if (group === null || group !== this.#group || this.#past.length === 0) {
      this.#past.push(this.#current);
      if (this.#past.length > this.cap) this.#past.shift();
    }
    this.#group = group;
    this.#future = [];
    this.#current = next;
    return true;
  }

  undo(): Graph | null {
    const prev = this.#past.pop();
    if (!prev) return null;
    this.#future.push(this.#current);
    this.#current = prev;
    this.#group = null;
    return prev;
  }

  redo(): Graph | null {
    const next = this.#future.pop();
    if (!next) return null;
    this.#past.push(this.#current);
    this.#current = next;
    this.#group = null;
    return next;
  }

  /** Starts over from `g` (a reload or discard): no undo across it. */
  reset(g: Graph): void {
    this.#past = [];
    this.#future = [];
    this.#group = null;
    this.#current = g;
  }
}

// The flow editor's controller (/flows/[id]): the working graph, its history, draft autosave,
// validation, external edits, live debug and the operations the canvas, palette, inspector and
// header call. One per open editor; the route component creates it and calls `connect()`.
//
//   const ed = new FlowEditorCtl(id);
//   $effect(() => ed.connect());   // loads, subscribes, and on teardown flushes and unsubscribes
//
// Draft model: the working graph is the flow's draft, else its deployed graph. Every edit makes a
// new graph (./draft.ts), recorded in the session history, saved with `save_flow_draft` 800 ms
// after the last edit (one save in flight; Autosave) and validated with `check_flow` 300 ms after
// it (latest wins). Errors block Deploy; issues map to nodes (node null: the header banner).
// Leaving flushes a pending save. Deploy flushes, then deploys; Discard drops the draft on the
// server and starts over from the deployed graph.
//
// External edits (`project-files-changed`, kind flow, this id): without unsaved edits the file is
// taken silently (one undo step), else a banner offers Reload / Keep mine. Removed: a banner.
//
// Live debug: subscribed (`subscribe_flow_debug`) while the editor is mounted AND the window is
// visible; `flow-debug` batches of this flow flash nodes and wires (./debug.svelte.ts).

import { on, toAppError } from "$lib/api";
import {
  checkFlow,
  deployFlow,
  discardFlowDraft,
  duplicateFlow,
  getFlow,
  renameFlow,
  saveFlowDraft,
  subscribeFlowDebug,
  unsubscribeFlowDebug,
} from "$lib/api/flows";
import { listSources } from "$lib/api/sources";
import { Autosave, diskReaction, type SaveState } from "$lib/components/templates/editor/sync";
import { toasts } from "$lib/stores/toasts.svelte";
import type {
  FilesChanged,
  FlowCheck,
  FlowInfo,
  FlowIssue,
  FlowNode,
  NodeKind,
  NodeType,
  PortRef,
  SourceSummary,
} from "$lib/types";
import { FLOW_NODES_MAX } from "$lib/types";
import type { Defaults } from "../defaults";
import { ScreenViews } from "../list/screens.svelte";
import { FlowList } from "../list/store.svelte";
import type { Lookups } from "../linear";
import { defaultsFrom, newNode } from "./catalog";
import { DebugState } from "./debug.svelte";
import {
  History,
  addNodes,
  canAdd,
  connect,
  graphKey,
  moveNodes,
  nudge,
  removeElements,
  setNodeKind,
  type Graph,
  type Pos,
} from "./draft";
import { accepts, checkConnection, isWait, portsOf, portsWithColumns, type Port, type Ports } from "./ports";

/** Which side of a node a port is on. */
export type Side = "input" | "output";

export const SAVE_DEBOUNCE_MS = 800;
export const CHECK_DEBOUNCE_MS = 300;
/** How long a refused connection's reason stays on the canvas. */
export const NOTICE_MS = 4000;

/** What the canvas offers the controller once it is loaded. */
export interface CanvasApi {
  /** Selects exactly these (others are deselected). */
  select(nodeIds: readonly string[], edgeIds?: readonly string[]): void;
  /** The flow position at the middle of the visible canvas. */
  center(): Pos;
  zoomIn(): void;
  zoomOut(): void;
  fit(): void;
  /** Moves keyboard focus to a node. */
  focusNode(id: string): void;
}

export interface NodeIssues {
  errors: string[];
  warnings: string[];
}

const EMPTY: Graph = { nodes: [], edges: [] };
const reduced = () =>
  typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

export class FlowEditorCtl {
  readonly id: string;
  readonly flows = new FlowList();
  readonly screens = new ScreenViews();
  readonly debug = new DebugState(reduced);

  loaded = $state(false);
  loadError = $state<string | null>(null);
  /** The working graph (draft, else deployed). Replaced wholesale on every edit. */
  graph = $state.raw<Graph>(EMPTY);
  canUndo = $state(false);
  canRedo = $state(false);
  check = $state.raw<FlowCheck>({ errors: [], warnings: [] });
  /** True until the first validation of the working graph arrived. */
  checking = $state(true);
  saveState = $state.raw<SaveState>({ kind: "saved", at: null });
  disk = $state<"conflict" | "removed" | null>(null);
  diskBusy = $state(false);
  deploying = $state(false);
  selectedNodes = $state.raw<ReadonlySet<string>>(new Set());
  selectedEdges = $state.raw<ReadonlySet<string>>(new Set());
  /** A refused connection's reason (shown on the canvas for NOTICE_MS). */
  notice = $state<string | null>(null);
  /** Bumped to ask the inspector to take focus (Enter on a node). */
  inspectorFocus = $state(0);
  sources = $state.raw<SourceSummary[]>([]);
  /** The port a wire is being dragged from (the canvas highlights the ports that can take it). */
  connecting = $state.raw<{ node: string; port: string; side: Side } | null>(null);
  canvas: CanvasApi | null = null;

  /** The flow as the server last reported it (list entry; kept current by its events). */
  info = $derived.by((): FlowInfo | null => this.flows.get(this.id) ?? null);
  readOnly = $derived(this.info?.readOnly ?? false);
  /** The draft differs from the deployed graph (as the server sees it), or an edit is unsaved. */
  notDeployed = $derived(
    !!this.info?.draft || this.saveState.kind === "pending" || this.saveState.kind === "saving",
  );
  lookups = $derived.by((): Lookups => ({
    screens: this.screens.map,
    sources: new Map(this.sources.map((s) => [s.id, s])),
    flows: new Map(this.flows.list.map((f) => [f.id, f])),
    self: this.id,
  }));
  /** Issues per node: validation errors and warnings plus run-time errors from live debug. */
  issues = $derived.by((): ReadonlyMap<string, NodeIssues> => {
    const out = new Map<string, NodeIssues>();
    const get = (id: string) => {
      let v = out.get(id);
      if (!v) out.set(id, (v = { errors: [], warnings: [] }));
      return v;
    };
    for (const e of this.check.errors) if (e.node) get(e.node).errors.push(e.message);
    for (const w of this.check.warnings) if (w.node) get(w.node).warnings.push(w.message);
    for (const [node, message] of this.debug.errors) get(node).errors.push(message);
    for (const [node, message] of this.debug.warnings) get(node).warnings.push(message);
    return out;
  });
  /** Issues that belong to no node (limits, ids): shown in the header banner. */
  flowIssues = $derived<FlowIssue[]>([
    ...this.check.errors.filter((e) => !e.node),
    ...this.check.warnings.filter((w) => !w.node),
  ]);
  /** Why Deploy is disabled, or null. */
  deployBlocked = $derived.by((): string | null => {
    if (this.readOnly) return "This flow is read-only.";
    if (this.disk === "removed") return "This flow was deleted on disk.";
    if (this.disk === "conflict") return "Changed on disk: choose Reload or Keep mine.";
    const n = this.check.errors.length;
    if (n > 0) return `${n} ${n === 1 ? "problem" : "problems"} to fix first.`;
    if (this.graph.nodes.length === 0) return "Add a trigger first.";
    return null;
  });
  /** The one selected node, or null. */
  selected = $derived.by(() => {
    if (this.selectedNodes.size !== 1) return null;
    const [id] = this.selectedNodes;
    return this.graph.nodes.find((n) => n.id === id) ?? null;
  });

  #history = new History(EMPTY);
  #autosave: Autosave<Graph>;
  #checkTimer: ReturnType<typeof setTimeout> | undefined;
  #checkGen = 0;
  #noticeTimer: ReturnType<typeof setTimeout> | undefined;
  #deleted = false;
  #subscribed = false;

  constructor(id: string) {
    this.id = id;
    this.#autosave = new Autosave<Graph>({
      delayMs: SAVE_DEBOUNCE_MS,
      snapshot: () => this.graph,
      blocked: () =>
        this.readOnly ? "read-only" : this.disk === "conflict" ? "Changed on disk: choose Reload or Keep mine." : null,
      save: async (g) => {
        const f = await saveFlowDraft(this.id, g.nodes, g.edges);
        this.flows.put(f);
        return f;
      },
      onState: (s) => (this.saveState = s),
      describe: (e) => toAppError(e).message,
    });
  }

  /** Loads and subscribes; the returned function flushes and releases everything. */
  connect(): () => void {
    let alive = true;
    const offLive = this.screens.connect();
    const offFlows = this.flows.connect();
    const offFiles = on("project-files-changed", (e) => this.#onFiles(e));
    const offState = on("flow-state", (s) => {
      if (s.id === this.id && s.state === "running" && !s.message) this.debug.newRun();
    });
    const offDebug = on("flow-debug", (b) => {
      if (b.id === this.id) this.debug.apply(b);
    });
    const offSources = on("source-changed", (s) => {
      this.sources = this.sources.some((x) => x.id === s.id)
        ? this.sources.map((x) => (x.id === s.id ? s : x))
        : [...this.sources, s];
    });
    const onVisibility = () => this.#syncSubscription(alive && document.visibilityState === "visible");
    document.addEventListener("visibilitychange", onVisibility);
    onVisibility();
    listSources().then(
      (s) => alive && (this.sources = s ?? []),
      () => {},
    );
    void this.#load(() => alive);
    return () => {
      alive = false;
      document.removeEventListener("visibilitychange", onVisibility);
      this.#syncSubscription(false);
      offSources();
      offState();
      offDebug();
      offFiles();
      offFlows();
      offLive();
      this.#clearCheck();
      this.#clearNotice();
      this.debug.dispose();
      if (this.#deleted || this.readOnly) this.#autosave.cancel();
      else void this.#autosave.flush();
    };
  }

  /** Saves a pending draft now (leaving the page, before Deploy). */
  flush(): Promise<void> {
    return this.#autosave.flush();
  }

  async #load(alive: () => boolean): Promise<void> {
    let f: FlowInfo;
    try {
      f = await getFlow(this.id);
    } catch (e) {
      if (alive()) this.loadError = toAppError(e).message;
      return;
    }
    if (!alive()) return;
    this.flows.put(f);
    this.#reset(workingGraph(f));
    this.loaded = true;
  }

  #syncSubscription(want: boolean) {
    if (want === this.#subscribed) return;
    this.#subscribed = want;
    if (want) subscribeFlowDebug(this.id).catch(() => (this.#subscribed = false));
    else {
      unsubscribeFlowDebug(this.id).catch(() => {});
      this.debug.clearFlashes();
    }
  }

  // ---- Edits ----------------------------------------------------------------------------------

  /** Makes `next` the working graph (one history step, or merged into the last one of `group`). */
  apply(next: Graph, group: string | null = null): boolean {
    if (this.readOnly || this.disk === "removed") return false;
    if (!this.#history.push(next, group)) return false;
    this.#set(next);
    this.#autosave.touch();
    return true;
  }

  undo = (): void => {
    if (this.readOnly) return;
    const g = this.#history.undo();
    if (!g) return;
    this.#set(g);
    this.#autosave.touch();
  };

  redo = (): void => {
    if (this.readOnly) return;
    const g = this.#history.redo();
    if (!g) return;
    this.#set(g);
    this.#autosave.touch();
  };

  #set(g: Graph) {
    this.graph = g;
    this.canUndo = this.#history.canUndo;
    this.canRedo = this.#history.canRedo;
    // Selection follows what still exists.
    const nodes = new Set(g.nodes.map((n) => n.id));
    const edges = new Set(g.edges.map((e) => e.id));
    if ([...this.selectedNodes].some((id) => !nodes.has(id))) {
      this.selectedNodes = new Set([...this.selectedNodes].filter((id) => nodes.has(id)));
    }
    if ([...this.selectedEdges].some((id) => !edges.has(id))) {
      this.selectedEdges = new Set([...this.selectedEdges].filter((id) => edges.has(id)));
    }
    this.#queueCheck();
  }

  #reset(g: Graph) {
    this.#history.reset(g);
    this.#set(g);
  }

  defaults(): Defaults {
    return defaultsFrom(this.screens.list, this.lookups.sources);
  }

  /** Adds a node of `type` at `at` (else near the middle of the view) and selects it. */
  addNode = (type: NodeType, at?: Pos): string | null => {
    if (this.readOnly) return null;
    if (!canAdd(this.graph)) {
      this.say(`A flow has at most ${FLOW_NODES_MAX} nodes.`);
      return null;
    }
    const c = at ?? this.canvas?.center() ?? { x: 0, y: 0 };
    const pos = at ? c : this.#freeSpot({ x: c.x - 100, y: c.y - 40 });
    const node = newNode(type, this.defaults(), { x: Math.round(pos.x), y: Math.round(pos.y) });
    if (!this.apply(addNodes(this.graph, [node]))) return null;
    this.setSelection([node.id]);
    this.canvas?.select([node.id]);
    return node.id;
  };

  /** Steps down from `p` until no node starts within 24 px of it (so repeated adds don't stack). */
  #freeSpot(p: Pos): Pos {
    let y = p.y;
    for (let i = 0; i < 40; i++) {
      if (!this.graph.nodes.some((n) => Math.abs(n.position.x - p.x) < 24 && Math.abs(n.position.y - y) < 24)) break;
      y += 32;
    }
    return { x: p.x, y };
  }

  removeSelected = (): void => {
    this.remove([...this.selectedNodes], [...this.selectedEdges]);
  };

  remove(nodeIds: readonly string[], edgeIds: readonly string[] = []): void {
    this.apply(removeElements(this.graph, nodeIds, edgeIds));
  }

  move(positions: ReadonlyMap<string, Pos>): void {
    this.apply(moveNodes(this.graph, positions));
  }

  /** Arrow keys: 8 px, Shift ×10. */
  nudge(dx: number, dy: number): void {
    if (this.selectedNodes.size === 0) return;
    this.apply(nudge(this.graph, this.selectedNodes, dx, dy), null);
  }

  /** Adds a wire; a refused one says why on the canvas. True when added. */
  wire(from: PortRef, to: PortRef): boolean {
    if (this.readOnly) return false;
    const r = connect(this.graph, from, to);
    if (!r.ok) {
      this.say(`Can't connect: ${r.reason}`);
      return false;
    }
    return this.apply(r.graph);
  }

  /**
   * While a wire is dragged: whether this port can take it (true / false), or null when nothing is
   * being wired or this is the port it started from.
   */
  canTake(node: string, port: string, side: Side): boolean | null {
    const c = this.connecting;
    if (!c || (c.node === node && c.port === port && c.side === side)) return null;
    if (c.side === side) return false;
    const r =
      c.side === "output"
        ? checkConnection(this.graph, { node: c.node, port: c.port }, { node, port })
        : checkConnection(this.graph, { node, port }, { node: c.node, port: c.port });
    return r.ok;
  }

  /** A node's ports as shown: a Source row node gets its source's columns (and wired ones). */
  portsFor(n: FlowNode): Ports {
    if (n.type !== "data.sourceRow") return portsOf(n);
    const cols = (this.lookups.sources.get(n.config.sourceId)?.columns ?? []).map((c) => c.key);
    for (const e of this.graph.edges) if (e.from.node === n.id && !cols.includes(e.from.port)) cols.push(e.from.port);
    return portsWithColumns(n, cols);
  }

  /**
   * The ports a port can be wired to ("Connect to…"): for an output, every input that takes it; for
   * an input, every output it takes. One pass over the graph's ports.
   */
  partners(nodeId: string, port: string, side: Side): { node: FlowNode; port: Port }[] {
    const g = this.graph;
    const self = g.nodes.find((n) => n.id === nodeId);
    if (!self) return [];
    const mine = (side === "output" ? this.portsFor(self).outputs : this.portsFor(self).inputs).find((p) => p.name === port);
    if (!mine) return [];
    const key = (a: string, b: string, c: string, d: string) => `${a}\n${b}\n${c}\n${d}`;
    const wired = new Set(g.edges.map((e) => key(e.from.node, e.from.port, e.to.node, e.to.port)));
    const fedInputs = new Set(g.edges.filter((e) => e.kind === "data").map((e) => `${e.to.node}\n${e.to.port}`));
    const out: { node: FlowNode; port: Port }[] = [];
    for (const n of g.nodes) {
      const ps = side === "output" ? this.portsFor(n).inputs : this.portsFor(n).outputs;
      for (const p of ps) {
        if (p.kind !== mine.kind) continue;
        const [from, fromPort, to, toPort] =
          side === "output" ? [self, mine, n, p] : [n, p, self, mine];
        if (wired.has(key(from.id, fromPort.name, to.id, toPort.name))) continue;
        // A node feeds itself only as an event loop through a waiting node.
        if (from.id === to.id && (p.kind === "data" || !isWait(from.type))) continue;
        if (p.kind === "data") {
          if (fedInputs.has(`${to.id}\n${toPort.name}`)) continue;
          if (!accepts(to, toPort, fromPort.type ?? "text")) continue;
        }
        out.push({ node: n, port: p });
      }
    }
    return out;
  }

  /** Why a wire from `from` to `to` is refused, or null when it is allowed. */
  refusal(from: PortRef, to: PortRef): string | null {
    const r = checkConnection(this.graph, from, to);
    return r.ok ? null : r.reason;
  }

  disconnect(edgeId: string): void {
    this.apply(removeElements(this.graph, [], [edgeId]));
  }

  /** A node's new type/config. `group` merges keystrokes in one field into one undo step. */
  setKind(id: string, kind: NodeKind, group: string | null = null): void {
    this.apply(setNodeKind(this.graph, id, kind), group);
  }

  /** The node wired into a data input of `nodeId`, if any. */
  feeding(nodeId: string, port: string): FlowNode | undefined {
    const e = this.graph.edges.find((x) => x.kind === "data" && x.to.node === nodeId && x.to.port === port);
    return e ? this.graph.nodes.find((x) => x.id === e.from.node) : undefined;
  }

  setSelection(nodes: Iterable<string>, edges: Iterable<string> = []): void {
    this.selectedNodes = new Set(nodes);
    this.selectedEdges = new Set(edges);
  }

  selectAll = (): void => {
    const ids = this.graph.nodes.map((n) => n.id);
    this.setSelection(ids);
    this.canvas?.select(ids);
  };

  clearSelection = (): void => {
    this.setSelection([]);
    this.canvas?.select([]);
  };

  /** Shows a short message on the canvas (one timer, only while shown). */
  say(message: string): void {
    this.#clearNotice();
    this.notice = message;
    this.#noticeTimer = setTimeout(() => {
      this.#noticeTimer = undefined;
      this.notice = null;
    }, NOTICE_MS);
  }

  #clearNotice() {
    if (this.#noticeTimer !== undefined) clearTimeout(this.#noticeTimer);
    this.#noticeTimer = undefined;
  }

  // ---- Validation -----------------------------------------------------------------------------

  #clearCheck() {
    if (this.#checkTimer !== undefined) clearTimeout(this.#checkTimer);
    this.#checkTimer = undefined;
  }

  #queueCheck() {
    this.#clearCheck();
    this.checking = true;
    this.#checkTimer = setTimeout(() => void this.runCheck(), CHECK_DEBOUNCE_MS);
  }

  /** Validates the working graph now (latest wins: an older reply is ignored). */
  async runCheck(): Promise<void> {
    this.#clearCheck();
    const gen = ++this.#checkGen;
    const g = this.graph;
    try {
      const r = await checkFlow(g.nodes, g.edges, this.id);
      if (gen !== this.#checkGen) return;
      this.check = r;
    } catch (e) {
      if (gen !== this.#checkGen) return;
      this.check = { errors: [{ node: null, message: `Couldn't check the flow: ${toAppError(e).message}` }], warnings: [] };
    }
    this.checking = false;
  }

  // ---- Deploy / discard -----------------------------------------------------------------------

  deploy = async (): Promise<boolean> => {
    if (this.deploying || this.deployBlocked) return false;
    this.deploying = true;
    try {
      // Validate what is deployed exactly: a pending check runs now.
      if (this.#checkTimer !== undefined || this.checking) await this.runCheck();
      if (this.deployBlocked) return false;
      await this.#autosave.flush();
      if (this.saveState.kind === "failed") {
        toasts.push({ kind: "error", message: `Couldn't deploy: the draft wasn't saved (${this.saveState.message}).` });
        return false;
      }
      this.flows.put(await deployFlow(this.id));
      toasts.push({ kind: "success", message: `Deployed “${this.info?.name ?? "flow"}”` });
      return true;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't deploy: ${toAppError(e).message}` });
      return false;
    } finally {
      this.deploying = false;
    }
  };

  discard = async (): Promise<boolean> => {
    if (this.readOnly) return false;
    this.#autosave.discard();
    try {
      const f = await discardFlowDraft(this.id);
      this.flows.put(f);
      this.#reset(workingGraph(f));
      this.saveState = { kind: "saved", at: f.updatedAt };
      return true;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't discard the draft: ${toAppError(e).message}` });
      return false;
    }
  };

  // ---- Header ---------------------------------------------------------------------------------

  setArmed = (armed: boolean): Promise<void> => this.flows.setArmed(this.id, armed);

  async rename(name: string): Promise<boolean> {
    try {
      this.flows.put(await renameFlow(this.id, name));
      return true;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't rename the flow: ${toAppError(e).message}` });
      return false;
    }
  }

  /** Saves pending edits, duplicates the flow and returns the copy's id. */
  async duplicate(): Promise<string | null> {
    try {
      if (!this.readOnly) await this.#autosave.flush();
      const f = await duplicateFlow(this.id);
      this.flows.put(f);
      return f.id;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't duplicate the flow: ${toAppError(e).message}` });
      return null;
    }
  }

  // ---- External edits -------------------------------------------------------------------------

  #onFiles(e: FilesChanged) {
    if (!this.loaded || this.#deleted) return;
    const dirty = this.#autosave.dirty || this.#autosave.saving || this.disk === "conflict";
    switch (diskReaction(e, "flow", this.id, dirty)) {
      case "reload":
        void this.#takeFromDisk(false);
        break;
      case "conflict":
        this.disk = "conflict";
        break;
      case "removed":
        this.#deleted = true;
        this.#autosave.cancel();
        this.disk = "removed";
        break;
      case "invalid":
        toasts.push({
          kind: "error",
          message: `This flow changed on disk but couldn't be loaded: ${e.message ?? "invalid file"}. Your copy here is kept.`,
        });
        break;
      case "ignore":
        break;
    }
  }

  /** Takes the file on disk as the working graph (an undo step; `force`: also drops unsaved edits). */
  async #takeFromDisk(force: boolean): Promise<boolean> {
    let f: FlowInfo;
    try {
      f = await getFlow(this.id);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't reload the flow: ${toAppError(e).message}` });
      return false;
    }
    this.flows.put(f);
    if (force) this.#autosave.discard();
    const g = workingGraph(f);
    if (graphKey(g) !== graphKey(this.graph) && this.#history.push(g)) this.#set(g);
    this.saveState = { kind: "saved", at: f.updatedAt };
    this.disk = null;
    return true;
  }

  diskReload = async (): Promise<void> => {
    this.diskBusy = true;
    try {
      await this.#takeFromDisk(true);
    } finally {
      this.diskBusy = false;
    }
  };

  diskKeep = async (): Promise<void> => {
    this.disk = null;
    this.diskBusy = true;
    try {
      await this.#autosave.saveNow();
    } finally {
      this.diskBusy = false;
    }
  };
}

/** The graph the editor works on: the draft, else the deployed graph. */
export function workingGraph(f: Pick<FlowInfo, "nodes" | "edges" | "draft">): Graph {
  return f.draft ? { nodes: f.draft.nodes, edges: f.draft.edges } : { nodes: f.nodes, edges: f.edges };
}

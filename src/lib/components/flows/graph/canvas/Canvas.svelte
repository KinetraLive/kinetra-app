<!--
  Canvas: the flow editor's node canvas, built on @xyflow/svelte. THE lazy chunk of the editor: the
  route loads it with one dynamic import() (./load.ts), and nothing else imports @xyflow/svelte
  (only this folder; enforced by a test). xyflow's base CSS comes with it; its look is ours (no
  shadows or gradients; tokens only): see the :global rules below.

  The controller owns the graph. This component maps it to xyflow nodes and edges (reusing the
  objects of unchanged ones, keeping selection and measured sizes) and reports back: a drag's end
  positions (one history step, saved debounced), selection, wiring (only ports that can take the
  wire light up; a refused drop says why), drops from the palette.

  Keyboard (on a focused node): arrows nudge the selection 8 px (Shift ×10), Enter selects it and
  opens the inspector. Anywhere on the canvas: Delete / Backspace removes the selection, Ctrl+A
  selects all, Escape clears the selection. Tab moves between nodes (xyflow).

  Props:
    ed: FlowEditorCtl
-->
<script lang="ts">
  import "@xyflow/svelte/dist/base.css";
  import {
    Background,
    BackgroundVariant,
    SvelteFlow,
    type Connection,
    type Edge,
    type Node,
    type OnConnectEnd,
    type OnConnectStartParams,
  } from "@xyflow/svelte";
  import { untrack } from "svelte";
  import Maximize from "lucide-svelte/icons/maximize";
  import ZoomIn from "lucide-svelte/icons/zoom-in";
  import ZoomOut from "lucide-svelte/icons/zoom-out";
  import IconButton from "$lib/components/IconButton.svelte";
  import { isTyping } from "$lib/commands/keys";
  import { theme } from "$lib/stores/theme.svelte";
  import type { FlowEdge, FlowNode, NodeType } from "$lib/types";
  import { nodeLabel } from "../catalog";
  import { provideEditor } from "../context";
  import type { FlowEditorCtl } from "../editor.svelte";
  import { PALETTE_MIME } from "../palette";
  import Api from "./Api.svelte";
  import EdgeView from "./EdgeView.svelte";
  import NodeView from "./NodeView.svelte";

  let { ed }: { ed: FlowEditorCtl } = $props();

  provideEditor(untrack(() => ed));

  type XNode = Node<{ node: FlowNode }, "flow">;
  type XEdge = Edge<{ edge: FlowEdge }>;

  const nodeTypes = { flow: NodeView };
  const edgeTypes = { event: EdgeView, data: EdgeView };
  const NUDGE = 8;

  let nodes = $state.raw<XNode[]>([]);
  let edges = $state.raw<XEdge[]>([]);
  let host: HTMLDivElement | undefined = $state();

  // The graph → xyflow, reusing the objects of unchanged nodes and wires.
  $effect(() => {
    const g = ed.graph;
    untrack(() => {
      const prevN = new Map(nodes.map((n) => [n.id, n]));
      nodes = g.nodes.map((fn): XNode => {
        const p = prevN.get(fn.id);
        if (p && p.data.node === fn) return p;
        const ariaLabel = nodeLabel(fn.type);
        if (p) return { ...p, position: { ...fn.position }, data: { node: fn }, ariaLabel };
        return {
          id: fn.id,
          type: "flow",
          position: { ...fn.position },
          data: { node: fn },
          ariaLabel,
          selected: ed.selectedNodes.has(fn.id),
        };
      });
      const prevE = new Map(edges.map((e) => [e.id, e]));
      edges = g.edges.map((fe): XEdge => {
        const p = prevE.get(fe.id);
        if (p && p.data?.edge === fe) return p;
        return {
          id: fe.id,
          type: fe.kind,
          source: fe.from.node,
          sourceHandle: fe.from.port,
          target: fe.to.node,
          targetHandle: fe.to.port,
          data: { edge: fe },
          selected: p?.selected ?? ed.selectedEdges.has(fe.id),
        };
      });
    });
  });

  function select(nodeIds: readonly string[], edgeIds: readonly string[] = []) {
    const n = new Set(nodeIds);
    const e = new Set(edgeIds);
    nodes = nodes.map((x) => (!!x.selected === n.has(x.id) ? x : { ...x, selected: n.has(x.id) }));
    edges = edges.map((x) => (!!x.selected === e.has(x.id) ? x : { ...x, selected: e.has(x.id) }));
    ed.setSelection(n, e);
  }

  function onDragStop({ nodes: moved }: { nodes: XNode[] }) {
    if (moved.length === 0) return;
    ed.move(new Map(moved.map((n) => [n.id, n.position])));
  }

  function onSelection({ nodes: ns, edges: es }: { nodes: XNode[]; edges: XEdge[] }) {
    ed.setSelection(
      ns.map((n) => n.id),
      es.map((e) => e.id),
    );
  }

  const ref = (c: Connection) => ({
    from: { node: c.source, port: c.sourceHandle ?? "" },
    to: { node: c.target, port: c.targetHandle ?? "" },
  });

  function isValidConnection(c: Connection | XEdge): boolean {
    const r = ref(c as Connection);
    return ed.refusal(r.from, r.to) === null;
  }

  function onConnectStart(_: unknown, p: OnConnectStartParams) {
    if (!p.nodeId || !p.handleId || !p.handleType) return;
    ed.connecting = { node: p.nodeId, port: p.handleId, side: p.handleType === "source" ? "output" : "input" };
  }

  function onConnectEnd(_: unknown, s: Parameters<OnConnectEnd>[1]) {
    const c = ed.connecting;
    ed.connecting = null;
    // Dropped on a port that can't take it: say why.
    if (!c || s.isValid !== false || !s.toHandle?.id || !s.toHandle.nodeId) return;
    const other = { node: s.toHandle.nodeId, port: s.toHandle.id };
    const here = { node: c.node, port: c.port };
    const toIsInput = s.toHandle.type === "target";
    if ((c.side === "output") !== toIsInput) {
      ed.say(`Can't connect: ${c.side === "output" ? "an output goes into an input" : "an input takes an output"}.`);
      return;
    }
    const why = c.side === "output" ? ed.refusal(here, other) : ed.refusal(other, here);
    if (why) ed.say(`Can't connect: ${why}`);
  }

  /** The wire is added through the controller (and so the graph); xyflow adds nothing itself. */
  function onBeforeConnect(c: Connection): false {
    const r = ref(c);
    ed.wire(r.from, r.to);
    return false;
  }

  function nodeOf(t: EventTarget | null): string | null {
    const el = t instanceof Element ? t.closest<HTMLElement>(".svelte-flow__node") : null;
    return el?.dataset.id ?? null;
  }

  const ARROWS: Record<string, [number, number]> = {
    ArrowUp: [0, -1],
    ArrowDown: [0, 1],
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
  };

  function onKeyCapture(e: KeyboardEvent) {
    if (isTyping(e.target) || e.altKey) return;
    const id = nodeOf(e.target);
    const arrow = ARROWS[e.key];
    if (id && arrow && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      e.stopPropagation();
      if (ed.readOnly) return;
      if (!ed.selectedNodes.has(id)) select([id]);
      const step = NUDGE * (e.shiftKey ? 10 : 1);
      ed.nudge(arrow[0] * step, arrow[1] * step);
      return;
    }
    if (id && e.key === "Enter" && !e.ctrlKey && !e.metaKey) {
      e.preventDefault();
      e.stopPropagation();
      select([id]);
      ed.inspectorFocus++;
      return;
    }
    if ((e.key === "Delete" || e.key === "Backspace") && !e.ctrlKey && !e.metaKey) {
      if (ed.readOnly || (ed.selectedNodes.size === 0 && ed.selectedEdges.size === 0)) return;
      e.preventDefault();
      e.stopPropagation();
      ed.removeSelected();
      host?.focus();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "a") {
      e.preventDefault();
      e.stopPropagation();
      ed.selectAll();
      return;
    }
    if (e.key === "Escape" && (ed.selectedNodes.size > 0 || ed.selectedEdges.size > 0)) {
      e.preventDefault();
      e.stopPropagation();
      ed.clearSelection();
    }
  }

  function onDragOver(e: DragEvent) {
    if (ed.readOnly || !e.dataTransfer?.types.includes(PALETTE_MIME)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
  }

  let api: { toFlow(x: number, y: number): { x: number; y: number } } | null = null;

  function onDrop(e: DragEvent) {
    const type = e.dataTransfer?.getData(PALETTE_MIME) as NodeType | undefined;
    if (!type || ed.readOnly || !api) return;
    e.preventDefault();
    const p = api.toFlow(e.clientX, e.clientY);
    ed.addNode(type, { x: p.x - 100, y: p.y - 24 });
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="canvas"
  bind:this={host}
  tabindex="-1"
  role="application"
  aria-label="Flow canvas"
  aria-roledescription="node editor"
  onkeydowncapture={onKeyCapture}
  ondragover={onDragOver}
  ondrop={onDrop}
>
  <SvelteFlow
    bind:nodes
    bind:edges
    {nodeTypes}
    {edgeTypes}
    colorMode={theme.resolved}
    fitView
    fitViewOptions={{ padding: 0.1, maxZoom: 1, minZoom: 0.75 }}
    minZoom={0.15}
    maxZoom={2}
    deleteKey={[]}
    selectionKey="Shift"
    zoomOnDoubleClick={false}
    nodesDraggable={!ed.readOnly}
    nodesConnectable={!ed.readOnly}
    edgesFocusable={false}
    connectionRadius={24}
    {isValidConnection}
    onbeforeconnect={onBeforeConnect}
    onconnectstart={onConnectStart}
    onconnectend={onConnectEnd}
    onnodedragstop={onDragStop}
    onselectionchange={onSelection}
    onselectiondragstop={(_, moved) => onDragStop({ nodes: moved })}
  >
    <Background variant={BackgroundVariant.Dots} gap={24} size={1} patternClass="dots" />
    <Api
      {host}
      register={(a) => {
        api = a;
        ed.canvas = {
          select,
          center: a.center,
          zoomIn: a.zoomIn,
          zoomOut: a.zoomOut,
          fit: a.fit,
          focusNode: (id) => host?.querySelector<HTMLElement>(`.svelte-flow__node[data-id="${CSS.escape(id)}"]`)?.focus(),
        };
        return () => {
          api = null;
          ed.canvas = null;
        };
      }}
    />
  </SvelteFlow>

  <div class="zoom" role="toolbar" aria-label="Zoom">
    <IconButton size="sm" label="Zoom in" tooltip="right" onclick={() => ed.canvas?.zoomIn()}><ZoomIn size={14} /></IconButton>
    <IconButton size="sm" label="Zoom out" tooltip="right" onclick={() => ed.canvas?.zoomOut()}><ZoomOut size={14} /></IconButton>
    <IconButton size="sm" label="Fit the flow in view" tooltip="right" onclick={() => ed.canvas?.fit()}><Maximize size={14} /></IconButton>
  </div>

  <p class="notice" role="status" aria-live="polite">{ed.notice ?? ""}</p>
</div>

<style>
  .canvas {
    position: relative;
    width: 100%;
    height: 100%;
    background: var(--surface-1);
    outline: none;
  }

  /* ---- xyflow, in our tokens ---- */
  .canvas :global(.svelte-flow) {
    --xy-background-color: var(--surface-1);
    --xy-edge-stroke: var(--text-muted);
    --xy-connectionline-stroke: var(--accent);
    --xy-connectionline-stroke-width: 2;
    --xy-selection-background-color: var(--accent-soft);
    --xy-selection-border: 1px solid var(--accent);
    --xy-attribution-background-color: transparent;
    background: var(--surface-1);
    font-family: var(--font-body);
  }
  /* app.css caps svg at max-width: 100%; xyflow's edge layers are zero-width boxes that overflow. */
  .canvas :global(.svelte-flow svg) {
    max-width: none;
  }
  .canvas :global(.svelte-flow__background .dots) {
    fill: var(--line-strong);
  }
  .canvas :global(.svelte-flow__node) {
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    box-shadow: none;
    outline: none;
  }
  /* Focus shows the node's own viewfinder brackets. */
  .canvas :global(.svelte-flow__node:focus-visible .vf::after) {
    opacity: 1;
    transform: scale(1);
  }
  .canvas :global(.svelte-flow__nodesselection-rect) {
    border: 1px dashed var(--accent);
    background: var(--accent-soft);
  }
  .canvas :global(.svelte-flow__attribution) {
    padding: 0 var(--s-1);
    background: transparent;
    font-size: 10px;
  }
  .canvas :global(.svelte-flow__attribution a) {
    color: var(--text-muted);
  }
  .canvas :global(.svelte-flow__connection-path) {
    stroke-width: 2;
  }

  .zoom {
    position: absolute;
    bottom: var(--s-3);
    left: var(--s-3);
    z-index: 5;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .notice {
    position: absolute;
    bottom: var(--s-3);
    left: 50%;
    z-index: 5;
    max-width: min(560px, 80%);
    margin: 0;
    padding: var(--s-1) var(--s-3);
    border-radius: var(--r-control);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--danger);
    font-size: var(--fs-sm);
    transform: translateX(-50%);
  }
  .notice:empty {
    display: none;
  }
</style>

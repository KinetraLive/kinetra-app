<!--
  NodeView: one flow node on the canvas (rendered by @xyflow/svelte; part of the lazy canvas chunk).

    TRIGGER                14:02:31     category (section label) and last-fired time (mono)
    Schedule item status                the node type
    Item becomes NOW in Main stage      its config in one line
    ▶ in                     fired ▶    ports: inputs on the left, outputs on the right
    ● index · number    row · row ●
    Pick a source                       errors (--danger) and warnings (--warning), as text

  Event ports are chevrons in --accent, data ports circles in --text-muted with their type in the
  label and the tooltip: shape and colour differ, so colour never carries meaning alone. While a
  wire is dragged, ports that can take it grow and the others fade. Selected: viewfinder brackets.
  A flash (live debug) fades an accent outline in and out (opacity only).

  Props: NodeProps of @xyflow/svelte (data.node is the FlowNode).
-->
<script lang="ts">
  import { Handle, Position, type NodeProps } from "@xyflow/svelte";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import type { FlowNode } from "$lib/types";
  import { clockOf, nodeLabel, nodeSummary, portLabel } from "../catalog";
  import { useEditor } from "../context";
  import { categoryOf, type Port } from "../ports";
  import type { Side } from "../editor.svelte";

  let { id, data, selected }: NodeProps & { data: { node: FlowNode } } = $props();

  const ed = useEditor();
  const node = $derived(data.node);
  const cat = $derived(categoryOf(node.type));
  const ports = $derived(ed.portsFor(node));
  const rows = $derived(Math.max(ports.inputs.length, ports.outputs.length));
  const issues = $derived(ed.issues.get(id));
  const fired = $derived(ed.debug.fired.get(id) ?? null);
  const flashing = $derived(ed.debug.nodes.has(id));
  const summary = $derived(nodeSummary(node, ed.lookups, (p) => ed.feeding(id, p)));
  const CAT_LABEL = { trigger: "Trigger", data: "Data", logic: "Logic", action: "Action" } as const;

  function tip(p: Port, side: Side): string {
    const what = p.kind === "event" ? "event" : `${p.type} value`;
    return `${portLabel(node, p.name)}: ${what} ${side === "input" ? "in" : "out"}`;
  }

  function state(p: Port, side: Side): "can" | "cannot" | undefined {
    const r = ed.canTake(id, p.name, side);
    return r === null ? undefined : r ? "can" : "cannot";
  }
</script>

{#snippet port(p: Port | undefined, side: Side)}
  {#if p}
    <span class="port {side}" title={tip(p, side)} data-port={p.name} data-side={side}>
      <Handle
        type={side === "input" ? "target" : "source"}
        position={side === "input" ? Position.Left : Position.Right}
        id={p.name}
        class={["h", p.kind, state(p, side)]}
        isConnectable={!ed.readOnly}
      />
      <span class="plabel">
        {portLabel(node, p.name)}{#if p.type}<span class="ptype mono"> · {p.type}</span>{/if}
      </span>
    </span>
  {:else}
    <span class="port {side} empty"></span>
  {/if}
{/snippet}

<div
  class="node vf {cat}"
  class:flash={flashing}
  class:has-error={!!issues?.errors.length}
  data-selected={selected || undefined}
  data-node={id}
  data-type={node.type}
>
  <div class="head">
    <span class="section-label">{CAT_LABEL[cat]}</span>
    {#if fired}<span class="fired mono" title="Last fired {new Date(fired).toLocaleString()}">{clockOf(fired)}</span>{/if}
  </div>
  <div class="title">{nodeLabel(node.type)}</div>
  {#if summary}<div class="sum" title={summary}>{summary}</div>{/if}
  {#if rows > 0}
    <div class="ports">
      {#each { length: rows } as _, i (i)}
        <div class="row">
          {@render port(ports.inputs[i], "input")}
          {@render port(ports.outputs[i], "output")}
        </div>
      {/each}
    </div>
  {/if}
  {#if issues && (issues.errors.length > 0 || issues.warnings.length > 0)}
    <div class="issues">
      {#each issues.errors as e, i (i)}<p class="err" title={e}>{e}</p>{/each}
      {#each issues.warnings as w, i (i)}
        <p class="warn" title={w}><TriangleAlert size={11} aria-hidden="true" />{w}</p>
      {/each}
    </div>
  {/if}
</div>

<style>
  .node {
    --vf-radius: var(--r-control);
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 200px;
    padding: var(--s-2) 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-size: var(--fs-sm);
    line-height: 1.3;
  }
  .node.has-error {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  /* Live debug: an accent outline that fades in (120 ms) and out (220 ms); opacity only. */
  .node::before {
    content: "";
    position: absolute;
    inset: -1px;
    border: 2px solid var(--accent);
    border-radius: inherit;
    opacity: 0;
    pointer-events: none;
  }
  .node.flash::before {
    animation: flash 340ms linear;
  }
  @keyframes flash {
    0% {
      opacity: 0;
    }
    35% {
      opacity: 1;
    }
    100% {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .node.flash::before {
      animation: none;
      opacity: 1;
    }
  }
  .head,
  .title,
  .sum,
  .issues {
    padding: 0 var(--s-2);
  }
  .head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--s-2);
    color: var(--text-muted);
  }
  .fired {
    font-size: var(--fs-xs);
  }
  .title {
    font-size: var(--fs-md);
    font-weight: 600;
  }
  .sum {
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ports {
    display: flex;
    flex-direction: column;
    margin-top: var(--s-1);
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: var(--s-2);
    min-height: 20px;
  }
  .port {
    position: relative;
    display: flex;
    flex: 0 1 auto;
    align-items: center;
    min-width: 0;
    padding: 0 var(--s-2) 0 var(--s-3);
  }
  .port.output {
    justify-content: flex-end;
    padding: 0 var(--s-3) 0 var(--s-2);
    text-align: right;
  }
  .port.empty {
    flex: 1 1 0;
    pointer-events: none;
  }
  .plabel {
    overflow: hidden;
    font-size: var(--fs-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ptype {
    color: var(--text-muted);
  }
  .issues {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: var(--s-1);
  }
  .issues p {
    display: -webkit-box;
    margin: 0;
    overflow: hidden;
    font-size: var(--fs-xs);
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }
  .err {
    color: var(--danger);
  }
  .warn {
    color: var(--warning);
  }
  .warn :global(svg) {
    display: inline;
    margin-right: 4px;
    vertical-align: -1px;
  }

  /* ---- Handles: event = chevron (accent), data = circle (muted) ----
     The handle is a 16 px hit area; its shape (10 px) is drawn by ::after. */
  .port :global(.h) {
    position: absolute;
    top: 50%;
    width: 16px;
    height: 16px;
    min-width: 0;
    min-height: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    transform: translateY(-50%);
  }
  .port.input :global(.h) {
    left: -8px;
  }
  .port.output :global(.h) {
    right: -8px;
    left: auto;
  }
  .port :global(.h::after) {
    content: "";
    position: absolute;
    inset: 3px;
    transition:
      transform var(--dur-fast) var(--ease),
      opacity var(--dur-fast) var(--ease);
  }
  .port :global(.h.event::after) {
    background: var(--accent);
    clip-path: polygon(15% 0, 100% 50%, 15% 100%);
  }
  .port :global(.h.data::after) {
    border: 2px solid var(--text-muted);
    border-radius: 50%;
    background: var(--surface-2);
  }
  .port :global(.h.can::after) {
    transform: scale(1.5);
  }
  .port :global(.h.cannot::after) {
    opacity: 0.25;
  }
  .port:has(:global(.h.cannot)) .plabel {
    opacity: 0.45;
  }
</style>

<!--
  Inspector: the selected node of the flow editor. Its type and id, its problems (errors in
  --danger, warnings in --warning), the config form of its category (a lazy chunk per category,
  ./inspector/*Form.svelte) and its ports: each with its wires (Remove) and "Connect to…", a menu
  of the ports on other nodes that can take a wire from (or give one to) it, which is the keyboard
  way to wire. Enter on a node in the canvas moves focus here (`ed.inspectorFocus`).

  Props:
    ed: FlowEditorCtl
    node: FlowNode   the selected node
-->
<script lang="ts">
  import Trash2 from "lucide-svelte/icons/trash-2";
  import X from "lucide-svelte/icons/x";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import IconButton from "$lib/components/IconButton.svelte";
  import ActionMenu, { type ActionItem } from "../ActionMenu.svelte";
  import type { FlowNode } from "$lib/types";
  import { nodeLabel, nodeSummary, portLabel } from "./catalog";
  import { catalogInfo } from "./nodes";
  import type { FlowEditorCtl, Side } from "./editor.svelte";
  import { loadForm } from "./load";
  import { categoryOf, type Port } from "./ports";

  let { ed, node }: { ed: FlowEditorCtl; node: FlowNode } = $props();

  const uid = $props.id();
  let root: HTMLElement | undefined = $state();
  const cat = $derived(categoryOf(node.type));
  const ports = $derived(ed.portsFor(node));
  const issues = $derived(ed.issues.get(node.id));
  const byId = $derived(new Map(ed.graph.nodes.map((n) => [n.id, n])));

  // Enter on a node: focus the first control of the form (or the inspector itself).
  // svelte-ignore state_referenced_locally
  let seen = ed.inspectorFocus;
  $effect(() => {
    const n = ed.inspectorFocus;
    if (n === seen) return;
    seen = n;
    requestAnimationFrame(() => {
      const first = root?.querySelector<HTMLElement>(".form :is(input, select, button):not(:disabled)");
      (first ?? root)?.focus();
    });
  });

  const nameOf = (n: FlowNode | undefined) => (n ? nodeLabel(n.type) : "a missing node");
  const describe = (n: FlowNode) => {
    const s = nodeSummary(n, ed.lookups, (p) => ed.feeding(n.id, p));
    return s ? `${nodeLabel(n.type)}: ${s.length > 40 ? `${s.slice(0, 39)}…` : s}` : nodeLabel(n.type);
  };

  function wiresOf(p: Port, side: Side) {
    return ed.graph.edges.filter((e) =>
      side === "input" ? e.to.node === node.id && e.to.port === p.name : e.from.node === node.id && e.from.port === p.name,
    );
  }

  function connectItems(p: Port, side: Side): ActionItem[] {
    return ed.partners(node.id, p.name, side).map(({ node: other, port }) => ({
      id: `${other.id}/${port.name}`,
      label: `${describe(other)} › ${portLabel(other, port.name)}`,
      meta: port.type,
      onselect: () => {
        const here = { node: node.id, port: p.name };
        const there = { node: other.id, port: port.name };
        ed.wire(side === "output" ? here : there, side === "output" ? there : here);
      },
    }));
  }
</script>

{#snippet portRow(p: Port, side: Side)}
  {@const wires = wiresOf(p, side)}
  <li class="port" data-port={p.name}>
    <div class="phead">
      <span class="glyph {p.kind}" aria-hidden="true"></span>
      <span class="pname">{portLabel(node, p.name)}</span>
      <span class="ptype mono">{p.kind === "event" ? "event" : p.type}</span>
      {#if !ed.readOnly}
        <span class="connect">
          <ActionMenu
            size="sm"
            label="Connect to… ({portLabel(node, p.name)})"
            text="Connect to…"
            align="end"
            items={() => connectItems(p, side)}
            empty="No port on the canvas fits here."
          />
        </span>
      {/if}
    </div>
    {#if wires.length > 0}
      <ul class="wires">
        {#each wires as w (w.id)}
          {@const other = byId.get(side === "input" ? w.from.node : w.to.node)}
          {@const otherPort = side === "input" ? w.from.port : w.to.port}
          <li class="wire">
            <span class="dir" aria-hidden="true">{side === "input" ? "←" : "→"}</span>
            <span class="wname">{nameOf(other)} › {other ? portLabel(other, otherPort) : otherPort}</span>
            {#if !ed.readOnly}
              <IconButton
                size="sm"
                label={side === "input"
                  ? `Remove the wire from ${nameOf(other)} ${otherPort} into ${p.name}`
                  : `Remove the wire from ${p.name} to ${nameOf(other)} ${otherPort}`}
                tooltip="left"
                onclick={() => ed.disconnect(w.id)}
              >
                <X size={14} />
              </IconButton>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </li>
{/snippet}

<section class="inspector" aria-labelledby="{uid}-title" bind:this={root} tabindex="-1">
  <header class="head">
    <div class="titles">
      <h2 id="{uid}-title" class="title">{nodeLabel(node.type)}</h2>
      <span class="id mono" title="Node id">{node.id}</span>
    </div>
    {#if !ed.readOnly}
      <IconButton variant="danger" label="Delete this node" tooltip="left" onclick={() => ed.remove([node.id])}>
        <Trash2 size={16} />
      </IconButton>
    {/if}
  </header>
  <p class="desc">{catalogInfo(node.type)?.description}</p>

  {#if issues && (issues.errors.length > 0 || issues.warnings.length > 0)}
    <ul class="issues" aria-label="Problems">
      {#each issues.errors as e, i (i)}<li class="err">{e}</li>{/each}
      {#each issues.warnings as w, i (i)}<li class="warn"><TriangleAlert size={12} aria-hidden="true" />{w}</li>{/each}
    </ul>
  {/if}

  <div class="form">
    {#await loadForm(cat) then Form}
      <Form {ed} {node} />
    {:catch}
      <p class="err">Couldn't load this form.</p>
    {/await}
  </div>

  {#if ports.inputs.length > 0}
    <h3 class="section-label">Inputs</h3>
    <ul class="ports">
      {#each ports.inputs as p (p.name)}{@render portRow(p, "input")}{/each}
    </ul>
  {/if}
  {#if ports.outputs.length > 0}
    <h3 class="section-label">Outputs</h3>
    <ul class="ports">
      {#each ports.outputs as p (p.name)}{@render portRow(p, "output")}{/each}
    </ul>
  {/if}
</section>

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    min-height: 0;
    padding: var(--s-3);
    outline: none;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--s-2);
  }
  .titles {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    margin: 0;
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .id,
  .desc,
  .ptype {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .desc {
    margin: calc(-1 * var(--s-2)) 0 0;
  }
  .issues {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--fs-sm);
  }
  .err {
    margin: 0;
    color: var(--danger);
  }
  .warn {
    display: flex;
    align-items: baseline;
    gap: var(--s-1);
    color: var(--warning);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
  }
  h3 {
    margin: var(--s-2) 0 0;
    color: var(--text-muted);
  }
  .ports,
  .wires {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .ports {
    gap: var(--s-2);
  }
  .phead {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--control-h);
  }
  .glyph {
    flex: none;
    width: 10px;
    height: 10px;
  }
  .glyph.event {
    background: var(--accent);
    clip-path: polygon(15% 0, 100% 50%, 15% 100%);
  }
  .glyph.data {
    border: 2px solid var(--text-muted);
    border-radius: 50%;
  }
  .pname {
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .connect {
    margin-left: auto;
  }
  .wire {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding-left: calc(10px + var(--s-2));
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .wname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

<!--
  FlowEditor: the /flows/[id] page body. Header (back to Flows, name, armed switch, run and save
  status, Undo / Redo, Discard draft, Deploy), banners (changed or deleted on disk, read-only,
  problems that belong to no node), then the palette, the canvas and the inspector of the selected
  node. The canvas (@xyflow/svelte), the palette, the inspector and each of its forms, and the disk
  banner are lazy chunks.

  Status: "Running" (with the run's note in --warning, e.g. a paced Repeat), "Saving…" while a draft save is pending, "Saved" after; "Not deployed" (dot + text) while
  the draft differs from what runs. Deploy (Ctrl+Enter, also from the palette) flushes the draft and
  deploys; it is disabled with the reason while there are errors. Discard draft asks first.
  Ctrl+Z / Ctrl+Shift+Z undo and redo in this session (not while typing in a field). These keys are
  the defaults of the editor's commands (key scope "flowEditor"), run by the shell's keydown.

  A read-only flow (from a newer app version) can be looked at and duplicated, not changed.

  Props:
    id: string   the flow id (remount to switch flows)
-->
<script lang="ts">
  import { onMount } from "svelte";
  import ArrowLeft from "lucide-svelte/icons/arrow-left";
  import Copy from "lucide-svelte/icons/copy";
  import Redo2 from "lucide-svelte/icons/redo-2";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Undo2 from "lucide-svelte/icons/undo-2";
  import { goto } from "$app/navigation";
  import { registerCommands } from "$lib/commands/registry.svelte";
  import { flowEditorCommands } from "./commands";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { FLOW_NAME_MAX } from "$lib/types";
  import { nameError } from "../names";
  import { FlowEditorCtl } from "./editor.svelte";
  import { loadCanvas, loadInspector, loadPalette } from "./load";

  let { id }: { id: string } = $props();

  // svelte-ignore state_referenced_locally
  const ed = new FlowEditorCtl(id);
  $effect(() => ed.connect());

  const uid = $props.id();
  const info = $derived(ed.info);
  const failed = $derived(ed.flows.failed.get(id));
  /** A note on the current run (`flow-state`'s message, else a debug warning): a paced Repeat. */
  const runNote = $derived(ed.flows.notes.get(id) ?? ed.debug.warnings.values().next().value ?? null);

  // ---- Name ----
  let nameDraft = $state("");
  let nameErr = $state<string | null>(null);
  let nameSeen = "";
  $effect(() => {
    const n = info?.name ?? "";
    if (n !== nameSeen) {
      nameSeen = n;
      nameDraft = n;
    }
  });

  async function commitName() {
    const next = nameDraft.trim();
    if (!info || next === info.name) {
      nameDraft = info?.name ?? next;
      nameErr = null;
      return;
    }
    nameErr = nameError(next);
    if (nameErr) return;
    if (!(await ed.rename(next))) nameDraft = info.name;
  }

  function onNameKey(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key === "Enter") {
      e.preventDefault();
      void commitName();
    } else if (e.key === "Escape") {
      nameDraft = info?.name ?? "";
      nameErr = null;
      e.currentTarget.blur();
    }
  }

  // ---- Discard / duplicate ----
  type ConfirmT = typeof import("$lib/components/ConfirmDialog.svelte").default;
  let Confirm = $state.raw<ConfirmT | null>(null);
  let confirmOpen = $state(false);
  let discarding = $state(false);

  async function askDiscard() {
    Confirm ??= (await import("$lib/components/ConfirmDialog.svelte")).default;
    confirmOpen = true;
  }

  async function confirmDiscard() {
    discarding = true;
    await ed.discard();
    discarding = false;
    confirmOpen = false;
  }

  let dupBusy = $state(false);
  async function duplicate() {
    dupBusy = true;
    const copy = await ed.duplicate();
    dupBusy = false;
    if (copy) await goto(`/flows/${copy}`);
  }

  // ---- Keys and commands ----

  // The inspector chunk is fetched once the editor is idle, so the first selection (often the
  // start of a drag) only has to render it.
  onMount(() => {
    const idle = window.requestIdleCallback ?? ((f: () => void) => setTimeout(f, 300));
    const cancel = window.cancelIdleCallback ?? clearTimeout;
    const h = idle(() => void loadInspector().catch(() => {}));
    return () => cancel(h as number);
  });

  onMount(() =>
    registerCommands(
      flowEditorCommands({
        deploy: () => ed.deploy(),
        deployBlocked: () => ed.deployBlocked,
        undo: ed.undo,
        redo: ed.redo,
        canUndo: () => ed.canUndo,
        canRedo: () => ed.canRedo,
        selectAll: ed.selectAll,
        fit: () => ed.canvas?.fit(),
      }),
    ),
  );

  const savedTitle = $derived(
    ed.saveState.kind === "saved" && ed.saveState.at ? `Saved ${new Date(ed.saveState.at).toLocaleString()}` : undefined,
  );
</script>

<svelte:head><title>{info ? `${info.name} · ` : ""}Flows · Kinetra</title></svelte:head>

<div class="editor" class:read-only={ed.readOnly}>
  <header class="bar">
    <Button variant="ghost" size="sm" href="/flows">
      {#snippet icon()}<ArrowLeft size={14} />{/snippet}
      Flows
    </Button>
    <span class="sep" aria-hidden="true"></span>

    {#if ed.loaded && info}
      <span class="name vf vf-within">
        <input
          class="name-input"
          type="text"
          aria-label="Flow name"
          autocomplete="off"
          maxlength={FLOW_NAME_MAX + 20}
          disabled={ed.readOnly}
          bind:value={nameDraft}
          aria-invalid={nameErr ? "true" : undefined}
          aria-describedby={nameErr ? `${uid}-name-err` : undefined}
          onkeydown={onNameKey}
          onblur={() => void commitName()}
        />
      </span>
      {#if nameErr}<span class="bad small" id="{uid}-name-err">{nameErr}</span>{/if}
      <Switch label="Armed" checked={info.armed} disabled={ed.readOnly} onchange={(on) => void ed.setArmed(on)} />

      <span class="status small" role="status">
        {#if info.running}
          <span class="running">Running</span>
          {#if runNote}<span class="warn" title={runNote}>{runNote}</span>{/if}
        {:else if failed}
          <span class="bad" title={failed.message ?? undefined}>Failed{failed.message ? `: ${failed.message}` : ""}</span>
        {/if}
        {#if ed.readOnly}
          <span class="muted">Read-only</span>
        {:else if ed.disk === "removed"}
          <span class="bad">Deleted on disk</span>
        {:else if ed.saveState.kind === "failed"}
          <span class="bad">Not saved: {ed.saveState.message}</span>
          <Button variant="ghost" size="sm" onclick={() => void ed.flush()}>
            {#snippet icon()}<RotateCw size={12} />{/snippet}
            Retry
          </Button>
        {:else if ed.saveState.kind === "saving" || ed.saveState.kind === "pending"}
          <span class="muted">Saving…</span>
        {:else}
          <span class="muted" title={savedTitle}>Saved</span>
        {/if}
        {#if ed.notDeployed && !ed.readOnly}
          <span class="undeployed"><span class="dot" aria-hidden="true"></span>Not deployed</span>
        {/if}
      </span>

      <span class="actions">
        {#if ed.readOnly}
          <Button size="sm" loading={dupBusy} onclick={duplicate}>
            {#snippet icon()}<Copy size={14} />{/snippet}
            Duplicate
          </Button>
        {:else}
          <IconButton size="sm" label="Undo (Ctrl+Z)" disabled={!ed.canUndo} onclick={ed.undo}><Undo2 size={14} /></IconButton>
          <IconButton size="sm" label="Redo (Ctrl+Shift+Z)" disabled={!ed.canRedo} onclick={ed.redo}><Redo2 size={14} /></IconButton>
          <Button size="sm" variant="ghost" disabled={!info.draft && !ed.notDeployed} onclick={askDiscard}>Discard draft</Button>
          {#if ed.deployBlocked && ed.notDeployed}
            <span class="bad small" id="{uid}-why">{ed.deployBlocked}</span>
          {/if}
          <Button
            size="sm"
            variant="primary"
            disabled={!!ed.deployBlocked}
            loading={ed.deploying}
            title="Deploy (Ctrl+Enter)"
            aria-describedby={ed.deployBlocked && ed.notDeployed ? `${uid}-why` : undefined}
            onclick={() => void ed.deploy()}
          >
            Deploy
          </Button>
        {/if}
      </span>
    {/if}
  </header>

  {#if ed.disk}
    {#await import("$lib/components/templates/editor/DiskBanner.svelte") then { default: DiskBanner }}
    <DiskBanner
      what="flow"
      state={ed.disk}
      busy={ed.diskBusy}
      onreload={ed.diskReload}
      onkeep={ed.diskKeep}
      onclose={() => void goto("/flows")}
    />
    {/await}
  {/if}
  {#if ed.readOnly}
    <p class="note">Saved by a newer version of Kinetra, so it's read-only. Duplicate it to edit a copy.</p>
  {/if}
  {#if ed.flowIssues.length > 0}
    <ul class="note issues" aria-label="Flow problems">
      {#each ed.flowIssues as i, k (k)}<li class:bad={ed.check.errors.includes(i)} class:warn={!ed.check.errors.includes(i)}>{i.message}</li>{/each}
    </ul>
  {/if}

  {#if ed.loadError}
    <div class="load-error">
      <p class="bad">Couldn't open this flow: {ed.loadError}</p>
      <Button href="/flows">Back to flows</Button>
    </div>
  {:else if ed.loaded}
    <div class="body">
      {#if !ed.readOnly}
        <aside class="left" aria-label="Nodes">
          {#await loadPalette() then Palette}<Palette {ed} />{/await}
        </aside>
      {/if}
      <div class="center">
        {#await loadCanvas()}
          <span class="sr-only" role="status">Loading the canvas</span>
        {:then Canvas}
          <Canvas {ed} />
        {:catch e}
          <p class="bad pad">Couldn't load the canvas: {e instanceof Error ? e.message : String(e)}</p>
        {/await}
      </div>
      <!-- Always laid out, so selecting a node never resizes the canvas. -->
      <aside class="right" aria-label="Inspector">
        {#if ed.selected}
          {#await loadInspector() then Inspector}<Inspector {ed} node={ed.selected} />{/await}
        {:else}
          <div class="overview">
            <h2 class="section-label">This flow</h2>
            <p class="counts">
              {ed.graph.nodes.length} {ed.graph.nodes.length === 1 ? "node" : "nodes"} · {ed.graph.edges.length}
              {ed.graph.edges.length === 1 ? "wire" : "wires"}
            </p>
            {#if !ed.readOnly}
              <p class="muted">Select a node to edit it. To add one, drag it from the list or press Enter on it.</p>
            {/if}
            <h2 class="section-label">Keyboard</h2>
            <dl class="keys">
              <dt class="mono">Tab</dt><dd>Next node</dd>
              <dt class="mono">Enter</dt><dd>Edit the node</dd>
              {#if !ed.readOnly}
                <dt class="mono">Arrows</dt><dd>Move 8 px (Shift: 80 px)</dd>
                <dt class="mono">Delete</dt><dd>Remove the selection</dd>
                <dt class="mono">Ctrl+Z</dt><dd>Undo (Ctrl+Shift+Z: redo)</dd>
                <dt class="mono">Ctrl+A</dt><dd>Select all</dd>
                <dt class="mono">Ctrl+Enter</dt><dd>Deploy</dd>
              {/if}
              <dt class="mono">Esc</dt><dd>Clear the selection</dd>
            </dl>
          </div>
        {/if}
      </aside>
    </div>
  {/if}
</div>

{#if Confirm && confirmOpen}
  <Confirm
    title="Discard the draft?"
    message="Your undeployed changes to “{info?.name ?? ''}” will be lost."
    confirmLabel="Discard draft"
    danger
    busy={discarding}
    onconfirm={confirmDiscard}
    oncancel={() => (confirmOpen = false)}
  />
{/if}

<style>
  .editor {
    display: flex;
    flex-direction: column;
    height: calc(100dvh - var(--topbar-h));
    min-width: 0;
  }
  .bar {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    min-height: calc(var(--row-h) + var(--s-4));
    padding: var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--line);
  }
  .name {
    --vf-radius: var(--r-control);
    display: inline-flex;
    min-width: 12rem;
    max-width: 24rem;
    flex: 0 1 20rem;
    border-radius: var(--r-control);
  }
  .name-input {
    width: 100%;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text);
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .name-input:hover:not(:disabled),
  .name-input:focus {
    background: var(--surface-2);
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: var(--s-3);
  }
  .small {
    font-size: var(--fs-sm);
  }
  .muted {
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger);
  }
  .warn {
    color: var(--warning);
  }
  .running {
    color: var(--accent-text);
  }
  .undeployed {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--text);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--accent);
  }
  .actions {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    margin-left: auto;
  }
  .note {
    flex: none;
    margin: 0;
    padding: var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface-2);
    font-size: var(--fs-sm);
  }
  .issues {
    display: flex;
    flex-direction: column;
    gap: 2px;
    list-style: none;
  }
  .load-error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-3);
    padding: var(--s-6);
  }
  .body {
    display: grid;
    flex: 1;
    grid-template-columns: 240px minmax(0, 1fr) 320px;
    min-height: 0;
  }
  .read-only .body {
    grid-template-columns: minmax(0, 1fr) 320px;
  }
  .overview {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3);
    font-size: var(--fs-sm);
  }
  .overview h2 {
    margin: var(--s-2) 0 0;
    color: var(--text-muted);
  }
  .overview p {
    margin: 0;
  }
  .keys {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--s-1) var(--s-3);
    margin: 0;
  }
  .keys dt {
    color: var(--text);
    font-size: var(--fs-xs);
  }
  .keys dd {
    margin: 0;
    color: var(--text-muted);
  }
  .left,
  .right {
    min-height: 0;
    background: var(--surface-2);
  }
  .left {
    border-right: 1px solid var(--line);
  }
  .right {
    border-left: 1px solid var(--line);
    overflow: auto;
  }
  .center {
    position: relative;
    min-width: 0;
    min-height: 0;
  }
  .pad {
    padding: var(--s-4);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  @media (max-width: 1100px) {
    .body {
      grid-template-columns: 200px minmax(0, 1fr) 280px;
    }
    .read-only .body {
      grid-template-columns: minmax(0, 1fr) 280px;
    }
  }
</style>

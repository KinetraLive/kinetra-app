<!--
  ListField: a `list` field inside FieldInputs (lazy chunk, loaded only when a list field shows).
  One line:
    own rows     "6 items · Edit" (the count in mono): Edit opens the drawer on the rows
    linked       "Linked to: Event schedule · Edit source": opens the drawer on the data table
                 (a linked schedule has no Edit here; it is edited on the Sources page)
  The drawer (ListDrawer, a lazy chunk with the spreadsheet editor) mounts only while open.

  Props:
    id: string                  base id (the summary is `{id}-count`)
    label: string               the field's label
    columns: SourceColumn[]
    value: string               the list value (JSON)
    linked?: LinkedList | null  the source the list is linked to
    disabled?: boolean
    onchange: (value: string) => void
-->
<script module lang="ts">
  let drawerChunk: Promise<typeof import("./ListDrawer.svelte")> | null = null;
  const loadDrawer = () => (drawerChunk ??= import("./ListDrawer.svelte"));
</script>

<script lang="ts">
  import type { SourceColumn } from "$lib/types";
  import type { LinkedList, ListDrawerTarget } from "./drawer";
  import { rowCount } from "./list";

  let {
    id,
    label,
    columns,
    value,
    linked = null,
    disabled = false,
    onchange,
  }: {
    id: string;
    label: string;
    columns: readonly SourceColumn[];
    value: string;
    linked?: LinkedList | null;
    disabled?: boolean;
    onchange: (value: string) => void;
  } = $props();

  let target = $state.raw<ListDrawerTarget | null>(null);
  let button: HTMLButtonElement | undefined = $state();
  const count = $derived(linked ? 0 : rowCount(value));
  const editable = $derived(!linked || linked.kind === "text");

  function open() {
    void loadDrawer(); // started at once; the {#await} below picks it up
    target = linked
      ? { kind: "source", label, sourceId: linked.sourceId, name: linked.name, usedBy: linked.usedBy, onsaved: linked.onsaved }
      : { kind: "list", label, columns, value, onchange };
  }
</script>

<span class="summary" id="{id}-count" title={linked?.detail}>
  {#if linked}
    <span class="text">Linked to: <span class="name">{linked.name}</span></span>
  {:else}
    <span class="text"><span class="n">{count}</span> {count === 1 ? "item" : "items"}</span>
  {/if}
  {#if editable}
    <span class="sep" aria-hidden="true">·</span>
    <button
      type="button"
      bind:this={button}
      class="edit vf"
      aria-haspopup="dialog"
      aria-label="{linked ? `Edit data table ${linked.name}` : `Edit items of ${label}`}"
      disabled={disabled && !linked}
      onclick={open}
    >
      {linked ? "Edit source" : "Edit"}
    </button>
  {/if}
</span>

{#if target}
  {#await loadDrawer() then { default: ListDrawer }}
    <ListDrawer {target} opener={button} onclose={() => (target = null)} />
  {/await}
{/if}

<style>
  .summary {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--s-1);
    min-width: 0;
    min-height: var(--control-h);
  }
  .text {
    min-width: 0;
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    color: var(--text);
  }
  .text .n {
    color: var(--text);
  }
  .sep {
    color: var(--text-muted);
  }
  .edit {
    flex: none;
    height: var(--control-h-sm);
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    color: var(--accent-text);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .edit:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .edit:disabled {
    opacity: 0.6;
  }
</style>

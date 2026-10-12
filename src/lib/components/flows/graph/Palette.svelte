<!--
  Palette: the node types, searchable, in four groups (Triggers, Data, Logic, Actions) with a short
  description each (lazy chunk). Add a node by dragging it onto the canvas, or with Enter / click
  (it lands near the middle of the view and is selected). In the search box, ArrowDown moves to the
  first node and Enter adds the first match; in the list, ArrowUp / ArrowDown move.

  Props: PaletteProps (./load.ts).
-->
<script lang="ts">
  import Search from "lucide-svelte/icons/search";
  import type { NodeType } from "$lib/types";
  import type { PaletteProps } from "./load";
  import { PALETTE_MIME, paletteGroups } from "./palette";

  let { ed }: PaletteProps = $props();

  const uid = $props.id();
  let query = $state("");
  let list: HTMLDivElement | undefined = $state();
  const groups = $derived(paletteGroups(query));
  const first = $derived(groups[0]?.items[0]?.type ?? null);

  function add(type: NodeType) {
    const id = ed.addNode(type);
    if (id) requestAnimationFrame(() => ed.canvas?.focusNode(id));
  }

  const items = () => [...(list?.querySelectorAll<HTMLButtonElement>("button[data-type]") ?? [])];

  function onSearchKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      items()[0]?.focus();
    } else if (e.key === "Enter" && first) {
      e.preventDefault();
      add(first);
    } else if (e.key === "Escape" && query) {
      e.preventDefault();
      e.stopPropagation();
      query = "";
    }
  }

  function onListKey(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const all = items();
    const i = all.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    e.preventDefault();
    if (e.key === "ArrowUp" && i === 0) document.getElementById(`${uid}-q`)?.focus();
    else all[Math.max(0, Math.min(all.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))]?.focus();
  }

  function onDragStart(e: DragEvent, type: NodeType) {
    if (!e.dataTransfer) return;
    e.dataTransfer.setData(PALETTE_MIME, type);
    e.dataTransfer.effectAllowed = "copy";
  }
</script>

<section class="palette" aria-label="Add a node">
  <div class="search vf vf-within">
    <Search size={14} aria-hidden="true" />
    <input
      id="{uid}-q"
      type="search"
      placeholder="Search nodes"
      aria-label="Search nodes"
      autocomplete="off"
      spellcheck="false"
      bind:value={query}
      onkeydown={onSearchKey}
      disabled={ed.readOnly}
    />
  </div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="groups" bind:this={list} onkeydown={onListKey}>
    {#each groups as g (g.id)}
      <div class="group" role="group" aria-labelledby="{uid}-{g.id}">
        <h3 class="section-label" id="{uid}-{g.id}">{g.label}</h3>
        {#each g.items as n (n.type)}
          <button
            type="button"
            class="item vf"
            data-type={n.type}
            draggable={!ed.readOnly}
            disabled={ed.readOnly}
            ondragstart={(e) => onDragStart(e, n.type)}
            onclick={() => add(n.type)}
          >
            <span class="label">{n.label}</span>
            <span class="desc">{n.description}</span>
          </button>
        {/each}
      </div>
    {:else}
      <p class="none">No node matches “{query}”.</p>
    {/each}
  </div>
</section>

<style>
  .palette {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .search {
    --vf-radius: var(--r-control);
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--s-2);
    height: var(--control-h);
    margin: var(--s-2);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
  }
  .groups {
    flex: 1;
    min-height: 0;
    padding: 0 var(--s-2) var(--s-3);
    overflow: auto;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: var(--s-3);
  }
  .group h3 {
    margin: 0;
    padding: var(--s-1) var(--s-2);
    color: var(--text-muted);
  }
  .item {
    --vf-radius: var(--r-control);
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: var(--s-1) var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: grab;
  }
  .item:hover:not(:disabled) {
    background: var(--surface-3);
  }
  .item:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .label {
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .desc,
  .none {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .none {
    margin: var(--s-2);
  }
</style>

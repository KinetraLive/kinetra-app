<!--
  SourceList: the Sources page's left column, a listbox of the project's sources. Each option shows
  the name, the kind ("Data table" / "Schedule" / "CSV link" / "API"), the row or item count and how many screens use it;
  an API source also its fetch status (TallyDot + Live / Waiting / Error / Paused). The selected one
  carries the viewfinder brackets.

  Keyboard: one tab stop (the selected option); ArrowUp/ArrowDown/Home/End move the selection.

  Props:
    sources: SourceSummary[]     sorted by the caller
    selectedId: string | null
    onselect: (id: string) => void
-->
<script lang="ts">
  import { tick } from "svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import type { SourceSummary } from "$lib/types";
  import { apiShort } from "./api/status";
  import { itemsLabel, kindLabel, rowsLabel, usedByLabel } from "./model";

  let {
    sources,
    selectedId,
    onselect,
  }: {
    sources: readonly SourceSummary[];
    selectedId: string | null;
    onselect: (id: string) => void;
  } = $props();

  let root: HTMLDivElement | undefined = $state();

  async function onkeydown(e: KeyboardEvent) {
    if (sources.length === 0) return;
    const i = Math.max(0, sources.findIndex((s) => s.id === selectedId));
    let next: number;
    switch (e.key) {
      case "ArrowUp":
        next = i - 1;
        break;
      case "ArrowDown":
        next = i + 1;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = sources.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    next = Math.min(sources.length - 1, Math.max(0, next));
    onselect(sources[next].id);
    await tick();
    root?.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')?.focus();
  }
</script>

<div class="list" role="listbox" aria-label="Sources" tabindex="-1" bind:this={root} {onkeydown}>
  {#each sources as s, i (s.id)}
    {@const selected = s.id === selectedId}
    <div
      class="item vf"
      role="option"
      aria-selected={selected}
      data-source={s.id}
      tabindex={selected || (selectedId === null && i === 0) ? 0 : -1}
      onclick={() => onselect(s.id)}
      onkeydown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onselect(s.id);
        }
      }}
    >
      <span class="top">
        <span class="name">{s.name}</span>
        {#if s.kind === "api"}
          {@const st = apiShort(s.api)}
          <span class="api" data-api-state={s.api?.state ?? "none"}><TallyDot state={st.tally} />{st.text}</span>
        {/if}
      </span>
      <span class="meta">
        <span class="kind">{kindLabel(s)}</span>
        <span class="dot" aria-hidden="true">·</span>
        <span>{s.kind === "schedule" ? itemsLabel(s.rowCount) : s.kind === "poll" ? `${s.rowCount} ${s.rowCount === 1 ? "answer" : "answers"}` : rowsLabel(s.rowCount)}</span>
        <span class="dot" aria-hidden="true">·</span>
        <span>{usedByLabel(s.usedBy.length)}</span>
      </span>
    </div>
  {/each}
</div>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding: var(--s-2);
  }
  .list:focus-visible {
    outline: none;
  }
  .item {
    --vf-radius: var(--r-control);
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    min-height: calc(var(--row-h) + var(--s-4));
    padding: var(--s-1) var(--s-3);
    border-radius: var(--r-control);
    color: var(--text);
    cursor: pointer;
    user-select: none;
  }
  .item:hover {
    background: var(--surface-2);
  }
  .item[aria-selected="true"] {
    background: var(--selected-bg);
  }
  .name {
    overflow: hidden;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .top .name {
    flex: 1;
    min-width: 0;
  }
  .api {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
</style>

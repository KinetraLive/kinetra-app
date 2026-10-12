<!--
  ScreenList: the single view's screen picker, a listbox (tally + name + mono on-air count).

  Keyboard: Tab reaches the selected screen; ArrowUp/ArrowDown/Home/End select and move focus.

  Props:
    screens: LiveScreenView[]   visible screens, in order
    selectedId: string | null
    onselect: (id: string) => void
-->
<script lang="ts">
  import { tick } from "svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import type { LiveScreenView } from "$lib/types";
  import { onAirCount } from "./control.svelte";

  let {
    screens,
    selectedId,
    onselect,
  }: {
    screens: readonly LiveScreenView[];
    selectedId: string | null;
    onselect: (id: string) => void;
  } = $props();

  let list: HTMLUListElement | undefined = $state();

  async function onkeydown(e: KeyboardEvent) {
    const i = screens.findIndex((s) => s.screen.id === selectedId);
    let next = -1;
    if (e.key === "ArrowDown") next = Math.min(screens.length - 1, i + 1);
    else if (e.key === "ArrowUp") next = Math.max(0, i - 1);
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = screens.length - 1;
    if (next < 0 || e.altKey || e.ctrlKey || e.metaKey) return;
    e.preventDefault();
    const id = screens[next].screen.id;
    onselect(id);
    await tick();
    list?.querySelector<HTMLElement>(`[data-screen="${CSS.escape(id)}"]`)?.focus();
  }
</script>

<ul class="list" role="listbox" aria-label="Screens" bind:this={list}>
  {#each screens as view (view.screen.id)}
    {@const id = view.screen.id}
    {@const n = onAirCount(view)}
    {@const sel = id === selectedId}
    <li
      class="opt vf"
      role="option"
      aria-selected={sel}
      tabindex={sel ? 0 : -1}
      data-screen={id}
      onclick={() => onselect(id)}
      {onkeydown}
    >
      <TallyDot state={n > 0 ? "program" : "idle"} label={n > 0 ? "On air" : "Off air"} />
      <span class="name">{view.screen.name}</span>
      {#if n > 0}<span class="count">{n}</span>{/if}
    </li>
  {/each}
</ul>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: var(--s-2);
    list-style: none;
  }
  .opt {
    --vf-radius: var(--r-control);
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    cursor: pointer;
  }
  .opt:hover {
    background: var(--surface-2);
  }
  .opt[aria-selected="true"] {
    background: var(--selected-bg);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
</style>

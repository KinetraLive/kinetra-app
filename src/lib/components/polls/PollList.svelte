<!--
  PollList: the Polls page's left column: "Your polls" (the account's, from the server) in one
  listbox. Each option shows the question, its status, votes and code.

  Keyboard: one tab stop (the selected option); ArrowUp/ArrowDown/Home/End move.

  Props:
    polls: PollData[]           newest first
    selection: Selection
    onselect: (s: Selection) => void
    locked?: Snippet            shown in "Your polls" when the account can't use polls (the lock)
-->
<script lang="ts">
  import { tick, type Snippet } from "svelte";
  import type { PollData } from "$lib/types";
  import { STATE_LABEL, stateOf, votesText, type PollState } from "./model";
  import type { Selection } from "./polls.svelte";

  let {
    polls,
    selection,
    onselect,
    locked,
  }: {
    polls: readonly PollData[];
    selection: Selection;
    onselect: (s: Selection) => void;
    locked?: Snippet;
  } = $props();

  type Entry = { sel: { kind: "poll"; id: string }; title: string; state: PollState; votes: number; code: string };

  const pollEntries = $derived<Entry[]>(
    polls.map((p) => ({
      sel: { kind: "poll", id: p.id },
      title: p.question,
      state: stateOf(p),
      votes: p.totalVotes,
      code: p.code,
    })),
  );
  const all = $derived(pollEntries);
  const isSel = (e: Entry) => selection !== null && selection.kind === e.sel.kind && "id" in selection && selection.id === e.sel.id;

  let root: HTMLDivElement | undefined = $state();

  async function onkeydown(e: KeyboardEvent) {
    if (all.length === 0) return;
    const i = Math.max(0, all.findIndex(isSel));
    const next = { ArrowUp: i - 1, ArrowDown: i + 1, Home: 0, End: all.length - 1 }[e.key];
    if (next === undefined) return;
    e.preventDefault();
    onselect(all[Math.min(all.length - 1, Math.max(0, next))].sel);
    await tick();
    root?.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')?.focus();
  }

  const firstFocusable = $derived(all.some(isSel) ? null : (all[0] ?? null));
</script>

{#snippet item(e: Entry)}
  {@const selected = isSel(e)}
  <div
    class="item vf"
    role="option"
    aria-selected={selected}
    data-poll={e.sel.id}
    tabindex={selected || e === firstFocusable ? 0 : -1}
    onclick={() => onselect(e.sel)}
    onkeydown={(ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        onselect(e.sel);
      }
    }}
  >
    <span class="name">{e.title || "Untitled"}</span>
    <span class="meta">
      <span class="state" data-state={e.state}>{STATE_LABEL[e.state]}</span>
      <span class="dot" aria-hidden="true">·</span>
      <span class="tnum">{votesText(e.votes)}</span>
      {#if e.code}
        <span class="dot" aria-hidden="true">·</span>
        <span class="mono">{e.code}</span>
      {/if}
    </span>
  </div>
{/snippet}

<div class="list" role="listbox" aria-label="Polls" tabindex="-1" bind:this={root} {onkeydown}>
  <div class="group" role="presentation">
    <span class="section-label" role="presentation">Your polls</span>
    {#each pollEntries as e (e.sel.id)}
      {@render item(e)}
    {/each}
    {@render locked?.()}
  </div>
</div>

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-2);
  }
  .list:focus-visible {
    outline: none;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  .section-label {
    padding: var(--s-1) var(--s-3) 0;
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
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .state[data-state="open"],
  .state[data-state="full"] {
    color: var(--accent-text);
  }
</style>

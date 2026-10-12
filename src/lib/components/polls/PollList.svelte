<!--
  PollList: the Polls page's left column. Two groups in one listbox: "Your polls" (the account's,
  from the server) and "Test polls" (this project's, simulated votes). Each option shows the
  question (or the test poll's name), its status and votes; server polls also their code.

  Keyboard: one tab stop (the selected option); ArrowUp/ArrowDown/Home/End move across both groups.

  Props:
    polls: PollData[]           newest first
    tests: SourceSummary[]      poll sources with test data
    selection: Selection
    onselect: (s: Selection) => void
    locked?: Snippet            shown in "Your polls" when the account can't use polls (the lock)
-->
<script lang="ts">
  import { tick, type Snippet } from "svelte";
  import type { PollData, SourceSummary } from "$lib/types";
  import { STATE_LABEL, stateOf, votesText, type PollState } from "./model";
  import type { Selection } from "./polls.svelte";

  let {
    polls,
    tests,
    selection,
    onselect,
    locked,
  }: {
    polls: readonly PollData[];
    tests: readonly SourceSummary[];
    selection: Selection;
    onselect: (s: Selection) => void;
    locked?: Snippet;
  } = $props();

  type Entry = { sel: { kind: "poll" | "test"; id: string }; title: string; state: PollState; votes: number; code: string };

  const pollEntries = $derived<Entry[]>(
    polls.map((p) => ({
      sel: { kind: "poll", id: p.id },
      title: p.question,
      state: stateOf(p),
      votes: p.totalVotes,
      code: p.code,
    })),
  );
  const testEntries = $derived<Entry[]>(
    tests.map((t) => ({
      sel: { kind: "test", id: t.id },
      title: t.name,
      state: (t.poll?.state as PollState | null) ?? "draft",
      votes: t.poll?.totalVotes ?? 0,
      code: "",
    })),
  );
  const all = $derived([...pollEntries, ...testEntries]);
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
  {#if testEntries.length}
    <div class="group" role="presentation">
      <span class="section-label" role="presentation">Test polls</span>
      {#each testEntries as e (e.sel.id)}
        {@render item(e)}
      {/each}
    </div>
  {/if}
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

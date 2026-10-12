<!--
  PollDetail: one of the account's polls. Status, Open / Close, live results, where to vote (code,
  link, QR), the editor, Add to screen (the Poll graphic, bound), and the rest: Reset votes,
  Duplicate, Use in a source, Delete. Opening and
  editing need Pro (the core refuses otherwise; here they say why); close, reset and delete always
  work, so a poll is never stranded.

  Props:
    polls: PollsState
    poll: PollData
    pro: boolean     the account has polls (display only)
-->
<script lang="ts">
  import { goto } from "$app/navigation";
  import Button from "$lib/components/Button.svelte";
  import type { PollData } from "$lib/types";
  import { deletesIn, draftOf, openBlocked, STATE_LABEL, stateOf, textLocked, updateInput, votesText } from "./model";
  import type { PollsState } from "./polls.svelte";
  import AddToScreen from "./AddToScreen.svelte";
  import PollEditor from "./PollEditor.svelte";
  import PollLink from "./PollLink.svelte";
  import PollResults from "./PollResults.svelte";

  let { polls, poll, pro }: { polls: PollsState; poll: PollData; pro: boolean } = $props();

  const st = $derived(stateOf(poll));
  const blocked = $derived(openBlocked(polls.quota, poll));
  const conn = $derived(polls.conn[poll.id]);
  const maxOptions = $derived(polls.quota?.limits?.optionsPerPoll ?? 8);
  const locked = $derived(pro ? textLocked(poll) : "Editing polls is part of Kinetra Pro.");
  const draft = $derived(draftOf(poll));
  let now = $state(Date.now());
  $effect(() => {
    // "Deleted in 11 h" needs no more than a minute's precision; only while a poll is closed.
    if (poll.status !== "closed") return;
    const t = setInterval(() => (now = Date.now()), 60_000);
    return () => clearInterval(t);
  });
  const leaving = $derived(deletesIn(poll, now));

  type Ask = "reset" | "delete" | null;
  let ask = $state<Ask>(null);

  async function useInSource() {
    const id = await polls.useInSource(poll);
    if (id) void goto(`/sources?source=${encodeURIComponent(id)}`);
  }
</script>

<article class="detail" aria-label="Poll">
  <header class="head">
    <div class="title">
      <h2>{poll.question}</h2>
      <span class="meta">
        <span class="state" data-state={st}>{STATE_LABEL[st]}</span>
        {#if st === "full"}<span>Reached {votesText(poll.maxVotes ?? poll.totalVotes)}: it stays on screen but takes no more votes.</span>{/if}
        {#if leaving}<span>{leaving}</span>{/if}
      </span>
    </div>
    <div class="primary">
      {#if poll.status === "open"}
        <Button variant="primary" loading={polls.busy} onclick={() => void polls.close(poll.id)}>Close poll</Button>
      {:else}
        <Button
          variant="primary"
          loading={polls.busy}
          disabled={!pro || blocked !== null}
          title={!pro ? "Opening polls is part of Kinetra Pro" : (blocked ?? undefined)}
          onclick={() => void polls.open(poll.id)}
        >
          {poll.status === "draft" ? "Open poll" : "Open again"}
        </Button>
      {/if}
    </div>
  </header>
  {#if !pro}
    <p class="note">Opening and editing polls is part of Kinetra Pro. You can still close, reset and delete this poll.</p>
  {:else if blocked && poll.status !== "open"}
    <p class="note">{blocked}</p>
  {/if}

  <div class="body">
    <section class="panel">
      <PollResults {poll} {conn} />
    </section>
    <section class="panel">
      <PollLink {poll} />
    </section>
    <section class="panel">
      <h3 class="section-label">Edit</h3>
      <PollEditor
        initial={draft}
        {maxOptions}
        textLocked={locked}
        durationLocked={!pro || poll.status === "open"}
        submitLabel="Save changes"
        busy={polls.busy}
        dirtyOnly
        onsubmit={(d) => {
          const input = updateInput(d, poll);
          if (input) void polls.update(poll.id, input);
        }}
      />
    </section>
    <section class="panel more">
      <AddToScreen source={() => polls.useInSource(poll)} />
      <Button onclick={useInSource}>Use in a source</Button>
      <Button disabled={!pro} onclick={() => void polls.duplicate(poll)}>Duplicate</Button>
      <Button disabled={poll.totalVotes === 0} onclick={() => (ask = "reset")}>Reset votes</Button>
      <Button variant="danger" onclick={() => (ask = "delete")}>Delete</Button>
    </section>
  </div>
</article>

{#if ask}
  {#await import("$lib/components/ConfirmDialog.svelte") then { default: ConfirmDialog }}
    {#if ask === "reset"}
      <ConfirmDialog
        title="Reset the votes?"
        message={poll.totalVotes === 1 ? "The vote is deleted and everyone can vote again." : `All ${votesText(poll.totalVotes)} are deleted and everyone can vote again.`}
        confirmLabel="Reset votes"
        danger
        onconfirm={() => {
          ask = null;
          void polls.reset(poll.id);
        }}
        oncancel={() => (ask = null)}
      />
    {:else}
      <ConfirmDialog
        title="Delete this poll?"
        message="Poll deleted. Graphics on screen will keep showing the final results."
        confirmLabel="Delete poll"
        danger
        onconfirm={() => {
          ask = null;
          void polls.remove(poll.id);
        }}
        oncancel={() => (ask = null)}
      />
    {/if}
  {/await}
{/if}

<style>
  .detail {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: auto;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--s-3);
    padding: var(--s-5) var(--s-6) var(--s-4);
  }
  .title {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-xl);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .state[data-state="open"],
  .state[data-state="full"] {
    color: var(--accent-text);
    font-weight: 600;
  }
  .note {
    margin: 0 var(--s-6) var(--s-3);
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .body {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 22rem), 1fr));
    gap: var(--s-4);
    padding: 0 var(--s-6) var(--s-6);
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  h3 {
    margin: 0;
  }
  .more {
    flex-direction: row;
    flex-wrap: wrap;
    align-content: flex-start;
    gap: var(--s-2);
  }
</style>

<!--
  Polls (Ctrl+6): the account's live polls. Left: the list; right: the selected poll (results,
  voting link, editor, actions) or a new poll. The quota sits in the header. Free users see the lock
  (no poll request is made).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Plus from "lucide-svelte/icons/plus";
  import ChartBar from "lucide-svelte/icons/chart-bar";
  import Button from "$lib/components/Button.svelte";
  import EmptyState from "$lib/components/EmptyState.svelte";
  import ProLock from "$lib/components/license/ProLock.svelte";
  import { license } from "$lib/components/license/state.svelte";
  import { createBlocked, createInput, emptyDraft, quotaLines } from "$lib/components/polls/model";
  import PollDetail from "$lib/components/polls/PollDetail.svelte";
  import PollEditor from "$lib/components/polls/PollEditor.svelte";
  import PollList from "$lib/components/polls/PollList.svelte";
  import { PollsState } from "$lib/components/polls/polls.svelte";

  const polls = new PollsState();
  license.start();
  const view = $derived(license.view);
  const pro = $derived(view?.features.polls ?? false);
  const signedIn = $derived(view?.signedIn ?? false);
  const lines = $derived(quotaLines(polls.quota));
  const blockedNew = $derived(createBlocked(polls.quota));

  onMount(() => polls.connect());

  // Signing in or Pro changing: ask again (the core decides whether a request is made).
  let lastAccess = "";
  $effect(() => {
    const key = `${signedIn}|${pro}`;
    if (key === lastAccess) return;
    const first = lastAccess === "";
    lastAccess = key;
    if (!first) void polls.loadPolls();
  });
</script>

<svelte:head><title>Polls · Kinetra</title></svelte:head>

<div class="page">
  <header class="head">
    <span class="title">
      <h1>Polls</h1>
      {#if lines.length}
        <span class="quota" aria-label="Your plan's poll limits">
          {#each lines as l (l.label)}
            <span class="line" data-full={l.full ? "" : undefined}>{l.label} <span class="tnum">{l.value}</span></span>
          {/each}
        </span>
      {/if}
    </span>
    <span class="actions">
      <ProLock feature="polls" label="New poll">
        <Button
          variant="primary"
          disabled={blockedNew !== null}
          title={blockedNew ?? undefined}
          onclick={() => polls.select({ kind: "new" })}
        >
          {#snippet icon()}<Plus size={14} />{/snippet}
          New poll
        </Button>
      </ProLock>
    </span>
  </header>

  {#if !polls.loaded}
    <!-- Loads in a moment; nothing in between. -->
  {:else if polls.polls.length === 0 && polls.selection?.kind !== "new"}
    <div class="empty">
      <EmptyState
        title="Run a live poll"
        description="Your audience votes on their phones. Results show on screen as they come in."
      >
        {#snippet icon()}<ChartBar size={20} />{/snippet}
        {#snippet actions()}
          <ProLock feature="polls" label="New poll">
            <Button variant="primary" disabled={blockedNew !== null} onclick={() => polls.select({ kind: "new" })}>New poll</Button>
          </ProLock>
          {#if signedIn && !pro && !polls.askedExisting}
            <Button variant="ghost" onclick={() => void polls.loadPolls(true)}>Show my existing polls</Button>
          {/if}
        {/snippet}
      </EmptyState>
      {#if blockedNew}<p class="note">{blockedNew}</p>{/if}
    </div>
  {:else}
    <div class="split">
      <aside class="left" aria-label="Poll list">
        <PollList polls={polls.polls} selection={polls.selection} onselect={(s) => polls.select(s)}>
          {#snippet locked()}
            {#if !pro && polls.polls.length === 0}
              <div class="locked">
                {#if signedIn && !polls.askedExisting}
                  <Button size="sm" variant="ghost" onclick={() => void polls.loadPolls(true)}>Show my existing polls</Button>
                {:else if polls.askedExisting}
                  <span class="muted">No polls on this account.</span>
                {/if}
              </div>
            {/if}
          {/snippet}
        </PollList>
      </aside>
      <section class="right" aria-label="Poll">
        {#if polls.selection?.kind === "new"}
          <div class="new">
            <h2>New poll</h2>
            {#if blockedNew}<p class="note">{blockedNew}</p>{/if}
            <PollEditor
              initial={emptyDraft()}
              maxOptions={polls.quota?.limits?.optionsPerPoll ?? 8}
              submitLabel="Create poll"
              busy={polls.busy}
              onsubmit={(d) => void polls.create(createInput(d))}
            />
          </div>
        {:else if polls.selectedPoll}
          {#key polls.selectedPoll.id}
            <PollDetail {polls} poll={polls.selectedPoll} {pro} />
          {/key}
        {/if}
      </section>
    </div>
  {/if}
</div>

<style>
  .page {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    height: calc(100dvh - var(--topbar-h));
    min-width: 0;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
    padding: var(--s-5) var(--s-6);
    border-bottom: 1px solid var(--line);
  }
  .title {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-4);
  }
  .quota {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--s-1) var(--s-4);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .line .tnum {
    color: var(--text);
  }
  .line[data-full] .tnum {
    color: var(--warning);
  }
  .actions {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .empty {
    padding: var(--s-6);
  }
  .note {
    margin: 0;
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .split {
    display: grid;
    grid-template-columns: minmax(15rem, 20rem) minmax(0, 1fr);
    min-height: 0;
  }
  .left {
    min-width: 0;
    min-height: 0;
    overflow: auto;
    border-right: 1px solid var(--line);
    background: var(--surface-1);
  }
  .locked {
    padding: var(--s-1) var(--s-3);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .right {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .right > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .new {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    max-width: 40rem;
    padding: var(--s-5) var(--s-6);
    overflow: auto;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-xl);
  }
  @media (max-width: 760px) {
    .page {
      height: auto;
    }
    .split {
      grid-template-columns: minmax(0, 1fr);
    }
    .left {
      border-right: 0;
      border-bottom: 1px solid var(--line);
    }
  }
</style>

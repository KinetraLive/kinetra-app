<!--
  TestPollDetail: a test poll (a poll source with local data, any plan, no network). Its question
  and answers, simulated votes (start / pause, how fast, which answer leads), open / close, reset,
  the results and the test link. Clearly marked as test data; on screen it looks like a real poll.

  Props:
    polls: PollsState
    source: SourceSummary   the test poll's source (kind "poll", poll.test)
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";
  import Select from "$lib/components/Select.svelte";
  import { getPollSource } from "$lib/api/polls";
  import type { PollSourceDoc, SourceSummary } from "$lib/types";
  import { STATE_LABEL, testData, type PollState } from "./model";
  import type { PollsState } from "./polls.svelte";
  import AddToScreen from "./AddToScreen.svelte";
  import PollEditor from "./PollEditor.svelte";
  import PollLink from "./PollLink.svelte";
  import PollResults from "./PollResults.svelte";

  let { polls, source }: { polls: PollsState; source: SourceSummary } = $props();

  let doc = $state.raw<PollSourceDoc | null>(null);
  // Reloaded when the summary says something changed (votes arrive at most 4 times a second).
  const changeKey = $derived(`${source.id}|${source.updatedAt}|${source.poll?.totalVotes}|${source.poll?.state}`);
  $effect(() => {
    void changeKey;
    let live = true;
    getPollSource(source.id).then(
      (d) => live && (doc = d),
      () => {},
    );
    return () => (live = false);
  });

  const test = $derived(doc?.test ?? null);
  const data = $derived(test ? testData(test) : null);
  const st = $derived<PollState>((source.poll?.state as PollState | null) ?? "draft");
  const simulating = $derived(source.poll?.simulating ?? false);

  type Speed = "1" | "5" | "20";
  const SPEEDS: { value: Speed; label: string }[] = [
    { value: "1", label: "Slow" },
    { value: "5", label: "Busy" },
    { value: "20", label: "Rush" },
  ];
  let speed = $state<Speed>("5");
  let lead = $state<string>("even");
  const leadOptions = $derived([
    { value: "even", label: "Even race" },
    ...(test?.options ?? []).map((o, i) => ({ value: String(i), label: `${o || `Answer ${i + 1}`} leads` })),
  ]);

  function run(on: boolean) {
    void polls.simulate(source.id, { run: on, rate: Number(speed), lead: lead === "even" ? null : Number(lead) });
  }
  // Changing speed or leader while running applies at once.
  function retune() {
    if (simulating) run(true);
  }

  let askDelete = $state(false);
</script>

<article class="detail" aria-label="Test poll">
  <header class="head">
    <div class="title">
      <h2>{source.name}</h2>
      <span class="meta">
        <span class="chip">Test data</span>
        <span class="state" data-state={st}>{STATE_LABEL[st]}</span>
        <span>Votes are simulated on this computer.</span>
      </span>
    </div>
    <div class="primary">
      {#if simulating}
        <Button variant="primary" onclick={() => run(false)}>Pause votes</Button>
      {:else}
        <Button variant="primary" disabled={!test || test.options.length < 2} onclick={() => run(true)}>Simulate votes</Button>
      {/if}
    </div>
  </header>

  <div class="body">
    <section class="panel">
      <div class="controls">
        <Segmented
          label="Votes per second"
          options={SPEEDS}
          value={speed}
          onchange={(v) => {
            speed = v;
            retune();
          }}
        />
        <Select
          size="sm"
          aria-label="Leading answer"
          options={leadOptions}
          value={lead}
          onchange={(v) => {
            lead = v;
            retune();
          }}
        />
      </div>
      {#if data}<PollResults poll={data} conn="test" />{/if}
      <div class="controls">
        {#if st === "open" || st === "full"}
          <Button size="sm" onclick={() => void polls.setTestStatus(source.id, "closed")}>Close poll</Button>
        {:else}
          <Button size="sm" onclick={() => void polls.setTestStatus(source.id, "open")}>Open poll</Button>
        {/if}
        <Button size="sm" disabled={!data || data.totalVotes === 0} onclick={() => void polls.resetTest(source.id)}>Reset votes</Button>
      </div>
    </section>
    {#if data}
      <section class="panel">
        <PollLink poll={data} test />
      </section>
    {/if}
    {#if test}
      <section class="panel">
        <h3 class="section-label">Question and answers</h3>
        <PollEditor
          simple
          initial={{ question: test.question, options: test.options.length ? test.options : ["", ""], duration: null, showResults: true }}
          submitLabel="Save"
          dirtyOnly
          onsubmit={(d) => void polls.saveTest(source.id, d.question, d.options)}
        />
      </section>
    {/if}
    <section class="panel more">
      <AddToScreen source={async () => source.id} />
      <Button href="/sources?source={encodeURIComponent(source.id)}">Open in Sources</Button>
      <Button variant="danger" onclick={() => (askDelete = true)}>Delete</Button>
    </section>
  </div>
</article>

{#if askDelete}
  {#await import("$lib/components/ConfirmDialog.svelte") then { default: ConfirmDialog }}
    <ConfirmDialog
      title="Delete this test poll?"
      message="Layers bound to it keep their own field values."
      confirmLabel="Delete test poll"
      danger
      onconfirm={() => {
        askDelete = false;
        void polls.removeTest(source.id);
      }}
      oncancel={() => (askDelete = false)}
    />
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
    align-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .chip {
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--warning);
    color: var(--warning);
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 20px;
  }
  .state[data-state="open"],
  .state[data-state="full"] {
    color: var(--accent-text);
    font-weight: 600;
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
  .controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
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

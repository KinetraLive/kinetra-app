<!--
  PollResults: a poll's answers as bars with votes and whole percentages (adding up to 100), the
  total, and a quiet line saying how the results arrive (Live / Reconnecting… / Updating every
  5 s). Bars move with transform only. The leader is marked in text, not only by colour.

  Props:
    poll: PollData
    conn?: PollConn     how the results arrive (omit for none)
-->
<script lang="ts">
  import TallyDot from "$lib/components/TallyDot.svelte";
  import type { PollConn, PollData } from "$lib/types";
  import { connLabel, fmt, leader, shares, votesText } from "./model";

  let { poll, conn }: { poll: PollData; conn?: PollConn } = $props();

  const votes = $derived(poll.options.map((o) => o.votes));
  const pct = $derived(shares(votes));
  const lead = $derived(leader(votes));
  const total = $derived(Math.max(poll.totalVotes, votes.reduce((a, b) => a + b, 0)));
  const status = $derived(conn ? connLabel(conn) : null);
</script>

<section class="results" aria-label="Results">
  <ol class="answers">
    {#each poll.options as o, i (o.position)}
      <li class="answer" data-leader={lead === i ? "" : undefined}>
        <div class="row">
          <span class="label">{o.label}</span>
          {#if lead === i}<span class="lead">Leading</span>{/if}
          <span class="votes tnum">{fmt(o.votes)}</span>
          <span class="pct tnum">{pct[i]}%</span>
        </div>
        <div class="track" aria-hidden="true"><div class="bar" style:transform="scaleX({pct[i] / 100})"></div></div>
      </li>
    {/each}
  </ol>
  <div class="foot">
    <span class="total tnum">{votesText(total)}</span>
    {#if status}
      <span class="conn" data-conn={conn}>
        <TallyDot state={status.tone} label={status.text} />
        {status.text}
      </span>
    {/if}
  </div>
</section>

<style>
  .results {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
  }
  .answers {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: flex;
    align-items: baseline;
    gap: var(--s-2);
    margin-bottom: var(--s-1);
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lead {
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .votes {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .pct {
    min-width: 4ch;
    font-weight: 600;
    text-align: right;
  }
  .track {
    height: 8px;
    overflow: hidden;
    border-radius: 4px;
    background: var(--surface-3);
  }
  .bar {
    height: 100%;
    background: var(--text-muted);
    transform-origin: left center;
    transition: transform 220ms cubic-bezier(0.2, 0, 0, 1);
  }
  [data-leader] .bar {
    background: var(--accent);
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .conn {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  @media (prefers-reduced-motion: reduce) {
    .bar {
      transition: none;
    }
  }
</style>

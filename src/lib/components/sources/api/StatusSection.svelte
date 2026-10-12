<!--
  StatusSection: the API editor's Status, from `SourceSummary.api` (follows `source-changed`): the
  state (TallyDot + text, HTTP code), the last good fetch and the next one (mono, relative; it holds
  the shared clock, which re-reads them once a second while shown), the rows, the error text, Retry now and
  Pause / Resume.

  Props:
    status: ApiStatus | null
    paused: boolean
    busy: "retry" | "pause" | null
    onretry: () => void
    onpause: (paused: boolean) => void
-->
<script lang="ts">
  import Pause from "lucide-svelte/icons/pause";
  import Play from "lucide-svelte/icons/play";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Button from "$lib/components/Button.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import type { ApiStatus } from "$lib/types";
  import { hold, now } from "./clock.svelte";
  import { ago, apiLong, until } from "./status";

  let {
    status,
    paused,
    busy,
    onretry,
    onpause,
  }: {
    status: ApiStatus | null;
    paused: boolean;
    busy: "retry" | "pause" | null;
    onretry: () => void;
    onpause: (paused: boolean) => void;
  } = $props();

  const uid = $props.id();
  // The relative times re-read once a second while shown (the editor's preview line too).
  $effect(() => hold());
  const label = $derived(apiLong(status));
  const isPaused = $derived(paused || status?.state === "paused");
  const stamp = (ms: number) => new Date(ms).toLocaleString();
</script>

<section class="section" aria-labelledby="{uid}-title">
  <h3 class="section-label" id="{uid}-title">Status</h3>
  <dl class="facts">
    <dt>State</dt>
    <dd class="state" data-api-state={status?.state ?? "none"}>
      <TallyDot state={label.tally} />
      <span>{label.text}</span>
      {#if status?.httpStatus}<span class="mono muted">HTTP {status.httpStatus}</span>{/if}
    </dd>
    <dt>Last fetch</dt>
    <dd>
      {#if status?.lastOkAt}
        <span title={stamp(status.lastOkAt)}>{ago(status.lastOkAt, now())}</span>
      {:else}
        <span class="muted">None yet</span>
      {/if}
    </dd>
    <dt>Next fetch</dt>
    <dd>
      {#if status?.nextAt && !isPaused}
        <span title={stamp(status.nextAt)}>{until(status.nextAt, now())}</span>
      {:else}
        <span class="muted">{isPaused ? "Paused" : "When the source is in use"}</span>
      {/if}
    </dd>
    <dt>Rows</dt>
    <dd>{status?.rows ?? 0}</dd>
  </dl>
  {#if status?.message}
    <p class="message" role="alert">{status.message}</p>
  {/if}
  <div class="actions">
    <Button size="sm" loading={busy === "retry"} disabled={busy !== null} onclick={onretry}>
      {#snippet icon()}<RotateCw size={14} />{/snippet}
      Retry now
    </Button>
    <Button size="sm" variant="ghost" loading={busy === "pause"} disabled={busy !== null} aria-pressed={isPaused} onclick={() => onpause(!isPaused)}>
      {#snippet icon()}{#if isPaused}<Play size={14} />{:else}<Pause size={14} />{/if}{/snippet}
      {isPaused ? "Resume" : "Pause"}
    </Button>
  </div>
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: var(--s-1) var(--s-4);
    margin: 0;
    font-size: var(--fs-sm);
  }
  dt {
    color: var(--text-muted);
  }
  dd {
    font-variant-numeric: tabular-nums;
    margin: 0;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .muted {
    color: var(--text-muted);
  }
  .message {
    margin: 0;
    color: var(--warning);
    font-size: var(--fs-sm);
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    gap: var(--s-2);
  }
</style>

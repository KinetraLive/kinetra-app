<!--
  SourcesStrip: the text sources the selected screen's layers are bound to (lazy chunk; ControlLayers
  loads it through ./strips.ts). Each: name, the current
  row (its first two columns), mono "3 / 12", Prev / Next (clamped, no wrap). Stepping re-applies
  every bound layer on the backend; the new field values arrive through `live-changed`.

  Auto-advance (server-side, only while a bound layer is on air): the "Auto" toggle turns it on
  with the source's settings (or the last ones / the default) and off; the settings button opens
  AutoSettings, a lazy chunk loaded on first open. While armed (`nextAt` set) a thin bar under the
  row runs down to the next step: one CSS animation whose duration is the remaining time,
  restarted (keyed) on every new `nextAt`. No timers, no rAF.

  API sources add a status line: TallyDot + Live / Waiting / Error / Paused, "updated 4 s ago"
  (mono; the shared clock re-reads it once a second only while such a line shows and the window is
  visible), Retry and Pause / Resume (their replies go to `onapi`).

  Props:
    sources: SourceSummary[]
    onstep: (id: string, delta: number) => void
    autoOf: (s: SourceSummary) => AutoAdvance      settings shown / turned on with
    onauto: (id: string, on: boolean) => void
    onautosettings: (id: string, auto: AutoAdvance) => void
-->
<script lang="ts" module>
  import type { Component } from "svelte";
  import type { AutoAdvance, PollConn, SourceSummary } from "$lib/types";

  /** Milliseconds until `nextAt` (server epoch ms), never negative. */
  function remainingMs(nextAt: number, now = Date.now()): number {
    return Math.max(0, nextAt - now);
  }

  /** The current row's first two text column values, joined (what the sources strip shows). Image
   *  columns are skipped: an asset path is not something to read. */
  function rowPreview(s: SourceSummary): string {
    return s.columns
      .filter((c) => c.kind !== "image")
      .slice(0, 2)
      .map((c) => s.current[c.key] ?? "")
      .filter((v) => v !== "")
      .join(" · ");
  }

  /** "3 / 12" (1-based); "0 / 0" without rows. */
  /** A poll source's line: its status and votes. */
  function pollLine(s: SourceSummary): string {
    const p = s.poll;
    if (!p?.state) return "No results yet";
    const state = { draft: "Draft", open: "Open", closed: "Closed", full: "Full" }[p.state] ?? p.state;
    return `${state} · ${p.totalVotes.toLocaleString("en-US")} ${p.totalVotes === 1 ? "vote" : "votes"}`;
  }

  /** How a linked poll's results arrive, for its status chip (idle: nothing shown). */
  const POLL_CONN: Partial<Record<PollConn, [string, "preview" | "warning" | "idle"]>> = {
    live: ["Live", "preview"],
    connecting: ["Connecting", "idle"],
    reconnecting: ["Reconnecting", "warning"],
    polling: ["Every 5 s", "warning"],
    signedOut: ["Signed out", "warning"],
    gone: ["Deleted", "idle"],
  };

  function rowPosition(s: SourceSummary): string {
    return s.rowCount === 0 ? "0 / 0" : `${Math.min(s.index, s.rowCount - 1) + 1} / ${s.rowCount}`;
  }

  type AutoSettingsProps = {
    name: string;
    value: AutoAdvance;
    active: boolean;
    onchange: (auto: AutoAdvance) => void;
    onclose: (refocus: boolean) => void;
  };
  let mod: Promise<Component<AutoSettingsProps>> | undefined;

  /** The settings popover (cached; a failed load is retried next time). */
  export function loadAutoSettings(): Promise<Component<AutoSettingsProps>> {
    mod ??= import("./AutoSettings.svelte").then((m) => m.default as Component<AutoSettingsProps>);
    mod.catch(() => (mod = undefined));
    return mod;
  }
</script>

<script lang="ts">
  import ChevronLeft from "lucide-svelte/icons/chevron-left";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Pause from "lucide-svelte/icons/pause";
  import Play from "lucide-svelte/icons/play";
  import RotateCw from "lucide-svelte/icons/rotate-cw";
  import Settings2 from "lucide-svelte/icons/settings-2";
  import { toAppError } from "$lib/api";
  import { retryApiSource, setApiPaused } from "$lib/api/api-sources";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { hold, now } from "$lib/components/sources/api/clock.svelte";
  import { ago, apiShort } from "$lib/components/sources/api/status";
  import { toasts } from "$lib/stores/toasts.svelte";

  let {
    sources,
    onstep,
    autoOf,
    onauto,
    onautosettings,
    onapi,
  }: {
    sources: readonly SourceSummary[];
    onstep: (id: string, delta: number) => void;
    autoOf: (s: SourceSummary) => AutoAdvance;
    onauto: (id: string, on: boolean) => void;
    onautosettings: (id: string, auto: AutoAdvance) => void;
    onapi?: (s: SourceSummary) => void;
  } = $props();

  // "updated 4 s ago" re-reads once a second, only while an API source shows.
  const hasApi = $derived(sources.some((s) => s.kind === "api"));
  $effect(() => {
    if (hasApi) return hold();
  });

  let apiBusy = $state<string | null>(null);
  async function apiAct(id: string, run: () => Promise<SourceSummary>) {
    if (apiBusy) return;
    apiBusy = id;
    try {
      onapi?.(await run());
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      apiBusy = null;
    }
  }

  const uid = $props.id();
  let Settings = $state.raw<Component<AutoSettingsProps> | null>(null);
  let openFor = $state<string | null>(null);
  const triggers: Record<string, HTMLButtonElement> = {};

  async function toggleSettings(id: string) {
    if (openFor === id) {
      openFor = null;
      return;
    }
    try {
      Settings ??= await loadAutoSettings();
      openFor = id;
    } catch {
      // Chunk failed to load (offline dev server); the button stays usable for a retry.
    }
  }

  function close(refocus: boolean) {
    const id = openFor;
    openFor = null;
    if (refocus && id) triggers[id]?.focus();
  }

  const seconds = (ms: number) => `${Math.round(ms / 100) / 10} s`;
</script>

<section class="strip" aria-labelledby="{uid}-title">
  <h2 class="section-label head" id="{uid}-title">Sources</h2>
  <ul class="list">
    {#each sources as s (s.id)}
      {@const row = rowPreview(s)}
      {@const auto = autoOf(s)}
      <li class="src" data-source={s.id}>
        <span class="text">
          <span class="name">{s.name}</span>
          {#if s.kind === "poll"}
            <span class="row">{pollLine(s)}</span>
          {:else}
            <span class="row" title={row}>{row || (s.rowCount === 0 ? "No rows" : "Empty row")}</span>
          {/if}
        </span>
        {#if s.kind === "poll"}
          <!-- A poll is pinned to its first row: no stepping or auto-advance, its status instead. -->
          {#if s.poll?.test}
            <span class="test">Test data</span>
          {:else if s.poll && POLL_CONN[s.poll.conn]}
            <span class="api"><TallyDot state={POLL_CONN[s.poll.conn]![1]} /><span class="api-text">{POLL_CONN[s.poll.conn]![0]}</span></span>
          {/if}
        {:else}
        <span class="pos">{rowPosition(s)}</span>
        <IconButton
          size="sm"
          variant="secondary"
          label="Previous row of {s.name}"
          tooltip="top"
          disabled={s.index <= 0}
          onclick={() => onstep(s.id, -1)}
        >
          <ChevronLeft size={14} />
        </IconButton>
        <IconButton
          size="sm"
          variant="secondary"
          label="Next row of {s.name}"
          tooltip="left"
          disabled={s.index >= s.rowCount - 1}
          onclick={() => onstep(s.id, 1)}
        >
          <ChevronRight size={14} />
        </IconButton>
        <span class="auto">
          <Button
            size="sm"
            variant={s.auto ? "primary" : "secondary"}
            aria-pressed={s.auto !== null}
            aria-label="Auto-advance {s.name}"
            onclick={() => onauto(s.id, s.auto === null)}
          >
            Auto
          </Button>
          {#if s.auto}<span class="every mono" title="Interval">{seconds(s.auto.everyMs)}</span>{/if}
          <span class="trigger" data-auto-trigger>
            <IconButton
              size="sm"
              label="Auto-advance settings of {s.name}"
              tooltip="left"
              pressed={openFor === s.id}
              pressedFill={false}
              aria-expanded={openFor === s.id}
              onclick={(e: MouseEvent) => {
                triggers[s.id] = e.currentTarget as HTMLButtonElement;
                void toggleSettings(s.id);
              }}
            >
              <Settings2 size={14} />
            </IconButton>
          </span>
        </span>
        {#if openFor === s.id && Settings}
          <Settings
            name={s.name}
            value={auto}
            active={s.auto !== null}
            onchange={(a) => onautosettings(s.id, a)}
            onclose={close}
          />
        {/if}
        {/if}
        {#if s.kind === "api"}
          {@const st = apiShort(s.api)}
          {@const paused = s.api?.state === "paused"}
          <span class="api" data-api-state={s.api?.state ?? "none"}>
            <TallyDot state={st.tally} />
            <span class="api-text">{st.text}</span>
            {#if s.api?.lastOkAt}<span class="muted tnum" title={new Date(s.api.lastOkAt).toLocaleString()}>updated {ago(s.api.lastOkAt, now())}</span>{/if}
            {#if s.api?.message && st.tally === "warning"}<span class="err" title={s.api.message}>{s.api.message}</span>{/if}
            <span class="api-acts">
              <IconButton size="sm" label="Retry {s.name} now" tooltip="left" disabled={apiBusy !== null} onclick={() => apiAct(s.id, () => retryApiSource(s.id))}>
                <RotateCw size={14} />
              </IconButton>
              <IconButton
                size="sm"
                label="{paused ? "Resume" : "Pause"} {s.name}"
                tooltip="left"
                pressed={paused}
                pressedFill={false}
                disabled={apiBusy !== null}
                onclick={() => apiAct(s.id, () => setApiPaused(s.id, !paused))}
              >
                {#if paused}<Play size={14} />{:else}<Pause size={14} />{/if}
              </IconButton>
            </span>
          </span>
        {/if}
        {#if s.auto && s.nextAt !== null}
          {#key s.nextAt}
            {@const ms = remainingMs(s.nextAt)}
            <span class="countdown" data-countdown style:animation-duration="{ms}ms" aria-hidden="true"></span>
            <span class="visually-hidden">Next row in about {seconds(ms)}</span>
          {/key}
        {/if}
      </li>
    {/each}
  </ul>
</section>

<style>
  .test {
    flex: none;
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--warning);
    color: var(--warning);
    font-size: var(--fs-xs);
    font-weight: 600;
    line-height: 18px;
  }
  .strip {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
  }
  .head {
    display: flex;
    align-items: center;
    min-height: calc(var(--row-h) + var(--s-2));
    margin: 0;
    padding: var(--s-1) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: var(--s-2);
    list-style: none;
  }
  .src {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-height: var(--row-h);
    padding: var(--s-1) var(--s-1) var(--s-1) var(--s-2);
    border-radius: var(--r-control);
  }
  .src:has(.api) {
    flex-wrap: wrap;
  }
  .api {
    display: flex;
    flex-basis: 100%;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .api-text {
    color: var(--text);
  }
  .api > span {
    flex: none;
    white-space: nowrap;
  }
  .muted {
    color: var(--text-muted);
  }
  .api > .err {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    color: var(--warning);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .api-acts {
    display: inline-flex;
    gap: 2px;
    margin-left: auto;
  }
  .src:hover {
    background: var(--surface-2);
  }
  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }
  .name,
  .row {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .pos,
  .every {
    flex: none;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .pos {
    padding: 0 var(--s-2);
    font-variant-numeric: tabular-nums;
  }
  .auto {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--s-1);
    margin-left: var(--s-1);
    padding-left: var(--s-2);
    border-left: 1px solid var(--line);
  }
  .trigger {
    display: inline-flex;
  }
  /* Runs down from full width to nothing over the remaining time (transform only). Kept under
     reduced motion: it is the information (time to the next step), not decoration. */
  .countdown {
    position: absolute;
    right: var(--s-2);
    bottom: 0;
    left: var(--s-2);
    height: 2px;
    border-radius: var(--r-full);
    background: var(--accent);
    transform-origin: left center;
    animation-name: run-down;
    animation-timing-function: linear;
    animation-fill-mode: forwards;
  }
  @keyframes run-down {
    from {
      transform: scaleX(1);
    }
    to {
      transform: scaleX(0);
    }
  }
</style>

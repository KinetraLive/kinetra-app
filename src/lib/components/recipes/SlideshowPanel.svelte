<!--
  SlideshowPanel: a slideshow recipe's live panel in Control (lazy chunk). The source's current
  slide (the image of its first image column, served by the output server, else its text), the
  position "3 / 12" (mono), Previous / Play-Pause / Next (large targets), and the settings: interval
  (s), Shuffle, Per-slide durations (a row's `duration` column), Loop and the transition. While the
  slideshow is armed (`nextAt` set) a thin bar runs down to the next slide: one CSS animation keyed
  to `nextAt` (no timers, no rAF).

  Play / Pause: Play turns auto-advance on (or resumes it); Pause keeps it on but stops stepping
  (`paused`). Next past the last slide wraps to the first when Loop is on (and Previous the other
  way). Everything goes through ControlCtl (optimistic, rolled back with a toast on failure;
  settings changed while off are remembered for the next Play).

  Props: SlideshowPanelProps (./lazy.ts).
-->
<script lang="ts">
  import ChevronLeft from "lucide-svelte/icons/chevron-left";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Pause from "lucide-svelte/icons/pause";
  import Play from "lucide-svelte/icons/play";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";

  import { server } from "$lib/stores/server.svelte";
  import { AUTO_ADVANCE_RANGE, type AutoAdvance } from "$lib/types";
  import type { SlideshowPanelProps } from "./lazy";

  let { ctl, sourceId }: SlideshowPanelProps = $props();

  const uid = $props.id();
  const TRANSITIONS = [
    { value: "update", label: "Crossfade in place" },
    { value: "retake", label: "Play out & in" },
  ] as const;

  const s = $derived(ctl.sources.find((x) => x.id === sourceId) ?? null);
  const auto = $derived(s ? ctl.autoOf(s) : null);
  const playing = $derived(!!s?.auto && !s.auto.paused);
  const imageCol = $derived(s?.columns.find((c) => c.kind === "image") ?? null);
  const image = $derived.by(() => {
    const v = imageCol && s ? (s.current[imageCol.key] ?? "") : "";
    const base = server.status?.state === "running" ? server.status.url : null;
    return v.startsWith("/assets/") && base ? `${base}${v}` : null;
  });
  const text = $derived(
    s
      ? s.columns
          .filter((c) => c.kind !== "image" && c.key !== "duration" && !c.key.startsWith("_"))
          .map((c) => s.current[c.key] ?? "")
          .filter((v) => v !== "")
          .slice(0, 2)
          .join(" · ")
      : "",
  );
  const last = $derived(s ? Math.max(0, s.rowCount - 1) : 0);
  // As SourcesStrip's rowPosition / remainingMs (two lines each; kept local rather than shared
  // between two lazy chunks).
  const position = $derived(s && s.rowCount > 0 ? `${Math.min(s.index, last) + 1} / ${s.rowCount}` : "0 / 0");
  const remainingMs = (at: number) => Math.max(0, at - Date.now());

  $effect(() => server.watch());

  function step(delta: 1 | -1) {
    if (!s || !auto || s.rowCount === 0) return;
    const i = Math.min(s.index, last);
    if (delta > 0 && i >= last) {
      if (auto.loop) void ctl.stepTo(s.id, { to: "index", index: 0 });
    } else if (delta < 0 && i <= 0) {
      if (auto.loop) void ctl.stepTo(s.id, { to: "index", index: last });
    } else {
      void ctl.step(s.id, delta);
    }
  }

  function playPause() {
    if (!s || !auto) return;
    if (!s.auto) {
      // Play from off: on with the remembered settings, never paused.
      if (auto.paused) void ctl.setAutoSettings(s.id, { ...auto, paused: false });
      void ctl.setAutoOn(s.id, true);
    } else {
      void ctl.setAutoSettings(s.id, { ...s.auto, paused: !s.auto.paused });
    }
  }

  function set(patch: Partial<AutoAdvance>) {
    if (s && auto) void ctl.setAutoSettings(s.id, { ...auto, ...patch });
  }

  const seconds = (ms: number) => `${Math.round(ms / 100) / 10} s`;
</script>

{#if !s}
  <p class="muted pad">This slideshow's source no longer exists.</p>
{:else if auto}
  <section class="panel" aria-label="Slideshow {s.name}">
    <div class="slide">
      <span class="thumb checker-dark">
        {#if image}
          <img src={image} alt={text || "Current slide"} loading="lazy" decoding="async" />
        {:else}
          <span class="slide-text">{text || (s.rowCount === 0 ? "No slides yet" : "Empty slide")}</span>
        {/if}
      </span>
      <span class="info">
        <span class="name">{s.name}</span>
        {#if image && text}<span class="caption">{text}</span>{/if}
        <span class="pos" aria-label="Slide {position}">{position}</span>
        <span class="state" role="status">
          {#if !s.auto}Stopped{:else if s.auto.paused}Paused{:else if s.nextAt !== null}Playing{:else}Waits for on air{/if}
        </span>
      </span>
      {#if s.auto && !s.auto.paused && s.nextAt !== null}
        {#key s.nextAt}
          {@const ms = remainingMs(s.nextAt)}
          <span class="progress" data-progress style:animation-duration="{ms}ms" aria-hidden="true"></span>
          <span class="visually-hidden">Next slide in about {seconds(ms)}</span>
        {/key}
      {/if}
    </div>

    <div class="transport">
      <Button aria-label="Previous slide" disabled={s.rowCount === 0 || (!auto.loop && s.index <= 0)} onclick={() => step(-1)}>
        {#snippet icon()}<ChevronLeft size={16} />{/snippet}
      </Button>
      <Button variant={playing ? "secondary" : "primary"} class="play" aria-pressed={playing} onclick={playPause}>
        {#snippet icon()}{#if playing}<Pause size={16} />{:else}<Play size={16} />{/if}{/snippet}
        {playing ? "Pause" : "Play"}
      </Button>
      <Button aria-label="Next slide" disabled={s.rowCount === 0 || (!auto.loop && s.index >= last)} onclick={() => step(1)}>
        {#snippet icon()}<ChevronRight size={16} />{/snippet}
      </Button>
    </div>

    <div class="settings">
      <div class="row">
        <label for="{uid}-every">Every</label>
        <NumberInput
          id="{uid}-every"
          value={auto.everyMs / 1000}
          min={AUTO_ADVANCE_RANGE.min / 1000}
          max={AUTO_ADVANCE_RANGE.max / 1000}
          step={1}
          digits={4}
          suffix="s"
          onchange={(sec) => set({ everyMs: Math.round(sec * 1000) })}
        />
      </div>
      <Switch label="Shuffle" checked={!!auto.shuffle} onchange={(shuffle) => set({ shuffle })} />
      <Switch label="Per-slide durations" checked={!!auto.itemDurations} onchange={(itemDurations) => set({ itemDurations })} />
      <Switch label="Loop" checked={auto.loop} onchange={(loop) => set({ loop })} />
      <div class="col">
        <span class="lbl">Transition</span>
        <Segmented label="Transition" options={TRANSITIONS} value={auto.transition} onchange={(transition) => set({ transition })} />
      </div>
    </div>
  </section>
{/if}

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: 0 var(--s-3) var(--s-2);
  }
  .slide {
    position: relative;
    display: flex;
    gap: var(--s-3);
    align-items: center;
    padding-bottom: var(--s-2);
  }
  .thumb {
    position: relative;
    display: grid;
    flex: none;
    place-items: center;
    width: 160px;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .thumb img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .slide-text {
    padding: 0 var(--s-2);
    overflow: hidden;
    color: var(--checker-dark-ink);
    font-size: var(--fs-sm);
    font-weight: 600;
    text-align: center;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .name,
  .caption {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    font-weight: 600;
  }
  .caption,
  .state {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .pos {
    font-size: var(--fs-lg);
    font-variant-numeric: tabular-nums;
  }
  .progress {
    position: absolute;
    right: 0;
    bottom: 0;
    left: 0;
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
  /* Large live targets. */
  .transport {
    --control-h: 40px;
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: var(--s-2);
  }
  .settings {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-4);
  }
  .row,
  .col {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .col {
    flex-basis: 100%;
  }
  label,
  .lbl {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .muted {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .pad {
    padding: var(--s-2) var(--s-3);
  }
</style>

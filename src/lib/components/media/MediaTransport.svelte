<!--
  MediaTransport: a media layer's transport in Control's layer row (a lazy chunk, loaded the first
  time a media layer shows). Play / Pause, Stop, Restart (thin wrappers around `media_control`,
  acting on Program at once, in Studio mode too), elapsed / remaining in mono tabular numbers and
  the readiness chip ("Loading 2/3", or the error in --warning).

  The time display updates from the playback and the clock (the app runs the server, so
  `Date.now()` is the server clock): one rAF-aligned 1 s timer only while the row is visible and
  the layer plays; nothing scheduled otherwise.

  Two parts, so the row's own code stays tiny: `file` (the file name, mono, under the layer name)
  and `transport` (the buttons, times and readiness chip).

  Props:
    ctl: ControlCtl          (the selected screen's doc: the file, range and end action)
    screenId: string
    layer: LiveLayerView     (with `media`)
    part: "file" | "transport"
-->
<script lang="ts">
  import Pause from "lucide-svelte/icons/pause";
  import Play from "lucide-svelte/icons/play";
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import Square from "lucide-svelte/icons/square";
  import { toAppError } from "$lib/api";
  import { mediaControl, type MediaAction } from "$lib/api/media";
  import IconButton from "$lib/components/IconButton.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { assetName } from "$lib/components/assets/assets";
  import type { ControlCtl } from "$lib/components/control/control.svelte";
  import type { LiveLayerView } from "$lib/types";
  import { elapsedRemaining, formatTime, readinessText } from "./media";
  import { confirmTake } from "./take";

  let {
    ctl,
    screenId,
    layer,
    part,
  }: { ctl: ControlCtl; screenId: string; layer: LiveLayerView; part: "file" | "transport" } = $props();

  const doc = $derived(ctl.doc?.id === screenId ? ctl.doc.layers.find((l) => l.id === layer.id) : undefined);
  const config = $derived(doc?.kind === "media" ? doc.media : null);
  const name = $derived(layer.name);
  // Take on a not-ready media layer asks once (Control calls it from any take path).
  // svelte-ignore state_referenced_locally
  ctl.confirmMedia ??= confirmTake;

  const media = $derived(layer.media ?? null);
  const playing = $derived(media?.playback.state === "playing");
  const ready = $derived(readinessText(media?.readiness ?? layer.readiness));

  // ---- Clock: ticks only while visible and playing ------------------------------------------------

  let now = $state(Date.now());
  let root: HTMLElement | undefined = $state();
  let visible = $state(true);
  let pageVisible = $state(typeof document === "undefined" ? true : document.visibilityState === "visible");

  $effect(() => {
    const el = root;
    if (!el || typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver((es) => (visible = es[es.length - 1]?.isIntersecting ?? true));
    io.observe(el);
    return () => io.disconnect();
  });
  $effect(() => {
    const on = () => (pageVisible = document.visibilityState === "visible");
    document.addEventListener("visibilitychange", on);
    return () => document.removeEventListener("visibilitychange", on);
  });
  $effect(() => {
    void media?.playback;
    now = Date.now();
    if (part !== "transport" || !playing || !visible || !pageVisible) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let frame = 0;
    const tick = () => {
      frame = requestAnimationFrame(() => {
        now = Date.now();
        // Next whole second of the media position.
        timer = setTimeout(tick, 1000 - ((now - (media?.playback.at ?? now)) % 1000 + 1000) % 1000);
      });
    };
    tick();
    return () => {
      clearTimeout(timer);
      cancelAnimationFrame(frame);
    };
  });

  const times = $derived(
    media && config ? elapsedRemaining(media.playback, now, config, media.durationMs) : null,
  );

  async function act(action: MediaAction) {
    try {
      await mediaControl(screenId, layer.id, action);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't ${action} ${name}: ${toAppError(e).message}` });
    }
  }
</script>

{#if part === "file"}
  {#if config}<span class="file mono" title={assetName(config.asset)}>{assetName(config.asset)}</span>{/if}
{:else if !media}
  <!-- A template layer: its video / image fields' readiness only (nothing without media). -->
  {#if ready}<span class="chip tnum" class:error={ready.error} role="status">{ready.text}</span>{/if}
{:else}
<span class="transport" bind:this={root} data-media-transport>
  {#if ready}
    <span class="chip tnum" class:error={ready.error} role="status">{ready.text}</span>
  {/if}
  {#if times}
    <span class="time mono" aria-label="Elapsed {formatTime(times.elapsed)}{times.remaining === null ? '' : `, remaining ${formatTime(times.remaining)}`}">
      {formatTime(times.elapsed)}{#if times.remaining !== null}<span class="muted">{` / −${formatTime(times.remaining)}`}</span>{/if}
    </span>
  {/if}
  {#if playing}
    <IconButton size="sm" label="Pause {name}" tooltip="top" onclick={() => void act("pause")}><Pause size={14} /></IconButton>
  {:else}
    <IconButton size="sm" label="Play {name}" tooltip="top" onclick={() => void act("play")}><Play size={14} /></IconButton>
  {/if}
  <IconButton size="sm" label="Stop {name}" tooltip="top" disabled={media?.playback.state === "stopped"} onclick={() => void act("stop")}>
    <Square size={14} />
  </IconButton>
  <IconButton size="sm" label="Restart {name}" tooltip="top" onclick={() => void act("restart")}><RotateCcw size={14} /></IconButton>
</span>
{/if}

<style>
  .file {
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .transport {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: 2px;
  }
  .time {
    margin-right: var(--s-1);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .muted {
    color: var(--text-muted);
  }
  .chip {
    margin-right: var(--s-1);
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 18px;
    white-space: nowrap;
  }
  .chip.error {
    box-shadow: inset 0 0 0 1px var(--warning);
    color: var(--warning);
  }
</style>

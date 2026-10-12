<!--
  MediaInspector: the layout editor Inspector's sections for a media layer (a lazy chunk, loaded
  when a media layer is selected; the Inspector keeps Layer / Position and size above it).

    File       the file (name, kind, mono duration / resolution) with "Choose…" (the media picker),
               its worst playback issue; "File missing" when it isn't in the project
    Picture    fit: Fill (crop) / Fit (letterbox) / Stretch (video and images)
    Play range video / audio: a scrub preview (muted <video>, the file from the output server;
               only metadata loads until it is scrubbed), a range with in / out marks, mono in /
               out times (typed "1:23.5"), "Set in" / "Set out" from the preview's position,
               "Reset" (whole file)
    Sound      volume 0–100 and Mute (video / audio)
    Playback   at the end: Hold last frame / Clear / Loop; "Play on take" (autoplay)

  Every change is a patch of the layer's `media` (`onpatch({ media })`): content, so it follows
  Live edit / Studio mode like `rect` and autosaves with the editor. The file's facts come from
  the assets list (loaded once per file).

  Props:
    layer: MediaScreenLayer
    screenId: string
    live: LiveLayerView | null   playback / readiness / the file's duration
    missing: boolean             the file isn't in assets/ (warning `assetMissing`)
    onpatch: (patch: Partial<ScreenLayer>) => void
-->
<script lang="ts">
  import Film from "lucide-svelte/icons/film";
  import ImageIcon from "lucide-svelte/icons/image";
  import Music from "lucide-svelte/icons/music";
  import Volume2 from "lucide-svelte/icons/volume-2";
  import VolumeX from "lucide-svelte/icons/volume-x";
  import { toAppError } from "$lib/api";
  import { listAssets } from "$lib/api/assets";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";
  import { assetName, thumbUrl } from "$lib/components/assets/assets";
  import { server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type {
    AssetInfo,
    LiveLayerView,
    MediaEndAction,
    MediaFit,
    MediaLayerConfig,
    MediaScreenLayer,
    ScreenLayer,
  } from "$lib/types";
  import { clampRange, codecText, formatTime, kindOfName, parseTime, resolutionText, sortIssues, worstIssue } from "./media";

  let {
    layer,
    screenId,
    live = null,
    missing = false,
    onpatch,
  }: {
    layer: MediaScreenLayer;
    screenId: string;
    live?: LiveLayerView | null;
    missing?: boolean;
    onpatch: (patch: Partial<ScreenLayer>) => void;
  } = $props();

  const uid = $props.id();
  const m = $derived(layer.media);
  const name = $derived(assetName(m.asset));

  // ---- The file's facts ---------------------------------------------------------------------------

  let info = $state.raw<AssetInfo | null>(null);
  let infoFor = "";
  $effect(() => {
    const n = name;
    if (n === infoFor) return;
    infoFor = n;
    info = null;
    listAssets().then(
      (list) => {
        if (infoFor === n) info = list.find((a) => a.name === n) ?? null;
      },
      () => {},
    );
  });
  $effect(() => server.watch());

  const kind = $derived(info?.media?.kind ?? kindOfName(name));
  const duration = $derived(live?.media?.durationMs ?? info?.media?.durationMs ?? null);
  const timed = $derived(kind === "video" || kind === "audio");
  const fileUrl = $derived(thumbUrl(server.status, m.asset));

  function patch(p: Partial<MediaLayerConfig>) {
    onpatch({ media: { ...m, ...p } } as Partial<ScreenLayer>);
  }

  async function chooseFile() {
    try {
      const { pickMedia } = await import("./pick");
      const a = await pickMedia({ accept: ["video", "audio", "image"], current: m.asset, label: layer.name });
      if (!a || a.url === m.asset) return;
      // A new file starts with its whole range.
      patch({ asset: a.url, inMs: 0, outMs: null, endAction: a.media?.kind === "image" ? "hold" : m.endAction });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't open the media: ${toAppError(e).message}` });
    }
  }

  // ---- Play range ---------------------------------------------------------------------------------

  let video: HTMLVideoElement | undefined = $state();
  /** The scrub preview's position (ms). */
  let scrub = $state(0);
  $effect(() => {
    void name;
    scrub = m.inMs;
  });
  function seekPreview(ms: number) {
    scrub = ms;
    if (video) video.currentTime = ms / 1000;
  }

  function setRange(inMs: number, outMs: number | null) {
    const r = clampRange(inMs, outMs, duration);
    if (r.inMs !== m.inMs || r.outMs !== m.outMs) patch(r);
  }

  let inDraft = $state<string | null>(null);
  let outDraft = $state<string | null>(null);
  const inText = $derived(inDraft ?? formatTime(m.inMs, true));
  const outText = $derived(outDraft ?? (m.outMs === null ? (duration === null ? "" : formatTime(duration, true)) : formatTime(m.outMs, true)));
  const inBad = $derived(inDraft !== null && parseTime(inDraft) === null);
  const outBad = $derived(outDraft !== null && outDraft.trim() !== "" && parseTime(outDraft) === null);

  function commitIn() {
    if (inDraft === null) return;
    const v = parseTime(inDraft);
    if (v !== null) setRange(v, m.outMs);
    if (v !== null) inDraft = null;
  }
  function commitOut() {
    if (outDraft === null) return;
    if (outDraft.trim() === "") {
      setRange(m.inMs, null);
      outDraft = null;
      return;
    }
    const v = parseTime(outDraft);
    if (v !== null) {
      setRange(m.inMs, duration !== null && v >= duration ? null : v);
      outDraft = null;
    }
  }
  const timeKey = (commit: () => void, reset: () => void) => (e: KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    } else if (e.key === "Escape") reset();
  };

  const pct = (ms: number) => (duration ? Math.min(100, Math.max(0, (ms / duration) * 100)) : 0);
  const outEnd = $derived(m.outMs ?? duration ?? 0);

  const FIT_OPTIONS: { value: MediaFit; label: string }[] = [
    { value: "fill", label: "Fill" },
    { value: "fit", label: "Fit" },
    { value: "stretch", label: "Stretch" },
  ];
  const END_OPTIONS: { value: MediaEndAction; label: string }[] = [
    { value: "hold", label: "Hold last frame" },
    { value: "clear", label: "Clear" },
    { value: "loop", label: "Loop" },
  ];
  const worst = $derived(worstIssue(info?.media?.issues));
  const firstIssue = $derived(info?.media ? sortIssues(info.media.issues)[0] : undefined);
</script>

<section class="block" aria-labelledby="{uid}-file">
  <h2 class="section-label" id="{uid}-file">File</h2>
  <div class="file">
    <span class="kind" aria-hidden="true">
      {#if kind === "video"}<Film size={16} />{:else if kind === "audio"}<Music size={16} />{:else}<ImageIcon size={16} />{/if}
    </span>
    <span class="file-text">
      <span class="file-name mono" title={name}>{name || "No file"}</span>
      <span class="facts mono tnum">
        {[duration !== null && timed ? formatTime(duration, true) : null, resolutionText(info?.media), codecText(info?.media)]
          .filter(Boolean)
          .join(" · ")}
      </span>
    </span>
    <Button size="sm" onclick={() => void chooseFile()} aria-label="Choose file for {layer.name}">Choose…</Button>
  </div>
  {#if missing}
    <p class="warn">This file is no longer in the project. Choose another file.</p>
  {:else if firstIssue}
    <p class={worst === "error" ? "bad" : "warn"}>{firstIssue.message}</p>
  {/if}
</section>

{#if kind !== "audio"}
  <section class="block" aria-labelledby="{uid}-pic">
    <h2 class="section-label" id="{uid}-pic">Picture</h2>
    <div class="grid">
      <span class="label" id="{uid}-fit">Fit</span>
      <Segmented label="Fit" options={FIT_OPTIONS} value={m.fit} onchange={(v) => patch({ fit: v })} />
    </div>
  </section>
{/if}

{#if timed}
  <section class="block" aria-labelledby="{uid}-range">
    <h2 class="section-label" id="{uid}-range">Play range</h2>
    {#if kind === "video" && fileUrl}
      <!-- svelte-ignore a11y_media_has_caption -->
      <video
        class="scrub-video"
        bind:this={video}
        src={fileUrl}
        muted
        preload="metadata"
        playsinline
        onloadedmetadata={() => video && (video.currentTime = scrub / 1000)}
      ></video>
    {/if}
    <div class="scrubber" style:--in="{pct(m.inMs)}%" style:--out="{pct(outEnd)}%">
      <span class="range-fill" aria-hidden="true"></span>
      <span class="vf vf-within slider">
        <input
          type="range"
          aria-label="Preview position"
          min="0"
          max={duration ?? 0}
          step="10"
          value={scrub}
          disabled={duration === null}
          oninput={(e) => seekPreview(Number(e.currentTarget.value))}
        />
      </span>
    </div>
    <div class="times">
      <span class="mono tnum muted small" aria-label="Preview position">{formatTime(scrub, true)}</span>
      <span class="spacer"></span>
      <Button size="sm" variant="ghost" disabled={duration === null} onclick={() => setRange(scrub, m.outMs !== null && m.outMs <= scrub ? null : m.outMs)}>Set in</Button>
      <Button size="sm" variant="ghost" disabled={duration === null || scrub <= m.inMs} onclick={() => setRange(m.inMs, scrub)}>Set out</Button>
    </div>
    <div class="grid">
      <label for="{uid}-in">In</label>
      <span class="time vf vf-within">
        <input
          id="{uid}-in"
          class="mono tnum"
          type="text"
          inputmode="decimal"
          autocomplete="off"
          spellcheck="false"
          value={inText}
          aria-invalid={inBad ? "true" : undefined}
          oninput={(e) => (inDraft = e.currentTarget.value)}
          onblur={commitIn}
          onkeydown={timeKey(commitIn, () => (inDraft = null))}
        />
      </span>
      <label for="{uid}-out">Out</label>
      <span class="time vf vf-within">
        <input
          id="{uid}-out"
          class="mono tnum"
          type="text"
          inputmode="decimal"
          autocomplete="off"
          spellcheck="false"
          value={outText}
          aria-invalid={outBad ? "true" : undefined}
          oninput={(e) => (outDraft = e.currentTarget.value)}
          onblur={commitOut}
          onkeydown={timeKey(commitOut, () => (outDraft = null))}
        />
      </span>
    </div>
    {#if m.inMs > 0 || m.outMs !== null}
      <Button size="sm" variant="ghost" class="reset" onclick={() => setRange(0, null)}>Play the whole file</Button>
    {/if}
  </section>

  <section class="block" aria-labelledby="{uid}-sound">
    <h2 class="section-label" id="{uid}-sound">Sound</h2>
    <div class="sound">
      <IconButton size="sm" label={m.muted ? "Unmute" : "Mute"} pressed={m.muted} pressedFill={false} onclick={() => patch({ muted: !m.muted })}>
        {#if m.muted}<VolumeX size={14} />{:else}<Volume2 size={14} />{/if}
      </IconButton>
      <span class="slider vf vf-within">
        <input
          type="range"
          aria-label="Volume"
          min="0"
          max="100"
          step="1"
          value={m.volume}
          disabled={m.muted}
          oninput={(e) => patch({ volume: Math.round(Number(e.currentTarget.value)) })}
        />
      </span>
      <span class="mono tnum small vol">{m.volume} %</span>
    </div>
    <p class="muted small">Previews in the app are always silent.</p>
  </section>

  <section class="block" aria-labelledby="{uid}-play">
    <h2 class="section-label" id="{uid}-play">Playback</h2>
    <div class="grid">
      <label for="{uid}-end">At the end</label>
      <Select id="{uid}-end" class="grow" value={m.endAction} options={END_OPTIONS} onchange={(v: MediaEndAction) => patch({ endAction: v })} />
      <span></span>
      <Switch label="Play on take" checked={m.autoplay} onchange={(v) => patch({ autoplay: v })} />
    </div>
  </section>
{/if}

<style>
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4);
    border-bottom: 1px solid var(--line);
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(5rem, auto) minmax(0, 1fr);
    gap: var(--s-2) var(--s-3);
    align-items: center;
  }
  .grid :global(.grow) {
    width: 100%;
  }
  .file {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .kind {
    display: inline-flex;
    flex: none;
    color: var(--text-muted);
  }
  .file-text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }
  .file-name,
  .facts {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file-name {
    font-size: var(--fs-sm);
  }
  .facts {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .scrub-video {
    width: 100%;
    max-height: 160px;
    border-radius: var(--r-control);
    background: var(--surface-2);
    object-fit: contain;
  }
  .scrubber {
    position: relative;
    display: flex;
    align-items: center;
  }
  .range-fill {
    position: absolute;
    left: var(--in);
    width: calc(var(--out) - var(--in));
    top: 50%;
    height: 6px;
    border-radius: 3px;
    background: var(--accent);
    opacity: 0.35;
    transform: translateY(-50%);
    pointer-events: none;
  }
  .slider {
    display: flex;
    flex: 1;
    min-width: 4rem;
    border-radius: var(--r-control);
  }
  input[type="range"] {
    width: 100%;
    accent-color: var(--accent);
  }
  .times,
  .sound {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .spacer {
    flex: 1;
  }
  .time {
    display: flex;
    border-radius: var(--r-control);
  }
  .time input {
    width: 10ch;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .time input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .vol {
    width: 5ch;
    text-align: right;
  }
  .block > :global(.reset) {
    align-self: flex-start;
  }
  .label {
    font-size: var(--fs-md);
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--fs-sm);
  }
  .warn {
    margin: 0;
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .bad {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>

<!--
  StudioRow: a Control layer row's Studio mode parts (lazy studio chunk), rendered by LayerRow in
  four places (`part`):
    tallies   PGM / PVW: the Program tally (on air) and the Preview tally (shown in Preview), each a
              TallyDot whose label carries the state
    acts      Play in / Play out (Preview only, while the layer shows there) and Cue in / Cue out
              (back to "as Program" when that is what Program shows)
    notes     "Changes in preview" while edits wait in Preview; "Program changed since cue" (amber)
              when automation changed the layer on Program after it was cued
    onair     above the fields while the layer is on air: edits show in Preview until a take, with
              Update on air (this layer only, settled; asks once per session) and Discard changes

  Props:
    ctl: ControlCtl
    screenId: string
    layer: LiveLayerView        the layer on Program
    part: "tallies" | "acts" | "notes"
-->
<script lang="ts">
  import LogIn from "lucide-svelte/icons/log-in";
  import LogOut from "lucide-svelte/icons/log-out";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { previews } from "$lib/stores/preview.svelte";
  import type { LiveLayerView } from "$lib/types";
  import type { ControlCtl } from "../control.svelte";
  import { cue as cueLayer, discard, inPreview, rehearse, updateOnAir } from "./ops";

  let {
    ctl,
    screenId,
    layer,
    part,
  }: { ctl: ControlCtl; screenId: string; layer: LiveLayerView; part: "tallies" | "acts" | "notes" | "onair" } = $props();

  const pv = $derived(previews.views.get(screenId));
  const cue = $derived(pv?.cues.find((c) => c.layerId === layer.id));
  const shown = $derived(inPreview(pv, layer.id, layer.onAir));
</script>

{#if part === "tallies"}
  <span class="tallies">
    <span class="tal" title="Program">
      <TallyDot state={layer.onAir ? "program" : "idle"} label={layer.onAir ? "On air" : "Off air"} />PGM
    </span>
    <span class="tal" title="Preview">
      <TallyDot state={shown ? "preview" : "idle"} label={shown ? "In Preview" : "Not in Preview"} />PVW
    </span>
  </span>
{:else if part === "acts"}
  {#if shown && !layer.media}
    <IconButton size="sm" label="Play in {layer.name} in Preview" tooltip="left" onclick={() => rehearse(screenId, [layer.id], "in")}>
      <LogIn size={14} />
    </IconButton>
    <IconButton size="sm" label="Play out {layer.name} in Preview" tooltip="left" onclick={() => rehearse(screenId, [layer.id], "out")}>
      <LogOut size={14} />
    </IconButton>
  {/if}
  <Button
    size="sm"
    variant={shown ? "secondary" : "primary"}
    disabled={!ctl.studio.on}
    onclick={() => cueLayer(screenId, layer.id)}
    aria-label="{shown ? 'Cue out' : 'Cue in'} {layer.name}"
    title={shown ? "Take it out of Preview" : "Put it in Preview"}
  >
    {shown ? "Cue out" : "Cue in"}
  </Button>
{:else if part === "notes"}
  {#if cue?.staged}<p class="note">Changes in preview</p>{/if}
  {#if cue?.programChanged}<p class="note changed">Program changed since cue</p>{/if}
{:else if layer.onAir}
  <div class="onair" role="status">
    <TallyDot state="program" label="On air" />
    <p>On air. Your changes show in Preview until you take.</p>
    <span class="onair-acts">
      <Button size="sm" disabled={!cue?.staged} onclick={() => updateOnAir(screenId, layer.id, layer.name)}>Update on air</Button>
      <Button size="sm" variant="danger" disabled={!cue?.staged} onclick={() => discard(screenId, layer.id, layer.name)}>Discard changes</Button>
    </span>
  </div>
{/if}

<style>
  .tallies {
    display: inline-flex;
    flex: none;
    flex-direction: column;
    gap: 2px;
  }
  .tal {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--text-muted);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: var(--tracking-label);
    line-height: 1;
  }
  .note {
    margin: 0;
    /* Lines up with the layer name: chevron, gap, tallies (~32px), gap. */
    padding: 0 var(--s-2) var(--s-1) calc(var(--control-h-sm) + var(--s-2) + 32px + var(--s-2));
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
  .changed {
    color: var(--warning);
  }
  .onair {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1) var(--s-2);
    margin: 0 var(--s-2) var(--s-1);
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--program-line);
  }
  .onair p {
    flex: 1 1 12rem;
    margin: 0;
    font-size: var(--fs-xs);
  }
  .onair-acts {
    display: inline-flex;
    gap: var(--s-1);
  }
</style>

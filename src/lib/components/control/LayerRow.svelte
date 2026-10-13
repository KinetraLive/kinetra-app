<!--
  LayerRow: one layer of the selected screen in the Control layers panel.

  Head: expand chevron, on-air tally (with label), name, template name, mono hotkey badge, a "MIDI"
  chip when a MIDI control is assigned (the source in its tooltip), TAKE / CLEAR. Right-click (or
  Shift+F10 / the menu key on the row) opens the lazy context menu (the page's side panel handles
  it, through LayerBody's chunk: midi/eager.svelte.ts sideMenu): Assign MIDI…, Remove MIDI, Set hotkey. "Changes on next take" under it while edits are staged.
  Studio mode: the studio chunk's StudioRow parts instead (PGM / PVW tallies, Cue in / Cue out,
  "In preview" and "Program changed since cue"); the fields show the layer as it is in Preview.
  Expanded: LayerBody (fields, variant, hotkey; a chunk of its own, requested when Control loads).
  Media layers: the file name under the name and the transport (Play / Pause, Stop, Restart, times,
  readiness) from the same chunk (MediaTransport, mounted on every row: template layers with video /
  image fields get just the readiness chip, other layers nothing).

  Props:
    ctl: ControlCtl
    screenId: string
    layer: LiveLayerView
-->
<script module lang="ts">
  /** The expanded area's chunk (LayerBody, with Control's variant support), requested when
   *  Control loads; null until it arrives. */
  export const lazy = $state<{ body: typeof import("./LayerBody.svelte") | null }>({ body: null });
  import("./LayerBody.svelte").then(
    (m) => (lazy.body = m),
    () => {},
  );
</script>

<script lang="ts">
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Button from "$lib/components/Button.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import type { LiveLayerView } from "$lib/types";
  import type { ControlCtl } from "./control.svelte";
  import { hotkeyKey } from "./hotkeys";

  let { ctl, screenId, layer }: { ctl: ControlCtl; screenId: string; layer: LiveLayerView } = $props();

  const uid = $props.id();
  const key = $derived(hotkeyKey(screenId, layer.id));
  const open = $derived(ctl.expanded.has(key));
  const hotkey = $derived(ctl.ui.hotkeys[key] ?? null);
  const template = $derived(ctl.templateOf(layer.id));
  /** Studio mode's row parts (null while off or loading). */
  const studio = $derived(ctl.studio.on ? ctl.studio.ui : null);
  /** The layer as the fields show it: in Studio mode, as it is in Preview. */
  const edited = $derived(studio?.previews.views.get(screenId)?.layers.find((l) => l.id === layer.id) ?? layer);
</script>

<li class="row" data-layer={layer.id}>
  <div class="head">
    <button
      type="button"
      class="expand vf"
      class:open
      aria-expanded={open}
      aria-controls={open ? `${uid}-body` : undefined}
      aria-label="Fields and hotkey of {layer.name}"
      onclick={() => ctl.toggleExpanded(key)}
    >
      <ChevronRight size={14} />
    </button>
    {#if studio}
      <studio.StudioRow {ctl} {screenId} {layer} part="tallies" />
    {:else}
      <TallyDot state={layer.onAir ? "program" : "idle"} label={layer.onAir ? "On air" : "Off air"} />
    {/if}
    <span class="names">
      <span class="name">{layer.name}</span>
      {#if template}<span class="tpl">{template.name}</span>{:else if layer.media && lazy.body}<lazy.body.MediaTransport {ctl} {screenId} {layer} part="file" />{/if}
    </span>
    {#if hotkey}<span class="badge mono" title="Hotkey">{hotkey}</span>{/if}
    {#if lazy.body}<lazy.body.MidiChip k={key} />{/if}
    {#if lazy.body}<lazy.body.MediaTransport {ctl} {screenId} {layer} part="transport" />{/if}
    <span class="acts">
      {#if studio}
        <studio.StudioRow {ctl} {screenId} {layer} part="acts" />
      {:else if !ctl.studio.on}
        <Button size="sm" variant="accent" onclick={() => ctl.take(screenId, layer.id)} aria-label="Take {layer.name}">
          Take
        </Button>
        <Button size="sm" disabled={!layer.onAir} onclick={() => ctl.clear(screenId, layer.id)} aria-label="Clear {layer.name}">
          Clear
        </Button>
      {/if}
    </span>
  </div>
  {#if studio}
    <studio.StudioRow {ctl} {screenId} {layer} part="notes" />
  {:else if layer.pending && !ctl.studio.on}<p class="pending">Changes on next take</p>{/if}

  {#if open && lazy.body}
    <lazy.body.default {ctl} {screenId} layer={edited} id="{uid}-body" />
  {/if}
</li>

<style>
  .row {
    display: flex;
    flex-direction: column;
    border-radius: var(--r-control);
  }
  .row:hover {
    background: var(--surface-2);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    padding: var(--s-1) var(--s-1) var(--s-1) 0;
  }
  .expand {
    --vf-radius: var(--r-control);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: var(--control-h-sm);
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    color: var(--text-muted);
  }
  .expand:hover {
    color: var(--text);
  }
  .expand :global(svg) {
    transition: transform var(--dur-fast) var(--ease);
  }
  .expand.open :global(svg) {
    transform: rotate(90deg);
  }
  .names {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }
  .name,
  .tpl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tpl {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .badge {
    flex: none;
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
  .acts {
    display: inline-flex;
    flex: none;
    gap: var(--s-1);
  }
  .pending {
    margin: 0;
    padding: 0 var(--s-2) var(--s-1) calc(var(--control-h-sm) + var(--s-2) + 8px + var(--s-2));
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
  @media (prefers-reduced-motion: reduce) {
    .expand :global(svg) {
      transition: none;
    }
  }
</style>

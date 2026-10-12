<!--
  LayerBody: the expanded area of a Control layer row (a chunk of its own, requested when Control
  loads, so the route's initial JS stays small).

  FieldInputs (edits debounced per layer; bound fields read-only with the source and column, plus
  Prev/Next that step the source; a list field is one summary line that opens the spreadsheet
  drawer: on its own rows (edits go through `ctl.field`, so setLayerFields debounced 150 ms, which
  follows Studio mode and Live edit server-side) or, linked, on the data table (saved to the
  source; the source list reloads after each save); timer fields' Start / Pause / Reset / Set go straight to
  `setTimer`, no debounce); with a template of several variants the Variant select and its
  transition (LayerVariant); Opacity (0–100 %, saved to the screen ~150 ms after the last move,
  so the live rules apply: on air with "On take" it waits for TAKE); and the hotkey: "Set hotkey" waits for the next F-key (Shift
  allowed, Esc cancels), "Remove hotkey"; when the key is also bound to another command (Preferences >
  Keyboard shortcuts) a warning says so and links there: the key runs nothing until one changes.

  Props:
    ctl: ControlCtl
    screenId: string
    layer: LiveLayerView
    id: string            the body's element id (the row's chevron controls it)
-->
<script module lang="ts">
  // Control's variant support lives with this chunk: ControlLayers registers the palette commands
  // through it, so the route has a single lazy import for both.
  export { registerVariantCommands } from "./variants.svelte";
  // Control's MIDI part (chips, mappings, the side panel's context menu) rides this chunk too.
  export { sideMenu, watchControls } from "$lib/components/midi/eager.svelte";
  export { default as MidiChip } from "$lib/components/midi/MidiChip.svelte";
  // Media layers' row parts (file name, transport, readiness) ride this chunk too.
  export { default as MediaTransport } from "$lib/components/media/MediaTransport.svelte";
</script>

<script lang="ts">
  import ChevronLeft from "lucide-svelte/icons/chevron-left";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Keyboard from "lucide-svelte/icons/keyboard";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import { LAYER_COMMAND } from "$lib/commands/bindings.svelte";
  import { conflicts } from "$lib/commands/conflicts";
  import { getCommand, titleOf } from "$lib/commands/registry.svelte";
  import { t } from "$lib/i18n";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { preferencesLink } from "$lib/components/settings/open";
  import FieldInputs from "$lib/components/fields/FieldInputs.svelte";
  import type { LinkedList } from "$lib/components/fields/drawer";
  import type { FieldDef, LiveLayerView } from "$lib/types";
  import type { ControlCtl } from "./control.svelte";
  import { hotkeyKey } from "./hotkeys";
  import LayerVariant from "./LayerVariant.svelte";
  import OpacityInput from "$lib/components/inputs/OpacityInput.svelte";
  import { LayerOpacity } from "./opacity.svelte";

  let { ctl, screenId, layer, id }: { ctl: ControlCtl; screenId: string; layer: LiveLayerView; id: string } = $props();

  const key = $derived(hotkeyKey(screenId, layer.id));
  const hotkey = $derived(ctl.ui.hotkeys[key] ?? null);
  const capturing = $derived(ctl.capturing === key);
  /** The other commands sharing this layer's key (a conflict: the key runs nothing). */
  const clash = $derived.by(() => {
    if (!hotkey) return null;
    const self = LAYER_COMMAND + key;
    const c = conflicts().find((x) => x.commandIds.includes(self));
    if (!c) return null;
    const others = c.commandIds.filter((id) => id !== self).map((id) => {
      const cmd = getCommand(id);
      return cmd ? titleOf(cmd) : t("another command");
    });
    return { keys: c.keys, others: others.join(", ") };
  });
  const template = $derived(ctl.templateOf(layer.id));
  const opacity = new LayerOpacity(() => ctl);
  $effect(() => () => void opacity.flush());
  const bound = $derived(ctl.boundLabels(layer.id));
  const values = $derived(ctl.values(screenId, layer));
  /** Bound list fields → the source they are linked to (the summary's "Linked to: …"). */
  const linked = $derived.by(() => {
    const out: Record<string, LinkedList> = {};
    for (const f of layer.fieldDefs) {
      if (f.type !== "list" || bound[f.key] === undefined) continue;
      const s = ctl.boundSource(layer.id, f.key);
      if (!s) continue;
      out[f.key] = {
        sourceId: s.id,
        name: s.name,
        kind: s.kind,
        usedBy: s.usedBy,
        detail: bound[f.key],
        onsaved: () => void ctl.loadSources(),
      };
    }
    return out;
  });
</script>

{#snippet stepper(f: FieldDef)}
  {#if bound[f.key] !== undefined}
    {@const src = ctl.boundSource(layer.id, f.key)}
    <span class="step">
      <IconButton
        size="sm"
        label="Previous row of {src?.name ?? 'the source'}"
        tooltip="top"
        disabled={!src || src.index <= 0}
        onclick={() => src && ctl.step(src.id, -1)}
      >
        <ChevronLeft size={14} />
      </IconButton>
      <IconButton
        size="sm"
        label="Next row of {src?.name ?? 'the source'}"
        tooltip="top"
        disabled={!src || src.index >= src.rowCount - 1}
        onclick={() => src && ctl.step(src.id, 1)}
      >
        <ChevronRight size={14} />
      </IconButton>
    </span>
  {/if}
{/snippet}

<div class="body" {id}>
  {#if layer.fieldDefs.length > 0}
    <FieldInputs
      defs={layer.fieldDefs}
      {values}
      {bound}
      {linked}
      onfield={(k, v) => ctl.field(screenId, layer.id, k, v)}
      onfocus={(k) => ctl.focus(screenId, layer.id, k)}
      onblur={(k) => ctl.blur(screenId, layer.id, k)}
      ontimer={(k, action) => ctl.timer(screenId, layer.id, k, action)}
      extra={stepper}
    />
  {:else}
    <p class="muted">This template has no fields.</p>
  {/if}
  {#if template && template.variantCount > 1}
    <LayerVariant {screenId} {layer} {template} />
  {/if}
  <div class="opacity">
    <label class="section-label" for="{id}-opacity">Opacity</label>
    <OpacityInput id="{id}-opacity" value={opacity.value(screenId, layer.id)} onvalue={(n) => opacity.set(screenId, layer.id, n)} />
  </div>
  <div class="hotkey">
    <span class="section-label">Hotkey</span>
    {#if capturing}
      <span class="capture" role="status">Press F1–F12 (Shift optional). Esc cancels.</span>
      <Button size="sm" variant="ghost" onclick={() => ctl.startCapture(key)}>Cancel</Button>
    {:else}
      <span class="current" class:mono={hotkey}>{hotkey ?? "None"}</span>
      <Button size="sm" onclick={() => ctl.startCapture(key)} aria-label="Set hotkey for {layer.name}">
        {#snippet icon()}<Keyboard size={14} />{/snippet}
        {hotkey ? "Change" : "Set hotkey"}
      </Button>
      {#if hotkey}
        <Button size="sm" variant="ghost" onclick={() => ctl.removeHotkey(key)} aria-label="Remove hotkey of {layer.name}">
          Remove
        </Button>
      {/if}
    {/if}
  </div>
  {#if clash && !capturing}
    <p class="clash" role="alert">
      <TriangleAlert size={12} aria-hidden="true" />
      <span>
        {t("{keys} runs nothing: it is also assigned to {commands}.", { keys: clash.keys, commands: clash.others })}
        <a href="/settings#shortcuts" onclick={preferencesLink("shortcuts")}>{t("Change one in Keyboard shortcuts")}</a>
      </span>
    </p>
  {/if}
</div>

<style>
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-2) var(--s-2) var(--s-3) var(--s-3);
    border-top: 1px solid var(--line);
  }
  .step {
    display: inline-flex;
    flex: none;
  }
  .muted {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .opacity {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    max-width: 24rem;
  }
  .hotkey {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .current {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .capture {
    color: var(--accent-text);
    font-size: var(--fs-sm);
  }
  .clash {
    display: flex;
    gap: var(--s-1);
    margin: calc(var(--s-2) * -1) 0 0;
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .clash :global(svg) {
    flex: none;
    margin-top: 3px;
  }
  .clash a {
    color: var(--accent-text);
  }
</style>

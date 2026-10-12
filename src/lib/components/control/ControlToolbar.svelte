<!--
  ControlToolbar: the Control page's top bar.
    Grid / Single · Columns (grid only) · Screens menu (show / hide) · Studio mode (a switch; its
    settings button comes with the studio chunk) · Live edit On take / Immediate (in Studio mode: "Edits go to
    Preview")
    · Clear all (selected screen, animated) + menu "Clear every screen" · Automation off (a switch;
    while on, an amber pill with the text) · PANIC (instant cut, its key Shift+Esc unless rebound, no confirm).
  Every choice is written to the project's ui-state through the controller.

  Props:
    ctl: ControlCtl
-->
<script lang="ts">
  import Eraser from "lucide-svelte/icons/eraser";
  import EyeOff from "lucide-svelte/icons/eye-off";
  import OctagonAlert from "lucide-svelte/icons/octagon-alert";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import type { ControlView, LiveEdit } from "$lib/types";
  import { GRID_COLUMNS, onAirCount, type ControlCtl } from "./control.svelte";
  import { keysFor } from "$lib/commands/bindings.svelte";
  import { ariaKeys, shortKeys } from "$lib/commands/keys";
  import Menu, { type MenuItem } from "./Menu.svelte";
  import Segmented from "./Segmented.svelte";

  let { ctl }: { ctl: ControlCtl } = $props();

  const uid = $props.id();
  /** PANIC's key (Shift+Escape unless rebound in Keyboard shortcuts). */
  const panicKey = $derived(keysFor("live.panic")[0] ?? null);

  const VIEWS: { value: ControlView; label: string }[] = [
    { value: "grid", label: "Grid" },
    { value: "single", label: "Single" },
  ];
  const EDITS: { value: LiveEdit; label: string }[] = [
    { value: "onTake", label: "On take" },
    { value: "immediate", label: "Immediate" },
  ];

  const hiddenCount = $derived(ctl.screens.filter((s) => ctl.hidden.has(s.screen.id)).length);
  const screenItems = $derived<MenuItem[]>([
    ...ctl.screens.map((s) => ({
      id: s.screen.id,
      label: s.screen.name,
      checked: !ctl.hidden.has(s.screen.id),
      meta: `${s.screen.width}×${s.screen.height}`,
      metaMono: true,
      onselect: () => ctl.setHidden(s.screen.id, !ctl.hidden.has(s.screen.id)),
    })),
    ...(hiddenCount > 0 ? [{ id: "__all", label: "Show all screens", onselect: ctl.showAll }] : []),
  ]);
  const anyOnAir = $derived(ctl.screens.some((s) => onAirCount(s) > 0));
  const clearItems = $derived<MenuItem[]>([
    { id: "every", label: "Clear every screen", disabled: !anyOnAir, onselect: ctl.clearEvery },
  ]);
  const sel = $derived(ctl.selected);
  const studio = $derived(ctl.studio);
</script>

<div class="bar">
  <Segmented label="View" options={VIEWS} value={ctl.ui.view} onchange={ctl.setView} />

  {#if ctl.ui.view === "grid"}
    <span class="group">
      <label class="lbl" for="{uid}-cols">Columns</label>
      <NumberInput
        id="{uid}-cols"
        value={ctl.ui.gridColumns}
        min={GRID_COLUMNS.min}
        max={GRID_COLUMNS.max}
        digits={1}
        onchange={ctl.setColumns}
      />
    </span>
  {/if}

  <Menu label="Show or hide screens" text="Screens" heading="Shown in Control" items={screenItems} disabled={ctl.screens.length === 0}>
    {#snippet icon()}<EyeOff size={14} />{/snippet}
  </Menu>
  {#if hiddenCount > 0}<span class="hint">{hiddenCount} hidden</span>{/if}

  <span class="sep" aria-hidden="true"></span>

  <span class="group tight">
    <span class="sw" class:on={studio.on}>
      <label for="{uid}-studio" id="{uid}-studio-label">Studio mode</label>
      <button
        id="{uid}-studio"
        type="button"
        role="switch"
        class="switch vf"
        aria-labelledby="{uid}-studio-label"
        aria-checked={studio.on}
        onclick={studio.toggle}
      >
        <span class="thumb" aria-hidden="true"></span>
      </button>
    </span>
    {#if studio.ui}<studio.ui.StudioSettingsMenu {studio} />{/if}
  </span>

  <span class="group">
    {#if studio.on}
      <span class="hint">Edits go to Preview; TAKE puts them on air</span>
    {:else}
      <span class="lbl" aria-hidden="true">Live edit</span>
      <Segmented label="Live edit" options={EDITS} value={ctl.ui.liveEdit} onchange={ctl.setLiveEdit} />
      <span class="hint">
        {ctl.ui.liveEdit === "onTake" ? "Edits to on-air layers wait for Take" : "Edits go to air at once"}
      </span>
    {/if}
  </span>

  <span class="spacer"></span>

  <span class="group tight">
    <Button
      disabled={!sel || onAirCount(sel) === 0}
      onclick={() => sel && ctl.clearScreen(sel.screen.id)}
      aria-label={sel ? `Clear all on ${sel.screen.name}` : "Clear all"}
    >
      {#snippet icon()}<Eraser size={14} />{/snippet}
      Clear all
    </Button>
    <Menu label="More clear options" items={clearItems} align="end" />
  </span>

  <!-- The Switch component's look, inline: this bar is in the route's initial chunk. -->
  <span class="auto" class:on={ctl.ui.automationOff} title="Stops automatic triggers; cues and hotkeys still work">
    <label for="{uid}-auto" id="{uid}-auto-label">Automation off</label>
    <button
      id="{uid}-auto"
      type="button"
      role="switch"
      class="switch vf"
      aria-labelledby="{uid}-auto-label"
      aria-checked={ctl.ui.automationOff}
      onclick={() => ctl.setAutomationOff(!ctl.ui.automationOff)}
    >
      <span class="thumb" aria-hidden="true"></span>
    </button>
  </span>

  <Button
    variant="danger"
    class="panic"
    onclick={ctl.panic}
    aria-keyshortcuts={panicKey ? ariaKeys(panicKey) : undefined}
    title="Cut everything off air at once"
  >
    {#snippet icon()}<OctagonAlert size={14} />{/snippet}
    Panic {#if panicKey}<span class="key mono">{shortKeys(panicKey)}</span>{/if}
  </Button>
</div>

<style>
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    min-height: calc(var(--row-h) + var(--s-4));
    padding: var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .group {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .group.tight {
    gap: var(--s-1);
  }
  .lbl {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .sep {
    width: 1px;
    height: var(--s-4);
    background: var(--line);
  }
  .spacer {
    flex: 1;
  }
  .auto {
    display: inline-flex;
    align-items: center;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-full);
    font-size: var(--fs-sm);
  }
  .auto,
  .sw {
    gap: var(--s-2);
  }
  .sw {
    display: inline-flex;
    align-items: center;
    font-size: var(--fs-sm);
  }
  .sw.on {
    color: var(--accent-text);
  }

  .auto.on {
    color: var(--warning);
    box-shadow: inset 0 0 0 1px var(--warning);
  }
  .switch {
    --vf-radius: var(--r-full);
    position: relative;
    width: 32px;
    height: 18px;
    border-radius: var(--r-full);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .switch[aria-checked="true"] {
    background: var(--accent);
    box-shadow: none;
  }
  .thumb {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 12px;
    height: 12px;
    border-radius: var(--r-full);
    background: var(--text);
    transition: transform var(--dur) var(--ease);
  }
  .switch[aria-checked="true"] .thumb {
    background: var(--accent-ink);
    transform: translateX(14px);
  }
  .key {
    font-size: var(--fs-xs);
    opacity: 0.8;
  }

  /* PANIC is an emergency control: a filled --danger button, not the quiet danger outline. */
  .bar :global(.btn.panic) {
    background: var(--danger);
    box-shadow: none;
    color: var(--danger-ink);
  }
  .bar :global(.btn.panic:hover:not(:disabled)) {
    background: var(--danger-hover);
    box-shadow: none;
  }
  .bar :global(.btn.panic:active:not(:disabled)) {
    background: var(--danger);
  }
</style>

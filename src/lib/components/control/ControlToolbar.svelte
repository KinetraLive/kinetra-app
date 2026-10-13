<!--
  ControlToolbar: the Control page's top bar.
    Grid / Single · Columns (grid only) · Screens menu (show / hide) · Studio mode (a switch; its
    settings button comes with the studio chunk) · Live edit On take / Immediate (in Studio mode: "Edits go to
    Preview")
    · Clear screen (the selected screen, animated) + menu "Clear every screen" · Automation off (a switch; while on, an amber pill with the
    text) · PANIC (instant cut, its key Shift+Esc unless rebound, no confirm).
  Every choice is written to the project's ui-state through the controller.

  One line, always: PANIC never wraps. When the bar is too narrow it collapses step by step, the
  least important first: (1) the hint texts, (2) Columns, (3) View and Live edit, (4) the Screens
  menu's text (its icon stays). What collapsed (Columns, View, Live edit) moves into a "More"
  menu. The fit is measured only when the bar's width or what it holds changes (one
  ResizeObserver; layout reads then, never at rest).

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
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import { tick } from "svelte";

  let { ctl }: { ctl: ControlCtl } = $props();

  /** How far the bar has collapsed (see the header): 0 = everything shows. */
  let level = $state(0);
  const MAX_LEVEL = 4;
  let bar: HTMLDivElement | undefined = $state();
  let fitting = false;
  /** A fit was asked for while one ran: run again when it ends. */
  let again = false;
  /** Finds the smallest collapse level at which the bar fits on one line. */
  async function fit() {
    if (!bar) return;
    if (fitting) {
      again = true;
      return;
    }
    fitting = true;
    try {
      do {
        again = false;
        for (let l = 0; l <= MAX_LEVEL; l++) {
          level = l;
          await tick();
          if (!bar || bar.scrollWidth <= bar.clientWidth) break;
        }
      } while (again);
      fittedWidth = bar?.clientWidth ?? 0;
      observe();
    } finally {
      fitting = false;
    }
  }
  /** The bar's width at the last fit: wider again lets collapsed controls come back. */
  let fittedWidth = 0;
  let ro: ResizeObserver | undefined;
  $effect(() => {
    if (!bar) return;
    let frame = 0;
    // The bar and its controls: a control growing (a key label, the studio menu, fonts) re-fits
    // when the bar then overflows; the bar growing re-fits so collapsed controls can come back.
    // Collapsing only shrinks the controls, so a fit never triggers another.
    ro = new ResizeObserver(() => {
      if (!bar) return;
      if (bar.scrollWidth <= bar.clientWidth && bar.clientWidth <= fittedWidth + 1) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => void fit());
    });
    observe();
    return () => {
      ro?.disconnect();
      ro = undefined;
      cancelAnimationFrame(frame);
    };
  });
  function observe() {
    if (!bar || !ro) return;
    ro.observe(bar);
    for (const c of bar.children) ro.observe(c);
  }
  // What the bar holds changes its width too (and so do the web fonts, once they are in).
  $effect(() => {
    void [ctl.ui.view, ctl.studio.on, ctl.studio.ui, hiddenCount, ctl.ui.liveEdit, ctl.ui.automationOff, ctl.screens.length];
    void fit();
  });
  $effect(() => {
    let alive = true;
    void document.fonts?.ready.then(() => {
      if (alive) void fit();
    });
    return () => {
      alive = false;
    };
  });


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

  /** The collapsed controls, in the More menu. */
  const moreItems = $derived<MenuItem[]>([
    ...(level >= 3
      ? VIEWS.map((v) => ({
          id: `view-${v.value}`,
          label: `${v.label} view`,
          checked: ctl.ui.view === v.value,
          onselect: () => ctl.setView(v.value),
        }))
      : []),
    ...(level >= 2 && ctl.ui.view === "grid"
      ? Array.from({ length: GRID_COLUMNS.max - GRID_COLUMNS.min + 1 }, (_, i) => GRID_COLUMNS.min + i).map((n) => ({
          id: `cols-${n}`,
          label: `${n} ${n === 1 ? "column" : "columns"}`,
          checked: ctl.ui.gridColumns === n,
          onselect: () => ctl.setColumns(n),
        }))
      : []),
    ...(level >= 3 && !studio.on
      ? EDITS.map((e) => ({
          id: `edit-${e.value}`,
          label: `Live edit: ${e.label}`,
          checked: ctl.ui.liveEdit === e.value,
          onselect: () => ctl.setLiveEdit(e.value),
        }))
      : []),
  ]);
</script>

<div class="bar" bind:this={bar} data-collapse={level}>
  {#if level < 3}
    <Segmented label="View" options={VIEWS} value={ctl.ui.view} onchange={ctl.setView} />
  {/if}

  {#if ctl.ui.view === "grid" && level < 2}
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

  <Menu
    label="Show or hide screens"
    text={level < 4 ? "Screens" : undefined}
    heading="Shown in Control"
    items={screenItems}
    disabled={ctl.screens.length === 0}
  >
    {#snippet icon()}<EyeOff size={14} />{/snippet}
  </Menu>
  {#if hiddenCount > 0 && level < 1}<span class="hint">{hiddenCount} hidden</span>{/if}

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
      {#if level < 1}<span class="hint">Edits go to Preview; TAKE puts them on air</span>{/if}
    {:else if level < 3}
      <span class="lbl" aria-hidden="true">Live edit</span>
      <Segmented label="Live edit" options={EDITS} value={ctl.ui.liveEdit} onchange={ctl.setLiveEdit} />
      {#if level < 1}
        <span class="hint">
          {ctl.ui.liveEdit === "onTake" ? "Edits to on-air layers wait for Take" : "Edits go to air at once"}
        </span>
      {/if}
    {/if}
  </span>

  {#if moreItems.length > 0}
    <Menu label="More Control options" items={moreItems}>
      {#snippet icon()}<Ellipsis size={14} />{/snippet}
    </Menu>
  {/if}

  <span class="spacer"></span>

  <span class="group tight">
    <Button
      disabled={!sel || onAirCount(sel) === 0}
      onclick={() => sel && ctl.clearScreen(sel.screen.id)}
      aria-label={sel ? `Clear ${sel.screen.name}` : "Clear screen"}
    >
      {#snippet icon()}<Eraser size={14} />{/snippet}
      Clear screen
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
    flex-wrap: nowrap;
    min-width: 0;
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
    white-space: nowrap;
  }
  .bar > :global(*) {
    flex-shrink: 0;
  }
  .bar > .spacer {
    flex-shrink: 1;
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

<!--
  AutoSettings: the auto-advance settings popover of one source in the Sources strip. Lazy chunk:
  SourcesStrip imports it with a dynamic import() on first open.

  Fields: interval in seconds (mono, AUTO_ADVANCE_RANGE), Loop (wrap after the last row), and the
  transition: "Update in place" (fields change on air) or "Play out & in" (bound layers retake).
  Every change calls `onchange` at once; the controller applies it while auto-advance is on and
  remembers it for the next "Auto" while off.

  Keyboard: focus moves to the interval on open; Esc closes and refocuses the opener (the parent
  does that from `onclose`). A press outside closes it.

  Props:
    name: string           the source's name (heading, labels)
    value: AutoAdvance
    active: boolean        auto-advance is on
    onchange: (auto: AutoAdvance) => void
    onclose: (refocus: boolean) => void
-->
<script lang="ts">
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { AUTO_ADVANCE_RANGE, type AutoAdvance } from "$lib/types";
  import Segmented from "./Segmented.svelte";

  let {
    name,
    value,
    active,
    onchange,
    onclose,
  }: {
    name: string;
    value: AutoAdvance;
    active: boolean;
    onchange: (auto: AutoAdvance) => void;
    onclose: (refocus: boolean) => void;
  } = $props();

  const uid = $props.id();
  let root: HTMLDivElement | undefined = $state();

  const TRANSITIONS = [
    { value: "update", label: "Update in place" },
    { value: "retake", label: "Play out & in" },
  ] as const;

  $effect(() => {
    root?.querySelector("input")?.focus();
    const onDown = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node) && !(e.target as Element).closest?.("[data-auto-trigger]"))
        onclose(false);
    };
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && !e.shiftKey) {
      e.preventDefault();
      e.stopPropagation();
      onclose(true);
    }
  }
</script>

<div
  class="pop"
  role="dialog"
  aria-labelledby="{uid}-title"
  tabindex="-1"
  bind:this={root}
  {onkeydown}
>
  <h3 class="section-label title" id="{uid}-title">Auto-advance · {name}</h3>
  <div class="row">
    <label for="{uid}-every">Every</label>
    <NumberInput
      id="{uid}-every"
      value={value.everyMs / 1000}
      min={AUTO_ADVANCE_RANGE.min / 1000}
      max={AUTO_ADVANCE_RANGE.max / 1000}
      step={1}
      digits={4}
      suffix="s"
      onchange={(sec) => onchange({ ...value, everyMs: Math.round(sec * 1000) })}
    />
  </div>
  <div class="row">
    <Switch label="Loop" checked={value.loop} onchange={(loop) => onchange({ ...value, loop })} />
  </div>
  <div class="col">
    <span class="lbl" id="{uid}-tr">Transition</span>
    <Segmented
      label="Transition"
      options={TRANSITIONS}
      value={value.transition}
      onchange={(transition) => onchange({ ...value, transition })}
    />
  </div>
  {#if !active}<p class="hint">Used when Auto is on.</p>{/if}
</div>

<style>
  .pop {
    position: absolute;
    z-index: 20;
    top: calc(100% + var(--s-1));
    right: 0;
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    width: max-content;
    min-width: 16rem;
    padding: var(--s-3);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .pop:focus {
    outline: none;
  }
  .title {
    margin: 0;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  label,
  .lbl {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
</style>

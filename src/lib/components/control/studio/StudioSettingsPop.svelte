<!--
  StudioSettingsPop: Studio mode's settings popover in the Control toolbar (lazy studio chunk).
    Preview after take   Keep (Preview shows the new Program) / Clear (a blank Preview)
    Default transition   Cut / Crossfade / Play out & in, and the crossfade duration (ms, mono)
  Every change calls `onchange` at once (saved through set_studio). Esc closes and refocuses the
  opener (the parent does that from `onclose`); a press outside closes it.

  Props:
    settings: StudioSettings
    onchange: (patch: Partial<StudioSettings>) => void
    onclose: (refocus: boolean) => void
-->
<script lang="ts">
  import NumberInput from "$lib/components/NumberInput.svelte";
  import type { StudioSettings } from "$lib/types";
  import Segmented from "../Segmented.svelte";
  import { TAKE_KINDS, TAKE_MS, clampMs } from "./transitions";

  let {
    settings,
    onchange,
    onclose,
  }: {
    settings: StudioSettings;
    onchange: (patch: Partial<StudioSettings>) => void;
    onclose: (refocus: boolean) => void;
  } = $props();

  const uid = $props.id();
  let root: HTMLDivElement | undefined = $state();

  const AFTER = [
    { value: "keep", label: "Keep" },
    { value: "clear", label: "Clear" },
  ] as const;

  $effect(() => {
    root?.querySelector<HTMLElement>("[aria-checked=true]")?.focus();
    const onDown = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node) && !(e.target as Element).closest?.("[data-studio-trigger]"))
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

<div class="pop" role="dialog" aria-labelledby="{uid}-title" tabindex="-1" bind:this={root} {onkeydown}>
  <h3 class="section-label title" id="{uid}-title">Studio mode</h3>
  <div class="col">
    <span class="lbl" aria-hidden="true">Preview after take</span>
    <Segmented
      label="Preview after take"
      options={AFTER}
      value={settings.previewAfterTake}
      onchange={(previewAfterTake) => onchange({ previewAfterTake })}
    />
    <p class="hint">
      {settings.previewAfterTake === "keep"
        ? "Preview shows the new Program."
        : "Preview starts empty."}
    </p>
  </div>
  <div class="col">
    <span class="lbl" aria-hidden="true">Default transition</span>
    <Segmented
      label="Default transition"
      options={TAKE_KINDS}
      value={settings.transition.kind}
      onchange={(kind) => onchange({ transition: { ...settings.transition, kind } })}
    />
  </div>
  <div class="row">
    <label for="{uid}-ms">Crossfade duration</label>
    <NumberInput
      id="{uid}-ms"
      value={settings.transition.ms}
      min={TAKE_MS.min}
      max={TAKE_MS.max}
      step={50}
      digits={4}
      suffix="ms"
      disabled={settings.transition.kind !== "crossfade"}
      onchange={(ms) => onchange({ transition: { ...settings.transition, ms: clampMs(ms) } })}
    />
  </div>
  <p class="hint">Used by TAKE, Space / Enter, the palette and flows.</p>
</div>

<style>
  .pop {
    position: absolute;
    z-index: 20;
    top: calc(100% + var(--s-1));
    left: 0;
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    width: max-content;
    min-width: 18rem;
    max-width: 22rem;
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
    align-items: flex-start;
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

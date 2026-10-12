<!--
  OpacityInput: a layer's opacity, 0–100 %, as a slider plus a number (the layout editor's
  Inspector and Control's expanded layer). Presentational: `onvalue` gets every change (slider
  moves included); the caller saves (coalesced) and follows the live rules.

  Props:
    value: number              0–100 (a layer without one is 100)
    onvalue: (n: number) => void
    id?: string                for an external <label for> (the number field)
    label?: string             accessible name of the slider (default "Opacity")
-->
<script lang="ts">
  import NumberInput from "$lib/components/NumberInput.svelte";

  let { value, onvalue, id, label = "Opacity" }: { value: number; onvalue: (n: number) => void; id?: string; label?: string } = $props();

  const clamp = (n: number) => Math.max(0, Math.min(100, Math.round(n)));
</script>

<span class="opacity">
  <span class="slider vf vf-within">
    <input
      type="range"
      aria-label="{label} slider"
      min="0"
      max="100"
      step="1"
      {value}
      oninput={(e) => onvalue(clamp(Number(e.currentTarget.value)))}
    />
  </span>
  <NumberInput {id} aria-label={id ? undefined : label} {value} min={0} max={100} digits={3} suffix="%" onchange={(n) => onvalue(clamp(n))} />
</span>

<style>
  .opacity {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
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
</style>

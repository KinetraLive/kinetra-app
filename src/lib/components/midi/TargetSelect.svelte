<!--
  TargetSelect: what a new MIDI mapping does. A native select with one group per kind (Layers,
  Cues, Slides and rows, Show, Faders and knobs), styled like Select.

  Props:
    value: string                 targetValue of the chosen target ("" = none yet)
    options: TargetOption[]       in group order (model.targetOptions)
    onchange: (value: string) => void
    ...rest                       passed to <select> (aria-label, id)
-->
<script lang="ts">
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import type { HTMLSelectAttributes } from "svelte/elements";
  import { GROUP_LABEL, GROUP_ORDER, type TargetOption } from "./model";

  let {
    value,
    options,
    onchange,
    ...rest
  }: Omit<HTMLSelectAttributes, "value" | "onchange"> & {
    value: string;
    options: readonly TargetOption[];
    onchange: (value: string) => void;
  } = $props();

  const groups = $derived(
    GROUP_ORDER.map((g) => ({ g, items: options.filter((o) => o.group === g) })).filter((x) => x.items.length > 0),
  );
</script>

<span class="wrap vf vf-within">
  <select {...rest} {value} onchange={(e) => onchange(e.currentTarget.value)}>
    <option value="" disabled>Choose an action…</option>
    {#each groups as { g, items } (g)}
      <optgroup label={GROUP_LABEL[g]}>
        {#each items as o (o.value)}
          <option value={o.value}>{o.label}</option>
        {/each}
      </optgroup>
    {/each}
  </select>
  <span class="chev" aria-hidden="true"><ChevronDown size={14} /></span>
</span>

<style>
  .wrap {
    --vf-radius: var(--r-control);
    position: relative;
    display: inline-flex;
    flex: 1 1 16rem;
    max-width: 36rem;
    min-width: 0;
    border-radius: var(--r-control);
  }
  select {
    appearance: none;
    width: 100%;
    height: var(--control-h-sm);
    padding: 0 calc(var(--s-2) + 18px) 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  select:hover {
    background: var(--surface-3);
  }
  option,
  optgroup {
    background: var(--surface-3);
    color: var(--text);
  }
  .chev {
    position: absolute;
    right: var(--s-2);
    top: 50%;
    display: inline-flex;
    color: var(--text-muted);
    transform: translateY(-50%);
    pointer-events: none;
  }
</style>

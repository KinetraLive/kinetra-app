<!--
  Numeric input that clamps to [min, max] and snaps to `step` when committed (change/blur/Enter).
  Invalid input reverts to the current value. Arrow keys step natively; the spin buttons are hidden.
  Values render in the mono face with tabular figures.

  Props:
    value: number        bindable: <NumberInput bind:value={mins} min={1} max={240} suffix="min" />
    min?: number
    max?: number
    step?: number                                           (default 1)
    digits?: number      size the field to fit this many digits, e.g. digits={5} for a port
                         (width = digits x 1ch of the mono face + padding). Default ~7ch.
    suffix?: string      unit text shown inside the field, e.g. "min"
    id?: string          for an external <label for>
    disabled?: boolean
    onchange?: (value: number) => void   called with the committed (clamped) value
    ...rest              passed to <input> (aria-label, name, ...); `class` goes on the wrapper

  The wrapper carries the focus ring (`vf vf-within`).
-->
<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";

  type Props = Omit<HTMLInputAttributes, "value" | "onchange" | "min" | "max" | "step" | "type"> & {
    value: number;
    min?: number;
    max?: number;
    step?: number;
    digits?: number;
    suffix?: string;
    id?: string;
    disabled?: boolean;
    onchange?: (value: number) => void;
  };

  let {
    value = $bindable(),
    min,
    max,
    step = 1,
    digits,
    suffix,
    id,
    disabled = false,
    onchange,
    class: className = "",
    ...rest
  }: Props = $props();

  function clampValue(n: number): number {
    let v = n;
    if (step > 0) {
      const base = min ?? 0;
      v = base + Math.round((v - base) / step) * step;
      // Avoid float noise like 0.30000000000000004.
      const decimals = (String(step).split(".")[1] ?? "").length;
      v = Number(v.toFixed(decimals));
    }
    if (min !== undefined) v = Math.max(min, v);
    if (max !== undefined) v = Math.min(max, v);
    return v;
  }

  function commit(e: Event & { currentTarget: HTMLInputElement }) {
    const input = e.currentTarget;
    const raw = input.value.trim() === "" ? NaN : Number(input.value);
    const next = Number.isFinite(raw) ? clampValue(raw) : value;
    input.value = String(next);
    if (next !== value) {
      value = next;
      onchange?.(next);
    }
  }

  function onkeydown(e: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (e.key === "Enter") commit(e);
  }
</script>

<span class="wrap vf vf-within {className}" class:disabled class:has-suffix={!!suffix}>
  <input
    {...rest}
    {id}
    class="no-spin"
    class:sized={digits !== undefined}
    style:--digits={digits}
    type="number"
    inputmode="decimal"
    {min}
    {max}
    {step}
    {disabled}
    {value}
    onchange={commit}
    onblur={commit}
    {onkeydown}
  />
  {#if suffix}<span class="suffix" aria-hidden="true">{suffix}</span>{/if}
</span>

<style>
  .wrap {
    --vf-radius: var(--r-control);
    --pad: var(--s-2);
    position: relative;
    display: inline-flex;
    align-items: center;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
  }
  .wrap.disabled {
    opacity: 0.5;
  }
  input {
    width: calc(7ch + 2 * var(--pad));
    min-width: 0;
    height: 100%;
    padding: 0 var(--pad);
    border: 0;
    background: transparent;
    font-family: var(--font-mono);
    font-size: var(--fs-md);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0;
    text-align: right;
  }
  input.sized {
    width: calc(var(--digits) * 1ch + 2 * var(--pad));
  }
  .has-suffix input {
    padding-right: var(--s-1);
  }
  .has-suffix input.sized {
    width: calc(var(--digits) * 1ch + var(--pad) + var(--s-1));
  }
  /* Spin buttons hidden; arrow keys still step (native for type=number). */
  .no-spin {
    appearance: textfield;
    -moz-appearance: textfield;
  }
  .no-spin::-webkit-inner-spin-button,
  .no-spin::-webkit-outer-spin-button {
    -webkit-appearance: none;
    appearance: none;
    display: none;
    margin: 0;
  }
  .suffix {
    padding-right: var(--pad);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>

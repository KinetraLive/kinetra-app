<!--
  Styled native <select> with a lucide chevron. Generic over the option value type (string literals
  keep their type). The wrapper carries the focus ring (`vf vf-within`); `class` goes on it.

  Props:
    value: T             bindable: <Select bind:value={pref} options={opts} id="theme" />
    options: { value: T; label: string; disabled?: boolean }[]
    id?: string          for an external <label for> (e.g. SettingsRow `for`)
    disabled?: boolean
    size?: "sm" | "md"                                      (default "md")
    onchange?: (value: T) => void   called after a user change
    ...rest              passed to <select> (aria-label, name, ...)
-->
<script lang="ts" generics="T extends string">
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import type { HTMLSelectAttributes } from "svelte/elements";

  type Option = { value: T; label: string; disabled?: boolean };
  type Props = Omit<HTMLSelectAttributes, "value" | "onchange" | "size"> & {
    value: T;
    options: Option[];
    id?: string;
    disabled?: boolean;
    size?: "sm" | "md";
    onchange?: (value: T) => void;
  };

  let {
    value = $bindable(),
    options,
    id,
    disabled = false,
    size = "md",
    onchange,
    class: className = "",
    ...rest
  }: Props = $props();

  function handle(e: Event & { currentTarget: HTMLSelectElement }) {
    value = e.currentTarget.value as T;
    onchange?.(value);
  }
</script>

<span class="wrap vf vf-within {size} {className}" class:disabled>
  <select {...rest} {id} {disabled} {value} onchange={handle}>
    {#each options as o (o.value)}
      <option value={o.value} disabled={o.disabled}>{o.label}</option>
    {/each}
  </select>
  <span class="chev" aria-hidden="true"><ChevronDown size={14} /></span>
</span>

<style>
  .wrap {
    --vf-radius: var(--r-control);
    position: relative;
    display: inline-flex;
    min-width: 0;
    border-radius: var(--r-control);
  }
  select {
    appearance: none;
    width: 100%;
    height: var(--control-h);
    padding: 0 calc(var(--s-3) + 18px) 0 var(--s-3);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--fs-md);
    cursor: pointer;
  }
  .sm select {
    height: var(--control-h-sm);
    padding: 0 calc(var(--s-2) + 18px) 0 var(--s-2);
    font-size: var(--fs-sm);
  }
  select:hover:not(:disabled) {
    background: var(--surface-3);
  }
  select:disabled {
    opacity: 0.5;
    cursor: default;
  }
  option {
    background: var(--surface-3);
    color: var(--text);
  }
  .chev {
    position: absolute;
    right: var(--s-2);
    top: 50%;
    transform: translateY(-50%);
    display: inline-flex;
    color: var(--text-muted);
    pointer-events: none;
  }
  .disabled .chev {
    opacity: 0.5;
  }
</style>

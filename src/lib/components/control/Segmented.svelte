<!--
  Segmented: a compact single-choice toggle (a radio group of buttons), e.g. Grid / Single.

  Keyboard: Tab reaches the checked option; ArrowLeft/ArrowRight (and Up/Down) move and choose,
  Home/End jump. Selection is accent text on --surface-3 (no tally colours).

  Props:
    label: string                            accessible name of the group
    options: { value: T; label: string }[]
    value: T
    onchange: (value: T) => void
-->
<script lang="ts" generics="T extends string">
  let {
    label,
    options,
    value,
    onchange,
  }: {
    label: string;
    options: readonly { value: T; label: string }[];
    value: T;
    onchange: (value: T) => void;
  } = $props();

  let root: HTMLDivElement | undefined = $state();

  function onkeydown(e: KeyboardEvent) {
    const i = options.findIndex((o) => o.value === value);
    let next = -1;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") next = (i + 1) % options.length;
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") next = (i - 1 + options.length) % options.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = options.length - 1;
    if (next < 0) return;
    e.preventDefault();
    onchange(options[next].value);
    root?.querySelectorAll<HTMLButtonElement>("button")[next]?.focus();
  }
</script>

<div class="seg" role="radiogroup" aria-label={label} bind:this={root}>
  {#each options as o (o.value)}
    {@const on = o.value === value}
    <button
      type="button"
      class="opt vf"
      role="radio"
      aria-checked={on}
      tabindex={on ? 0 : -1}
      onclick={() => onchange(o.value)}
      {onkeydown}
    >
      {o.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    flex: none;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .opt {
    --vf-radius: calc(var(--r-control) - 2px);
    height: calc(var(--control-h) - 4px);
    padding: 0 var(--s-3);
    border-radius: calc(var(--r-control) - 2px);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: 500;
    white-space: nowrap;
  }
  .opt:hover {
    color: var(--text);
  }
  .opt[aria-checked="true"] {
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
  }
</style>

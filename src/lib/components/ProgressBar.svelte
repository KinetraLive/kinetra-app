<!--
  Progress bar (role="progressbar"). Animates with transform: scaleX, never width.

  Props:
    value?: number       omit (or set `indeterminate`) for an indeterminate bar
    max?: number                                             (default 100)
    indeterminate?: boolean
    label?: string       accessible name                      (default "Progress")
    valueText?: string   aria-valuetext, e.g. "12 of 40 files"
    tone?: "accent" | "success" | "warning" | "danger"       (default "accent"; success = accent)
    ...rest              passed to the outer element
-->
<script lang="ts">
  import type { HTMLAttributes } from "svelte/elements";

  type Props = HTMLAttributes<HTMLDivElement> & {
    value?: number;
    max?: number;
    indeterminate?: boolean;
    label?: string;
    valueText?: string;
    tone?: "accent" | "success" | "warning" | "danger";
  };

  let {
    value,
    max = 100,
    indeterminate = false,
    label = "Progress",
    valueText,
    tone = "accent",
    class: className = "",
    ...rest
  }: Props = $props();

  const isIndeterminate = $derived(indeterminate || value === undefined || !Number.isFinite(value));
  const ratio = $derived(
    isIndeterminate || max <= 0 ? 0 : Math.min(1, Math.max(0, (value as number) / max)),
  );
</script>

<div
  {...rest}
  class="track {tone} {className}"
  role="progressbar"
  aria-label={label}
  aria-valuemin={isIndeterminate ? undefined : 0}
  aria-valuemax={isIndeterminate ? undefined : max}
  aria-valuenow={isIndeterminate ? undefined : Math.min(max, Math.max(0, value as number))}
  aria-valuetext={valueText}
  aria-busy={isIndeterminate || undefined}
>
  {#if isIndeterminate}
    <div class="fill indeterminate"></div>
  {:else}
    <div class="fill" style:transform="scaleX({ratio})"></div>
  {/if}
</div>

<style>
  .track {
    --c: var(--accent);
    position: relative;
    width: 100%;
    height: 4px;
    border-radius: var(--r-full);
    background: var(--surface-3);
    overflow: hidden;
  }
  /* No success green: completion reads as the accent. Never a tally colour. */
  .success {
    --c: var(--accent);
  }
  .warning {
    --c: var(--warning);
  }
  .danger {
    --c: var(--danger);
  }
  .fill {
    position: absolute;
    inset: 0;
    background: var(--c);
    border-radius: inherit;
    transform-origin: left center;
    transition: transform var(--dur-slow) var(--ease);
  }
  .fill.indeterminate {
    right: auto;
    width: 40%;
    animation: slide var(--period-indeterminate) var(--ease) infinite;
  }
  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }
</style>

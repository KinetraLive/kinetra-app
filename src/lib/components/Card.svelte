<!--
  Flush panel. Static <div> by default; `interactive` makes it a <button> (or an <a> with `href`).
  Interactive cards brighten to --surface-2 on hover; keyboard focus draws the focus ring and
  selection the selection border (--selected-bg + a 1px accent edge).

  Props:
    interactive?: boolean
    href?: string        with `interactive`: render as a link
    disabled?: boolean   interactive only: sets disabled / aria-disabled, no hover
    selected?: boolean   draws the selection border (sets data-selected)
    padding?: "none" | "sm" | "md" | "lg"                   (default "md")
    children?: Snippet
    ...rest              passed to the element (onclick, aria-*, class, ...)

  Uses `contain: layout style` (no paint containment, so the focus ring outside the box shows).
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  type Props = HTMLAttributes<HTMLElement> & {
    interactive?: boolean;
    href?: string;
    disabled?: boolean;
    selected?: boolean;
    padding?: "none" | "sm" | "md" | "lg";
    type?: "button" | "submit";
    children?: Snippet;
  };

  let {
    interactive = false,
    href,
    disabled = false,
    selected = false,
    padding = "md",
    type = "button",
    children,
    class: className = "",
    ...rest
  }: Props = $props();

  const cls = $derived(`card vf pad-${padding} ${className}`.trim());
</script>

{#if interactive && href !== undefined}
  <a
    {...rest}
    class="{cls} interactive"
    href={disabled ? undefined : href}
    aria-disabled={disabled || undefined}
    tabindex={disabled ? -1 : undefined}
    data-selected={selected || undefined}
  >
    {@render children?.()}
  </a>
{:else if interactive}
  <button
    {...rest}
    class="{cls} interactive"
    {type}
    {disabled}
    aria-disabled={disabled || undefined}
    data-selected={selected || undefined}
  >
    {@render children?.()}
  </button>
{:else}
  <div {...rest} class={cls} aria-disabled={disabled || undefined} data-selected={selected || undefined}>
    {@render children?.()}
  </div>
{/if}

<style>
  .card {
    --vf-radius: var(--r-panel);
    display: block;
    contain: layout style;
    background: var(--surface-1);
    color: var(--text);
    border: 0;
    box-shadow: inset 0 0 0 1px var(--line);
    border-radius: var(--r-panel);
    font: inherit;
    text-align: left;
    text-decoration: none;
  }
  .pad-none {
    padding: 0;
  }
  .pad-sm {
    padding: var(--s-2) var(--s-3);
  }
  .pad-md {
    padding: var(--s-4);
  }
  .pad-lg {
    padding: var(--s-6);
  }

  .interactive {
    width: 100%;
    cursor: pointer;
  }
  .interactive:hover:not(:disabled):not([aria-disabled="true"]) {
    background: var(--surface-2);
  }
  .interactive:active:not(:disabled):not([aria-disabled="true"]) {
    background: var(--surface-1);
  }
  .card[aria-disabled="true"] {
    opacity: 0.55;
    cursor: default;
  }
</style>

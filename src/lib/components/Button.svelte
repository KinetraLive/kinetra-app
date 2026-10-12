<!--
  Button. Renders <button>, or <a> when `href` is given.

  Props:
    variant?: "primary" | "accent" | "secondary" | "ghost" | "danger"   (default "secondary")
    size?: "sm" | "md"   24px / 32px tall                    (default "md")
    loading?: boolean   disables the button; a spinner appears only after 300 ms
    disabled?: boolean
    href?: string       render as a link (disabled links drop href + get aria-disabled)
    type?: "button" | "submit" | "reset"                      (default "button")
    icon?: Snippet      leading icon, e.g. {#snippet icon()}<Plus size={14} />{/snippet}
    children?: Snippet  label
    ...rest             passed to the element (onclick, aria-*, class, ...)

  Keyboard focus draws the focus ring (`vf`). `primary` is neutral (light on dark, dark on light);
  `accent` is for TAKE only; `danger` (red text and border, a red
  tint on hover) only for destructive actions.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAnchorAttributes, HTMLButtonAttributes } from "svelte/elements";

  type Props = Omit<HTMLButtonAttributes & HTMLAnchorAttributes, "type"> & {
    variant?: "primary" | "accent" | "secondary" | "ghost" | "danger";
    size?: "sm" | "md";
    loading?: boolean;
    disabled?: boolean;
    href?: string;
    type?: "button" | "submit" | "reset";
    icon?: Snippet;
    children?: Snippet;
  };

  let {
    variant = "secondary",
    size = "md",
    loading = false,
    disabled = false,
    href,
    type = "button",
    icon,
    children,
    class: className = "",
    ...rest
  }: Props = $props();

  const SPINNER_DELAY_MS = 300;
  let showSpinner = $state(false);

  // The timer exists only while `loading` is true.
  $effect(() => {
    if (!loading) {
      showSpinner = false;
      return;
    }
    const t = setTimeout(() => (showSpinner = true), SPINNER_DELAY_MS);
    return () => clearTimeout(t);
  });

  const inactive = $derived(disabled || loading);
  const cls = $derived(`btn vf ${variant} ${size} ${className}`.trim());
</script>

{#snippet content()}
  {#if icon}<span class="icon" class:hidden={showSpinner} aria-hidden="true">{@render icon()}</span>{/if}
  {#if children}<span class="label" class:hidden={showSpinner && !icon}>{@render children()}</span>{/if}
  {#if showSpinner}<span class="spinner" class:overlay={!icon} aria-hidden="true"></span>{/if}
{/snippet}

{#if href !== undefined}
  <a
    {...rest}
    class={cls}
    href={inactive ? undefined : href}
    aria-disabled={inactive || undefined}
    aria-busy={loading || undefined}
    tabindex={inactive ? -1 : rest.tabindex}
  >
    {@render content()}
  </a>
{:else}
  <button
    {...rest}
    class={cls}
    {type}
    disabled={inactive}
    aria-busy={loading || undefined}
  >
    {@render content()}
  </button>
{/if}

<style>
  .btn {
    --vf-radius: var(--r-control);
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--s-2);
    height: var(--control-h);
    padding: 0 var(--s-3);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--fs-md);
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
    text-decoration: none;
    user-select: none;
    cursor: pointer;
  }
  .btn.sm {
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    gap: var(--s-1);
    font-size: var(--fs-sm);
  }

  /* Pressed = a subtle background change, never a scale bounce. */
  .primary {
    background: var(--primary);
    color: var(--primary-ink);
  }
  .primary:hover:not(:disabled):not([aria-disabled="true"]) {
    background: var(--primary-hover);
  }
  .primary:active:not(:disabled):not([aria-disabled="true"]) {
    background: var(--primary);
  }

  .accent {
    background: var(--accent);
    color: var(--accent-ink);
  }
  .accent:hover:not(:disabled):not([aria-disabled="true"]) {
    background: var(--accent-hover);
  }
  .accent:active:not(:disabled):not([aria-disabled="true"]) {
    background: var(--accent);
  }

  .secondary {
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .secondary:hover:not(:disabled):not([aria-disabled="true"]) {
    background: color-mix(in srgb, var(--text) 6%, var(--surface-3));
  }
  .secondary:active:not(:disabled):not([aria-disabled="true"]) {
    background: var(--surface-2);
  }

  .ghost {
    color: var(--text-muted);
  }
  .ghost:hover:not(:disabled):not([aria-disabled="true"]) {
    background: var(--surface-3);
    color: var(--text);
  }
  .ghost:active:not(:disabled):not([aria-disabled="true"]) {
    background: var(--surface-2);
  }

  .danger {
    color: var(--danger);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--danger) 55%, transparent);
  }
  .danger:hover:not(:disabled):not([aria-disabled="true"]) {
    background: var(--danger-soft);
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .danger:active:not(:disabled):not([aria-disabled="true"]) {
    background: transparent;
  }

  .btn:disabled,
  .btn[aria-disabled="true"] {
    opacity: 0.5;
    cursor: default;
  }
  .btn[aria-busy="true"] {
    opacity: 1;
    cursor: progress;
  }

  .icon {
    display: inline-flex;
    flex: none;
  }
  .hidden {
    visibility: hidden;
  }

  .spinner {
    width: 14px;
    height: 14px;
    flex: none;
    border-radius: var(--r-full);
    border: 2px solid currentColor;
    border-right-color: transparent;
    animation: spin var(--period-spin) linear infinite;
  }
  .icon + .label + .spinner,
  .icon + .spinner {
    position: absolute;
    left: var(--s-3);
  }
  .sm .icon + .label + .spinner,
  .sm .icon + .spinner {
    left: var(--s-2);
  }
  .spinner.overlay {
    position: absolute;
    left: calc(50% - 7px);
  }

  @keyframes spin {
    to {
      transform: rotate(1turn);
    }
  }
</style>

<!--
  Square icon-only button with a built-in label tooltip (CSS, shown on hover and keyboard focus).

  Props:
    label: string        REQUIRED. Accessible name (aria-label) and tooltip text.
    pressed?: boolean    toggle buttons: sets aria-pressed (omit for plain buttons)
    pressedFill?: boolean   draw the pressed state as an accent fill (default true). Pass false
                         for a neutral toggle whose icon already shows the state (e.g. Pin/PinOff)
    variant?: "ghost" | "secondary" | "danger"             (default "ghost"; danger: delete /
                         remove, ghost at rest, --danger on hover)
    size?: "sm" | "md"   24px / 32px square                (default "md")
    tooltip?: "top" | "bottom" | "left" | "right" | "none" (default "bottom")
    disabled?: boolean
    children: Snippet    the icon, e.g. <IconButton label="Close"><X size={16} /></IconButton>
    ...rest              passed to <button> (onclick, type, class, ...)

  The tooltip bubble (::before) only exists while hovered/focused, so it never adds scrollable
  overflow to a clipping container at rest. ::after is the focus ring (`vf`).
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  type Props = Omit<HTMLButtonAttributes, "aria-label" | "aria-pressed"> & {
    label: string;
    pressed?: boolean;
    pressedFill?: boolean;
    variant?: "ghost" | "secondary" | "danger";
    size?: "sm" | "md";
    tooltip?: "top" | "bottom" | "left" | "right" | "none";
    disabled?: boolean;
    children: Snippet;
  };

  let {
    label,
    pressed,
    pressedFill = true,
    variant = "ghost",
    size = "md",
    tooltip = "bottom",
    disabled = false,
    type = "button",
    children,
    class: className = "",
    ...rest
  }: Props = $props();
</script>

<button
  {...rest}
  {type}
  {disabled}
  class="ib vf {variant} {size} tip-{tooltip} {className}"
  class:fill={pressedFill}
  aria-label={label}
  aria-pressed={pressed === undefined ? undefined : pressed}
  data-tip={tooltip === "none" ? undefined : label}
>
  <span class="glyph" aria-hidden="true">{@render children()}</span>
</button>

<style>
  .ib {
    --vf-radius: var(--r-control);
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: var(--control-h);
    height: var(--control-h);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text-muted);
  }
  .ib.sm {
    width: var(--control-h-sm);
    height: var(--control-h-sm);
  }
  .glyph {
    display: inline-flex;
  }

  .ghost:hover:not(:disabled) {
    background: var(--surface-3);
    color: var(--text);
  }
  .ghost:active:not(:disabled) {
    background: var(--surface-2);
  }
  /* Destructive (delete / remove): quiet at rest, --danger on hover. */
  .danger:hover:not(:disabled) {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .danger:active:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 18%, transparent);
  }
  .secondary {
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
  }
  .secondary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text) 6%, var(--surface-3));
  }
  .secondary:active:not(:disabled) {
    background: var(--surface-2);
  }
  .ib.fill[aria-pressed="true"],
  .ib.fill[aria-pressed="true"]:hover:not(:disabled) {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .ib:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* Tooltip: generated only on hover / keyboard focus. */
  .ib[data-tip]:hover::before,
  .ib[data-tip]:focus-visible::before {
    content: attr(data-tip);
    position: absolute;
    z-index: 50;
    width: max-content;
    max-width: 240px;
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--fs-sm);
    font-weight: 400;
    line-height: 1.35;
    white-space: normal;
    pointer-events: none;
    animation: tip-in var(--dur-fast) var(--ease) var(--delay-tooltip) both;
  }
  .ib[data-tip]:focus-visible::before {
    animation-delay: 0s;
  }
  .ib[data-tip]:active::before {
    display: none;
  }
  .tip-bottom::before {
    --tip-from: 0 -4px;
    top: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
  }
  .tip-top::before {
    --tip-from: 0 4px;
    bottom: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
  }
  .tip-right::before {
    --tip-from: -4px 0;
    left: calc(100% + 6px);
    top: 50%;
    transform: translateY(-50%);
  }
  .tip-left::before {
    --tip-from: 4px 0;
    right: calc(100% + 6px);
    top: 50%;
    transform: translateY(-50%);
  }

  @keyframes tip-in {
    from {
      opacity: 0;
      translate: var(--tip-from);
    }
    to {
      opacity: 1;
      translate: 0 0;
    }
  }
</style>

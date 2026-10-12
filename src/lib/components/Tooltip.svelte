<!--
  Tooltip shown on hover AND keyboard focus (CSS only; Esc hides it until the pointer/focus leaves).
  Wraps its trigger; the first child element gets `aria-describedby` pointing at the tooltip.

  Props:
    text: string
    placement?: "top" | "bottom" | "left" | "right"   (default "top")
    focusable?: boolean   set when the wrapped element is disabled (disabled buttons get neither
                          focus nor reliable hover): the wrapper itself becomes focusable and
                          described. e.g. <Tooltip text="Open a project first" focusable={!project}>
    children: Snippet     the trigger

  The bubble exists (display) only while hovered/focused, so it never makes a clipping ancestor
  scroll at rest. With `focusable`, keyboard focus on the wrapper draws the viewfinder brackets.
  IconButton has its own built-in label tooltip; don't wrap it in this.
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    text: string;
    placement?: "top" | "bottom" | "left" | "right";
    focusable?: boolean;
    children: Snippet;
  };

  let { text, placement = "top", focusable = false, children }: Props = $props();

  const id = $props.id();
  let wrapper: HTMLSpanElement | undefined = $state();
  let dismissed = $state(false);

  // Describe the wrapped element (unless the wrapper itself is the focus target).
  $effect(() => {
    if (focusable || !wrapper) return;
    const target = wrapper.firstElementChild;
    if (!target || target.id === id) return;
    const prev = target.getAttribute("aria-describedby");
    target.setAttribute("aria-describedby", prev ? `${prev} ${id}` : id);
    return () => {
      if (prev) target.setAttribute("aria-describedby", prev);
      else target.removeAttribute("aria-describedby");
    };
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") dismissed = true;
  }
  function reset() {
    dismissed = false;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_static_element_interactions -->
<span
  bind:this={wrapper}
  class="tt {placement}"
  class:vf={focusable}
  class:dismissed
  class:focusable
  tabindex={focusable ? 0 : undefined}
  aria-describedby={focusable ? id : undefined}
  {onkeydown}
  onpointerleave={reset}
  onfocusout={reset}
>
  {@render children()}
  <span {id} role="tooltip" class="bubble">{text}</span>
</span>

<style>
  .tt {
    position: relative;
    display: inline-flex;
  }
  .tt.focusable {
    --vf-radius: var(--r-control);
    border-radius: var(--r-control);
  }

  /* The bubble only gets a box while shown, so at rest it never adds scrollable overflow to a
     clipping container. aria-describedby still reads its text while it is display: none. */
  .bubble {
    position: absolute;
    z-index: 50;
    display: none;
    width: max-content;
    max-width: 280px;
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-family: var(--font-body);
    font-size: var(--fs-sm);
    font-weight: 400;
    line-height: 1.35;
    text-transform: none;
    letter-spacing: 0;
    white-space: normal;
    pointer-events: none;
  }

  .top > .bubble {
    --tip-from: 0 4px;
    bottom: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
  }
  .bottom > .bubble {
    --tip-from: 0 -4px;
    top: calc(100% + 6px);
    left: 50%;
    transform: translateX(-50%);
  }
  .left > .bubble {
    --tip-from: 4px 0;
    right: calc(100% + 6px);
    top: 50%;
    transform: translateY(-50%);
  }
  .right > .bubble {
    --tip-from: -4px 0;
    left: calc(100% + 6px);
    top: 50%;
    transform: translateY(-50%);
  }

  .tt:hover > .bubble,
  .tt:focus-visible > .bubble,
  .tt:has(:focus-visible) > .bubble {
    display: block;
    animation: tip-in var(--dur-fast) var(--ease) var(--delay-tooltip) both;
  }
  .tt:focus-visible > .bubble,
  .tt:has(:focus-visible) > .bubble {
    animation-delay: 0s;
  }
  .tt.dismissed > .bubble {
    display: none;
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

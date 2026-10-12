<!--
  Renders the toast queue from $lib/stores/toasts.svelte. Mounted ONCE in the root layout; pages
  just call `toasts.push(...)`. No props.

  Errors live in an aria-live="assertive" region, info/success in a "polite" one. Enter/exit and
  reordering animate transform/opacity only, with durations read from the motion tokens (so
  prefers-reduced-motion makes them instant).
-->
<script lang="ts">
  import { flip } from "svelte/animate";
  import CircleAlert from "lucide-svelte/icons/circle-alert";
  import CircleCheck from "lucide-svelte/icons/circle-check";
  import Info from "lucide-svelte/icons/info";
  import X from "lucide-svelte/icons/x";
  import { toasts, type Toast } from "$lib/stores/toasts.svelte";
  import IconButton from "./IconButton.svelte";

  const errors = $derived(toasts.list.filter((t) => t.kind === "error"));
  const others = $derived(toasts.list.filter((t) => t.kind !== "error"));

  /** Duration of a motion token in ms (0 under prefers-reduced-motion). */
  function tokenMs(name: string): number {
    if (typeof document === "undefined" || typeof getComputedStyle !== "function") return 0;
    const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    const n = parseFloat(v);
    if (!Number.isFinite(n)) return 0;
    return v.endsWith("ms") ? n : n * 1000;
  }

  /** cubic-bezier(0.2, 0, 0, 1), the app's --ease, as a JS easing for Svelte transitions. */
  function ease(t: number): number {
    const x1 = 0.2, y1 = 0, x2 = 0, y2 = 1;
    const at = (a: number, b: number, s: number) =>
      3 * a * (1 - s) * (1 - s) * s + 3 * b * (1 - s) * s * s + s * s * s;
    let lo = 0, hi = 1, s = t;
    for (let i = 0; i < 16; i++) {
      const x = at(x1, x2, s);
      if (Math.abs(x - t) < 1e-4) break;
      if (x < t) lo = s;
      else hi = s;
      s = (lo + hi) / 2;
    }
    return at(y1, y2, s);
  }

  function toastFx(_node: Element) {
    return {
      duration: tokenMs("--dur-slow"),
      easing: ease,
      css: (t: number, u: number) =>
        `opacity:${t};transform:translateY(${u * 8}px)`,
    };
  }

  function moveFx(node: Element, rects: { from: DOMRect; to: DOMRect }) {
    return flip(node, rects, { duration: tokenMs("--dur"), easing: ease });
  }
</script>

{#snippet item(t: Toast)}
  <span class="icon" aria-hidden="true">
    {#if t.kind === "error"}
      <CircleAlert size={16} />
    {:else if t.kind === "success"}
      <CircleCheck size={16} />
    {:else}
      <Info size={16} />
    {/if}
  </span>
  <p class="msg">{t.message}</p>
  {#if t.action}
    {@const a = t.action}
    <button
      type="button"
      class="act vf"
      onclick={() => {
        toasts.dismiss(t.id);
        a.run();
      }}>{a.label}</button
    >
  {/if}
  <IconButton
    label="Dismiss notification"
    size="sm"
    tooltip="none"
    onclick={() => toasts.dismiss(t.id)}
  >
    <X size={14} />
  </IconButton>
{/snippet}

<section class="host" aria-label="Notifications">
  <ol class="region" aria-live="assertive" aria-relevant="additions">
    {#each errors as t (t.id)}
      <li class="toast error" animate:moveFx transition:toastFx>{@render item(t)}</li>
    {/each}
  </ol>
  <ol class="region" aria-live="polite" aria-relevant="additions">
    {#each others as t (t.id)}
      <li class="toast {t.kind}" animate:moveFx transition:toastFx>{@render item(t)}</li>
    {/each}
  </ol>
</section>

<style>
  .host {
    position: fixed;
    right: var(--s-4);
    bottom: var(--s-4);
    z-index: 100;
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    width: min(380px, calc(100vw - 2 * var(--s-4)));
    pointer-events: none;
  }
  .region {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .region:empty {
    display: none;
  }
  /* Kind = a 2px leading bar (accent for info/success, danger for errors). Tally colours are
     reserved for on-air/preview status, so toasts never use them. */
  .toast {
    --bar: var(--accent);
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: var(--s-2);
    padding: var(--s-1) var(--s-1) var(--s-1) var(--s-3);
    border-radius: var(--r-panel);
    background: var(--surface-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    overflow: hidden;
    pointer-events: auto;
  }
  .toast::before {
    content: "";
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--bar);
  }
  .error {
    --bar: var(--danger);
  }
  .icon {
    display: inline-flex;
    flex: none;
    padding-top: 4px;
    color: var(--bar);
  }
  .msg {
    flex: 1;
    min-width: 0;
    padding: 3px 0;
    overflow-wrap: anywhere;
  }
  .act {
    flex: none;
    height: 24px;
    margin-top: 2px;
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-weight: 600;
  }
  .act:hover {
    background: var(--surface-2);
  }
</style>

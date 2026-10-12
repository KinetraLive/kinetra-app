<!--
  A large create-action tile for an empty project ("Create your first screen"), permanently framed
  by the .vf selection border (forced on with data-selected: --selected-bg + a 1px accent edge).
  Keyboard focus adds the focus ring outside it; hover lights the arrow.

  A link when `href` is given, otherwise a button calling `onclick` (e.g. opening the gallery).

  Props:
    href?: string
    onclick?: () => void
    title: string
    description: string
    icon: Snippet         e.g. {#snippet icon()}<Monitor size={22} />{/snippet}
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import ArrowRight from "lucide-svelte/icons/arrow-right";

  type Props = {
    href?: string;
    onclick?: () => void;
    title: string;
    description: string;
    icon: Snippet;
  };
  let { href, onclick, title, description, icon }: Props = $props();
</script>

{#snippet body()}
  <span class="icon" aria-hidden="true">{@render icon()}</span>
  <span class="text">
    <span class="title">{title}</span>
    <span class="desc">{description}</span>
  </span>
  <span class="go" aria-hidden="true"><ArrowRight size={16} /></span>
{/snippet}

{#if href !== undefined}
  <a class="create vf" data-selected {href}>{@render body()}</a>
{:else}
  <button type="button" class="create vf" data-selected {onclick}>{@render body()}</button>
{/if}

<style>
  .create {
    --vf-radius: var(--r-panel);
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--s-4);
    width: 100%;
    min-height: 168px;
    padding: var(--s-6);
    border-radius: var(--r-panel);
    background: var(--selected-bg);
    color: var(--text);
    text-align: left;
    text-decoration: none;
  }

  .icon {
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border-radius: var(--r-panel);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-xl);
    font-weight: 600;
    line-height: 1.25;
  }
  .desc {
    max-width: 44ch;
    color: var(--text-muted);
    font-size: var(--fs-md);
  }
  .go {
    display: grid;
    place-items: center;
    width: var(--control-h);
    height: var(--control-h);
    border-radius: var(--r-full);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
  }
  .create:hover .go,
  .create:focus-visible .go {
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--text);
  }
</style>

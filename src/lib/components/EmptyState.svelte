<!--
  Centered empty state for lists and views with nothing in them yet.

  Props:
    title: string
    description?: string
    icon?: Snippet       e.g. {#snippet icon()}<FolderOpen size={20} />{/snippet}
    actions?: Snippet    buttons under the text
    children?: Snippet   extra content (rendered after the description)
    headingLevel?: 2 | 3 | 4                               (default 2)
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    title: string;
    description?: string;
    icon?: Snippet;
    actions?: Snippet;
    children?: Snippet;
    headingLevel?: 2 | 3 | 4;
  };

  let { title, description, icon, actions, children, headingLevel = 2 }: Props = $props();
</script>

<div class="empty">
  {#if icon}<div class="icon" aria-hidden="true">{@render icon()}</div>{/if}
  <svelte:element this={`h${headingLevel}`} class="title">{title}</svelte:element>
  {#if description}<p class="desc">{description}</p>{/if}
  {@render children?.()}
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s-1);
    max-width: 400px;
    margin: 0 auto;
    padding: var(--s-6) var(--s-4);
    text-align: center;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    margin-bottom: var(--s-2);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
    color: var(--text);
  }
  .desc {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: var(--s-2);
    margin-top: var(--s-3);
  }
</style>

<!--
  One template in the Templates grid: a listbox option (the grid is the listbox). A preset
  template whose preset ships a newer look says so ("Newer look available"; Update is in the
  side pane, since an option holds no controls). Selecting it
  (click / Enter / Space) draws the selection border via aria-selected; the parent handles
  double-click (open), arrow keys and Delete through the bubbling key/mouse events.

  Props:
    template: TemplateSummary
    origin: string          "From Classic bar" / "Blank"
    selected: boolean
    now: number             reference time for the relative "updated" label (no ticking timers)
    onselect: () => void
    onopen: () => void      double-click
-->
<script lang="ts">
  import { relativeTime } from "$lib/components/picker/format";
  import type { TemplateSummary } from "$lib/types";
  import { absoluteTime, countsLabel } from "./format";

  type Props = {
    template: TemplateSummary;
    origin: string;
    selected: boolean;
    now: number;
    onselect: () => void;
    onopen: () => void;
  };
  let { template, origin, selected, now, onselect, onopen }: Props = $props();

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onselect();
    }
  }
</script>

<div
  class="card vf"
  role="option"
  id="tpl-{template.id}"
  data-template-id={template.id}
  aria-selected={selected}
  tabindex={selected ? 0 : -1}
  onclick={onselect}
  ondblclick={onopen}
  {onkeydown}
>
  <span class="name">{template.name}</span>
  <span class="meta">
    <time datetime={template.updatedAt} title={absoluteTime(template.updatedAt)}
      >{relativeTime(template.updatedAt, now)}</time
    >
    <span class="sep" aria-hidden="true">·</span>
    <span class="origin">{origin}</span>
  </span>
  <span class="counts"
    ><span>{countsLabel(template)}</span
    >{#if template.variantCount > 1}{` · ${template.variantCount} variants`}{/if}</span
  >
  {#if template.presetUpdate}<span class="update">Newer look available</span>{/if}
</div>

<style>
  .card {
    --vf-radius: var(--r-panel);
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
    padding: var(--s-3) var(--s-4);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    cursor: pointer;
    user-select: none;
  }
  .card:hover {
    background: var(--surface-2);
  }
  /* Selected: --selected-bg + the 1px accent border from .vf (no second border). */
  .card[aria-selected="true"] {
    background: var(--selected-bg);
  }
  .name {
    overflow: hidden;
    font-size: var(--fs-md);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Time (mono) · origin: the dot keeps the two values apart. */
  .meta {
    display: flex;
    align-items: baseline;
    gap: var(--s-2);
    min-width: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .meta time,
  .sep {
    flex: none;
  }
  .origin {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .counts {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  /* The preset ships a newer look: Update is in the side pane. */
  .update {
    color: var(--accent-text);
    font-size: var(--fs-xs);
  }
</style>

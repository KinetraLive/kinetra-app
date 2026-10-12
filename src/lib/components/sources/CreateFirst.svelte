<!--
  Empty Sources page: one large create-action tile, permanently framed by the .vf selection
  border (the dashboard's CreateAction, as a button that opens the New source dialog). Keyboard focus adds the focus ring; hover lights the arrow.

  Props:
    onclick: () => void
-->
<script lang="ts">
  import ArrowRight from "lucide-svelte/icons/arrow-right";
  import Database from "lucide-svelte/icons/database";

  let { onclick }: { onclick: () => void } = $props();
</script>

<button type="button" class="create vf" data-selected {onclick}>
  <span class="icon" aria-hidden="true"><Database size={22} /></span>
  <span class="text">
    <span class="title">Create your first source</span>
    <span class="desc">A guest list, a run of show or API data.</span>
  </span>
  <span class="go" aria-hidden="true"><ArrowRight size={16} /></span>
</button>

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

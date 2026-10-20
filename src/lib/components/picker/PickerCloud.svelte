<!--
  The picker's "In the cloud" card and, under it, the account's cloud projects (the list is a lazy
  chunk, loaded like the New project form: pointer or focus on the card). Only in builds with cloud
  backup (`$cloud`). Props: onopened(folder) (a downloaded project to open).
-->
<script lang="ts">
  import Cloud from "lucide-svelte/icons/cloud";
  import Card from "$lib/components/Card.svelte";
  import type CloudListComponent from "./CloudList.svelte";

  let { onopened }: { onopened: (folder: string) => void } = $props();

  let open = $state(false);
  let CloudList = $state.raw<typeof CloudListComponent | null>(null);
  let loading: Promise<void> | null = null;
  function load() {
    loading ??= import("./CloudList.svelte").then((m) => {
      CloudList = m.default;
    });
    return loading;
  }
</script>

<Card
  interactive
  padding="none"
  class="action"
  selected={open}
  aria-expanded={open}
  aria-controls="cloud-projects"
  onpointerenter={load}
  onfocus={load}
  onclick={() => {
    void load();
    open = !open;
  }}
>
  <span class="icon" class:on={open} aria-hidden="true"><Cloud size={18} /></span>
  <span class="text">
    <span class="title">In the cloud</span>
    <span class="desc">Download a project from your cloud backup.</span>
  </span>
</Card>
{#if open && CloudList}
  <CloudList id="cloud-projects" {onopened} />
{/if}

<style>
  /* The picker's action card look (src/routes/+page.svelte). */
  .icon {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: var(--r-control);
    background: var(--surface-3);
    color: var(--accent-text);
  }
  .icon.on {
    background: var(--accent-soft);
  }
  .text {
    display: grid;
    min-width: 0;
  }
  .title {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--fs-md);
    font-weight: 600;
  }
  .desc {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>

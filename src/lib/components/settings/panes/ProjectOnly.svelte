<!--
  A block of project settings in the Preferences window: its children while a project is open,
  otherwise the block's title with one line ("Open a project to change its settings.").

  Props:
    title: string        the block's section label (the same one its children show)
    children: Snippet
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { project } from "$lib/stores/project.svelte";

  let { title, children }: { title: string; children: Snippet } = $props();
</script>

{#if project.current}
  {@render children()}
{:else}
  <SettingsSection {title}>
    <p class="none">Open a project to change its settings.</p>
  </SettingsSection>
{/if}

<style>
  .none {
    padding: var(--s-3) 0;
    border-top: 1px solid var(--line);
    color: var(--text-muted);
  }
</style>

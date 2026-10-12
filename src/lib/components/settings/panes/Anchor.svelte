<!--
  A deep-link target inside a Preferences section: keeps an old `/settings#<id>` anchor id
  (`backups`, `server`, …) so `openPreferences("backups")` scrolls here and focuses it.

  Props:
    id: string
    children: Snippet
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  let { id, children }: { id: string; children: Snippet } = $props();
</script>

<div {id} class="anchor" tabindex="-1">
  {@render children()}
</div>

<style>
  .anchor {
    min-width: 0;
    scroll-margin-top: var(--s-5);
  }
  .anchor:focus {
    outline: none;
  }
</style>

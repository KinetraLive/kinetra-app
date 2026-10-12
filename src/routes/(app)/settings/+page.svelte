<!--
  /settings is gone: Preferences is a window of its own. An old link (`/settings#backups`) opens
  the Preferences window at the matching section, then goes back where it came from (or to the
  dashboard when there is no history).
-->
<!-- Runes mode: without runes Svelte compiles legacy mode, whose flag module becomes a chunk the picker loads. -->
<svelte:options runes />

<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { redirectSettings } from "$lib/components/settings/open";

  onMount(() =>
    redirectSettings(location.hash, {
      canGoBack: history.length > 1,
      back: () => history.back(),
      replace: (href) => void goto(href, { replaceState: true }),
    }),
  );
</script>

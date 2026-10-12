<!--
  Layout for every in-project route (/dashboard, /templates, ...). Guards on an open project:
  after a webview reload the backend may still have one open, so we ask it once before sending the
  user back to the picker. Until that resolves only the empty --bg frame shows (no shell flash).
-->
<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  import { goto } from "$app/navigation";
  import { toAppError } from "$lib/api";
  import AppShell from "$lib/components/shell/AppShell.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { children }: { children: Snippet } = $props();

  let resolved = $state(project.current !== null);

  onMount(() => {
    if (resolved) return;
    project
      .refresh()
      .catch((e: unknown) => {
        toasts.push({
          kind: "error",
          message: `Couldn't restore the open project: ${toAppError(e).message}`,
        });
      })
      .finally(() => {
        resolved = true;
      });
  });

  // No project (none restored, or it was closed from elsewhere): back to the picker.
  $effect(() => {
    if (resolved && !project.current && !project.busy) void goto("/");
  });
</script>

{#if resolved && project.current}
  <AppShell>{@render children()}</AppShell>
{/if}

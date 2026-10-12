<!--
  AI jobs in progress or just finished (bottom-left, main window): "Designing your template…" with
  Cancel; "Template ready" with Open; a failure in plain words. The user keeps working meanwhile.
-->
<script lang="ts">
  import { goto } from "$app/navigation";
  import Button from "$lib/components/Button.svelte";
  import { aiCancel } from "$lib/api/ai";
  import { API_MESSAGES } from "$lib/components/license/messages";
  import type { AiJobView } from "$lib/types";
  import { jobs } from "./jobs.svelte";

  jobs.start();
  const shown = $derived(jobs.visible.filter((j) => j.status !== "canceled").slice(0, 3));

  function failure(j: AiJobView): string {
    if (j.code === "import" || !j.code) return j.message ?? "Something went wrong. Try again.";
    return API_MESSAGES[j.code] ?? j.message ?? "Something went wrong. Try again.";
  }
</script>

{#if shown.length}
  <ul class="jobs" aria-label="AI templates">
    {#each shown as j (j.id)}
      <li role="status">
        <span class="text">
          {#if j.status === "pending"}
            <strong>{j.task === "tweak" ? "Tweaking your template…" : "Designing your template…"}</strong>
          {:else if j.status === "succeeded"}
            <strong>Template ready</strong>
          {:else}
            <strong class="bad">{failure(j)}</strong>
          {/if}
          <span class="label">{j.label}</span>
        </span>
        {#if j.status === "pending"}
          <Button size="sm" variant="ghost" onclick={() => void aiCancel(j.id)}>Cancel</Button>
        {:else}
          {#if j.status === "succeeded" && j.templateId}
            <Button size="sm" onclick={() => { jobs.hide(j.id); void goto(`/templates/${j.templateId}`); }}>Open</Button>
          {/if}
          <Button size="sm" variant="ghost" onclick={() => jobs.hide(j.id)}>Close</Button>
        {/if}
      </li>
    {/each}
  </ul>
{/if}

<style>
  .jobs {
    position: fixed;
    bottom: var(--s-4);
    left: var(--s-4);
    z-index: 850;
    display: grid;
    gap: var(--s-2);
    width: min(360px, calc(100vw - 32px));
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .text {
    display: grid;
    flex: 1;
    min-width: 0;
  }
  strong {
    font-weight: 600;
    font-size: var(--fs-sm);
  }
  .bad {
    color: var(--danger);
  }
  .label {
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>

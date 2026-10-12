<!--
  DiskBanner: the file open in an editor changed (or was deleted) outside Kinetra while the editor
  had unsaved edits. Used by the template editor and the screen editor.

  Props:
    what: string                 "template" | "screen" (wording only)
    state: "conflict" | "removed"
    busy?: boolean               a Reload / Keep mine request is in flight
    onreload?: () => void        conflict: replace the working copy with the file on disk
    onkeep?: () => void          conflict: keep the working copy and save it over the file
    onclose?: () => void         removed: leave the editor (back to the list)
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";

  let {
    what,
    state,
    busy = false,
    onreload,
    onkeep,
    onclose,
  }: {
    what: string;
    state: "conflict" | "removed";
    busy?: boolean;
    onreload?: () => void;
    onkeep?: () => void;
    onclose?: () => void;
  } = $props();
</script>

<div class="banner" role="alert">
  {#if state === "conflict"}
    <p>
      <strong>Changed on disk.</strong>
      This {what} was edited outside Kinetra while you have unsaved changes.
    </p>
    <span class="acts">
      <Button size="sm" disabled={busy} onclick={onreload}>Reload</Button>
      <Button size="sm" variant="primary" disabled={busy} onclick={onkeep}>Keep mine</Button>
    </span>
  {:else}
    <p>
      <strong>Deleted on disk.</strong>
      This {what} was deleted outside Kinetra. Changes here are no longer saved.
    </p>
    <span class="acts">
      <Button size="sm" onclick={onclose}>Close</Button>
    </span>
  {/if}
</div>

<style>
  .banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-4);
    padding: var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface-2);
    border-left: 3px solid var(--warning);
  }
  p {
    flex: 1 1 20rem;
    min-width: 0;
    font-size: var(--fs-sm);
  }
  strong {
    color: var(--warning);
    font-weight: 600;
  }
  .acts {
    display: inline-flex;
    gap: var(--s-2);
  }
</style>

<!--
  Inline "Move project" panel, shown under Project location once a destination is picked.
  Loaded lazily. Subscribes to `project-move-progress` only while the move runs.

  Props:
    dest: string          the folder the project is copied into (must be empty or new)
    onclose: () => void   hide the panel (after success, cancel, or Close)
-->
<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import { on, toAppError } from "$lib/api";
  import { cancelMove, moveProject } from "$lib/api/move";
  import Button from "$lib/components/Button.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { MoveProgress } from "$lib/types";
  import { describeMove, moveRatio } from "./format";
  import { Confirm } from "./lazy.svelte";
  import { settingsBusy } from "./state.svelte";

  type Props = { dest: string; onclose: () => void };
  let { dest, onclose }: Props = $props();

  const id = $props.id();
  let deleteOriginal = $state(false);
  let running = $state(false);
  let cancelling = $state(false);
  let error = $state<string | null>(null);
  /** Replaced per event (the backend throttles to ≤ 10/s). */
  let progress = $state.raw<MoveProgress | null>(null);
  let stopListening: (() => void) | null = null;
  let heading: HTMLElement | undefined = $state();

  const confirmDelete = new Confirm<true>();
  const status = $derived(describeMove(progress));
  const ratio = $derived(moveRatio(progress));
  const source = $derived(project.current?.path ?? "");

  onMount(() => heading?.focus());

  onDestroy(() => {
    stopListening?.();
    stopListening = null;
    if (running) settingsBusy.moving = false;
  });

  function requestMove() {
    if (deleteOriginal) void confirmDelete.open(true);
    else void run();
  }

  async function run() {
    if (running) return;
    error = null;
    progress = null;
    running = true;
    settingsBusy.moving = true;
    stopListening = on("project-move-progress", (p) => (progress = p));
    // Move/Close are swapped for Cancel: keep keyboard focus in the panel.
    void tick().then(() => document.getElementById(`${id}-cancel`)?.focus());
    try {
      const p = await moveProject(dest, deleteOriginal);
      project.replace(p);
      toasts.push({ kind: "success", message: `Project moved to ${p.path}` });
      onclose();
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "cancelled") {
        toasts.push({ kind: "info", message: "Move canceled. Nothing was changed." });
        onclose();
      } else {
        error = err.message;
      }
    } finally {
      stopListening?.();
      stopListening = null;
      running = false;
      cancelling = false;
      settingsBusy.moving = false;
    }
  }

  async function cancel() {
    cancelling = true;
    try {
      await cancelMove();
    } catch (e) {
      cancelling = false;
      toasts.push({ kind: "error", message: `Couldn't cancel: ${toAppError(e).message}` });
    }
  }
</script>

<div class="panel" role="group" aria-labelledby="{id}-h">
  <h3 id="{id}-h" class="title" tabindex="-1" bind:this={heading}>Move project</h3>
  <p class="muted">Everything, media included, is copied. The project then opens from there.</p>
  <dl class="paths">
    <dt>From</dt>
    <dd class="mono">{source}</dd>
    <dt>To</dt>
    <dd class="mono">{dest}</dd>
  </dl>

  <label class="check">
    <input type="checkbox" bind:checked={deleteOriginal} disabled={running} />
    Delete the original folder after a successful copy
  </label>
  {#if deleteOriginal}
    <p class="warn">
      <TriangleAlert size={14} aria-hidden="true" />
      The original folder is deleted permanently once the copy is verified.
    </p>
  {/if}

  {#if running}
    <div class="progress">
      <ProgressBar label="Moving project" value={ratio} max={1} valueText={status} />
      <p class="status" aria-hidden="true">{status}</p>
    </div>
  {/if}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <div class="actions">
    {#if running}
      <Button id="{id}-cancel" variant="ghost" loading={cancelling} onclick={cancel}>Cancel</Button>
    {:else}
      <Button variant="ghost" onclick={onclose}>Close</Button>
      <Button variant="primary" onclick={requestMove}>Move</Button>
    {/if}
  </div>
</div>

{#if confirmDelete.Dialog && confirmDelete.value}
  <confirmDelete.Dialog
    title="Delete the original folder?"
    message="After the copy is verified, {source} is deleted. This can't be undone."
    confirmLabel="Move and delete"
    danger
    onconfirm={() => {
      confirmDelete.close();
      void run();
    }}
    oncancel={() => confirmDelete.close()}
  />
{/if}

<style>
  /* Inline panel under its row: one step up in surface, no border. */
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    margin: 0 0 var(--s-3);
    padding: var(--s-4);
    border-radius: var(--r-panel);
    background: var(--surface-2);
  }
  .title {
    font-size: var(--fs-md);
  }
  .muted,
  .status {
    color: var(--text-muted);
  }
  .paths {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--s-1) var(--s-3);
    font-size: var(--fs-sm);
  }
  .paths dt {
    color: var(--text-muted);
  }
  .paths dd {
    overflow-wrap: anywhere;
    user-select: text;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    margin-top: var(--s-1);
  }
  .check input {
    accent-color: var(--accent);
  }
  .warn {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .progress {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin-top: var(--s-1);
  }
  .status {
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
  }
  .error {
    color: var(--danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-2);
    margin-top: var(--s-1);
  }
</style>

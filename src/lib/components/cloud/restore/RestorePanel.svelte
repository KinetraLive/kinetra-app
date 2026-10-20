<!--
  RestorePanel: restores one cloud version (a lazy chunk; the History dialog, the conflict notice's
  "Open the other version" and the picker's "In the cloud" use it). Before: where it goes (a new
  folder, Change…) or, for Replace, what happens first (backed up, or copied next to it). While it
  runs: progress and Cancel. After: how it ended, the damaged files, Open project.
  The core decides and checks everything (on air, the folder, every file's hash); see
  core::cloud::projects::restore.
  Props: projectId (server), versionId (null = newest), name (the cloud project's, for Download),
  at (the version's time, names the folder), modes (the choices, first = default), onopen(folder)
  (Open project), onclose.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { toAppError } from "$lib/api";
  import { cloudRestoreCancel, cloudRestoreDismiss, cloudRestorePlan, cloudRestoreStart } from "$lib/api/cloud-restore";
  import Button from "$lib/components/Button.svelte";
  import { pickFolder } from "$lib/projectFlow";
  import type { CloudRestoreMode, CloudRestorePlan } from "$lib/types";
  import { cloud } from "../state.svelte";
  import { damagedList, joinPath, leaf, restoreWords, resultWords } from "./words";

  let {
    projectId,
    versionId,
    name,
    at,
    modes,
    onopen,
    onclose,
  }: {
    projectId: string;
    versionId: string | null;
    name: string;
    at: string | null;
    modes: CloudRestoreMode[];
    onopen?: (folder: string) => void;
    onclose: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let mode = $state<CloudRestoreMode>(modes[0]);
  let plan = $state<CloudRestorePlan | null>(null);
  let folder = $state("");
  let error = $state<string | null>(null);
  let busy = $state(false);
  /** This panel started the restore shown (or one for this version was already running). */
  let mine = $state(false);

  onMount(() => cloud.watch());

  const restore = $derived.by(() => {
    const r = cloud.view?.restore ?? null;
    if (!r) return null;
    const running = !r.result;
    return mine || (running && (versionId === null || r.versionId === versionId)) ? r : null;
  });
  const result = $derived(restore ? resultWords(restore) : null);
  const damaged = $derived(restore ? damagedList(restore) : null);

  $effect(() => {
    const m = mode;
    plan = null;
    error = null;
    cloudRestorePlan(m, name, at).then(
      (p) => {
        if (m !== mode) return;
        plan = p;
        folder = p.folder;
      },
      (e) => (error = toAppError(e).message),
    );
  });

  async function change() {
    const parent = await pickFolder("Choose where to put the project");
    if (parent) folder = joinPath(parent, leaf(folder));
  }

  async function start() {
    if (busy) return;
    busy = true;
    error = null;
    try {
      const v = await cloudRestoreStart({
        projectId,
        versionId,
        mode,
        folder: mode === "replace" ? null : folder,
      });
      cloud.view = v;
      mine = true;
    } catch (e) {
      error = toAppError(e).message;
    } finally {
      busy = false;
    }
  }

  async function finish() {
    try {
      cloud.view = await cloudRestoreDismiss();
    } catch {
      // Nothing to forget.
    }
    mine = false;
    onclose();
  }

  const label = (m: CloudRestoreMode) =>
    m === "replace" ? "Replace this project" : m === "download" ? "Download" : "Restore to a new folder";
</script>

<div class="panel" role="group" aria-label="Restore">
  {#if restore && !restore.result}
    <p class="status tnum" role="status">{restoreWords(restore)}</p>
    {#if restore.totalBytes > 0}
      <progress max={restore.totalBytes} value={restore.doneBytes}></progress>
    {/if}
    <div class="row">
      <Button size="sm" onclick={() => void cloudRestoreCancel()}>Cancel</Button>
    </div>
  {:else if restore && result}
    <p class="status" class:warning={result.tone === "warning"} role="status">{result.text}</p>
    {#if damaged && damaged.files.length > 0}
      <ul class="files">
        {#each damaged.files as f (f)}<li class="mono">{f}</li>{/each}
        {#if damaged.more > 0}<li>and {damaged.more} more</li>{/if}
      </ul>
    {/if}
    {#if restore.result?.copyFolder}
      <p class="note">The project as it was is in <span class="mono">{restore.result.copyFolder}</span>.</p>
    {/if}
    <div class="row">
      {#if restore.result?.ok && restore.mode !== "replace" && onopen}
        <Button size="sm" variant="primary" onclick={() => onopen?.(restore.folder)}>Open project</Button>
      {/if}
      <Button size="sm" onclick={finish}>Done</Button>
    </div>
  {:else}
    {#if modes.length > 1}
      <div class="modes" role="radiogroup" aria-label="How to restore">
        {#each modes as m (m)}
          <label class="mode">
            <input type="radio" name="restore-mode" value={m} bind:group={mode} />
            {label(m)}
          </label>
        {/each}
      </div>
    {/if}
    {#if mode === "replace"}
      {#if plan}
        <p class="note">
          {#if plan.backupFirst}
            Kinetra backs up the project as it is first, then replaces its files with this version.
          {:else}
            Kinetra copies the project as it is to <span class="mono">{plan.copyFolder}</span> first, then replaces its
            files with this version.
          {/if}
          Not possible while anything is on air.
        </p>
      {/if}
    {:else if plan}
      <div class="where">
        <span class="mono path" title={folder}>{folder}</span>
        <Button size="sm" variant="ghost" onclick={change}>Change…</Button>
      </div>
    {/if}
    {#if error}<p class="status warning" role="alert">{error}</p>{/if}
    <div class="row">
      <Button
        size="sm"
        variant={mode === "replace" ? "danger" : "primary"}
        disabled={busy || !plan}
        onclick={start}>{mode === "replace" ? "Replace project" : mode === "download" ? "Download" : "Restore"}</Button
      >
      <Button size="sm" variant="ghost" onclick={onclose}>Cancel</Button>
    </div>
  {/if}
</div>

<style>
  .panel {
    display: grid;
    gap: var(--s-2);
    margin-top: var(--s-2);
    padding: var(--s-3);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
    background: var(--surface-1);
  }
  .modes {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-4);
  }
  .mode {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
  }
  .where {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-sm);
  }
  .note,
  .files {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .files {
    padding-left: var(--s-4);
  }
  .status {
    margin: 0;
  }
  .warning {
    color: var(--warning);
  }
  progress {
    width: 100%;
    height: 4px;
    accent-color: var(--accent);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
</style>

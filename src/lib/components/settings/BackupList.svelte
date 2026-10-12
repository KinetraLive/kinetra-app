<!--
  Snapshot rows with Restore / Delete (each confirmed). Loaded lazily by BackupsSection.

  Props:
    backups: BackupInfo[]            newest first
    disabled?: boolean               e.g. while a move runs
    onremoved: (id: string) => void  a snapshot was deleted (or no longer exists)
-->
<script lang="ts">
  import { toAppError } from "$lib/api";
  import { deleteBackup, restoreBackup } from "$lib/api/backups";
  import Button from "$lib/components/Button.svelte";
  import { relativeTime } from "$lib/components/picker/format";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { uiState } from "$lib/stores/uiState.svelte";
  import type { BackupInfo } from "$lib/types";
  import { REASON_LABELS, formatBytes, formatDateTime } from "./format";
  import { Confirm } from "./lazy.svelte";

  type Props = { backups: BackupInfo[]; disabled?: boolean; onremoved: (id: string) => void };
  let { backups, disabled = false, onremoved }: Props = $props();

  type Action = { kind: "restore" | "delete"; backup: BackupInfo };
  const confirm = new Confirm<Action>();

  // Computed once per list change; relative times don't tick (no timers at rest).
  const rows = $derived.by(() => {
    const now = Date.now();
    return backups.map((b) => ({
      ...b,
      when: relativeTime(b.createdAt, now) || formatDateTime(b.createdAt),
      at: formatDateTime(b.createdAt),
      label: REASON_LABELS[b.reason] ?? b.reason,
      filesText: `${b.files} ${b.files === 1 ? "file" : "files"}`,
      bytesText: formatBytes(b.bytes),
    }));
  });

  async function act({ kind, backup }: Action) {
    try {
      if (kind === "restore") {
        project.replace(await restoreBackup(backup.id));
        await uiState.load();
        toasts.push({ kind: "success", message: "Snapshot restored." });
      } else {
        await deleteBackup(backup.id);
        onremoved(backup.id);
      }
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "notFound") {
        onremoved(backup.id);
        toasts.push({ kind: "error", message: "That snapshot no longer exists." });
      } else {
        const what = kind === "restore" ? "restore" : "delete";
        toasts.push({ kind: "error", message: `Couldn't ${what} the snapshot: ${err.message}` });
      }
    }
  }
</script>

<!-- Rows under "Snapshots", on the same two columns as the settings rows (reason left; time, size
     and actions right), full-width dividers. No row is ever "selected": brackets only on keyboard
     focus (the action buttons). -->
<ul class="list" aria-label="Snapshots">
  {#each rows as r (r.id)}
    <li class="row">
      <span class="reason">{r.label}</span>
      <span class="meta">
        <time class="mono dim" datetime={r.createdAt} title={r.when}>{r.at}</time>
        <span class="dim size">{r.filesText} · <span class="mono">{r.bytesText}</span></span>
        <span class="actions">
          <Button
            size="sm"
            variant="ghost"
            {disabled}
            aria-label="Restore snapshot from {r.at}"
            onclick={() => confirm.open({ kind: "restore", backup: r })}
          >
            Restore
          </Button>
          <Button
            size="sm"
            variant="danger"
            {disabled}
            aria-label="Delete snapshot from {r.at}"
            onclick={() => confirm.open({ kind: "delete", backup: r })}
          >
            Delete
          </Button>
        </span>
      </span>
    </li>
  {/each}
</ul>

{#if confirm.Dialog && confirm.value}
  {#if confirm.value.kind === "restore"}
    <confirm.Dialog
      title="Restore snapshot"
      message="Restore this snapshot? Your current settings and content are backed up first."
      confirmLabel="Restore"
      busy={confirm.busy}
      onconfirm={() => confirm.run(act)}
      oncancel={() => confirm.close()}
    />
  {:else}
    <confirm.Dialog
      title="Delete snapshot"
      message="Delete the snapshot from {formatDateTime(confirm.value.backup.createdAt)}? This can't be undone."
      confirmLabel="Delete"
      danger
      busy={confirm.busy}
      onconfirm={() => confirm.run(act)}
      oncancel={() => confirm.close()}
    />
  {/if}
{/if}

<style>
  /* Up to ten rows visible; the rest scroll. */
  .list {
    max-height: calc((var(--row-h) + 1px) * 10);
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }
  .row {
    display: grid;
    grid-template-columns: var(--settings-cols);
    align-items: center;
    gap: var(--s-6);
    height: var(--row-h);
    border-top: 1px solid var(--line);
  }
  .reason {
    min-width: 0;
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-width: 0;
  }
  .dim {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .size {
    min-width: 16ch;
  }
  .actions {
    display: flex;
    gap: var(--s-1);
  }
</style>

<!--
  Preferences > Backups: automatic backup settings (app prefs), "Create snapshot now", and the
  snapshot list (flush 32px rows under the "Snapshots" row, which shows the count).
  The list (and its row component) loads only when the section first scrolls into
  view; `backup-created` is subscribed only after that, and released on teardown. No props.
-->
<script lang="ts">
  import Plus from "lucide-svelte/icons/plus";
  import { on, toAppError } from "$lib/api";
  import { createBackup, listBackups } from "$lib/api/backups";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import {
    BACKUP_INTERVAL_RANGE,
    BACKUP_RETENTION_RANGE,
    type BackupInfo,
    type BackupPrefs,
  } from "$lib/types";
  import { firstVisible } from "./lazy.svelte";
  import { settingsBusy } from "./state.svelte";

  type ListComponent = typeof import("./BackupList.svelte").default;

  /** Rows kept in memory/rendered; the backend's retention is at most 100. */
  const MAX_ROWS = 100;

  const DEFAULTS: BackupPrefs = { enabled: true, intervalMinutes: 15, retention: 10 };
  const backup = $derived(prefs.value?.backup ?? DEFAULTS);
  // Writable deriveds: follow prefs (including SyncedStore rollbacks) and snap back on failure.
  let enabled = $derived(backup.enabled);
  let interval = $derived(backup.intervalMinutes);
  let retention = $derived(backup.retention);

  let List = $state.raw<ListComponent | null>(null);
  /** null until the section is first seen. Newest first. */
  let backups = $state.raw<BackupInfo[] | null>(null);
  let loadError = $state<string | null>(null);
  let creating = $state(false);
  const loaded = $derived(backups !== null);

  async function savePrefs(patch: Partial<BackupPrefs>) {
    const current = prefs.value;
    if (!current) return;
    try {
      await prefs.update({ ...current, backup: { ...current.backup, ...patch } });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't save the setting: ${toAppError(e).message}` });
      // SyncedStore rolled back; make sure the controls show it even if nothing else changed.
      const b = prefs.value?.backup ?? DEFAULTS;
      enabled = b.enabled;
      interval = b.intervalMinutes;
      retention = b.retention;
    }
  }

  async function loadList() {
    loadError = null;
    try {
      const [list, mod] = await Promise.all([listBackups(), import("./BackupList.svelte")]);
      List = mod.default;
      backups = sortNewest(list).slice(0, MAX_ROWS);
    } catch (e) {
      loadError = `Couldn't load snapshots: ${toAppError(e).message}`;
    }
  }

  function sortNewest(list: BackupInfo[]): BackupInfo[] {
    return [...list].sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt));
  }

  // A new snapshot can prune old ones (retention), which the event doesn't report. Show the new
  // row immediately, then re-read the list; listing reads only small manifest files and snapshots
  // are rare, so this costs nothing noticeable. `refreshSeq` drops a stale response.
  let refreshSeq = 0;
  function prepend(b: BackupInfo) {
    if (!backups) return;
    backups = [b, ...backups.filter((x) => x.id !== b.id)].slice(0, MAX_ROWS);
    const seq = ++refreshSeq;
    listBackups()
      .then((list) => {
        if (seq === refreshSeq && backups) backups = sortNewest(list).slice(0, MAX_ROWS);
      })
      .catch(() => {});
  }

  function removeRow(id: string) {
    if (backups) backups = backups.filter((x) => x.id !== id);
  }

  // Only while the list is loaded (and this page is mounted).
  $effect(() => {
    if (!loaded) return;
    return on("backup-created", prepend);
  });

  async function snapshotNow() {
    creating = true;
    try {
      prepend(await createBackup());
      toasts.push({ kind: "success", message: "Snapshot created." });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't create a snapshot: ${toAppError(e).message}` });
    } finally {
      creating = false;
    }
  }
</script>

<div class="wrap" {@attach firstVisible(loadList)}>
  <SettingsSection title="Backups">
    {#snippet actions()}
      <Button
        size="sm"
        loading={creating}
        disabled={settingsBusy.moving}
        onclick={snapshotNow}
      >
        {#snippet icon()}<Plus size={14} />{/snippet}
        Create snapshot now
      </Button>
    {/snippet}
    <SettingsRow label="Automatic backups" for="pref-bk-on">
      <Switch
        id="pref-bk-on"
        label="Automatic backups"
        hideLabel
        bind:checked={enabled}
        disabled={!prefs.value}
        onchange={(v) => savePrefs({ enabled: v })}
      />
    </SettingsRow>
    <SettingsRow label="Interval" for="pref-bk-int">
      <NumberInput
        id="pref-bk-int"
        bind:value={interval}
        min={BACKUP_INTERVAL_RANGE.min}
        max={BACKUP_INTERVAL_RANGE.max}
        digits={3}
        suffix="min"
        disabled={!prefs.value || !enabled}
        onchange={(v) => savePrefs({ intervalMinutes: v })}
      />
    </SettingsRow>
    <SettingsRow label="Keep" for="pref-bk-keep">
      <NumberInput
        id="pref-bk-keep"
        bind:value={retention}
        min={BACKUP_RETENTION_RANGE.min}
        max={BACKUP_RETENTION_RANGE.max}
        digits={3}
        disabled={!prefs.value}
        onchange={(v) => savePrefs({ retention: v })}
      />
    </SettingsRow>
    <SettingsRow label="Snapshots">
      {#if loadError}
        <span class="muted">{loadError}</span>
      {:else if !backups || !List}
        <span class="muted">Loading…</span>
      {:else if backups.length === 0}
        <span class="muted">No snapshots yet.</span>
      {:else}
        <span class="muted">{backups.length}</span>
      {/if}
    </SettingsRow>
    {#if List && backups && backups.length > 0}
      <List {backups} disabled={settingsBusy.moving} onremoved={removeRow} />
    {/if}
  </SettingsSection>
  <p class="note">Backups cover everything except media files.</p>
</div>

<style>
  .muted,
  .note {
    color: var(--text-muted);
  }
  .note {
    margin-top: var(--s-2);
    font-size: var(--fs-sm);
  }
</style>

<!--
  Inline import preview: one checkbox per section in the file (checked when it has changes,
  disabled when it has none) with its list of changes; Apply imports only the checked sections.
  Loaded lazily by SettingsFileSection.

  Props:
    path: string              the settings file
    preview: ImportPreview    from `previewSettingsImport(path)`
    onclose: () => void       hide the panel (after Apply succeeded, or Cancel)
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { toAppError } from "$lib/api";
  import { importSettings } from "$lib/api/settings-file";
  import Button from "$lib/components/Button.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { AppError, ImportPreview, SettingsSection } from "$lib/types";
  import { SECTION_LABELS, formatDateTime, formatValue } from "./format";
  import { settingsBusy } from "./state.svelte";

  type Props = { path: string; preview: ImportPreview; onclose: () => void };
  let { path, preview, onclose }: Props = $props();

  const id = $props.id();
  let heading: HTMLElement | undefined = $state();
  let applying = $state(false);
  let error = $state<string | null>(null);

  // Initial selection: every section that has changes. The panel is remounted per file.
  let checked = $state<Partial<Record<SettingsSection, boolean>>>(
    untrack(() =>
      Object.fromEntries(preview.sections.map((s) => [s.section, s.changes.length > 0])),
    ),
  );

  const sections = $derived(
    preview.sections.map((s) => ({
      ...s,
      label: SECTION_LABELS[s.section] ?? s.section,
      count: s.changes.length === 1 ? "1 change" : `${s.changes.length} changes`,
      lines: s.changes.map((c) => ({
        key: c.key,
        from: formatValue(c.from),
        to: formatValue(c.to),
      })),
    })),
  );
  const selected = $derived(
    preview.sections
      .filter((s) => s.changes.length > 0 && checked[s.section])
      .map((s) => s.section),
  );
  const exportedAt = $derived(formatDateTime(preview.exportedAt));

  onMount(() => heading?.focus());

  async function apply() {
    if (selected.length === 0) return;
    applying = true;
    error = null;
    try {
      await importSettings(path, selected);
    } catch (e) {
      error = describe(toAppError(e));
      applying = false;
      return;
    }
    try {
      await prefs.load();
      // Reloads ui-state too when a project is open.
      await project.refresh();
      toasts.push({ kind: "success", message: "Settings imported. A backup was taken first." });
    } catch (e) {
      toasts.push({
        kind: "error",
        message: `Settings were imported, but reloading them failed: ${toAppError(e).message}`,
      });
    } finally {
      applying = false;
    }
    onclose();
  }

  function describe(e: AppError): string {
    switch (e.code) {
      case "invalid":
        return `This isn't a valid settings file. ${e.message}`;
      case "newerVersion":
        return `This file is from a newer version of the app. ${e.message}`;
      case "noProject":
        return "Open a project to import project settings or control layout.";
      default:
        return `Couldn't import: ${e.message}`;
    }
  }
</script>

<div class="panel" role="group" aria-labelledby="{id}-h">
  <h3 id="{id}-h" class="title" tabindex="-1" bind:this={heading}>Import settings</h3>
  <p class="muted">
    Exported <span class="mono">{exportedAt}</span> from version
    <span class="mono">{preview.appVersion}</span>. Choose what to apply; a backup is taken
    first.
  </p>
  <p class="file mono">{path}</p>

  {#if sections.length === 0}
    <p class="muted">This file contains no settings.</p>
  {/if}

  {#each sections as s (s.section)}
    {@const empty = s.changes.length === 0}
    <div class="sec" class:empty>
      <label class="check">
        <input
          type="checkbox"
          bind:checked={checked[s.section]}
          disabled={empty || applying}
        />
        <span class="name">{s.label}</span>
        <span class="count">{empty ? "No changes" : s.count}</span>
      </label>
      {#if !empty}
        <ul class="changes mono">
          {#each s.lines as c (c.key)}
            <li>
              <span class="key">{c.key}:</span>
              <span class="from">{c.from}</span>
              <span aria-hidden="true">→</span><span class="visually-hidden">changes to</span>
              <span class="to">{c.to}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/each}

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <div class="actions">
    <Button variant="ghost" disabled={applying} onclick={onclose}>Cancel</Button>
    <Button
      variant="primary"
      loading={applying}
      disabled={selected.length === 0 || settingsBusy.moving}
      onclick={apply}
    >
      Apply
    </Button>
  </div>
</div>

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
  .count {
    color: var(--text-muted);
  }
  .file {
    font-size: var(--fs-sm);
    color: var(--text-muted);
    overflow-wrap: anywhere;
    user-select: text;
  }
  .sec {
    min-width: 0;
    padding: var(--s-1) 0;
    border-top: 1px solid var(--line);
  }
  .sec.empty .check {
    opacity: 0.6;
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--control-h-sm);
  }
  .check input {
    accent-color: var(--accent);
  }
  .name {
    font-weight: 500;
  }
  .count {
    margin-left: auto;
    font-size: var(--fs-sm);
  }
  .changes {
    margin: var(--s-1) 0 var(--s-1);
    padding-left: calc(var(--s-2) + 13px + var(--s-2));
    list-style: none;
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .changes li {
    overflow-wrap: anywhere;
  }
  .key,
  .to {
    color: var(--text);
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

<!--
  Preferences > Project location: where the project lives (mono path), Show in Explorer / Finder, and
  Move project… (the move panel loads lazily after a destination folder is picked). No props.
-->
<script lang="ts">
  import { tick } from "svelte";
  import FolderOpen from "lucide-svelte/icons/folder-open";
  import { toAppError } from "$lib/api";
  import Button from "$lib/components/Button.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { pickFolder, revealInExplorer } from "$lib/projectFlow";
  import { revealLabel } from "$lib/platform";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { settingsBusy } from "./state.svelte";

  type MovePanel = typeof import("./MoveProject.svelte").default;

  let Panel = $state.raw<MovePanel | null>(null);
  let dest = $state<string | null>(null);
  let picking = $state(false);

  const path = $derived(project.current?.path ?? "");

  async function reveal() {
    try {
      await revealInExplorer(path);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't show the folder: ${toAppError(e).message}` });
    }
  }

  async function startMove() {
    picking = true;
    try {
      // The panel chunk loads while the folder picker is open.
      const [picked, mod] = await Promise.all([
        pickFolder("Choose a new location for the project"),
        import("./MoveProject.svelte"),
      ]);
      if (!picked) return;
      Panel = mod.default;
      dest = picked;
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      picking = false;
    }
  }

  async function closePanel() {
    dest = null;
    await tick();
    document.getElementById("pref-move")?.focus();
  }
</script>

<SettingsSection title="Project location">
  <SettingsRow label="Folder">
    <span class="where">
      <span class="path mono" title={path}>{path}</span>
      <Button size="sm" variant="ghost" onclick={reveal}>
        {#snippet icon()}<FolderOpen size={14} />{/snippet}
        {revealLabel()}
      </Button>
    </span>
  </SettingsRow>
  <SettingsRow
    label="Move project"
    description="Copies everything, media included, to a new folder."
  >
    <Button
      id="pref-move"
      size="sm"
      loading={picking}
      disabled={settingsBusy.moving || dest !== null}
      onclick={startMove}
    >
      Move project…
    </Button>
  </SettingsRow>
  {#if Panel && dest}
    <Panel {dest} onclose={closePanel} />
  {/if}
</SettingsSection>

<style>
  /* A long path ellipsizes (full path in the tooltip) instead of widening the row. */
  .where {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .path {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-sm);
    color: var(--text-muted);
    user-select: text;
  }
</style>

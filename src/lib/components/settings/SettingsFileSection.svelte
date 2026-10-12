<!--
  Preferences > Settings file: export to / import from one JSON file. The dialog plugin and the
  import preview load on demand. No props.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { toAppError } from "$lib/api";
  import { exportSettings, previewSettingsImport } from "$lib/api/settings-file";
  import Button from "$lib/components/Button.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { AppError, ImportPreview } from "$lib/types";
  import { settingsBusy } from "./state.svelte";

  type PreviewComponent = typeof import("./ImportPreview.svelte").default;

  const FILTERS = [{ name: "Settings", extensions: ["json"] }];

  let exporting = $state(false);
  let reading = $state(false);
  let Preview = $state.raw<PreviewComponent | null>(null);
  let pending = $state.raw<{ path: string; preview: ImportPreview } | null>(null);

  async function exportFile() {
    exporting = true;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        title: "Export settings",
        defaultPath: "kinetra-settings.json",
        filters: FILTERS,
      });
      if (!path) return;
      await exportSettings(path);
      toasts.push({ kind: "success", message: `Settings exported to ${path}` });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't export: ${toAppError(e).message}` });
    } finally {
      exporting = false;
    }
  }

  async function importFile() {
    reading = true;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        title: "Import settings",
        multiple: false,
        directory: false,
        filters: FILTERS,
      });
      if (typeof picked !== "string") return;
      const [preview, mod] = await Promise.all([
        previewSettingsImport(picked),
        import("./ImportPreview.svelte"),
      ]);
      Preview = mod.default;
      pending = { path: picked, preview };
    } catch (e) {
      toasts.push({ kind: "error", message: describeImportError(toAppError(e)) });
    } finally {
      reading = false;
    }
  }

  async function closePreview() {
    pending = null;
    await tick();
    document.getElementById("pref-import")?.focus();
  }

  function describeImportError(e: AppError): string {
    switch (e.code) {
      case "invalid":
        return `This isn't a valid settings file. ${e.message}`;
      case "newerVersion":
        return `This file is from a newer version of the app. ${e.message}`;
      default:
        return `Couldn't read the settings file: ${e.message}`;
    }
  }
</script>

<SettingsSection title="Settings file">
  <SettingsRow
    label="Export settings"
    description="API keys and secrets are never included."
  >
    <Button id="pref-export" size="sm" loading={exporting} onclick={exportFile}>Export settings…</Button>
  </SettingsRow>
  <SettingsRow
    label="Import settings"
    description="You choose what to apply."
  >
    <Button
      id="pref-import"
      size="sm"
      loading={reading}
      disabled={settingsBusy.moving || pending !== null}
      onclick={importFile}
    >
      Import settings…
    </Button>
  </SettingsRow>
  {#if Preview && pending}
    <Preview path={pending.path} preview={pending.preview} onclose={closePreview} />
  {/if}
</SettingsSection>

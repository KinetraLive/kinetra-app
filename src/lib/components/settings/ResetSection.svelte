<!--
  Preferences > Reset: four scoped resets, each confirmed (ConfirmDialog loads on first use).
  Only the two broad resets use the danger button; the scoped ones are secondary.
  No props.
-->
<script lang="ts">
  import { toAppError } from "$lib/api";
  import { resetSettings } from "$lib/api/reset";
  import Button from "$lib/components/Button.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { ResetScope } from "$lib/types";
  import { RESET_DONE } from "./format";
  import { Confirm } from "./lazy.svelte";
  import { settingsBusy } from "./state.svelte";

  type Item = {
    scope: ResetScope;
    label: string;
    description: string;
    title: string;
    message: string;
    danger: boolean;
  };

  const BACKUP = "A backup of the project is taken first.";
  const ITEMS: Item[] = [
    {
      scope: "appearance",
      label: "Reset appearance",
      description: "Follow the system theme again.",
      title: "Reset appearance?",
      message: "The theme follows the system again, light or dark.",
      danger: false,
    },
    {
      scope: "controlLayout",
      label: "Reset control layout",
      description: "Control view and hotkeys for this project.",
      title: "Reset control layout?",
      message: `Control mode goes back to its default view and all layer hotkeys are removed. ${BACKUP}`,
      danger: false,
    },
    {
      scope: "preferences",
      label: "Reset all preferences",
      description: "Theme, backup settings and this project's layout.",
      title: "Reset all preferences?",
      message: `Theme, backup settings and this project's layout go back to their defaults. ${BACKUP}`,
      danger: true,
    },
    {
      scope: "everything",
      label: "Reset everything",
      description: "All preferences, plus recent projects. Closes the project.",
      title: "Reset everything?",
      message: `All preferences are reset, the recent projects list is cleared and this project closes. Project files stay on disk. ${BACKUP}`,
      danger: true,
    },
  ];

  const confirm = new Confirm<Item>();

  async function reset(item: Item) {
    try {
      await resetSettings(item.scope);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't reset: ${toAppError(e).message}` });
      return;
    }
    try {
      await prefs.load();
      // Reloads ui-state when the project is still open; null after "everything".
      await project.refresh();
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't reload settings: ${toAppError(e).message}` });
    }
    // The main window follows `project-changed` (back to the picker when the project closed).
    toasts.push({ kind: "success", message: RESET_DONE[item.scope] });
  }

  // Without a project only the machine-wide resets apply.
  const shown = $derived(project.current ? ITEMS : ITEMS.filter((i) => i.scope !== "controlLayout"));
</script>

<SettingsSection title="Reset">
  {#each shown as item (item.scope)}
    <SettingsRow label={item.label} description={item.description}>
      <Button
        id="pref-reset-{item.scope}"
        size="sm"
        variant={item.danger ? "danger" : "secondary"}
        disabled={settingsBusy.moving}
        aria-label={item.label}
        onclick={() => confirm.open(item)}
      >
        Reset…
      </Button>
    </SettingsRow>
  {/each}
</SettingsSection>

{#if confirm.Dialog && confirm.value}
  <confirm.Dialog
    title={confirm.value.title}
    message={confirm.value.message}
    confirmLabel={confirm.value.label}
    danger={confirm.value.danger}
    busy={confirm.busy}
    onconfirm={() => confirm.run(reset)}
    oncancel={() => confirm.close()}
  />
{/if}

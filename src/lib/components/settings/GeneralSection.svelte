<!--
  Preferences > General: Mode (Simple / Advanced, per machine) and the project's default screen
  resolution (one line instead while no project is open). The theme and zoom are under Appearance.
  No props.
-->
<script lang="ts">
  import Segmented from "$lib/components/control/Segmented.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { setUiMode, uiMode } from "$lib/stores/mode.svelte";
  import { project } from "$lib/stores/project.svelte";
  import type { UiMode } from "$lib/types";
  import { saveProjectSettings } from "./save";

  const MODES: { value: UiMode; label: string }[] = [
    { value: "simple", label: "Simple" },
    { value: "advanced", label: "Advanced" },
  ];

  type ResKey = "1920x1080" | "1280x720" | "3840x2160" | "1080x1920" | "custom";
  const RESOLUTIONS: { value: ResKey; label: string }[] = [
    { value: "1920x1080", label: "1920 × 1080" },
    { value: "1280x720", label: "1280 × 720" },
    { value: "3840x2160", label: "3840 × 2160" },
    { value: "1080x1920", label: "1080 × 1920 vertical" },
    { value: "custom", label: "Custom" },
  ];
  const DIM = { min: 16, max: 8192 };

  const res = $derived(project.current?.settings.defaultResolution ?? { width: 1920, height: 1080 });
  const savedKey = $derived.by((): ResKey => {
    const k = `${res.width}x${res.height}`;
    return RESOLUTIONS.some((r) => r.value === k) ? (k as ResKey) : "custom";
  });

  /** Set when the user picks Custom while a preset is saved; shows the size fields. */
  let customOpen = $state(false);
  // Writable deriveds: controls follow the project, and a rejected save snaps them back.
  let resKey = $derived<ResKey>(customOpen ? "custom" : savedKey);
  let width = $derived(res.width);
  let height = $derived(res.height);

  async function pickResolution(key: ResKey) {
    if (key === "custom") {
      customOpen = true;
      return;
    }
    customOpen = false;
    const [w, h] = key.split("x").map(Number);
    if (w === res.width && h === res.height) return;
    if (!(await saveProjectSettings({ defaultResolution: { width: w, height: h } }))) resync();
  }

  async function saveSize() {
    if (!(await saveProjectSettings({ defaultResolution: { width, height } }))) resync();
  }

  function resync() {
    resKey = customOpen ? "custom" : savedKey;
    width = res.width;
    height = res.height;
  }
</script>

<SettingsSection title="General">
  <SettingsRow label="Mode" description="Simple hides Flows, template code, MIDI and APIs.">
    <Segmented label="Mode" options={MODES} value={uiMode()} onchange={(m: UiMode) => void setUiMode(m)} />
  </SettingsRow>
  {#if !project.current}
    <p class="none">Open a project to change its settings.</p>
  {:else}
    <SettingsRow label="Default screen resolution" for="pref-resolution">
      <!-- Resolutions are technical values: the whole select is mono. -->
      <Select
        id="pref-resolution"
        class="mono"
        bind:value={resKey}
        options={RESOLUTIONS}
        onchange={pickResolution}
      />
    </SettingsRow>
    {#if resKey === "custom"}
      <SettingsRow label="Custom size" for="pref-res-w">
        <NumberInput
          id="pref-res-w"
          aria-label="Width"
          bind:value={width}
          min={DIM.min}
          max={DIM.max}
          digits={4}
          suffix="px"
          onchange={saveSize}
        />
        <span class="x mono" aria-hidden="true">×</span>
        <NumberInput
          aria-label="Height"
          bind:value={height}
          min={DIM.min}
          max={DIM.max}
          digits={4}
          suffix="px"
          onchange={saveSize}
        />
      </SettingsRow>
    {/if}
  {/if}
</SettingsSection>

<style>
  .none {
    padding: var(--s-3) 0;
    border-top: 1px solid var(--line);
    color: var(--text-muted);
  }
  .x {
    color: var(--text-muted);
  }
</style>

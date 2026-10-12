<!--
  Preferences > Appearance: Theme (System / Light / Dark) and Zoom of the app windows (both per
  machine), then the project's Style & brand (`#style`). No props.
-->
<script lang="ts">
  import { toAppError } from "$lib/api";
  import { setZoom } from "$lib/api/app";
  import Button from "$lib/components/Button.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { theme } from "$lib/stores/theme.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { ZOOM_MAX, ZOOM_MIN, ZOOM_STEP, type Theme } from "$lib/types";
  import StyleSection from "../StyleSection.svelte";
  import Anchor from "./Anchor.svelte";
  import ProjectOnly from "./ProjectOnly.svelte";

  const THEMES: { value: Theme; label: string }[] = [
    { value: "system", label: "System" },
    { value: "light", label: "Light" },
    { value: "dark", label: "Dark" },
  ];

  const pct = (z: number) => Math.round(z * 100);
  const saved = $derived(pct(prefs.value?.zoom ?? 1));
  // Writable derived: follows prefs (other window, reset), snaps back when a save fails.
  let zoom = $derived(saved);

  async function saveZoom(value: number) {
    try {
      const stored = await setZoom(value / 100);
      const p = prefs.value;
      if (p) prefs.set({ ...p, zoom: stored === 1 ? undefined : stored });
      zoom = pct(stored);
    } catch (e) {
      zoom = saved;
      toasts.push({ kind: "error", message: `Couldn't change the zoom: ${toAppError(e).message}` });
    }
  }
</script>

<SettingsSection title="Appearance">
  <SettingsRow label="Theme">
    <Segmented label="Theme" options={THEMES} value={theme.pref} onchange={(v) => theme.set(v)} />
  </SettingsRow>
  <SettingsRow label="Zoom" for="pref-zoom">
    <NumberInput
      id="pref-zoom"
      bind:value={zoom}
      min={pct(ZOOM_MIN)}
      max={pct(ZOOM_MAX)}
      step={pct(ZOOM_STEP)}
      digits={3}
      suffix="%"
      onchange={saveZoom}
    />
    {#if zoom !== 100}
      <Button size="sm" variant="ghost" onclick={() => saveZoom(100)}>Actual size</Button>
    {/if}
  </SettingsRow>
</SettingsSection>

<Anchor id="style">
  <ProjectOnly title="Style & brand">
    <StyleSection />
  </ProjectOnly>
</Anchor>

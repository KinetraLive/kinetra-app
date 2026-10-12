<!--
  Preferences > Server: port and LAN access, saved per project, plus the live server status as a
  tally dot and a mono URL. The Preferences window wraps this section in the `#server` anchor
  (Network & API; the top bar links here when the port is taken). Watches the
  server status store while mounted. OSC (./midi/Osc.svelte: on/off, port, allowlist, on-air status
  target, the address list, `#osc`) follows as a lazy chunk, loaded when the section mounts, in
  Advanced mode only. No props.
-->
<script lang="ts">
  import NumberInput from "$lib/components/NumberInput.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { lanAccessNote } from "$lib/platform";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { server } from "$lib/stores/server.svelte";
  import { saveProjectSettings } from "./save";

  type Osc = typeof import("./midi/Osc.svelte").default;
  let Osc = $state.raw<Osc | null>(null);
  const advanced = $derived(isAdvanced());
  $effect(() => {
    if (advanced && !Osc) void import("./midi/Osc.svelte").then((m) => (Osc = m.default), () => {});
  });

  const settings = $derived(project.current?.settings);
  // Writable deriveds: follow the project, snap back when a save is rejected.
  let port = $derived(settings?.port ?? 7770);
  let lanAccess = $derived(settings?.lanAccess ?? false);

  $effect(() => server.watch());
  const status = $derived(server.status);
  const lanUrl = $derived(status?.state === "running" ? status.lanUrl : null);
  const dot = $derived.by(() => {
    switch (status?.state) {
      case "running":
        return "preview";
      case "error":
        return "warning";
      default:
        return "idle";
    }
  });

  async function savePort(value: number) {
    if (!(await saveProjectSettings({ port: value }))) port = project.current?.settings.port ?? port;
  }

  async function saveLan(value: boolean) {
    if (!(await saveProjectSettings({ lanAccess: value })))
      lanAccess = project.current?.settings.lanAccess ?? false;
  }
</script>

<SettingsSection title="Server">
  <SettingsRow label="Port" for="pref-port">
    <NumberInput
      id="pref-port"
      bind:value={port}
      min={1024}
      max={65535}
      digits={5}
      onchange={savePort}
    />
  </SettingsRow>
  <SettingsRow
    label="Allow access from other devices on the network"
    description={lanAccessNote()}
    for="pref-lan"
  >
    <Switch
      id="pref-lan"
      label="Allow access from other devices on the network"
      hideLabel
      bind:checked={lanAccess}
      onchange={saveLan}
    />
  </SettingsRow>
  <SettingsRow label="Status">
    <p class="status" role="status">
      <TallyDot state={dot} />
      {#if status?.state === "running"}
        <span>Running at <span class="mono url">{status.url}</span></span>
      {:else if status?.state === "starting"}
        <span>Starting on port <span class="mono">{status.port}</span>…</span>
      {:else if status?.state === "error" && status.code === "portInUse"}
        <span class="warn">
          Port <span class="mono">{status.port}</span> is in use — choose another port.
        </span>
      {:else if status?.state === "error"}
        <span class="warn">{status.message}</span>
      {:else}
        <span class="muted">Not running</span>
      {/if}
    </p>
  </SettingsRow>
  {#if lanUrl}
    <SettingsRow label="On your network">
      <span class="mono url">{lanUrl}</span>
    </SettingsRow>
  {/if}
  {#if Osc && advanced}
    <div id="osc" class="anchor" tabindex="-1"><Osc /></div>
  {/if}
</SettingsSection>

<style>
  .anchor {
    scroll-margin-top: var(--s-5);
  }
  .anchor:focus {
    outline: none;
  }
  .status {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .url {
    user-select: text;
  }
  .warn {
    color: var(--warning);
  }
  .muted {
    color: var(--text-muted);
  }
</style>

<!--
  Preferences > Server > OSC (a lazy chunk the Server section loads when it first scrolls near the
  viewport). Saved per project (`ProjectSettings.osc`): OSC on/off, the UDP port (mono) with the
  listener's status under it (`osc_status` + `osc-status` while mounted; nothing when off), the
  allowlist of other computers (IP literals; used only with "Allow access from other devices"),
  where on-air status is sent (host:port, empty = nowhere), and the address list with the ids of a
  picked layer, cue and source filled in, each copyable. No props.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Copy from "lucide-svelte/icons/copy";
  import X from "lucide-svelte/icons/x";
  import { listFlows } from "$lib/api/flows";
  import { onOscStatus, oscStatus } from "$lib/api/midi";
  import { listScreens } from "$lib/api/live";
  import { listSources } from "$lib/api/sources";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { cueText } from "$lib/components/flows/cue";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { TEST_SCREEN_ID, type FlowInfo, type LiveScreenView, type OscSettings, type OscStatus, type SourceSummary } from "$lib/types";
  import { saveProjectSettings } from "../save";
  import { OSC_DEFAULTS, oscAddresses, oscStatusLine, parseTarget, targetString, withAllowed } from "./osc";

  const osc = $derived<OscSettings>(project.current?.settings.osc ?? OSC_DEFAULTS);
  let enabled = $derived(osc.enabled);
  let port = $derived(osc.port);
  let target = $derived(osc.feedback ? targetString(osc.feedback) : "");
  let targetBad = $state(false);
  let ip = $state("");
  let ipBad = $state(false);

  const save = (patch: Partial<OscSettings>) => saveProjectSettings({ osc: { ...osc, ...patch } });

  function addIp() {
    const next = withAllowed(osc.allow, ip);
    ipBad = next === null;
    if (!next) return;
    ip = "";
    void save({ allow: next });
  }

  function commitTarget() {
    const v = target.trim();
    if (v === "") {
      targetBad = false;
      if (osc.feedback) void save({ feedback: null });
      return;
    }
    const f = parseTarget(v);
    targetBad = f === null;
    if (f && (f.host !== osc.feedback?.host || f.port !== osc.feedback?.port)) void save({ feedback: f });
  }

  // ---- Listener status ---------------------------------------------------------------------------
  let status = $state.raw<OscStatus | null>(null);
  const line = $derived(oscStatusLine(status));
  onMount(() => {
    let live = false;
    const off = onOscStatus((s) => {
      live = true;
      status = s;
    });
    oscStatus().then((s) => !live && (status = s), () => {});
    return off;
  });

  // ---- Address list ------------------------------------------------------------------------------
  let screens = $state.raw<LiveScreenView[]>([]);
  let flows = $state.raw<FlowInfo[]>([]);
  let sources = $state.raw<SourceSummary[]>([]);
  onMount(() => {
    listScreens().then((l) => (screens = l.filter((v) => v.screen.id !== TEST_SCREEN_ID && !v.screen.id.startsWith("__"))), () => {});
    listFlows().then((l) => (flows = l.filter((f) => cueText(f) !== null)), () => {});
    listSources().then((l) => (sources = l.filter((s) => s.kind !== "schedule")), () => {});
  });
  const layerOptions = $derived(
    screens.flatMap((v) => v.layers.map((l) => ({ value: `${v.screen.id}/${l.id}`, label: `${v.screen.name} · ${l.name}` }))),
  );
  let layerPick = $state("");
  let flowPick = $state("");
  let sourcePick = $state("");
  const layer = $derived(layerPick || layerOptions[0]?.value || "");
  const flow = $derived(flowPick || flows[0]?.id || "");
  const source = $derived(sourcePick || sources[0]?.id || "");
  const addresses = $derived.by(() => {
    const [screenId, layerId] = layer ? layer.split("/") : [null, null];
    return oscAddresses({ screenId, layerId, flowId: flow || null, sourceId: source || null });
  });

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.push({ kind: "info", message: `Copied ${text}` });
    } catch {
      toasts.push({ kind: "error", message: "Couldn't copy to the clipboard." });
    }
  }
</script>

<SettingsRow label="OSC control" for="pref-osc">
  <Switch id="pref-osc" label="OSC control" hideLabel bind:checked={enabled} onchange={(on) => void save({ enabled: on })} />
</SettingsRow>
<SettingsRow label="OSC port" for="pref-osc-port">
  <NumberInput id="pref-osc-port" class="mono" bind:value={port} min={1024} max={65535} digits={5} onchange={(n) => void save({ port: n })} />
</SettingsRow>
{#if line}
  <SettingsRow label="OSC status">
    <p class="status" role="status">
      {#if line.kind === "listening"}
        <TallyDot state="preview" />
        <span>
          Listening on <span class="mono">{line.port}</span>{#if line.lan}{" "}<span class="muted">· other devices allowed</span>{/if}
        </span>
      {:else if line.kind === "portInUse"}
        <TallyDot state="warning" />
        <span class="warn">Port <span class="mono">{line.port}</span> is in use — choose another port.</span>
      {:else}
        <TallyDot state="warning" />
        <span class="warn">{line.message}</span>
      {/if}
    </p>
  </SettingsRow>
{/if}
<SettingsRow label="Allowed computers" description="Used only when other devices may connect." stacked>
  <div class="allow">
    {#each osc.allow as a (a)}
      <span class="chip">
        <span class="mono">{a}</span>
        <IconButton size="sm" label="Remove {a}" tooltip="top" onclick={() => void save({ allow: osc.allow.filter((x) => x !== a) })}>
          <X size={12} />
        </IconButton>
      </span>
    {/each}
    <span class="field vf vf-within">
      <input
        class="mono"
        type="text"
        aria-label="IP address to allow"
        placeholder="192.168.1.20"
        spellcheck="false"
        autocomplete="off"
        aria-invalid={ipBad ? "true" : undefined}
        bind:value={ip}
        oninput={() => (ipBad = false)}
        onkeydown={(e) => e.key === "Enter" && addIp()}
      />
    </span>
    <Button size="sm" disabled={ip.trim() === ""} onclick={addIp}>Allow</Button>
    {#if ipBad}<span class="err" role="alert">Enter an IP address, like 192.168.1.20.</span>{/if}
  </div>
</SettingsRow>
<SettingsRow label="Send on-air status to" for="pref-osc-fb">
  <span class="fbrow">
    <span class="field vf vf-within">
      <input
        id="pref-osc-fb"
        class="mono"
        type="text"
        placeholder="host:port"
        spellcheck="false"
        autocomplete="off"
        aria-invalid={targetBad ? "true" : undefined}
        bind:value={target}
        onchange={commitTarget}
        onkeydown={(e) => e.key === "Enter" && commitTarget()}
      />
    </span>
    {#if targetBad}<span class="err" role="alert">Use host:port, like 192.168.1.20:9001.</span>{/if}
  </span>
</SettingsRow>
<SettingsRow label="Addresses" stacked>
  <div class="addr">
    <div class="picks">
      {#if layerOptions.length > 0}
        <Select size="sm" aria-label="Layer for the addresses" value={layer} options={layerOptions} onchange={(v: string) => (layerPick = v)} />
      {/if}
      {#if flows.length > 0}
        <Select size="sm" aria-label="Cue for the addresses" value={flow} options={flows.map((f) => ({ value: f.id, label: cueText(f) ?? f.name }))} onchange={(v: string) => (flowPick = v)} />
      {/if}
      {#if sources.length > 0}
        <Select size="sm" aria-label="Source for the addresses" value={source} options={sources.map((s) => ({ value: s.id, label: s.name }))} onchange={(v: string) => (sourcePick = v)} />
      {/if}
    </div>
    <ul class="list">
      {#each addresses as a (a.address)}
        <li>
          <span class="what">{a.label}</span>
          <code class="mono">{a.address}{#if a.arg}<span class="arg">{a.arg}</span>{/if}</code>
          <IconButton size="sm" label="Copy {a.address}" tooltip="left" onclick={() => void copy(a.address)}>
            <Copy size={12} />
          </IconButton>
        </li>
      {/each}
    </ul>
  </div>
</SettingsRow>

<style>
  .status {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    margin: 0;
  }
  .warn {
    color: var(--warning);
  }
  .muted {
    color: var(--text-muted);
  }
  .allow,
  .picks,
  .fbrow {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding-left: var(--s-2);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-size: var(--fs-sm);
  }
  .field {
    display: inline-flex;
    border-radius: var(--r-control);
  }
  .field input {
    width: 14rem;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-size: var(--fs-sm);
  }
  .field input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .addr {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .list {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr) auto;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .list li {
    display: grid;
    grid-column: 1 / -1;
    grid-template-columns: subgrid;
    align-items: center;
    min-height: var(--row-h);
    border-top: 1px solid var(--line);
  }
  .what,
  code {
    padding-right: var(--s-3);
  }
  .what {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  code {
    overflow-wrap: anywhere;
    font-size: var(--fs-xs);
    user-select: text;
  }
  .arg {
    margin-left: 1ch;
    color: var(--text-muted);
  }
</style>

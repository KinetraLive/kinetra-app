<!--
  Preferences › Outputs: one NDI® output (an NDI source on the network; Windows). The head: an
  on/off switch, the source name (edited in place; receivers see "PC (name)"), its status (a red
  TallyDot "On program" / "Sending" while a receiver has it, a green one "On preview", muted
  "Ready · no receiver", amber for problems with the runtime's Download link) and Delete. Below:
  Screen, Frame rate, Transparency (off: Background), and in Advanced mode Alpha (Straight /
  Premultiplied), Bus (Program / Preview) and NDI groups. "About NDI®" links to ndi.video.

  Props:
    output: OutputConfig            kind "ndi"
    outputs: OutputConfig[]         all of them (edits)
    status?: OutputStatus
    screens: { id, name }[] | null  null while no project is open
    downloadUrl: string             the NDI Runtime's download page
    onsave: (next: OutputConfig[]) => void
-->
<script lang="ts">
  import ExternalLink from "lucide-svelte/icons/external-link";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import Button from "$lib/components/Button.svelte";
  import Segmented from "$lib/components/control/Segmented.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { BACKGROUNDS, patchOutput, removeOutput, statusLine, withBackground, withNdi, type BackgroundKind } from "$lib/components/outputs/model";
  import { NDI_URL, openExternal } from "$lib/components/outputs/ndi-links";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { NDI_RATES, type NdiOutputSettings, type NdiRate, type OutputConfig, type OutputStatus } from "$lib/types";

  type Props = {
    output: OutputConfig;
    outputs: readonly OutputConfig[];
    status?: OutputStatus;
    screens: readonly { id: string; name: string }[] | null;
    downloadUrl: string;
    onsave: (next: OutputConfig[]) => void;
  };
  let { output: o, outputs, status, screens, downloadUrl, onsave }: Props = $props();

  const uid = $props.id();
  const ALPHA = [
    { value: "straight", label: "Straight" },
    { value: "premultiplied", label: "Premultiplied" },
  ] as const;
  const BUS = [
    { value: "program", label: "Program" },
    { value: "preview", label: "Preview" },
  ] as const;
  const RATES = NDI_RATES.map((r) => ({ value: r, label: `${r} fps` }));

  const line = $derived(statusLine(o, status));
  const ndi = $derived<NdiOutputSettings>(o.ndi ?? { rate: "50", alpha: true, premultiplied: false, groups: null, bus: "program" });
  let name = $derived(o.name);
  let groups = $derived(ndi.groups ?? "");

  const patch = (p: Partial<OutputConfig>) => onsave(patchOutput(outputs, o.id, p));
  const setNdi = (p: Partial<NdiOutputSettings>) => patch(withNdi(o, p));

  function rename() {
    const next = name.trim().slice(0, 40);
    if (!next) {
      name = o.name;
      return;
    }
    if (outputs.some((c) => c.id !== o.id && c.name.toLowerCase() === next.toLowerCase())) {
      toasts.push({ kind: "error", message: `There is already an output named “${next}”.` });
      name = o.name;
      return;
    }
    if (next !== o.name) patch({ name: next });
  }

  function saveGroups() {
    const next = groups.trim().slice(0, 128);
    if (next !== (ndi.groups ?? "")) setNdi({ groups: next || null });
  }

  const screenOptions = $derived.by(() => {
    if (!screens) return [{ value: "", label: "Open a project to choose a screen" }];
    const opts = [{ value: "", label: "No screen" }, ...screens.map((s) => ({ value: s.id, label: s.name }))];
    if (o.screenId && !screens.some((s) => s.id === o.screenId)) opts.push({ value: o.screenId, label: "Screen not found" });
    return opts;
  });
</script>

<li class="output" data-output={o.id} data-kind="ndi" aria-labelledby="{uid}-name">
  <div class="head">
    <Switch label="Send {o.name} over NDI®" hideLabel checked={o.enabled} onchange={(on) => patch({ enabled: on })} />
    <span class="field vf vf-within name">
      <input
        id="{uid}-name"
        type="text"
        aria-label="NDI® source name"
        maxlength="40"
        spellcheck="false"
        autocomplete="off"
        bind:value={name}
        onchange={rename}
        onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
    </span>
    <span class="kind">NDI®</span>
    <span class="status {line.tone}" role="status">
      {#if line.tone === "program" || line.tone === "preview"}
        <TallyDot state={line.tone} />
      {:else if line.tone === "warning"}
        <TriangleAlert size={14} aria-hidden="true" />
      {/if}
      {line.text}
      {#if status?.state === "ndiMissing"}
        <a class="link" href={downloadUrl} onclick={(e) => void openExternal(e, downloadUrl)}>
          Download <ExternalLink size={12} aria-hidden="true" />
        </a>
      {/if}
    </span>
    <a class="link about" href={NDI_URL} onclick={(e) => void openExternal(e, NDI_URL)}>
      About NDI® <ExternalLink size={12} aria-hidden="true" />
    </a>
    <Button size="sm" variant="danger" onclick={() => onsave(removeOutput(outputs, o.id))}>Delete</Button>
  </div>

  <div class="fields">
    <label class="lbl" for="{uid}-screen">Screen</label>
    <Select
      id="{uid}-screen"
      size="sm"
      disabled={!screens}
      value={screens ? (o.screenId ?? "") : ""}
      options={screenOptions}
      onchange={(v: string) => patch({ screenId: v || null })}
    />

    <label class="lbl" for="{uid}-rate">Frame rate</label>
    <Select id="{uid}-rate" size="sm" value={ndi.rate} options={RATES} onchange={(v: NdiRate) => setNdi({ rate: v })} />

    <label class="lbl" for="{uid}-alpha">Transparency</label>
    <span class="ctl">
      <Switch id="{uid}-alpha" label="Send transparency" hideLabel checked={ndi.alpha} onchange={(on) => setNdi({ alpha: on })} />
    </span>

    {#if !ndi.alpha}
      <span class="lbl">Background</span>
      <span class="ctl">
        <Segmented
          label="Background of {o.name}"
          options={BACKGROUNDS}
          value={o.background.kind}
          onchange={(k: BackgroundKind) => patch({ background: withBackground(o, k) })}
        />
        {#if o.background.kind === "color"}
          <span class="swatch vf vf-within">
            <input
              type="color"
              aria-label="Background color of {o.name}"
              value={o.background.color}
              onchange={(e) => patch({ background: { kind: "color", color: e.currentTarget.value.toLowerCase() } })}
            />
          </span>
          <span class="mono note">{o.background.color}</span>
        {/if}
      </span>
    {/if}

    {#if isAdvanced()}
      {#if ndi.alpha}
        <span class="lbl">Alpha</span>
        <span class="ctl">
          <Segmented
            label="Alpha of {o.name}"
            options={ALPHA}
            value={ndi.premultiplied ? "premultiplied" : "straight"}
            onchange={(v: "straight" | "premultiplied") => setNdi({ premultiplied: v === "premultiplied" })}
          />
        </span>
      {/if}

      <span class="lbl">Bus</span>
      <span class="ctl"><Segmented label="Bus of {o.name}" options={BUS} value={ndi.bus} onchange={(v: "program" | "preview") => setNdi({ bus: v })} /></span>

      <label class="lbl" for="{uid}-groups">NDI® groups</label>
      <span class="field vf vf-within">
        <input
          id="{uid}-groups"
          type="text"
          maxlength="128"
          spellcheck="false"
          autocomplete="off"
          placeholder="Public"
          bind:value={groups}
          onchange={saveGroups}
          onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
      </span>
    {/if}
  </div>
</li>

<style>
  .output {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3) 0 var(--s-4);
    border-top: 1px solid var(--line);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-width: 0;
  }
  .field {
    display: inline-flex;
    justify-self: start;
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
  }
  .name input {
    font-weight: 500;
  }
  .kind {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .status {
    display: inline-flex;
    flex: 1;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .status.muted,
  .note {
    color: var(--text-muted);
  }
  .status.warning {
    color: var(--warning);
  }
  .note {
    font-size: var(--fs-sm);
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--accent-text);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .about {
    color: var(--text-muted);
  }
  .about:hover {
    color: var(--text);
  }
  .fields {
    display: grid;
    grid-template-columns: 8rem minmax(0, 1fr);
    align-items: center;
    gap: var(--s-2) var(--s-3);
    padding-left: calc(32px + var(--s-3));
  }
  .lbl {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .fields > :global(.wrap) {
    justify-self: start;
    min-width: 16rem;
    max-width: 100%;
  }
  .ctl {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .swatch {
    --vf-radius: var(--r-control);
    display: inline-flex;
    width: var(--control-h-sm);
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .swatch input {
    width: 100%;
    height: 100%;
    padding: 2px;
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    cursor: pointer;
  }
  @container (max-width: 560px) {
    .fields {
      grid-template-columns: minmax(0, 1fr);
      padding-left: 0;
    }
  }
</style>

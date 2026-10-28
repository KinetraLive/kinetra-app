<!--
  Preferences > Network & API > Remote control (`#remote`): people control this show from a
  browser at control.kinetra.live. Project-only; a Pro feature (ProLock "control").
    Remote control   the switch (this project, until Kinetra quits: off at every start)
    Status           TallyDot + words; Start again after it ended or stopped
    Control link     while on: the link with Copy and Open (remote/ControlLink.svelte)
    Collaborators    Invite collaborator: the app's dialog (lazy) when the server offers invites,
                     else the control site's Collaborators page
    Lock remote take while on
    Button panel     Edit button panel… (a lazy dialog, $lib/components/remote/panel/PanelEditor.svelte):
                     buttons collaborators press on the control site; also with remote control off
    Allow remote preview   this project (project.json `remotePreview`, on by default)
    Preview streams  this computer (prefs `remote.previewStreams`, 1–4); both only where the
                     computer can send live pictures (`view.preview`: Windows)
    Separate show    "Make this a separate show…" once the project has a show (a copied project
                     shares it with the original; this gives it its own link and collaborators)
    Connection details  a lazy chunk (./remote/RemoteDetails.svelte): Test connection, numbers,
                     Export diagnostics…, Copy run ID
  The core decides everything; this mirrors `remote` (watched while mounted). No props.
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import Select from "$lib/components/Select.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import ProLock from "$lib/components/license/ProLock.svelte";
  import { toAppError } from "$lib/api";
  import { remoteLock, remoteNewShow, remoteSet, remoteStartAgain, remoteTakeover } from "$lib/api/remote";
  import { chipOf } from "$lib/components/remote/chip.svelte";
  import ControlLink from "$lib/components/remote/ControlLink.svelte";
  import { openInBrowser } from "$lib/components/license/open";
  import { remote } from "$lib/components/remote/state.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { PREVIEW_STREAMS } from "$lib/types";
  import { saveProjectSettings } from "./save";

  type Details = typeof import("./remote/RemoteDetails.svelte").default;
  type Confirm = typeof import("$lib/components/ConfirmDialog.svelte").default;

  $effect(() => remote.watch());
  const view = $derived(remote.view);
  const chip = $derived(chipOf(view));

  let busy = $state(false);
  async function run(f: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      await f();
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      busy = false;
    }
  }

  const status = $derived.by(() => {
    if (!view?.on) return "Off";
    switch (view.state) {
      case "live":
        return view.crew.length === 0 ? "On · no one connected yet" : `On · ${view.crew.length} connected`;
      case "connecting":
        return "Connecting…";
      case "reconnecting":
        return "Reconnecting…";
      case "showInUse":
        return `Remote control is running on ${view.machine}. Move it here?`;
      case "ended":
        return "Ended";
      case "stopped":
        return view.message;
      default:
        return "Off";
    }
  });

  let Details = $state.raw<Details | null>(null);
  $effect(() => {
    void import("./remote/RemoteDetails.svelte").then((m) => (Details = m.default), () => {});
  });

  type Invite = typeof import("$lib/components/remote/InviteDialog.svelte").default;
  let Invite = $state.raw<Invite | null>(null);
  let inviting = $state(false);
  async function invite() {
    if (!view?.invites) {
      if (view?.crewUrl) openInBrowser(view.crewUrl);
      return;
    }
    Invite ??= (await import("$lib/components/remote/InviteDialog.svelte")).default;
    inviting = true;
  }

  const allowPreview = $derived(project.current?.settings.remotePreview ?? true);
  const STREAMS = ["1", "2", "3", "4"].map((n) => ({ value: n, label: n }));
  const storedStreams = () => String(prefs.value?.remote?.previewStreams ?? PREVIEW_STREAMS.default);
  let streams = $state(String(PREVIEW_STREAMS.default));
  $effect(() => {
    streams = storedStreams();
  });
  async function saveStreams(v: string) {
    const n = Number(v);
    const current = prefs.value;
    if (!current) return;
    try {
      await prefs.update({ ...current, remote: { ...current.remote, previewStreams: n } });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't save the setting: ${toAppError(e).message}` });
      streams = storedStreams();
    }
  }

  type Panel = typeof import("$lib/components/remote/panel/PanelEditor.svelte").default;
  let Panel = $state.raw<Panel | null>(null);
  let editingPanel = $state(false);
  async function editPanel() {
    Panel ??= (await import("$lib/components/remote/panel/PanelEditor.svelte")).default;
    editingPanel = true;
  }

  let Confirm = $state.raw<Confirm | null>(null);
  let confirming = $state(false);
  async function askSeparate() {
    Confirm ??= (await import("$lib/components/ConfirmDialog.svelte")).default;
    confirming = true;
  }
</script>

<SettingsSection title="Remote control">
  <ProLock feature="control" label="Remote control">
    <SettingsRow label="Remote control" description="Control this show from a browser." for="pref-remote">
      <Switch
        id="pref-remote"
        label="Remote control"
        hideLabel
        checked={view?.on ?? false}
        disabled={busy}
        onchange={(on) => run(() => remoteSet(on))}
      />
    </SettingsRow>
    <SettingsRow label="Status">
      <p class="status" role="status">
        <TallyDot state={chip?.tally ?? "idle"} />
        <span>{status}</span>
        {#if view?.on && (view.state === "ended" || view.state === "stopped")}
          <Button size="sm" disabled={busy} onclick={() => run(remoteStartAgain)}>Start again</Button>
        {:else if view?.state === "showInUse"}
          <Button size="sm" variant="primary" disabled={busy} onclick={() => run(remoteTakeover)}>Move it here</Button>
          <Button size="sm" disabled={busy} onclick={() => run(remoteNewShow)}>Make this a separate show</Button>
        {/if}
      </p>
    </SettingsRow>
    {#if view?.on && view.state !== "ended" && view.state !== "stopped"}
      <SettingsRow label="Control link">
        <ControlLink url={view.controllerUrl} labeled={false} />
      </SettingsRow>
    {/if}
    {#if view?.on && (view.invites || view.crewUrl)}
      <SettingsRow label="Collaborators" description="Only people you invite can control this show.">
        <Button onclick={invite}>Invite collaborator{view.invites ? "…" : " ↗"}</Button>
      </SettingsRow>
    {/if}
    {#if view?.on}
      <SettingsRow label="Lock remote take" description="Collaborators can't change what's on air." for="pref-remote-lock">
        <Switch
          id="pref-remote-lock"
          label="Lock remote take"
          hideLabel
          checked={view.takeLocked}
          disabled={busy}
          onchange={(on) => run(() => remoteLock(on))}
        />
      </SettingsRow>
    {/if}
    <SettingsRow label="Button panel" description="Buttons collaborators can press.">
      <Button onclick={editPanel}>Edit button panel…</Button>
    </SettingsRow>
    {#if view?.preview}
      <SettingsRow label="Allow remote preview" description="Collaborators can watch your screens live." for="pref-remote-preview">
        <Switch
          id="pref-remote-preview"
          label="Allow remote preview"
          hideLabel
          checked={allowPreview}
          onchange={(on) => saveProjectSettings({ remotePreview: on })}
        />
      </SettingsRow>
      <SettingsRow label="Preview streams" description="Each one uses about 100 MB of memory." for="pref-remote-streams">
        <Select id="pref-remote-streams" bind:value={streams} options={STREAMS} onchange={saveStreams} />
      </SettingsRow>
    {/if}
    {#if view?.hasShowId}
      <SettingsRow label="Separate show" description="A copied project shares its show and collaborators.">
        <Button disabled={busy} onclick={askSeparate}>Make this a separate show…</Button>
      </SettingsRow>
    {/if}
  </ProLock>
</SettingsSection>

{#if Details}
  <Details />
{/if}

{#if Invite}
  <Invite open={inviting} onclose={() => (inviting = false)} crewUrl={view?.crewUrl ?? null} />
{/if}

{#if Panel}
  <Panel open={editingPanel} onclose={() => (editingPanel = false)} />
{/if}

{#if Confirm && confirming}
  <Confirm
    title="Make this a separate show?"
    message="This project gets its own remote control link and collaborators. The other copy keeps the current ones."
    confirmLabel="Make a separate show"
    {busy}
    onconfirm={() =>
      run(async () => {
        await remoteNewShow();
        confirming = false;
      })}
    oncancel={() => (confirming = false)}
  />
{/if}

<style>
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    margin: 0;
    min-width: 0;
  }
</style>

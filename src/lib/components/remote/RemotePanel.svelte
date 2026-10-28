<!--
  RemotePanel: remote control at a glance, in the chip's popover (every app page; it opens by
  itself each time remote control is switched on) — never modal, nothing here touches what is on
  air.
    link      the control link with Copy and Open (ControlLink)
    status    the state in words (+ round trip), a license warning, "Move it here?" choices,
              Start again after it ended or stopped
    people    "N collaborators connected": name · what they may do · watching · since (names are
              the owner's to see)
    lock      Lock remote take: remote take / clear / GO and on-air edits are refused
    activity  recent remote actions of this run (in memory only)
    actions   Invite collaborator (`oninvite`: the app's dialog, or the control site's page while
              the server doesn't offer invites), Edit button panel… (`onpanel`), Connection
              details (Preferences), End remote control (the switch stays on: Start again)
  Props: view (RemoteView), activity (RemoteActivity[], newest first), oninvite, onpanel.
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { openPreferences, toAppError } from "$lib/api";
  import { remoteEnd, remoteLock, remoteNewShow, remoteSet, remoteStartAgain, remoteTakeover } from "$lib/api/remote";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { RemoteActivity, RemoteView } from "$lib/types";
  import ControlLink from "./ControlLink.svelte";
  import { summaryOf } from "./permissions";

  let {
    view,
    activity,
    oninvite,
    onpanel,
  }: { view: RemoteView; activity: RemoteActivity[]; oninvite: () => void; onpanel?: () => void } = $props();
  const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

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
    switch (view.state) {
      case "live":
        return view.resumed ? "Live · reconnected" : "Live";
      case "connecting":
        return "Connecting…";
      case "reconnecting":
        return "Reconnecting…";
      case "showInUse":
        return `Remote control is running on ${view.machine}.`;
      case "ended":
        return "Remote control ended.";
      case "stopped":
        return view.message;
      default:
        return "Off";
    }
  });
  const tally = $derived(view.state === "live" ? "preview" : view.state === "reconnecting" || view.state === "stopped" || view.state === "showInUse" ? "warning" : "idle");
</script>

<div class="panel">
  <header class="head">
    <h3>Remote control</h3>
    <Switch label="Remote control" hideLabel checked={view.on} disabled={busy} onchange={(on) => run(() => remoteSet(on))} />
  </header>

  <p class="status" role="status">
    <TallyDot state={tally} />
    <span>{status}</span>
    {#if view.state === "live" && view.rttMs !== null}<span class="muted tnum">· {view.rttMs} ms</span>{/if}
  </p>

  {#if view.on && view.state !== "ended" && view.state !== "stopped"}
    <ControlLink url={view.controllerUrl} />
  {/if}

  {#if view.on && view.screensHidden > 0}
    <p class="note">Collaborators see the first 32 screens.</p>
  {/if}

  {#if view.licenseWarning}
    <p class="warn">{view.licenseWarning.message || "Your license ended."} Remote control stops at {clock(view.licenseWarning.endsMs)}.</p>
  {/if}

  {#if view.state === "showInUse"}
    <p>Move it here?</p>
    <div class="row">
      <Button variant="primary" disabled={busy} onclick={() => run(remoteTakeover)}>Move it here</Button>
      <Button disabled={busy} onclick={() => run(remoteNewShow)}>Make this a separate show</Button>
      <Button variant="ghost" disabled={busy} onclick={() => run(() => remoteSet(false))}>Cancel</Button>
    </div>
  {:else if view.state === "ended" || view.state === "stopped"}
    <div class="row">
      <Button variant="primary" disabled={busy} onclick={() => run(remoteStartAgain)}>Start again</Button>
    </div>
  {/if}

  {#if view.state === "live"}
    <section aria-labelledby="rp-people">
      <h4 class="section-label" id="rp-people">
        {view.crew.length === 0 ? "No one connected yet" : view.crew.length === 1 ? "1 collaborator connected" : `${view.crew.length} collaborators connected`}
      </h4>
      {#if view.crew.length > 0}
        <ul class="list">
          {#each view.crew as p (p.id ?? p.name + p.sinceMs)}
            <li>
              <span class="name">{p.name}</span>
              <span class="muted when">{summaryOf(p)}{p.watching.length > 0 ? " · watching" : ""} · since <span class="tnum">{clock(p.sinceMs)}</span></span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <div class="lock">
      <Switch label="Lock remote take" checked={view.takeLocked} disabled={busy} onchange={(on) => run(() => remoteLock(on))} />
      <span class="muted">Collaborators can still prepare graphics that aren't on air.</span>
    </div>
  {/if}

  {#if activity.length > 0}
    <section aria-labelledby="rp-recent">
      <h4 class="section-label" id="rp-recent">Recent remote actions</h4>
      <ul class="list recent">
        {#each activity.slice(0, 20) as a (a.at + a.text)}
          <li><span class="tnum muted when">{clock(a.at)}</span> <span>{a.text}</span></li>
        {/each}
      </ul>
    </section>
  {/if}

  <div class="row links">
    {#if view.invites || view.crewUrl}
      <Button size="sm" onclick={oninvite}>Invite collaborator{view.invites ? "…" : " ↗"}</Button>
    {/if}
    {#if onpanel}
      <Button size="sm" onclick={onpanel}>Edit button panel…</Button>
    {/if}
    <Button size="sm" variant="ghost" onclick={() => void openPreferences("remote")}>Connection details</Button>
  </div>
  {#if view.state === "live" || view.state === "reconnecting" || view.state === "connecting"}
    <div class="row">
      <Button size="sm" variant="danger" disabled={busy} onclick={() => run(remoteEnd)}>End remote control</Button>
    </div>
  {/if}
</div>

<style>
  .panel {
    display: grid;
    gap: var(--s-3);
    width: 340px;
    max-width: calc(100vw - var(--s-5));
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-2);
  }
  h3 {
    margin: 0;
    font: 600 var(--fs-md) / 1.3 var(--font-heading);
  }
  h4 {
    margin: 0 0 var(--s-1);
  }
  p {
    margin: 0;
  }
  .status {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .links {
    gap: var(--s-1);
  }
  .list {
    display: grid;
    gap: var(--s-1);
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--fs-sm);
  }
  .list li {
    display: flex;
    justify-content: space-between;
    gap: var(--s-2);
    min-width: 0;
  }
  .recent li {
    justify-content: flex-start;
  }
  .when {
    flex: none;
    white-space: nowrap;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lock {
    display: grid;
    gap: var(--s-1);
    font-size: var(--fs-sm);
  }
  .muted {
    color: var(--text-muted);
  }
  .warn {
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .note {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>

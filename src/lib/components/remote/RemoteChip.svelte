<!--
  RemoteChip: remote control's indicator in the top bar, next to the server status, on every app
  page; shown only while the switch is on (a lazy chunk). A remote icon, TallyDot + text ("3 connected", "Reconnecting", "Ended";
  the full wording is its label), the latest remote action as a brief line, and a popover with the
  panel (RemotePanel). The popover opens by itself each time remote control is switched on (the
  control link comes first) and when the show runs on another machine ("Move it here?"); it is
  never modal. Invite collaborator opens InviteDialog (lazy, outside the popover) when the server
  offers invites, else the control site's Collaborators page.
  Props: none (reads `remote`, which the top bar watches).
-->
<script lang="ts">
  import RadioTower from "lucide-svelte/icons/radio-tower";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { remoteActivity } from "$lib/api/remote";
  import { openInBrowser } from "$lib/components/license/open";
  import type { RemoteActivity } from "$lib/types";
  import RemotePanel from "./RemotePanel.svelte";
  import { ActivityLine, chipOf } from "./chip.svelte";
  import { remote } from "./state.svelte";

  const id = $props.id();
  let pop: HTMLDivElement | undefined = $state();
  let activity = $state.raw<RemoteActivity[]>([]);
  const lines = new ActivityLine();
  // The brief line; the open panel's list follows new actions too.
  $effect(() =>
    lines.listen((a) => {
      if (pop?.matches(":popover-open")) activity = [a, ...activity];
    }),
  );

  const view = $derived(remote.view);
  const chip = $derived(chipOf(view));

  /** The list is read when the panel opens and follows new actions while it is open. */
  async function opened(e: ToggleEvent) {
    if (e.newState !== "open") return;
    activity = await remoteActivity().catch(() => []);
  }

  // Just switched on: show the link (once per switch-on, from any window).
  $effect(() => {
    if (remote.justOn && pop) {
      remote.justOn = false;
      if (!pop.matches(":popover-open")) pop.showPopover();
    }
  });

  type Invite = typeof import("./InviteDialog.svelte").default;
  let Invite = $state.raw<Invite | null>(null);
  let inviting = $state(false);
  async function invite() {
    if (!view?.invites) {
      if (view?.crewUrl) openInBrowser(view.crewUrl);
      return;
    }
    pop?.hidePopover();
    Invite ??= (await import("./InviteDialog.svelte")).default;
    inviting = true;
  }

  type Panel = typeof import("./panel/PanelEditor.svelte").default;
  let Panel = $state.raw<Panel | null>(null);
  let editingPanel = $state(false);
  async function editPanel() {
    pop?.hidePopover();
    Panel ??= (await import("./panel/PanelEditor.svelte")).default;
    editingPanel = true;
  }

  // "Move it here?" needs an answer: show the choices.
  let asked = false;
  $effect(() => {
    const ask = view?.state === "showInUse";
    if (ask && !asked && pop && !pop.matches(":popover-open")) pop.showPopover();
    asked = ask;
  });
</script>

{#if chip && view}
  <button type="button" class="chip vf" popovertarget="{id}-panel" aria-label={chip.label} title={chip.label}>
    <RadioTower size={14} aria-hidden="true" />
    <TallyDot state={chip.tally} />
    <span>{chip.text}</span>
    {#if lines.line}<span class="line" role="status">{lines.line.text}</span>{/if}
  </button>
  <div id="{id}-panel" popover bind:this={pop} class="pop" ontoggle={opened}>
    <RemotePanel {view} {activity} oninvite={invite} onpanel={editPanel} />
  </div>
{/if}
{#if Invite}
  <Invite open={inviting} onclose={() => (inviting = false)} crewUrl={view?.crewUrl ?? null} />
{/if}
{#if Panel}
  <Panel open={editingPanel} onclose={() => (editingPanel = false)} />
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    max-width: 360px;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-full);
    font-size: var(--fs-sm);
    box-shadow: inset 0 0 0 1px var(--line);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .chip:hover {
    background: var(--surface-2);
  }
  .line {
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pop {
    margin: auto;
    padding: var(--s-4);
    border: 0;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
  }
  /* Under the chip where anchor positioning exists (the popover's invoker is its anchor). */
  @supports (position-area: bottom) {
    .pop {
      margin: var(--s-1) 0 0;
      inset: auto;
      position-area: bottom span-left;
      position-try-fallbacks: flip-block;
    }
  }
</style>

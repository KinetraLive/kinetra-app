<!--
  InviteDialog: the show's collaborators, for the owner (protocol 2 §9; a lazy chunk, a native
  modal <dialog>). Opened by "Invite collaborator" from the remote panel and Preferences when the
  server announces `invites`; otherwise those open the control site's Collaborators page.
    invite   email + permissions (presets or the ten boxes) → Send invite; the server emails it
    limit    "2 of 5 collaborators" (joined + open invites, the plan's limit per show)
    list     joined first, then pending ("Invite sent", its expiry): name or email, what they may
             do, Edit (the same picker inline, Save) and Remove / Cancel invite (inline confirm)
  Changes apply on the server at once, also to open connections. Nothing here touches what's on
  air. Props: open, onclose, crewUrl (the control site's page, offered when the show isn't there).
-->
<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import { remoteCollaboratorRemove, remoteCollaborators, remoteCollaboratorSet, remoteInvite } from "$lib/api/remote";
  import Badge from "$lib/components/Badge.svelte";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { cloudMessage } from "$lib/components/license/messages";
  import { openInBrowser } from "$lib/components/license/open";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { Collaborator, Collaborators } from "$lib/types";
  import PermissionPicker from "./PermissionPicker.svelte";
  import { PRESETS, summaryOf } from "./permissions";

  let { open, onclose, crewUrl = null }: { open: boolean; onclose: () => void; crewUrl?: string | null } = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let list = $state.raw<Collaborators | null>(null);
  let loadError = $state<string | null>(null);
  /** The show isn't on the control site (never registered, or not this account's). */
  let missing = $state(false);

  let email = $state("");
  let perms = $state<string[]>([...PRESETS.find((p) => p.id === "watch")!.codes]);
  let sending = $state(false);
  let inviteError = $state<string | null>(null);

  /** The row being edited / asked about, and its draft. */
  let editing = $state<string | null>(null);
  let draft = $state<string[]>([]);
  let removing = $state<string | null>(null);
  let busy = $state<string | null>(null);

  const full = $derived(!!list && list.limit.max > 0 && list.limit.used >= list.limit.max);

  const isCode = (e: unknown, code: string) => {
    const err = toAppError(e);
    return err.code === "cloud" && err.apiCode === code;
  };

  /** The API's codes in the words this dialog needs; anything else as the general table says.
   *  `not_found` means the show for the list, the person for a change or removal. */
  function words(e: unknown, about: "show" | "person" = "show"): string {
    const err = toAppError(e);
    if (err.code === "cloud") {
      const max = (err.details as { max?: number } | undefined)?.max;
      switch (err.apiCode) {
        case "already_collaborator":
          return "Already a collaborator. Change their permissions instead.";
        case "collaborator_limit":
          return max ? `Your plan allows ${max} collaborators per show.` : "Your plan's collaborators per show are all used.";
        case "not_found":
          return about === "show" ? "This show isn't on the control site." : "They're no longer on this show.";
      }
    }
    return cloudMessage(e);
  }

  async function load() {
    loadError = null;
    missing = false;
    try {
      list = await remoteCollaborators();
    } catch (e) {
      missing = isCode(e, "not_found");
      loadError = words(e);
    }
  }

  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (open && !d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
      editing = removing = null;
      inviteError = null;
      void load();
    } else if (!open && d.open) {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
    }
  });

  async function invite(e: SubmitEvent) {
    e.preventDefault();
    if (sending) return;
    sending = true;
    inviteError = null;
    try {
      const c = await remoteInvite(email, perms);
      toasts.push({ kind: "success", message: `Invite sent to ${c.email}` });
      email = "";
      await load();
    } catch (err) {
      inviteError = words(err);
    } finally {
      sending = false;
    }
  }

  async function act(c: Collaborator, f: () => Promise<unknown>) {
    if (busy) return;
    busy = c.id;
    try {
      await f();
      editing = removing = null;
      await load();
    } catch (e) {
      toasts.push({ kind: "error", message: words(e, "person") });
      // Gone already (removed on the control site, or the invite expired): show the list as it is.
      if (isCode(e, "not_found")) {
        editing = removing = null;
        await load();
      }
    } finally {
      busy = null;
    }
  }

  function startEdit(c: Collaborator) {
    editing = c.id;
    draft = [...c.permissions];
    removing = null;
  }

  const who = (c: Collaborator) => c.name ?? c.email;
  const day = (iso: string | null) =>
    iso ? new Date(iso).toLocaleDateString([], { day: "numeric", month: "short" }) : null;

  function onCancelEvent(e: Event) {
    e.preventDefault();
    onclose();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{id}-title"
  oncancel={onCancelEvent}
  {@attach backdropClose(() => onclose())}
>
  <header class="head">
    <h2 id="{id}-title" class="title">Invite collaborator</h2>
    <IconButton label="Close" onclick={onclose}><X size={16} /></IconButton>
  </header>
  <div class="body">
    {#if missing}
      <p class="msg" role="status">{loadError}</p>
      {#if crewUrl}
        <Button size="sm" onclick={() => crewUrl && openInBrowser(crewUrl)}>Open the control site</Button>
      {/if}
    {:else}
      <form class="invite" onsubmit={invite}>
        <label class="field" for="{id}-email">
          <span class="section-label">Email</span>
          <span class="vf vf-within input-wrap">
            <input
              id="{id}-email"
              class="input"
              type="email"
              autocomplete="off"
              spellcheck="false"
              maxlength="254"
              required
              bind:value={email}
            />
          </span>
        </label>
        <PermissionPicker bind:value={perms} disabled={sending} />
        {#if inviteError}<p class="error" role="alert">{inviteError}</p>{/if}
        <div class="row">
          <Button type="submit" variant="primary" disabled={sending || full || !email.trim()} loading={sending}>Send invite</Button>
          {#if list && list.limit.max > 0}
            <span class="muted tnum">{list.limit.used} of {list.limit.max} collaborators</span>
          {/if}
        </div>
      </form>

      <section class="people" aria-labelledby="{id}-list">
        <h3 class="section-label" id="{id}-list">Collaborators</h3>
        {#if loadError}
          <p class="error" role="alert">{loadError}</p>
          <Button size="sm" onclick={load}>Try again</Button>
        {:else if list && list.collaborators.length === 0}
          <p class="msg">No one yet.</p>
        {/if}
        {#if list && list.collaborators.length > 0}
          <ul class="list">
            {#each list.collaborators as c (c.id)}
              <li class="person" aria-busy={busy === c.id}>
                <div class="main">
                  <div class="line">
                    <span class="name">{who(c)}</span>
                    {#if c.status === "pending"}<Badge tone="neutral">Invite sent</Badge>{/if}
                  </div>
                  <div class="facts">
                    {#if c.name}<span>{c.email}</span>{/if}
                    <span>{summaryOf(c)}</span>
                    {#if c.status === "pending" && day(c.expiresAt)}<span>Expires {day(c.expiresAt)}</span>{/if}
                  </div>
                  {#if editing === c.id}
                    <div class="edit">
                      <PermissionPicker bind:value={draft} disabled={busy === c.id} label="Permissions of {who(c)}" />
                      <div class="row">
                        <Button size="sm" variant="primary" disabled={busy === c.id} onclick={() => act(c, () => remoteCollaboratorSet(c.id, draft))}>Save</Button>
                        <Button size="sm" variant="ghost" onclick={() => (editing = null)}>Cancel</Button>
                      </div>
                    </div>
                  {:else if removing === c.id}
                    <div class="row confirm" role="group" aria-label={c.status === "pending" ? "Cancel this invite?" : `Remove ${who(c)}?`}>
                      <span>{c.status === "pending" ? "Cancel this invite?" : `Remove ${who(c)}? They're disconnected at once.`}</span>
                      <Button size="sm" variant="danger" disabled={busy === c.id} onclick={() => act(c, () => remoteCollaboratorRemove(c.id))}>
                        {c.status === "pending" ? "Cancel invite" : "Remove"}
                      </Button>
                      <Button size="sm" variant="ghost" onclick={() => (removing = null)}>Keep</Button>
                    </div>
                  {/if}
                </div>
                {#if editing !== c.id && removing !== c.id}
                  <div class="actions">
                    <Button size="sm" variant="ghost" onclick={() => startEdit(c)}>Edit</Button>
                    <Button size="sm" variant="danger" onclick={() => ((removing = c.id), (editing = null))}>
                      {c.status === "pending" ? "Cancel invite" : "Remove"}
                    </Button>
                  </div>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/if}
  </div>
</dialog>

<style>
  .dialog {
    width: min(720px, calc(100vw - 2 * var(--s-6)));
    max-height: calc(100vh - 2 * var(--s-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: hidden;
    display: none;
    flex-direction: column;
  }
  .dialog[open] {
    display: flex;
    animation: enter var(--dur) var(--ease);
  }
  .dialog::backdrop {
    background: var(--overlay);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--s-3) var(--s-3) var(--s-3) var(--s-5);
    border-bottom: 1px solid var(--line);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--s-5);
    padding: var(--s-4) var(--s-5) var(--s-5);
    overflow: auto;
  }
  .invite {
    display: grid;
    gap: var(--s-3);
  }
  .field {
    display: grid;
    gap: var(--s-1);
  }
  .input-wrap {
    display: flex;
    border-radius: var(--r-control);
  }
  .input {
    flex: 1;
    outline: none;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    background: var(--surface-1);
    color: var(--text);
    font: inherit;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .people {
    display: grid;
    gap: var(--s-2);
    justify-items: start;
  }
  .list {
    justify-self: stretch;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .person {
    display: flex;
    gap: var(--s-3);
    align-items: flex-start;
    padding: var(--s-2) 0;
    border-top: 1px solid var(--line);
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .name {
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .edit {
    display: grid;
    gap: var(--s-3);
    margin-top: var(--s-3);
  }
  .confirm {
    margin-top: var(--s-2);
  }
  .actions {
    display: flex;
    gap: var(--s-1);
  }
  .msg,
  .muted {
    margin: 0;
    color: var(--text-muted);
  }
  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  /* Slides only, never fades: opaque from its first frame, which a busy first load can hold for
     seconds (a fade then showed the page, on-air tally lines too, through the dialog). */
  @keyframes enter {
    from {
      transform: translateY(8px);
    }
  }
</style>

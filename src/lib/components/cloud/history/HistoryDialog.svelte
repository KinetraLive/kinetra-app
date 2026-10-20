<!--
  Version history of the open project (a lazy chunk; opened from the cloud chip, the palette and
  Preferences › Cloud backup). A modal the user opened: the versions newest first, grouped by day,
  50 at a time ("Show older"). Each row: time, computer, label, marks (Newest, Kept, Also backed up
  elsewhere, Damaged), what it holds, "Deleted on …" when the plan's history length removes it.
  Per version: Keep (retention never deletes it), Label (inline), Delete (inline confirm; not the
  newest). Restoring a version comes with restore.
  Props: open, onclose.
-->
<script lang="ts">
  import Pin from "lucide-svelte/icons/pin";
  import PinOff from "lucide-svelte/icons/pin-off";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Tag from "lucide-svelte/icons/tag";
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import { cloudBackupNow, cloudVersionDelete, cloudVersionKeep, cloudVersionLabel, cloudVersions } from "$lib/api/cloud";
  import Badge from "$lib/components/Badge.svelte";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { CloudVersionView } from "$lib/types";
  import { byDay, expiresWords, timeLabel, versionBadges, versionFacts } from "./words";

  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let versions = $state.raw<CloudVersionView[]>([]);
  let next = $state<string | null>(null);
  let loading = $state(false);
  let failed = $state<string | null>(null);
  let busy = $state<string | null>(null);
  /** The version whose label is being edited, and the text. */
  let labeling = $state<string | null>(null);
  let labelText = $state("");
  /** The version waiting for "Delete?" to be confirmed. */
  let deleting = $state<string | null>(null);

  const groups = $derived(byDay(versions));

  async function load(before: string | null) {
    loading = true;
    failed = null;
    try {
      const page = await cloudVersions(before);
      versions = before ? [...versions, ...page.versions] : page.versions;
      next = page.next;
    } catch (e) {
      failed = toAppError(e).message;
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (open && !d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
      void load(null);
    } else if (!open && d.open) {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
    }
  });

  function replace(v: CloudVersionView) {
    versions = versions.map((x) => (x.id === v.id ? v : x));
  }

  async function act(versionId: string, f: () => Promise<void>) {
    if (busy) return;
    busy = versionId;
    try {
      await f();
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      busy = null;
    }
  }

  const keep = (v: CloudVersionView) => act(v.id, async () => replace(await cloudVersionKeep(v.id, !v.kept)));

  function startLabel(v: CloudVersionView) {
    labeling = v.id;
    labelText = v.label ?? "";
    deleting = null;
  }

  const saveLabel = (v: CloudVersionView) =>
    act(v.id, async () => {
      replace(await cloudVersionLabel(v.id, labelText.trim() || null));
      labeling = null;
    });

  const remove = (v: CloudVersionView) =>
    act(v.id, async () => {
      await cloudVersionDelete(v.id);
      versions = versions.filter((x) => x.id !== v.id);
      deleting = null;
    });

  function onLabelKey(e: KeyboardEvent, v: CloudVersionView) {
    if (e.key === "Enter") void saveLabel(v);
    if (e.key === "Escape") {
      e.stopPropagation();
      e.preventDefault();
      labeling = null;
    }
  }

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
  onclick={(e) => e.target === dialog && onclose()}
>
  <header class="head">
    <h2 id="{id}-title" class="title">Version history</h2>
    <IconButton label="Close" onclick={onclose}><X size={16} /></IconButton>
  </header>
  <div class="body">
    {#if failed}
      <p class="msg" role="alert">{failed}</p>
      <Button size="sm" onclick={() => load(null)}>Try again</Button>
    {:else if !loading && versions.length === 0}
      <p class="msg">This project hasn't been backed up yet.</p>
      <Button size="sm" variant="primary" onclick={() => act("new", async () => void (await cloudBackupNow()))}>Back up now</Button>
    {/if}
    {#each groups as g (g.day)}
      <h3 class="section-label">{g.day}</h3>
      <ul class="list">
        {#each g.versions as v (v.id)}
          <li class="row" aria-busy={busy === v.id}>
            <div class="main">
              <div class="line">
                <span class="time tnum">{timeLabel(v.createdAt)}</span>
                {#if v.machine}<span class="machine">{v.machine}</span>{/if}
                {#if v.label && labeling !== v.id}<span class="label">{v.label}</span>{/if}
                {#each versionBadges(v) as b (b)}
                  <Badge tone={b === "Damaged" ? "danger" : b === "Newest" ? "accent" : "neutral"}>{b}</Badge>
                {/each}
              </div>
              <div class="facts">
                <span>{versionFacts(v)}</span>
                {#if expiresWords(v)}<span>{expiresWords(v)}</span>{/if}
              </div>
              {#if labeling === v.id}
                <div class="edit">
                  <span class="vf vf-within input-wrap">
                    <!-- svelte-ignore a11y_autofocus -->
                    <input
                      class="input"
                      aria-label="Label"
                      maxlength="100"
                      autofocus
                      bind:value={labelText}
                      onkeydown={(e) => onLabelKey(e, v)}
                    />
                  </span>
                  <Button size="sm" variant="primary" disabled={busy === v.id} onclick={() => saveLabel(v)}>Save</Button>
                  <Button size="sm" variant="ghost" onclick={() => (labeling = null)}>Cancel</Button>
                </div>
              {:else if deleting === v.id}
                <div class="edit" role="group" aria-label="Delete this version?">
                  <span>Delete this version?</span>
                  <Button size="sm" variant="danger" disabled={busy === v.id} onclick={() => remove(v)}>Delete</Button>
                  <Button size="sm" variant="ghost" onclick={() => (deleting = null)}>Cancel</Button>
                </div>
              {/if}
            </div>
            <div class="actions">
              <IconButton
                label={v.kept ? "Don't keep" : "Keep"}
                size="sm"
                pressed={v.kept}
                pressedFill={false}
                disabled={busy === v.id}
                onclick={() => keep(v)}
              >
                {#if v.kept}<PinOff size={14} />{:else}<Pin size={14} />{/if}
              </IconButton>
              <IconButton label="Label" size="sm" disabled={busy === v.id} onclick={() => startLabel(v)}>
                <Tag size={14} />
              </IconButton>
              {#if !v.isHead}
                <IconButton
                  label="Delete version"
                  size="sm"
                  variant="danger"
                  disabled={busy === v.id}
                  onclick={() => {
                    deleting = v.id;
                    labeling = null;
                  }}
                >
                  <Trash2 size={14} />
                </IconButton>
              {/if}
            </div>
          </li>
        {/each}
      </ul>
    {/each}
    {#if next}
      <Button size="sm" variant="ghost" disabled={loading} onclick={() => load(next)}>Show older</Button>
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
    align-items: flex-start;
    gap: var(--s-2);
    padding: var(--s-4) var(--s-5) var(--s-5);
    overflow: auto;
  }
  .section-label {
    margin-top: var(--s-2);
  }
  .list {
    align-self: stretch;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
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
  .time {
    font-weight: 600;
  }
  .machine,
  .facts {
    color: var(--text-muted);
  }
  .label {
    overflow-wrap: anywhere;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3);
    font-size: var(--fs-sm);
  }
  .edit {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    margin-top: var(--s-2);
  }
  .input-wrap {
    display: flex;
    flex: 1;
    min-width: 160px;
    border-radius: var(--r-control);
  }
  .input {
    flex: 1;
    outline: none;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    background: var(--surface-1);
    color: var(--text);
    font: inherit;
  }
  .actions {
    display: flex;
    gap: var(--s-1);
  }
  .msg {
    color: var(--text-muted);
  }
  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
</style>

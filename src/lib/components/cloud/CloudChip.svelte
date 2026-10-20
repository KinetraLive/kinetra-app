<!--
  CloudChip: cloud backup's indicator in the top bar, next to the server status, while the open
  project backs up (a lazy chunk). A cloud icon and quiet words (`chipWords`: a question first,
  then work in progress, the one-time "Backing up to the cloud" notice, problems, then "Last
  backup …"); the popover has the whole sentence, the question's choices, the notice's Turn off /
  OK, the conflict's Open the other version (the history's restore) and Dismiss, Back up now and a
  link to Preferences › Cloud backup. Never modal,
  never opens by itself.
  Props: none (reads `cloud`, which the top bar watches).
-->
<script lang="ts">
  import CloudIcon from "lucide-svelte/icons/cloud";
  import Button from "$lib/components/Button.svelte";
  import { toAppError } from "$lib/api";
  import { cloudAnswer, cloudBackupNow, cloudConflictDismiss, cloudNoticeSeen, cloudSetBackup } from "$lib/api/cloud-backup";
  import { preferencesLink } from "$lib/components/settings/open";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { CloudAnswer } from "$lib/types";
  import { cloud } from "./state.svelte";
  import { openHistory } from "./history/host";
  import { chipWords, conflictWords, questionWords } from "./words";

  const id = $props.id();
  let pop: HTMLDivElement | undefined = $state();
  const view = $derived(cloud.view);
  const chip = $derived(chipWords(view));
  const project = $derived(view?.project ?? null);
  const question = $derived(project?.question ? questionWords(project.question) : null);

  let busy = $state(false);
  async function act(f: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      const v = await f();
      if (v && typeof v === "object" && "access" in v) cloud.view = v as typeof cloud.view;
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      busy = false;
    }
  }
  const answer = (a: CloudAnswer) => act(() => cloudAnswer(a));
</script>

{#if chip && project}
  <button
    type="button"
    class="chip vf"
    class:warning={chip.tone === "warning"}
    popovertarget="{id}-panel"
    aria-label={chip.label}
    title={chip.label}
  >
    <CloudIcon size={14} aria-hidden="true" />
    <span class="text tnum">{chip.text}</span>
  </button>
  <div id="{id}-panel" popover class="pop" bind:this={pop}>
    <p class="lead" role="status">{chip.notice ? "This project backs up to the cloud while you work." : chip.label}</p>
    {#if question}
      <div class="row">
        {#each question.choices as c (c.answer)}
          <Button size="sm" variant={c.primary ? "primary" : undefined} disabled={busy} onclick={() => answer(c.answer)}>{c.label}</Button>
        {/each}
      </div>
    {:else if chip.notice}
      <div class="row">
        <Button size="sm" variant="primary" disabled={busy} onclick={() => act(cloudNoticeSeen)}>OK</Button>
        <Button size="sm" disabled={busy} onclick={() => act(() => cloudSetBackup(false))}>Turn off for this project</Button>
      </div>
    {:else}
      {#if project.conflict && chip.text !== "Backed up on two computers"}
        <p class="more">{conflictWords(project.conflict)}</p>
      {/if}
      <div class="row">
        <Button size="sm" disabled={busy || !!project.run} onclick={() => act(cloudBackupNow)}>Back up now</Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={!project.registered}
          onclick={() => {
            pop?.hidePopover?.();
            openHistory();
          }}>Version history…</Button
        >
        {#if project.conflict?.otherVersionId}
          <Button
            size="sm"
            onclick={() => {
              pop?.hidePopover?.();
              openHistory(project.conflict?.otherVersionId ?? undefined);
            }}>Open the other version</Button
          >
        {/if}
        {#if project.conflict}
          <Button size="sm" disabled={busy} onclick={() => act(cloudConflictDismiss)}>Dismiss</Button>
        {/if}
      </div>
    {/if}
    {#if project.skipped > 0}
      <p class="more">{project.skipped === 1 ? "1 file isn't backed up." : `${project.skipped} files aren't backed up.`}</p>
    {/if}
    <a class="link" href="/settings#cloud" onclick={preferencesLink("cloud")}>Cloud backup settings</a>
  </div>
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    max-width: 280px;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-full);
    font-size: var(--fs-sm);
    box-shadow: inset 0 0 0 1px var(--line);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .chip:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .chip.warning {
    color: var(--warning);
  }
  .text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pop {
    width: 320px;
    margin: auto;
    padding: var(--s-4);
    border: 0;
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
  }
  @supports (position-area: bottom) {
    .pop {
      margin: var(--s-1) 0 0;
      inset: auto;
      position-area: bottom span-left;
      position-try-fallbacks: flip-block;
    }
  }
  .lead {
    margin: 0 0 var(--s-3);
  }
  .more {
    margin: var(--s-3) 0 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .link {
    display: inline-block;
    margin-top: var(--s-3);
    color: var(--accent-text);
    font-size: var(--fs-sm);
  }
</style>

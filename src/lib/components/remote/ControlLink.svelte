<!--
  ControlLink: where collaborators control this show (the control site's link for this show),
  with Copy and Open. "Getting the link…" until the server has registered the show. No QR code
  (owner decision). Used by the remote panel and Preferences › Remote control.
  Props: url (RemoteView.controllerUrl), labeled (the "Control link" heading; off where the row
  already says it, as in Preferences).
-->
<script lang="ts">
  import Copy from "lucide-svelte/icons/copy";
  import ExternalLink from "lucide-svelte/icons/external-link";
  import Button from "$lib/components/Button.svelte";
  import { openInBrowser } from "$lib/components/license/open";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { url, labeled = true }: { url: string | null; labeled?: boolean } = $props();

  async function copy() {
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      toasts.push({ kind: "success", message: "Link copied" });
    } catch {
      toasts.push({ kind: "error", message: "Couldn't copy the link" });
    }
  }
</script>

<section class="link" aria-label="Control link">
  {#if labeled}<span class="section-label">Control link</span>{/if}
  {#if url}
    <span class="url mono">{url}</span>
    <div class="actions">
      <Button size="sm" variant="primary" onclick={copy}>
        {#snippet icon()}<Copy size={14} />{/snippet}
        Copy link
      </Button>
      <Button size="sm" variant="ghost" onclick={() => url && openInBrowser(url)}>
        {#snippet icon()}<ExternalLink size={14} />{/snippet}
        Open
      </Button>
    </div>
  {:else}
    <span class="muted" role="status">Getting the link…</span>
  {/if}
</section>

<style>
  .link {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }
  .url {
    overflow-wrap: anywhere;
    font-size: var(--fs-sm);
    user-select: text;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    margin-top: var(--s-1);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>

<!--
  In place of a Windows-only feature on macOS (display outputs, MIDI): says so and links to the
  macOS page of the docs.

  Props:
    feature: string   what isn't available, e.g. "Display outputs"
-->
<script lang="ts">
  import ExternalLink from "lucide-svelte/icons/external-link";
  import { toAppError } from "$lib/api";
  import { MAC_DOCS_URL } from "$lib/platform";
  import { toasts } from "$lib/stores/toasts.svelte";

  let { feature }: { feature: string } = $props();

  async function open(e: MouseEvent) {
    e.preventDefault();
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(MAC_DOCS_URL);
    } catch (err) {
      toasts.push({ kind: "error", message: `Couldn't open the browser: ${toAppError(err).message}` });
    }
  }
</script>

<div class="not-on-mac" role="note">
  <p><strong>{feature}</strong>: not available on macOS yet.</p>
  <a class="link" href={MAC_DOCS_URL} onclick={open}>
    What works on macOS <ExternalLink size={12} aria-hidden="true" />
  </a>
</div>

<style>
  .not-on-mac {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s-2) var(--s-3);
    padding: var(--s-3) 0;
    border-top: 1px solid var(--line);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  p {
    margin: 0;
  }
  strong {
    color: var(--text);
    font-weight: 600;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    border-radius: var(--r-control);
    color: var(--accent-text);
    text-decoration: none;
  }
  .link:hover {
    text-decoration: underline;
  }
</style>

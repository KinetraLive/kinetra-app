<!--
  PollLink: where the audience votes. The code (large, mono), the short link with Copy, and the QR
  code (made locally by the core, the same one the graphics get) with Copy image (a 1024 px PNG,
  so it pastes crisply into slides).

  Props:
    poll: PollData
    test?: boolean   a test poll: the link goes nowhere, so the panel says so
-->
<script lang="ts">
  import Copy from "lucide-svelte/icons/copy";
  import Button from "$lib/components/Button.svelte";
  import { pollQr } from "$lib/api/polls";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { PollData } from "$lib/types";
  import { shortUrl } from "./model";

  let { poll, test = false }: { poll: PollData; test?: boolean } = $props();

  let qr = $state("");
  $effect(() => {
    const url = poll.url;
    let live = true;
    pollQr(url).then(
      (u) => live && (qr = u),
      () => live && (qr = ""),
    );
    return () => (live = false);
  });

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(poll.url);
      toasts.push({ kind: "success", message: "Voting link copied" });
    } catch {
      toasts.push({ kind: "error", message: "Couldn't copy the link" });
    }
  }

  async function copyImage() {
    try {
      const img = new Image();
      img.src = qr;
      await img.decode();
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 1024;
      const ctx = canvas.getContext("2d");
      if (!ctx) throw new Error("no canvas");
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(img, 0, 0, 1024, 1024);
      const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
      if (!blob) throw new Error("no image");
      await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      toasts.push({ kind: "success", message: "QR code copied" });
    } catch {
      toasts.push({ kind: "error", message: "Couldn't copy the QR code" });
    }
  }
</script>

<section class="link" aria-label="Voting link">
  <div class="text">
    <span class="section-label">Vote at</span>
    <span class="code mono" aria-label="Code {poll.code.split('').join(' ')}">{poll.code}</span>
    <span class="url mono">{shortUrl(poll.url)}</span>
    {#if test}
      <span class="note">Test link: it doesn't take votes.</span>
    {/if}
    <div class="actions">
      <Button size="sm" onclick={copyLink}>
        {#snippet icon()}<Copy size={14} />{/snippet}
        Copy link
      </Button>
      {#if qr}
        <Button size="sm" onclick={copyImage}>Copy QR image</Button>
      {/if}
    </div>
  </div>
  {#if qr}
    <img class="qr" src={qr} alt="QR code for {shortUrl(poll.url)}" width="144" height="144" />
  {/if}
</section>

<style>
  .link {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
  }
  .code {
    font-size: 28px;
    font-weight: 600;
    letter-spacing: 0.08em;
  }
  .url {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .note {
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    margin-top: var(--s-2);
  }
  .qr {
    flex: none;
    border-radius: var(--r-control);
    image-rendering: pixelated;
  }
</style>

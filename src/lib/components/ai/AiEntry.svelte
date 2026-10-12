<!--
  The AI entry button (Templates page: "Generate template"; template editor: "Tweak with AI"),
  behind the Pro lock (locked: a lock + "Pro" that explains what Pro unlocks). Loaded lazily by the
  pages; the dialog is its own chunk, loaded on click.
  Props: mode ({kind:"generate"} | {kind:"tweak", templateId, name}).
-->
<script lang="ts">
  import Sparkles from "lucide-svelte/icons/sparkles";
  import Button from "$lib/components/Button.svelte";
  import ProLock from "$lib/components/license/ProLock.svelte";
  import type AiDialogType from "./AiDialog.svelte";

  type Mode = { kind: "generate" } | { kind: "tweak"; templateId: string; name: string };
  let { mode }: { mode: Mode } = $props();

  let Dialog = $state.raw<typeof AiDialogType | null>(null);
  let open = $state(false);
  const label = $derived(mode.kind === "generate" ? "Generate template" : "Tweak with AI");

  async function show() {
    Dialog ??= (await import("./AiDialog.svelte")).default;
    open = true;
  }
</script>

<ProLock feature="ai" {label}>
  <Button size="sm" onclick={() => void show()}>
    {#snippet icon()}<Sparkles size={14} />{/snippet}
    {label}
  </Button>
</ProLock>
{#if open && Dialog}
  <Dialog {mode} onclose={() => (open = false)} />
{/if}

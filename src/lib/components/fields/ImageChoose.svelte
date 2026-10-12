<!--
  ImageChoose: the file part of an `image` or `video` field inside FieldInputs (lazy chunk, loaded
  only when such a field shows). Images: a fixed-size thumbnail of the value (a project image,
  loaded from the running output server; nothing while it isn't running) and "Choose…" (the asset
  picker, its own lazy chunk); video: "Choose…" opens the media library's picker (video only).
  Clear. The field's text input stays for typing a URL; a web (http) link gets a warning: templates
  load nothing from the network, so it never shows on air.

  Props:
    label: string             the field's label (accessible names)
    kind?: "image" | "video"  (default image)
    value: string
    disabled?: boolean
    onchange: (value: string) => void
-->
<script lang="ts">
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import IconButton from "$lib/components/IconButton.svelte";
  import { thumbUrl } from "$lib/components/assets/assets";
  import { server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  let {
    label,
    kind = "image",
    value,
    disabled = false,
    onchange,
  }: {
    label: string;
    kind?: "image" | "video";
    value: string;
    disabled?: boolean;
    onchange: (value: string) => void;
  } = $props();

  $effect(() => server.watch());

  const src = $derived(kind === "image" ? thumbUrl(server.status, value) : null);
  let busy = $state(false);
  /** A web link (http, https or //host): never shows on air (templates load nothing from the network). */
  const web = $derived(/^(https?:)?\/\//i.test(value.trim()));

  async function choose() {
    if (busy) return;
    busy = true;
    try {
      let v: string | null;
      if (kind === "video") {
        const { pickMedia } = await import("$lib/components/media/pick");
        const a = await pickMedia({ accept: ["video"], current: value, label, allowNone: true });
        v = a === null ? null : a === "" ? "" : a.url;
      } else {
        const { pickAsset } = await import("$lib/components/assets/pick");
        v = await pickAsset({ current: value, label });
      }
      if (v !== null && v !== value) onchange(v);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't open the ${kind === "video" ? "media" : "images"}: ${toAppError(e).message}` });
    } finally {
      busy = false;
    }
  }
</script>

{#if src}
  <img class="thumb" {src} alt="" width="40" height="28" loading="lazy" decoding="async" />
{/if}
<button type="button" class="choose vf" aria-label="Choose {kind} for {label}" {disabled} onclick={() => void choose()}>
  Choose…
</button>
{#if web}
  <!-- Templates load nothing from the network (the renderer's CSP): web links never show on air. -->
  <span class="web" role="note">Web {kind === "video" ? "videos" : "images"} don't show on air. Import it into Kinetra.</span>
{/if}
{#if value !== ""}
  <IconButton size="sm" label="Clear {label}" tooltip="left" {disabled} onclick={() => onchange("")}>
    <X size={14} />
  </IconButton>
{/if}

<style>
  .web {
    flex-basis: 100%;
    color: var(--warning);
    font-size: var(--fs-xs);
  }
  .thumb {
    flex: none;
    order: -1;
    width: 40px;
    height: 28px;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    object-fit: contain;
  }
  .choose {
    flex: none;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    color: var(--accent-text);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .choose:hover {
    background: var(--surface-2);
  }
  .choose:disabled {
    opacity: 0.5;
  }
</style>

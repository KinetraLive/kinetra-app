<!--
  ImageChoose: the file part of an `image` or `video` field inside FieldInputs (lazy chunk, loaded
  only when such a field shows). Images: a fixed-size thumbnail of the value (a project image,
  loaded from the running output server; nothing while it isn't running) and "Choose…" (the asset
  picker, its own lazy chunk); video: "Choose…" opens the media library's picker (video only).
  Clear. The field's text input stays for typing a URL. A web image link (http/https) shows on air
  from a copy Kinetra keeps in the project; its thumbnail is that copy (`/web-image`, downloaded
  when a layer holds the link: tried again twice, 1.5 s apart, since the layer may get the value a
  moment after the field), and "Couldn't download this image." when it fails. A `//host` link and
  web videos still get a warning: they never show on air.

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
  import { isWebImage, thumbUrl, webThumbUrl } from "$lib/components/assets/assets";
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

  let tried = $state.raw<{ value: string; n: number; failed: boolean }>({ value: "", n: 0, failed: false });
  const attempt = $derived(tried.value === value ? tried : { value, n: 0, failed: false });
  const src = $derived.by(() => {
    if (kind !== "image") return null;
    return thumbUrl(server.status, value) ?? (attempt.failed ? null : webThumbUrl(server.status, value, attempt.n));
  });
  let busy = $state(false);
  /** A link that never shows on air: `//host` (no http/https), or a web video. */
  const noAir = $derived(/^\/\//.test(value.trim()) || (kind === "video" && /^https?:\/\//i.test(value.trim())));

  function thumbFailed() {
    if (!isWebImage(value)) return;
    const { value: v, n } = attempt;
    if (n >= 2) {
      tried = { value: v, n, failed: true };
      return;
    }
    setTimeout(() => {
      if (value === v) tried = { value: v, n: n + 1, failed: false };
    }, 1500);
  }

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
  <img class="thumb" {src} alt="" width="40" height="28" loading="lazy" decoding="async" onerror={thumbFailed} />
{/if}
<button type="button" class="choose vf" aria-label="Choose {kind} for {label}" {disabled} onclick={() => void choose()}>
  Choose…
</button>
{#if attempt.failed}
  <span class="web" role="note">Couldn't download this image.</span>
{:else if noAir}
  <!-- Only http/https images get a local copy (the renderer's CSP loads nothing from the internet). -->
  <span class="web" role="note">
    {kind === "video" ? "Web videos don't show on air. Import it into Kinetra." : "Start the link with https:// to show it on air."}
  </span>
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

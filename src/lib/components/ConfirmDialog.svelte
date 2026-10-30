<!--
  Modal confirmation built on native <dialog> + showModal() (focus trap, Esc, inert background).
  Self-contained so pages can load it lazily: `const { default: ConfirmDialog } = await import(...)`.

  The parent owns visibility: render it (or set `open`) and hide it from `onconfirm` / `oncancel`.
  Esc and backdrop clicks call `oncancel` (ignored while `busy`).

  Props:
    title: string
    message?: string
    confirmLabel?: string                                    (default "Confirm")
    cancelLabel?: string                                     (default "Cancel")
    danger?: boolean     destructive tone; initial focus goes to Cancel instead of Confirm
    busy?: boolean       confirm button shows loading, dialog can't be dismissed
    open?: boolean                                           (default true)
    children?: Snippet   extra content under the message (e.g. a "also delete files" checkbox)
    onconfirm: () => void
    oncancel: () => void
-->
<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import type { Snippet } from "svelte";
  import Button from "./Button.svelte";

  type Props = {
    title: string;
    message?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    danger?: boolean;
    busy?: boolean;
    open?: boolean;
    children?: Snippet;
    onconfirm: () => void;
    oncancel: () => void;
  };

  let {
    title,
    message,
    confirmLabel = "Confirm",
    cancelLabel = "Cancel",
    danger = false,
    busy = false,
    open = true,
    children,
    onconfirm,
    oncancel,
  }: Props = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();

  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (open && !d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
      const target = d.querySelector<HTMLButtonElement>(danger ? "[data-cancel]" : "[data-confirm]");
      target?.focus();
    } else if (!open && d.open) {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
    }
  });

  function cancel() {
    if (!busy) oncancel();
  }

  function onCancelEvent(e: Event) {
    // Keep the dialog open; the parent decides (it hides us from `oncancel`).
    e.preventDefault();
    cancel();
  }

</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{id}-title"
  aria-describedby={message ? `${id}-msg` : undefined}
  oncancel={onCancelEvent}
  {@attach backdropClose(cancel)}
>
  <div class="inner">
    <h2 id="{id}-title" class="title">{title}</h2>
    {#if message}<p id="{id}-msg" class="msg">{message}</p>{/if}
    {#if children}<div class="extra">{@render children()}</div>{/if}
    <div class="actions">
      <Button variant="ghost" data-cancel disabled={busy} onclick={cancel}>{cancelLabel}</Button>
      <Button variant={danger ? "danger" : "primary"} data-confirm loading={busy} onclick={onconfirm}>
        {confirmLabel}
      </Button>
    </div>
  </div>
</dialog>

<style>
  .dialog {
    width: min(440px, calc(100vw - 2 * var(--s-6)));
    max-height: calc(100vh - 2 * var(--s-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: auto;
  }
  .dialog[open] {
    animation: enter var(--dur) var(--ease);
  }
  .dialog::backdrop {
    background: var(--overlay);
    animation: fade var(--dur) var(--ease);
  }
  .inner {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-5);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .msg {
    color: var(--text-muted);
  }
  .extra {
    margin-top: var(--s-2);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-2);
    margin-top: var(--s-4);
  }

  /* Slides only, never fades: opaque from its first frame, which a busy first load can hold for

     seconds (a fade then showed the page, on-air tally lines too, through the dialog). */

  @keyframes enter {
    from {
      transform: translateY(8px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
</style>

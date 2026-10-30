/**
 * Closing a modal `<dialog>` from its backdrop: `{@attach backdropClose(() => cancel())}` on the
 * dialog. Only a press that both starts and ends on the backdrop counts, so a text selection
 * dragged out of the dialog (mouse down inside, up outside) never closes it. The click target is
 * the dialog element itself only outside its content: on the backdrop.
 */
export function backdropClose(onBackdrop: () => void) {
  return (dialog: HTMLDialogElement) => {
    let downOnBackdrop = false;
    const down = (e: PointerEvent) => {
      downOnBackdrop = e.target === dialog;
    };
    const click = (e: MouseEvent) => {
      const both = downOnBackdrop && e.target === dialog;
      downOnBackdrop = false;
      if (both) onBackdrop();
    };
    dialog.addEventListener("pointerdown", down);
    dialog.addEventListener("click", click);
    return () => {
      dialog.removeEventListener("pointerdown", down);
      dialog.removeEventListener("click", click);
    };
  };
}

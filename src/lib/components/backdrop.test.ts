import { afterEach, describe, expect, it, vi } from "vitest";
import { backdropClose } from "./backdrop";

function setup() {
  const dialog = document.createElement("dialog");
  const field = document.createElement("input");
  dialog.append(field);
  document.body.append(dialog);
  const onBackdrop = vi.fn();
  const detach = backdropClose(onBackdrop)(dialog);
  // A click goes to the nearest common ancestor of the press and the release.
  const press = (down: Element, up: Element) => {
    down.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    const common = down === up ? down : dialog;
    common.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  };
  return { dialog, field, onBackdrop, detach, press };
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("backdropClose", () => {
  it("a press on the backdrop closes", () => {
    const { dialog, onBackdrop, press } = setup();
    press(dialog, dialog);
    expect(onBackdrop).toHaveBeenCalledTimes(1);
  });

  it("a selection dragged from inside out to the backdrop doesn't close", () => {
    const { dialog, field, onBackdrop, press } = setup();
    press(field, dialog);
    expect(onBackdrop).not.toHaveBeenCalled();
    // The next real backdrop press still works.
    press(dialog, dialog);
    expect(onBackdrop).toHaveBeenCalledTimes(1);
  });

  it("a press inside doesn't close", () => {
    const { field, onBackdrop, press } = setup();
    press(field, field);
    expect(onBackdrop).not.toHaveBeenCalled();
  });

  it("a click without a press (keyboard) doesn't close; detaching removes the listeners", () => {
    const { dialog, onBackdrop, detach, press } = setup();
    dialog.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(onBackdrop).not.toHaveBeenCalled();
    detach();
    press(dialog, dialog);
    expect(onBackdrop).not.toHaveBeenCalled();
  });
});

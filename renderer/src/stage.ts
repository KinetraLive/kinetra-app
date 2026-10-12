// Full-viewport stage with a screen-sized box scaled to fit (letterboxed, centered).
import { scr } from "./layer";
import type { ScreenInfo } from "./protocol";

export class Stage {
  readonly el: HTMLElement;
  readonly box: HTMLElement;
  w = 0;
  h = 0;

  constructor(parent: Element) {
    this.el = document.createElement("div");
    this.el.style.cssText = "position:fixed;inset:0;overflow:hidden";
    this.box = document.createElement("div");
    this.box.style.cssText = "position:absolute;left:0;top:0;overflow:hidden;transform-origin:0 0";
    this.el.append(this.box);
    parent.append(this.el);
    addEventListener("resize", () => this.fit());
  }

  apply(s: ScreenInfo) {
    this.w = Math.max(0, +s.width || 0);
    this.h = Math.max(0, +s.height || 0);
    scr.w = this.w;
    scr.h = this.h;
    const st = this.box.style;
    st.width = this.w + "px";
    st.height = this.h + "px";
    st.background = !s.background || s.background === "transparent" ? "" : s.background;
    this.fit();
  }

  fit() {
    if (!this.w || !this.h) return;
    const vw = innerWidth;
    const vh = innerHeight;
    const k = Math.min(vw / this.w, vh / this.h);
    const x = (vw - this.w * k) / 2;
    const y = (vh - this.h * k) / 2;
    this.box.style.transform = `translate(${x}px,${y}px) scale(${k})`;
  }
}

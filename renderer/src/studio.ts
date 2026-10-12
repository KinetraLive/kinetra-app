// Studio mode on the Program bus: take transitions and the T-bar. A lazy chunk, loaded on the
// first studio message (`App.load`); only opacity is set on layer hosts, never layout.
import type { App } from "./app";
import { FONT_CAP_MS, faces, fontsReady, loadFonts } from "./fonts";
import { Layer, preview } from "./layer";
import type { ServerMsg, WireLayer } from "./protocol";

/**
 * One transition. `add`: incoming hosts, new Layers mounted settled (a changed on-air layer gets a
 * second host next to its current one); `gone`: the on-air layers they replace; `hide`: layers
 * going off air; `later`: definitions that are not on air (applied as they are); `stop`: set
 * while a crossfade runs (cancels its timer and animations).
 */
interface Run {
  add: [string, Layer][];
  gone: Layer[];
  hide: Layer[];
  later: WireLayer[];
  stop?: () => void;
}

const op = (l: Layer, v: number | string) => (l.host.style.opacity = "" + v);
const fade = (l: Layer, a: number, ms: number) => l.host.animate?.({ opacity: [a, 1 - a] }, { duration: ms, fill: "forwards" });

export class Studio {
  private readonly app: App;
  /** The prepared T-bar (no `stop`) or the running crossfade. */
  private run: Run | undefined;

  constructor(app: App) {
    this.app = app;
  }

  handle(m: ServerMsg) {
    const r = this.run;
    const bar = r && !r.stop ? r : undefined;
    if (m.t == "tbar") {
      const p = Math.min(1, Math.max(0, +m.p || 0));
      if (!bar) return;
      this.outs(bar).forEach((l) => op(l, 1 - p));
      bar.add.forEach(([, n]) => op(n, p));
    } else if (m.t == "commit" || m.t == "abort") {
      if (!bar) return;
      this.run = undefined;
      if (m.t == "abort") return this.undo(bar);
      this.adopt(bar);
      this.done(bar);
    } else if (m.t == "prepare") {
      this.end();
      if (!preview) this.run = this.prep(m.layers || [], m.hide || []);
    } else if (m.t == "take") {
      this.end();
      this.take(m.layers || [], m.hide || [], preview ? "cut" : m.transition?.kind, +m.transition?.ms || 0);
    }
  }

  /** Before a take, a prepare, a snapshot, a clear or PANIC: a prepared T-bar is aborted (Program
   *  unchanged), a running crossfade finished. */
  end() {
    const r = this.run;
    this.run = undefined;
    if (r?.stop) this.done(r);
    else if (r) this.undo(r);
  }

  /**
   * `cut`: incoming settled at once, outgoing removed with no animation (PANIC's cut for those
   * layers). `crossfade`: host opacity over `ms` once the incoming fonts are in (capped), then the
   * outgoing hosts go. `outIn`: the outgoing play their out-animations and the incoming their
   * in-animations, together; a changed on-air layer plays out, then in (the variant swap).
   */
  private take(defs: WireLayer[], hide: string[], kind: string | undefined, ms: number) {
    const ls = this.app.layers;
    if (kind == "crossfade" && ms > 0) {
      const r = (this.run = this.prep(defs, hide));
      this.adopt(r);
      const go = () => {
        const a = [...r.add.map(([, n]) => fade(n, 0, ms)), ...this.outs(r).map((l) => fade(l, 1, ms))];
        const t = setTimeout(() => this.run == r && this.end(), ms + 20);
        r.stop = () => (clearTimeout(t), a.forEach((x) => x?.cancel()));
      };
      const f = r.add.flatMap(([, n]) => faces(n.def.css || "", n.vars.values()));
      if (fontsReady(f)) return go();
      const g = () => this.run == r && r.stop == w && (w(), go());
      const t = setTimeout(g, FONT_CAP_MS);
      const w = (r.stop = () => clearTimeout(t));
      loadFonts(f).then(g);
      return;
    }
    if (kind == "outIn") {
      for (const id of hide) ls.get(id)?.setState("out");
      for (const d of defs) {
        const l = ls.get(d.id);
        // On air: the variant swap's out-then-in, also for same-rev content (a stale rev forces it).
        if (l?.state == "in") l.def = { ...l.def, templateRev: -1 };
        this.app.upsert(d, l?.state == "in" ? "outIn" : undefined);
      }
      return;
    }
    for (const id of hide) ls.get(id)?.cut();
    for (const d of defs) {
      const l = ls.get(d.id);
      // On air: recompiled or patched in place, settled. Off air: a new layer, settled.
      if (l?.state == "in" || d.state != "in") this.app.upsert(d);
      else l?.dispose(), ls.set(d.id, this.app.make(d, true));
    }
  }

  /** Mounts the incoming hosts settled at opacity 0; nothing in Program changes yet. */
  private prep(defs: WireLayer[], hide: string[]): Run {
    const ls = this.app.layers;
    const r: Run = { add: [], gone: [], hide: [], later: [] };
    for (const d of defs) {
      if (d.state != "in") {
        r.later.push(d);
        continue;
      }
      const o = ls.get(d.id);
      if (o?.state == "in") r.gone.push(o);
      const n = this.app.make(d, true);
      op(n, 0);
      r.add.push([d.id, n]);
    }
    for (const id of hide) {
      const l = ls.get(id);
      if (l?.state == "in") r.hide.push(l);
    }
    return r;
  }

  private outs(r: Run) {
    return [...r.gone, ...r.hide];
  }

  /** The incoming layers become Program's (the replaced ones stay visible until `done`). */
  private adopt(r: Run) {
    const ls = this.app.layers;
    for (const [id, n] of r.add) {
      const o = ls.get(id);
      if (o && !r.gone.includes(o)) o.dispose();
      ls.set(id, n);
    }
    r.later.forEach((d) => this.app.upsert(d));
  }

  /** The transition's end state: incoming at full opacity, outgoing gone. */
  private done(r: Run) {
    r.stop?.();
    r.add.forEach(([, n]) => op(n, ""));
    r.gone.forEach((l) => l.dispose());
    r.hide.forEach((l) => (l.cut(), op(l, "")));
  }

  /** Abort: the prepared hosts go, the outgoing ones are restored. */
  private undo(r: Run) {
    r.add.forEach(([, n]) => n.dispose());
    this.outs(r).forEach((l) => op(l, ""));
  }
}

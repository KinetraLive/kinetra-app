// Message handling and snapshot reconciliation.
import { setServerNow } from "./clock";
import { Layer, type Swap } from "./layer";
import { PROTOCOL_VERSION, type ServerMsg, type WireLayer } from "./protocol";
import type { Stage } from "./stage";
import type { Studio } from "./studio";

/** Studio messages: handled by the lazily loaded `studio` chunk. */
const STUDIO = /^(take|prepare|tbar|commit|abort)$/;
/** Media layers and video/image fields (in a snapshot, layer, take or prepare): the `media` chunk. */
const MEDIA = /"(media|video|image)"/;

export class App {
  readonly layers = new Map<string, Layer>();
  readonly stage: Stage;
  private readonly onUnknown: () => void;
  /** The studio chunk, once loaded. */
  private st: Studio | undefined;
  /** Every message received while the studio chunk loads, in order. */
  private q: ServerMsg[] | undefined;
  /** The Layer class: the media chunk's once loaded (it handles every layer from then on). */
  private L: typeof Layer | undefined;

  constructor(stage: Stage, onUnknown: () => void = () => {}) {
    this.stage = stage;
    this.onUnknown = onUnknown;
  }

  /** Never throws: a failure is logged and the renderer keeps running. */
  handle(m: ServerMsg) {
    if (this.q) return void this.q.push(m);
    if (!this.st && STUDIO.test(m.t))
      return this.load(m, import("./studio").then((s) => (this.st = new s.Studio(this))));
    const x = m as { layers?: unknown; layer?: unknown };
    try {
      if (!this.L && MEDIA.test(JSON.stringify(x.layers || x.layer || 0)))
        return this.load(
          m,
          import("./media").then(
            (s) => (this.L = s.MediaLayer),
            (e) => {
              this.L = Layer;
              throw e;
            },
          ),
        );
    } catch {
      // Not serializable: dispatched below, the bad layer is skipped there.
    }
    try {
      this.dispatch(m);
    } catch (e) {
      console.warn("renderer:", e);
    }
  }

  /**
   * Loads a chunk (studio: on the first studio message; media: on the first media layer or
   * video/image field); until it is in, every message is queued (order kept). A failed studio load
   * drops the queued studio messages (the next one retries); a failed media load falls back to
   * plain layers.
   */
  private load(m: ServerMsg, p: Promise<unknown>) {
    const q: ServerMsg[] = (this.q = [m]);
    p.catch((e) => console.warn("renderer:", e))
      .then(() => {
        this.q = undefined;
        for (const x of q) if (this.st || !STUDIO.test(x.t)) this.handle(x);
      });
  }

  private dispatch(m: ServerMsg) {
    // A prepared T-bar is aborted, a running crossfade finished, before a new look replaces it.
    if (m.t == "snapshot" || m.t == "cut" || m.t == "clear") this.st?.end();
    switch (m.t) {
      case "hello":
        if (m.protocol !== PROTOCOL_VERSION) console.warn(`renderer: protocol ${m.protocol}, expected ${PROTOCOL_VERSION}`);
        setServerNow(m.now);
        break;
      case "snapshot":
        this.stage.apply(m.screen);
        this.reconcile(m.layers || []);
        break;
      case "layer":
        this.upsert(m.layer, m.layer.swap);
        break;
      case "state":
        this.layers.get(m.layerId)?.setState(m.state);
        break;
      case "fields":
        this.layers.get(m.layerId)?.patch(m.fields || {});
        break;
      case "vars":
        this.layers.get(m.layerId)?.setVars(m.vars || {});
        break;
      case "rect":
        if (m.rect) this.layers.get(m.layerId)?.setRect(m.rect);
        break;
      case "remove":
        this.drop(m.layerId);
        break;
      case "clear":
        for (const l of this.layers.values()) this.guard(l.def.id, () => l.setState("out"));
        break;
      case "cut":
        for (const l of this.layers.values()) this.guard(l.def.id, () => l.cut());
        break;
      case "take":
      case "prepare":
      case "tbar":
      case "commit":
      case "abort":
        this.st?.handle(m);
        break;
      case "media":
        (this.layers.get(m.layerId) as { play?: (p: unknown) => void } | undefined)?.play?.(m.playback);
        break;
      case "error":
        if (m.code === "unknownScreen") {
          for (const id of [...this.layers.keys()]) this.drop(id);
          this.onUnknown();
        } else console.warn("renderer: server error", m.code);
        break;
    }
  }

  private guard(id: string, f: () => void) {
    try {
      f();
    } catch (e) {
      console.warn(`renderer: layer "${id}"`, e);
    }
  }

  /** Adds a layer or applies a new definition to an existing one; `swap` only from live `layer`
   *  messages (snapshots never swap). */
  upsert(def: WireLayer, swap?: Swap) {
    const l = this.layers.get(def.id);
    if (l) l.apply(def, swap);
    else this.layers.set(def.id, this.make(def));
  }

  /** A new layer on this stage (`calm`: its next play-in appears settled). */
  make(def: WireLayer, calm?: boolean): Layer {
    return new (this.L || Layer)(def, this.stage.box, calm);
  }

  drop(id: string) {
    this.layers.get(id)?.dispose();
    this.layers.delete(id);
  }

  /**
   * Same id + templateRev → patch fields/z/rect/vars in place; a state transition only if the state changed (an
   * on-air layer keeps its nodes and data-state). New rev → recompile. New id → create.
   * Missing id → removed.
   */
  reconcile(defs: WireLayer[]) {
    const ids = new Set(defs.map((d) => d.id));
    for (const id of [...this.layers.keys()]) if (!ids.has(id)) this.drop(id);
    for (const d of defs) this.guard(d.id, () => this.upsert(d));
  }
}

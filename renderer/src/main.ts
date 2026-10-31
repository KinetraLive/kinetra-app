// Browser-source entry: /screen/<id> → WebSocket /ws?screen=<id>;
// /screen/<id>/preview → /ws?screen=<id>&bus=preview (layers shown settled).
// `?output=<id>` (a Kinetra display output window) loads the `output` chunk: fit, background,
// key and dual (fill + key) from one connection. `?mute=1` (the app's own previews) is forwarded
// to the WebSocket: such a connection never counts toward media readiness. `?cap=1` (an NDI
// output's capture page) is forwarded too: it counts for readiness but not as an OBS-style viewer.
import { App } from "./app";
import { connect } from "./connection";
import { setPreview, useWebImages } from "./layer";
import type { ServerMsg } from "./protocol";
import { Stage } from "./stage";
import { isPreview, screenId, wsUrl } from "./url";

function start() {
  let badge: HTMLElement | null = null;
  const show = (text: string, color: string) => {
    if (!badge) return;
    badge.textContent = text;
    badge.style.background = color;
  };
  const q = new URLSearchParams(location.search);
  // Nothing but graphics is ever visible unless ?debug is in the URL.
  if (q.has("debug")) {
    badge = document.createElement("div");
    badge.style.cssText =
      "position:fixed;right:8px;top:8px;z-index:2147483647;font:12px/1.4 system-ui,sans-serif;color:#fff;padding:2px 8px;border-radius:4px";
    document.body.append(badge);
  }
  const unknown = () => show("unknown screen", "#b45309");
  const id = screenId(location.pathname);
  const pv = isPreview(location.pathname);
  setPreview(pv);
  useWebImages();
  const go = (h: (m: ServerMsg) => void) =>
    id
      ? connect(wsUrl(location, id, pv) + (q.get("mute") == "1" ? "&mute=1" : q.get("cap") == "1" ? "&cap=1" : ""), h, {
          onStatus: (s) => show(s, s === "connected" ? "#15803d" : "#6b7280"),
        })
      : (console.warn("renderer: no screen id in the URL"), show("no screen id", "#b45309"));
  if (q.has("output")) import("./output").then((o) => go(o.output(q, unknown)), console.warn);
  else {
    const app = new App(new Stage(document.body), unknown);
    go((m) => app.handle(m));
  }
}

try {
  start();
} catch (e) {
  console.warn("renderer:", e);
}

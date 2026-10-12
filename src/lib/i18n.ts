// The one translation function for UI copy. Today English: the message is its own key and `{name}`
// placeholders are filled from `params`. A table (e.g. Norwegian) can be installed later with
// `setMessages`; a message missing from it falls back to English. Never concatenate translated
// fragments: pass params. Part of the app shell, so it stays tiny.

export type Msg = string;

let table: Readonly<Record<Msg, string>> | null = null;

/** Installs a translation table (English message → translation); null restores English. */
export function setMessages(messages: Readonly<Record<Msg, string>> | null): void {
  table = messages;
}

/** `t("Take {layer}", { layer: "Lower third" })` → "Take Lower third". Unknown placeholders stay. */
export function t(msg: Msg, params?: Readonly<Record<string, string | number>>): string {
  const s = table?.[msg] ?? msg;
  if (!params) return s;
  return s.replace(/\{(\w+)\}/g, (m, k: string) => (Object.prototype.hasOwnProperty.call(params, k) ?String(params[k]) : m));
}

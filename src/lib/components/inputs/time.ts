// Forgiving time-of-day parsing for TimeInput. Pure: no Svelte, no DOM.
//
//   parseTime("12.00") → { ok: true, value: "12:00" }
//   parseTime("930")   → { ok: true, value: "09:30" }
//   parseTime("24:00") → { ok: false, reason: "Hours go from 00 to 23." }
//
// Accepted: "H", "HH" (whole hours), "HMM", "HHMM", and "H:MM" / "HH:MM" with ":", "." or "h" as
// the separator (minutes always two digits after a separator). Surrounding spaces are ignored.

export type TimeParse = { ok: true; value: string } | { ok: false; reason: string };

const pad = (n: number) => String(n).padStart(2, "0");

function hm(h: number, m: number): TimeParse {
  if (h > 23) return { ok: false, reason: "Hours go from 00 to 23." };
  if (m > 59) return { ok: false, reason: "Minutes go from 00 to 59." };
  return { ok: true, value: `${pad(h)}:${pad(m)}` };
}

/** A typed time as `HH:mm`, or why it isn't one. Empty text is invalid ("Enter a time."). */
export function parseTime(text: string): TimeParse {
  const t = text.trim();
  if (t === "") return { ok: false, reason: "Enter a time." };
  const sep = /^(\d{1,2})[:.h](\d*)$/i.exec(t);
  if (sep) {
    if (sep[2].length !== 2) return { ok: false, reason: "Use two digits for the minutes, like 09:30." };
    return hm(Number(sep[1]), Number(sep[2]));
  }
  if (/^\d+$/.test(t)) {
    if (t.length <= 2) return hm(Number(t), 0);
    if (t.length <= 4) return hm(Number(t.slice(0, -2)), Number(t.slice(-2)));
    return { ok: false, reason: "Too many digits: use HH:mm, like 09:30." };
  }
  return { ok: false, reason: "Use numbers, like 09:30." };
}

/** `HH:mm` (00:00–23:59): a canonical time-of-day value. */
export const isTime = (v: string) => /^([01]\d|2[0-3]):[0-5]\d$/.test(v);

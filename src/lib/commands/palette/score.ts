// The palette's hand-written fuzzy scorer (part of the palette's lazy chunk; no dependencies).
//
// `query` must be a subsequence of `text` (case- and accent-insensitive: é→e, ü→u, but æ ø å stay
// distinct letters). Results fall into tiers, so a better kind of match always wins:
//   3 substring      "screen" in "Clear Screen 1" (a word start, then earlier, scores higher)
//   2 word starts    every run of matched characters starts a word: "cs" → "Clear Screen";
//                    word starts are after a space or - _ . / : ( ) , and camelCase / letter↔digit
//   1 consecutive    some matched characters are adjacent
//   0 scattered      the rest
// Within a tier, an alignment score (fzf-style: per-character points, word-start and run bonuses,
// gap penalties; the best alignment found with a small dynamic program) decides; ties go to the
// shorter text. One tier is `TIER` points; a whole score stays below `TIER * 4`.
//
// Hot path (`scoreFolded`) works on pre-folded text and a precomputed boundary table so the
// palette can prepare 2000 entries once and score them per keystroke without allocating; the DP
// buffers are module-level typed arrays, grown on demand and reused.

export const TIER = 10_000;

const MATCH = 16;
const BONUS_BOUNDARY = 9; // after a separator / at the start
const BONUS_CAMEL = 8; // camelCase hump, letter↔digit
const BONUS_CONSECUTIVE = 5;
const FIRST_MULT = 2;
const GAP_START = 3;
const GAP_EXT = 1;
/** Texts are scored over their first MAX_TEXT characters; longer queries only match as substrings. */
const MAX_TEXT = 256;
const MAX_QUERY = 64;
const NEG = -1_000_000;

// ---- Folding ------------------------------------------------------------------------------------

const foldCache = new Map<string, string>();

/** One UTF-16 unit folded: lowercase, accents stripped (é→e), æ ø å kept. Always one unit. */
function foldChar(ch: string): string {
  const c = ch.charCodeAt(0);
  if (c < 128) return c >= 65 && c <= 90 ? String.fromCharCode(c + 32) : ch;
  let f = foldCache.get(ch);
  if (f !== undefined) return f;
  const lower = ch.toLowerCase();
  if (lower === "å" || lower === "æ" || lower === "ø") f = lower;
  else {
    const base = lower.normalize("NFD");
    // A base letter plus combining marks (U+0300–U+036F) folds to the base letter.
    f = base.length > 1 && /^[̀-ͯ]+$/.test(base.slice(1)) ? base[0] : lower;
  }
  if (f.length !== 1) f = ch; // keep the 1:1 index mapping (ranges point into the original)
  foldCache.set(ch, f);
  return f;
}

/** Folds a string unit by unit (same length as the input). */
export function fold(s: string): string {
  let ascii = true;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c >= 128 || (c >= 65 && c <= 90)) {
      ascii = false;
      break;
    }
  }
  if (ascii) return s;
  let out = "";
  for (let i = 0; i < s.length; i++) out += foldChar(s[i]);
  return out;
}

/** The query as matched: folded, trimmed, inner whitespace collapsed to one space. */
export function foldQuery(q: string): string {
  return fold(q.trim().replace(/\s+/g, " "));
}

const SEPARATORS = " \t-_./:\\(),[]|&+'\"\u00a0\u2013\u2014\u00b7\u2022\u2026";

/** Character class: 0 separator, 1 lowercase, 2 uppercase, 3 digit, 4 other (incl. uncased letters). */
function cls(ch: string): number {
  const c = ch.charCodeAt(0);
  if (c >= 97 && c <= 122) return 1;
  if (c >= 65 && c <= 90) return 2;
  if (c >= 48 && c <= 57) return 3;
  if (SEPARATORS.includes(ch)) return 0;
  if (c < 128) return 4;
  const lo = ch.toLowerCase();
  const up = ch.toUpperCase();
  if (lo === up) return 4;
  return ch === lo ? 1 : 2;
}

/** Per character of `text` (original case): its word-start bonus (0 inside a word). */
export function boundaries(text: string): Uint8Array {
  const n = Math.min(text.length, MAX_TEXT);
  const b = new Uint8Array(n);
  let prev = 0; // "separator" before the start
  for (let j = 0; j < n; j++) {
    const k = cls(text[j]);
    if (k !== 0) {
      if (prev === 0) b[j] = BONUS_BOUNDARY;
      else if (prev === 1 && k === 2) b[j] = BONUS_CAMEL;
      else if ((prev === 3 && (k === 1 || k === 2)) || (k === 3 && (prev === 1 || prev === 2))) b[j] = BONUS_CAMEL;
    }
    prev = k;
  }
  return b;
}

// ---- Scoring ------------------------------------------------------------------------------------

let cap = 0;
let H = new Int32Array(0); // best score with q[i] matched at t[j]
let P = new Int16Array(0); // previous match position (backtracking), -1 at i = 0
let RB = new Uint8Array(0); // the bonus of the run's first character (carried along a run)

function ensure(size: number) {
  if (size <= cap) return;
  cap = Math.max(size, cap * 2, 1024);
  H = new Int32Array(cap);
  P = new Int16Array(cap);
  RB = new Uint8Array(cap);
}

/** Positions of the best alignment's matched characters, written by `scoreFolded`. */
const positions = new Int16Array(MAX_QUERY);
let positionsLen = 0;
let substringAt = -1;

/**
 * Scores a folded query (from `foldQuery`) against folded text with its `boundaries`. Returns the
 * score or -1 when it doesn't match; `lastRanges()` then returns its ranges.
 */
export function scoreFolded(q: string, t: string, b: Uint8Array): number {
  const m = q.length;
  const n = t.length;
  substringAt = -1;
  positionsLen = 0;
  if (m === 0) return 0;
  if (m > n) return -1;
  const tie = -n / 1000; // shorter text wins ties (stays below 1 point)

  // Tier 3: substring. Prefer an occurrence at a word start; else the first one.
  let at = t.indexOf(q);
  if (at >= 0) {
    let best = at;
    while (at >= 0 && at < b.length && b[at] === 0) at = t.indexOf(q, at + 1);
    if (at >= 0 && at < b.length) best = at;
    substringAt = best;
    const start = best < b.length ? b[best] : 0;
    return 3 * TIER + (start ? 2000 : 0) + (best === 0 ? 1000 : 0) - Math.min(best, 999) + tie;
  }

  // Spaces in the query only separate words; the subsequence ignores them.
  const qs = q.includes(" ") ? q.replace(/ /g, "") : q;
  const mq = qs.length;
  if (mq === 0) return -1;
  if (mq > MAX_QUERY) return -1;

  // Quick reject: is it a subsequence at all? (Most entries stop here.)
  const nt = Math.min(n, MAX_TEXT);
  let ti = 0;
  for (let i = 0; i < mq; i++) {
    const k = t.indexOf(qs[i], ti);
    if (k < 0 || k >= nt) return -1;
    ti = k + 1;
  }

  ensure(mq * nt);
  for (let i = 0; i < mq; i++) {
    const qc = qs.charCodeAt(i);
    const row = i * nt;
    const prow = row - nt;
    let carry = NEG; // best H[i-1][k] - gap penalty for k <= j - 2
    let carryK = -1;
    for (let j = 0; j < nt; j++) {
      if (i > 0 && j >= 2) {
        carry -= GAP_EXT;
        const g = H[prow + j - 2];
        if (g > NEG && g - GAP_START > carry) {
          carry = g - GAP_START;
          carryK = j - 2;
        }
      }
      const idx = row + j;
      if (j < i || t.charCodeAt(j) !== qc) {
        H[idx] = NEG;
        continue;
      }
      const bj = b[j];
      if (i === 0) {
        H[idx] = MATCH + bj * FIRST_MULT;
        P[idx] = -1;
        RB[idx] = bj;
        continue;
      }
      let best = NEG;
      let from = -1;
      let rb = bj;
      const diag = j > 0 ? H[prow + j - 1] : NEG;
      if (diag > NEG) {
        const runBonus = Math.max(RB[prow + j - 1], bj);
        best = diag + MATCH + Math.max(runBonus, BONUS_CONSECUTIVE);
        from = j - 1;
        rb = runBonus;
      }
      if (carry > NEG) {
        const v = carry + MATCH + bj;
        if (v > best) {
          best = v;
          from = carryK;
          rb = bj;
        }
      }
      H[idx] = best;
      P[idx] = from;
      RB[idx] = rb;
    }
  }

  const last = (mq - 1) * nt;
  let end = -1;
  let top = NEG;
  for (let j = mq - 1; j < nt; j++) {
    if (H[last + j] > top) {
      top = H[last + j];
      end = j;
    }
  }
  if (end < 0) return -1;

  // Backtrack the alignment to classify it (and keep the positions for highlighting).
  let j = end;
  for (let i = mq - 1; i >= 0; i--) {
    positions[i] = j;
    j = P[i * nt + j];
  }
  positionsLen = mq;
  let tier = 2;
  let consecutive = false;
  for (let i = 0; i < mq; i++) {
    const p = positions[i];
    const runStart = i === 0 || positions[i - 1] !== p - 1;
    if (!runStart) consecutive = true;
    else if (b[p] === 0) tier = 0;
  }
  if (tier === 0 && consecutive) tier = 1;
  return tier * TIER + Math.max(1, Math.min(top, TIER - 1)) + tie;
}

/** [start, end) ranges of the last `scoreFolded` match (allocates; call for shown rows only). */
export function lastRanges(q: string): [number, number][] {
  if (substringAt >= 0) return [[substringAt, substringAt + q.length]];
  const out: [number, number][] = [];
  for (let i = 0; i < positionsLen; i++) {
    const p = positions[i];
    const prev = out[out.length - 1];
    if (prev && prev[1] === p) prev[1] = p + 1;
    else out.push([p, p + 1]);
  }
  return out;
}

/** The contract's scorer: null unless `query` is a subsequence of `text`; higher is better. */
export function score(query: string, text: string): { score: number; ranges: [number, number][] } | null {
  const q = foldQuery(query);
  if (q === "") return { score: 0, ranges: [] };
  const s = scoreFolded(q, fold(text), boundaries(text));
  if (s < 0) return null;
  return { score: s, ranges: lastRanges(q) };
}

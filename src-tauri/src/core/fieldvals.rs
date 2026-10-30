//! Values of the structured field types (`list`, `clock`, `timer`). Every field value is a string
//! (so staging, persistence, diffs and bindings work unchanged); these helpers validate those
//! strings and give each type one canonical spelling, so equality and diffs are stable:
//!
//! - `list`: a JSON array of row objects. Canonical form: compact JSON, every row has every column
//!   of the field definition in column order (missing cells are `""`). At most `LIST_ROWS_MAX`
//!   rows, cells at most `SOURCE_CELL_MAX` bytes, the whole value at most `LIST_VALUE_MAX` bytes.
//! - `clock`: renderers get the display format (`FieldDef.format`, default `HH:mm:ss`), never a
//!   stored value.
//! - `timer`: a `TimerState` as compact JSON (key order `run, at, base, dir, end`); `""` means the
//!   field's default state.
//!
//! Pure: no I/O, no clock (callers pass `now`).

use std::cmp::Ordering;
use std::collections::HashSet;

use serde_json::Value;

use crate::core::live::FieldDef;
use crate::core::model::{ColumnKind, ListSort, SourceColumn, TimerAction, SOURCE_CELL_MAX};
use crate::core::protocol::{
    FieldType, Fields, TimerDir, TimerEnd, TimerState, LIST_ROWS_MAX, LIST_VALUE_MAX,
};
use crate::core::templates::{is_key, LABEL_MAX};

/// Default `clock` format.
pub const CLOCK_FORMAT_DEFAULT: &str = "HH:mm:ss";
/// Longest `clock` format, in characters.
pub const CLOCK_FORMAT_MAX: usize = 32;
/// Most columns of a `list` field.
pub const LIST_COLUMNS_MAX: usize = 16;
/// Rows of the preview sample of a `list` field.
pub const SAMPLE_ROWS: usize = 3;
/// Largest timer value (1000 hours); larger `Set` values are clamped to it.
pub const TIMER_MS_MAX: u64 = 1000 * 3600 * 1000;

// ---- clock ---------------------------------------------------------------------------------

/// `""` (the default) or 1..=`CLOCK_FORMAT_MAX` characters of the tokens `HH`, `mm`, `ss` and the
/// separators `:`, `.` and space.
pub fn validate_clock_format(format: &str) -> Result<(), String> {
    if format.is_empty() {
        return Ok(());
    }
    let bad = || {
        format!(
            "format may use HH, mm, ss, \":\", \".\" and spaces (at most {CLOCK_FORMAT_MAX} characters)"
        )
    };
    if format.chars().count() > CLOCK_FORMAT_MAX {
        return Err(bad());
    }
    let mut rest = format;
    while !rest.is_empty() {
        if let Some(r) = ["HH", "mm", "ss"].iter().find_map(|t| rest.strip_prefix(t)) {
            rest = r;
        } else if let Some(r) = rest
            .strip_prefix(':')
            .or_else(|| rest.strip_prefix('.'))
            .or_else(|| rest.strip_prefix(' '))
        {
            rest = r;
        } else {
            return Err(bad());
        }
    }
    Ok(())
}

/// What renderers get for a `clock` field: its format, or `HH:mm:ss`.
pub fn clock_format(def: &FieldDef) -> String {
    match def.format.as_deref() {
        Some(f) if !f.is_empty() && validate_clock_format(f).is_ok() => f.to_string(),
        _ => CLOCK_FORMAT_DEFAULT.to_string(),
    }
}

// ---- list ----------------------------------------------------------------------------------

/// A parsed `list` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedList {
    /// The canonical JSON.
    pub json: String,
    /// Cells of unknown columns were dropped (lenient parsing only).
    pub dropped: bool,
}

/// Reserved row keys a `list` value may carry besides its columns (set by the server on rows bound
/// to a source; renderers turn `_status` into `data-status` etc.). Values are [`is_row_token`]s.
/// Canonical order: this order, after the columns.
pub const RESERVED_ROW_KEYS: &[&str] = &[
    "_current",
    "_duration",
    "_status",
    "_highlight",
    "_winner",
    "_leader",
];

/// A reserved row value: `[a-z0-9-]{1,32}`.
pub fn is_row_token(v: &str) -> bool {
    (1..=32).contains(&v.len())
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// One row of a list value: cells in column order plus reserved keys (in `RESERVED_ROW_KEYS`
/// order, valid tokens only).
pub type ListRow = (Vec<String>, Vec<(&'static str, String)>);

/// Canonical JSON of `rows` (cells in `columns` order).
pub fn serialize_rows(columns: &[SourceColumn], rows: &[Vec<String>]) -> String {
    let rows: Vec<ListRow> = rows.iter().map(|r| (r.clone(), Vec::new())).collect();
    serialize_list_rows(columns, &rows)
}

/// Canonical JSON of rows with reserved keys (after the columns).
pub fn serialize_list_rows(columns: &[SourceColumn], rows: &[ListRow]) -> String {
    let mut out = String::with_capacity(2 + rows.len() * columns.len() * 16);
    out.push('[');
    for (i, (row, extra)) in rows.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('{');
        let pairs = columns
            .iter()
            .zip(row)
            .map(|(c, cell)| (c.key.as_str(), cell.as_str()))
            .chain(extra.iter().map(|(k, v)| (*k, v.as_str())));
        for (j, (k, v)) in pairs.enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&json_string(k));
            out.push(':');
            out.push_str(&json_string(v));
        }
        out.push('}');
    }
    out.push(']');
    out
}

/// The reserved keys of a row (valid tokens only), in canonical order.
fn reserved_of(row: &Fields) -> Vec<(&'static str, String)> {
    RESERVED_ROW_KEYS
        .iter()
        .filter_map(|k| {
            row.get(*k)
                .filter(|v| is_row_token(v))
                .map(|v| (*k, v.clone()))
        })
        .collect()
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

// ---- Row marks of library templates (poll, chat) -------------------------------------------

/// A list column whose truthy cells (`true`, `1`, `yes`, any case) mark their row
/// `_highlight: "true"` (chat: a highlighted message).
pub const HIGHLIGHT_COLUMN: &str = "highlighted";
/// A list column ranked for `_winner` (poll: the option's percentage, e.g. `42` or `42.5%`).
pub const WINNER_COLUMN: &str = "pct";
/// The template field whose value `closed` (any case) makes the highest `pct` row(s) of every
/// list field with a [`WINNER_COLUMN`] the winner (`_winner: "true"`; ties all win).
pub const STATUS_FIELD: &str = "status";

fn truthy(v: &str) -> bool {
    matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes")
}

fn pct(v: &str) -> Option<f64> {
    let v = v.trim();
    let v = v.strip_suffix('%').unwrap_or(v).trim();
    v.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Sets the server's row marks on the list values in `values` (wire values, by field key), by
/// convention rather than template configuration, so they work for hand-filled lists and lists
/// bound to a source alike:
/// - `_highlight`: rows whose [`HIGHLIGHT_COLUMN`] cell is truthy;
/// - `_winner`: when the template has a [`STATUS_FIELD`] field whose value is `closed`, the rows
///   with the highest numeric [`WINNER_COLUMN`] cell (none when no cell is a number).
///
/// Marks the convention doesn't call for are removed (a row is never marked by stale data). Lists
/// without those columns are untouched; a value that changes is re-serialized canonically.
pub fn apply_row_marks(defs: &[FieldDef], values: &mut Fields) {
    let closed = defs.iter().any(|d| d.key == STATUS_FIELD)
        && values
            .get(STATUS_FIELD)
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("closed"));
    for def in defs.iter().filter(|d| d.kind == FieldType::List) {
        let hl = def.columns.iter().position(|c| c.key == HIGHLIGHT_COLUMN);
        let win = def.columns.iter().position(|c| c.key == WINNER_COLUMN);
        if hl.is_none() && win.is_none() {
            continue;
        }
        let Some(raw) = values.get(&def.key) else {
            continue;
        };
        let Ok(items) = serde_json::from_str::<Vec<serde_json::Map<String, Value>>>(raw) else {
            continue;
        };
        let mut rows: Vec<ListRow> = items
            .iter()
            .map(|obj| {
                let cells = def
                    .columns
                    .iter()
                    .map(|c| match obj.get(&c.key) {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Null) | None => String::new(),
                        Some(v) => v.to_string(),
                    })
                    .collect();
                let extra = RESERVED_ROW_KEYS
                    .iter()
                    .filter_map(|k| match obj.get(*k) {
                        Some(Value::String(t)) if is_row_token(t) => Some((*k, t.clone())),
                        _ => None,
                    })
                    .collect();
                (cells, extra)
            })
            .collect();
        let best = match (closed, win) {
            (true, Some(w)) => rows
                .iter()
                .filter_map(|(cells, _)| pct(&cells[w]))
                .fold(None, |m: Option<f64>, n| Some(m.map_or(n, |m| m.max(n)))),
            _ => None,
        };
        let mut changed = false;
        for (cells, extra) in &mut rows {
            let want_hl = hl.is_some_and(|i| truthy(&cells[i]));
            let want_win = match (best, win) {
                (Some(b), Some(w)) => pct(&cells[w]).is_some_and(|n| (n - b).abs() <= 1e-9),
                _ => false,
            };
            for (key, want, applies) in [
                ("_highlight", want_hl, hl.is_some()),
                ("_winner", want_win, win.is_some()),
            ] {
                if !applies {
                    continue;
                }
                let has = extra.iter().any(|(k, _)| *k == key);
                if want && !has {
                    extra.push((key, "true".into()));
                    changed = true;
                } else if !want && has {
                    extra.retain(|(k, _)| *k != key);
                    changed = true;
                }
            }
            extra.sort_by_key(|(k, _)| RESERVED_ROW_KEYS.iter().position(|r| r == k));
        }
        if changed {
            let json = serialize_list_rows(&def.columns, &rows);
            values.insert(def.key.clone(), json);
        }
    }
}

/// Parses a `list` value against the field's columns: a JSON array (at most `LIST_ROWS_MAX`) of
/// objects whose cells are text (numbers and booleans are taken as text, `null` as `""`). Cells of
/// unknown columns are an error with `strict`, dropped otherwise. `""` is the empty list.
pub fn parse_list(columns: &[SourceColumn], raw: &str, strict: bool) -> Result<ParsedList, String> {
    if raw.trim().is_empty() {
        return Ok(ParsedList {
            json: "[]".into(),
            dropped: false,
        });
    }
    if raw.len() > LIST_VALUE_MAX {
        return Err(format!(
            "the list is too long (at most {} KB)",
            LIST_VALUE_MAX / 1024
        ));
    }
    let v: Value = serde_json::from_str(raw).map_err(|e| format!("not a valid list: {e}"))?;
    let Value::Array(items) = v else {
        return Err("a list must be a JSON array of rows".into());
    };
    if items.len() > LIST_ROWS_MAX {
        return Err(format!(
            "too many rows ({}, at most {LIST_ROWS_MAX})",
            items.len()
        ));
    }
    let mut dropped = false;
    let mut rows = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        let Value::Object(obj) = item else {
            return Err(format!("row {}: not a valid row", i + 1));
        };
        let mut row = vec![String::new(); columns.len()];
        let mut extra: Vec<(&'static str, String)> = Vec::new();
        for (k, v) in obj {
            if let Some(rk) = RESERVED_ROW_KEYS.iter().find(|r| **r == k) {
                match v {
                    Value::String(t) if is_row_token(&t) => extra.push((rk, t)),
                    _ if strict => {
                        return Err(format!(
                            "row {}: \"{k}\" must be a short token (a-z, 0-9, -)",
                            i + 1
                        ))
                    }
                    _ => dropped = true,
                }
                continue;
            }
            let Some(pos) = columns.iter().position(|c| c.key == k) else {
                if strict {
                    return Err(format!("row {}: unknown column \"{k}\"", i + 1));
                }
                dropped = true;
                continue;
            };
            let cell = match v {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Null => String::new(),
                _ => return Err(format!("row {}, column \"{k}\": must be text", i + 1)),
            };
            if cell.len() > SOURCE_CELL_MAX {
                return Err(format!(
                    "row {}, column \"{k}\" is too long (at most {} KB)",
                    i + 1,
                    SOURCE_CELL_MAX / 1024
                ));
            }
            row[pos] = cell;
        }
        extra.sort_by_key(|(k, _)| RESERVED_ROW_KEYS.iter().position(|r| r == k));
        rows.push((row, extra));
    }
    let json = serialize_list_rows(columns, &rows);
    if json.len() > LIST_VALUE_MAX {
        return Err(format!(
            "the list is too long (at most {} KB)",
            LIST_VALUE_MAX / 1024
        ));
    }
    Ok(ParsedList { json, dropped })
}

/// A whole source as a `list` value: every row (cells matched by column key; columns the source
/// lacks are `""`), ordered by `sort` (numeric: numbers first, non-numbers last; text:
/// case-insensitive; stable), then at most `limit` rows. Rows beyond `LIST_ROWS_MAX` or the byte
/// limit are cut, with a note in the second value. A sort column the source lacks is ignored
/// (also noted).
pub fn list_from_rows(
    columns: &[SourceColumn],
    source_columns: &[String],
    rows: &[Fields],
    sort: Option<&ListSort>,
    limit: Option<u32>,
) -> (String, Vec<String>) {
    let mut notes = Vec::new();
    let mut order: Vec<&Fields> = rows.iter().collect();
    if let Some(s) = sort {
        if source_columns.contains(&s.column) {
            sort_rows(&mut order, s);
        } else {
            notes.push(format!(
                "the source has no column \"{}\" to sort by",
                s.column
            ));
        }
    }
    let cap = limit
        .map(|l| (l as usize).min(LIST_ROWS_MAX))
        .unwrap_or(LIST_ROWS_MAX);
    if order.len() > cap {
        if limit.is_none_or(|l| l as usize > LIST_ROWS_MAX) {
            notes.push(format!("only the first {LIST_ROWS_MAX} rows are shown"));
        }
        order.truncate(cap);
    }
    let mut cells: Vec<ListRow> = order
        .iter()
        .map(|r| {
            let row = columns
                .iter()
                .map(|c| r.get(&c.key).cloned().unwrap_or_default())
                .collect();
            (row, reserved_of(r))
        })
        .collect();
    let mut json = serialize_list_rows(columns, &cells);
    if json.len() > LIST_VALUE_MAX {
        while json.len() > LIST_VALUE_MAX && !cells.is_empty() {
            cells.pop();
            json = serialize_list_rows(columns, &cells);
        }
        notes.push(format!(
            "the list was cut to {} rows (at most {} KB)",
            cells.len(),
            LIST_VALUE_MAX / 1024
        ));
    }
    (json, notes)
}

fn sort_rows(rows: &mut [&Fields], s: &ListSort) {
    let dir = |o: Ordering| if s.desc { o.reverse() } else { o };
    if s.numeric {
        let num = |r: &Fields| {
            r.get(&s.column)
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|n| n.is_finite())
        };
        rows.sort_by(|a, b| match (num(a), num(b)) {
            (Some(x), Some(y)) => dir(x.partial_cmp(&y).unwrap_or(Ordering::Equal)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        });
    } else {
        let text = |r: &Fields| {
            r.get(&s.column)
                .map(|v| v.to_lowercase())
                .unwrap_or_default()
        };
        rows.sort_by(|a, b| dir(text(a).cmp(&text(b))));
    }
}

fn validate_columns(columns: &[SourceColumn]) -> Result<(), String> {
    if columns.is_empty() || columns.len() > LIST_COLUMNS_MAX {
        return Err(format!("a list needs 1 to {LIST_COLUMNS_MAX} columns"));
    }
    let mut seen = HashSet::new();
    for c in columns {
        if !is_key(&c.key) {
            return Err(format!(
                "invalid column key \"{}\": start with a lowercase letter, then up to 31 lowercase letters, digits, - or _",
                c.key
            ));
        }
        if !seen.insert(c.key.as_str()) {
            return Err(format!("duplicate column key \"{}\"", c.key));
        }
        let n = c.label.trim().chars().count();
        if n == 0 || n > LABEL_MAX {
            return Err(format!(
                "column \"{}\": label must be 1 to {LABEL_MAX} characters",
                c.key
            ));
        }
    }
    Ok(())
}

// ---- timer ---------------------------------------------------------------------------------

/// A stopped count-up at zero.
pub fn zero_timer() -> TimerState {
    TimerState {
        run: false,
        at: 0,
        base: 0,
        dir: TimerDir::Up,
        end: TimerEnd::Stop,
    }
}

/// Parses a `timer` value (`base` at most `TIMER_MS_MAX`).
pub fn parse_timer(raw: &str) -> Result<TimerState, String> {
    let s: TimerState =
        serde_json::from_str(raw).map_err(|e| format!("the timer value is not valid ({e})"))?;
    if s.base > TIMER_MS_MAX {
        return Err("the timer value is too large".into());
    }
    Ok(s)
}

/// The canonical JSON of a timer state.
pub fn timer_json(s: &TimerState) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// The field's default state (from its `default`; never running).
pub fn default_timer(def: &FieldDef) -> TimerState {
    let mut s = match def.default.trim() {
        "" => zero_timer(),
        d => parse_timer(d).unwrap_or_else(|_| zero_timer()),
    };
    s.run = false;
    s
}

/// A stored timer value, `""` meaning the default state.
pub fn timer_state(def: &FieldDef, raw: &str) -> Result<TimerState, String> {
    match raw.trim() {
        "" => Ok(default_timer(def)),
        v => parse_timer(v),
    }
}

/// The time shown at `now`: elapsed (up) or remaining (down, never below zero) ms.
pub fn displayed_ms(s: &TimerState, now: u64) -> u64 {
    if !s.run {
        return s.base;
    }
    let elapsed = now.saturating_sub(s.at);
    match s.dir {
        TimerDir::Up => s.base.saturating_add(elapsed),
        TimerDir::Down => s.base.saturating_sub(elapsed),
    }
}

/// The start time for a timer (re)started at `now` showing `base`, moved by less than 500 ms (at
/// most 500) so the shown value changes exactly on whole seconds of the server clock: renderers
/// tick once per second aligned to those, and a countdown reaches zero (`at + base`) on a whole
/// second, exactly when renderers show 0:00. Down: `(at + base) % 1000 == 0`; up:
/// `(at - base) % 1000 == 0`.
pub fn snap_at(now: u64, base: u64, dir: TimerDir) -> u64 {
    let r = match dir {
        TimerDir::Down => (now % 1000 + base % 1000) % 1000,
        TimerDir::Up => (now % 1000 + 1000 - base % 1000) % 1000,
    };
    if r < 500 {
        now.saturating_sub(r)
    } else {
        now + (1000 - r)
    }
}

/// The state after `action` at server time `now`. `default` is what Reset returns to. A timer
/// that starts running (Start, or Configure while running) gets a snapped `at` ([`snap_at`]).
pub fn timer_next(
    cur: TimerState,
    action: TimerAction,
    now: u64,
    default: TimerState,
) -> TimerState {
    match action {
        TimerAction::Start if cur.run => cur,
        TimerAction::Start => TimerState {
            run: true,
            at: snap_at(now, cur.base, cur.dir),
            ..cur
        },
        TimerAction::Pause if !cur.run => cur,
        TimerAction::Pause => TimerState {
            run: false,
            at: now,
            base: displayed_ms(&cur, now),
            ..cur
        },
        TimerAction::Reset => TimerState {
            run: false,
            ..default
        },
        TimerAction::Set { ms } => TimerState {
            run: false,
            at: now,
            base: ms.min(TIMER_MS_MAX),
            ..cur
        },
        TimerAction::Configure { dir, end } => {
            // A running timer continues from what it shows now, in the new direction.
            let (at, base) = if cur.run {
                let base = displayed_ms(&cur, now);
                (snap_at(now, base, dir), base)
            } else {
                (cur.at, cur.base)
            };
            TimerState {
                at,
                base,
                dir,
                end,
                ..cur
            }
        }
    }
}

/// Server epoch ms at which a running countdown with `end: hide` reaches zero.
pub fn countdown_hide_at(s: &TimerState) -> Option<u64> {
    (s.run && s.dir == TimerDir::Down && s.end == TimerEnd::Hide)
        .then(|| s.at.saturating_add(s.base))
}

// ---- per field type --------------------------------------------------------------------------

/// Checks the type-specific parts of a field definition: a list's columns and default, a clock's
/// format, a timer's default. Other types have none.
pub fn validate_def(def: &FieldDef) -> Result<(), String> {
    match def.kind {
        FieldType::List => {
            validate_columns(&def.columns)?;
            parse_list(&def.columns, &def.default, true)
                .map(drop)
                .map_err(|e| format!("default: {e}"))
        }
        FieldType::Clock => validate_clock_format(def.format.as_deref().unwrap_or("")),
        FieldType::Timer => match def.default.trim() {
            "" => Ok(()),
            d => parse_timer(d)
                .map(drop)
                .map_err(|e| format!("default: {e}")),
        },
        _ => Ok(()),
    }
}

/// The default value renderers get (canonical for lists and timers, the format for clocks).
pub fn wire_default(def: &FieldDef) -> String {
    match def.kind {
        FieldType::List => parse_list(&def.columns, &def.default, false)
            .map(|p| p.json)
            .unwrap_or_else(|_| "[]".into()),
        FieldType::Timer => timer_json(&default_timer(def)),
        FieldType::Clock => clock_format(def),
        _ => def.default.clone(),
    }
}

/// Words that make a text field number-like: scores, counts, percentages, places. Matched as
/// whole words of the key or the label (`blue-score`, "Player 1 boost"). Mirrored in the
/// editor (`templates/editor/fieldTypes.ts` `NUMBER_WORDS`).
pub const NUMBER_WORDS: &[&str] = &[
    "score", "scores", "points", "pts", "goals", "goal", "boost", "count", "total", "round",
    "rounds", "set", "sets", "game", "games", "lap", "laps", "rank", "position", "pos", "place",
    "number", "num", "kills", "deaths", "assists", "wins", "losses", "saves", "shots", "fouls",
    "percent", "pct", "%", "speed", "age", "year", "series", "quarter", "period", "inning",
    "frame",
];

fn words<'a>(key: &'a str, label: &'a str) -> impl Iterator<Item = String> + 'a {
    let split = |s: &'a str| s.split(|c: char| !c.is_alphanumeric() && c != '%');
    split(key)
        .chain(split(label))
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

/// Whether a key or label names a number (see [`NUMBER_WORDS`]).
pub fn number_words(key: &str, label: &str) -> bool {
    words(key, label).any(|w| NUMBER_WORDS.contains(&w.as_str()))
}

/// A realistic preview number for a number-like field: a place 1, a percentage 45, a round 3, a
/// year 2026, an age 28, a speed 88, else (scores, counts) 2. Never the label: a long label in a
/// number's slot overflowed the graphic.
pub fn sample_number(key: &str, label: &str) -> &'static str {
    for w in words(key, label) {
        match w.as_str() {
            "rank" | "position" | "pos" | "place" => return "1",
            "boost" | "percent" | "pct" | "%" => return "45",
            "round" | "rounds" | "set" | "sets" | "game" | "games" | "lap" | "laps" | "quarter"
            | "period" | "inning" | "frame" | "series" => return "3",
            "year" => return "2026",
            "age" => return "28",
            "speed" => return "88",
            _ => {}
        }
    }
    "2"
}

/// Preview value (never what goes on air; see `preview::label_values`): the field's own default
/// when it has one; a number-like field (type `number`, or a text field named like a number)
/// without a numeric default a realistic number ([`sample_number`]); other text-like fields the
/// label; three sample rows of column labels for a list; the default state for a timer; the
/// format for a clock.
pub fn sample_value(def: &FieldDef) -> String {
    let default = def.default.trim();
    let numeric = !default.is_empty() && default.parse::<f64>().is_ok();
    match def.kind {
        FieldType::Number if numeric => return default.to_string(),
        FieldType::Number => return sample_number(&def.key, &def.label).to_string(),
        FieldType::Text | FieldType::Multiline if number_words(&def.key, &def.label) => {
            return if numeric {
                default.to_string()
            } else {
                sample_number(&def.key, &def.label).to_string()
            };
        }
        FieldType::Text | FieldType::Multiline if !default.is_empty() => {
            return def.default.clone();
        }
        _ => {}
    }
    match def.kind {
        FieldType::List => {
            let rows: Vec<Vec<String>> = (1..=SAMPLE_ROWS)
                .map(|i| {
                    def.columns
                        .iter()
                        .map(|c| {
                            // An image column gets no sample (a label is not a URL).
                            if c.kind == ColumnKind::Image {
                                String::new()
                            } else {
                                format!("{} {i}", c.label.trim())
                            }
                        })
                        .collect()
                })
                .collect();
            serialize_rows(&def.columns, &rows)
        }
        FieldType::Timer => timer_json(&default_timer(def)),
        FieldType::Clock => clock_format(def),
        _ => def.label.clone(),
    }
}

/// A stored value as renderers get it (lenient: a list drops unknown columns, reported with
/// `true`). `Err` = invalid: the caller warns and uses [`wire_default`].
pub fn own_value(def: &FieldDef, raw: &str) -> Result<(String, bool), String> {
    match def.kind {
        FieldType::List => parse_list(&def.columns, raw, false).map(|p| (p.json, p.dropped)),
        FieldType::Timer => timer_state(def, raw).map(|s| (timer_json(&s), false)),
        FieldType::Clock => Ok((clock_format(def), false)),
        _ => Ok((raw.to_string(), false)),
    }
}

/// A value from a Control edit (strict): returns what to store. Lists are canonicalized (unknown
/// columns are an error), timers must parse (`""` = default state), clocks take no value.
pub fn edit_value(def: &FieldDef, raw: &str) -> Result<String, String> {
    match def.kind {
        FieldType::List => parse_list(&def.columns, raw, true).map(|p| p.json),
        FieldType::Timer => match raw.trim() {
            "" => Ok(String::new()),
            v => parse_timer(v).map(|s| timer_json(&s)),
        },
        FieldType::Clock => Err("a clock shows the time of day and has no value to set".into()),
        _ => Ok(raw.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(key: &str, label: &str, kind: FieldType, default: &str) -> FieldDef {
        FieldDef {
            key: key.into(),
            label: label.into(),
            kind,
            default: default.into(),
            columns: Vec::new(),
            format: None,
        }
    }

    /// Previews never put a long label in a number's slot (it overflowed the graphic).
    #[test]
    fn number_like_fields_preview_a_realistic_number() {
        let v = |k, l, t, d| sample_value(&plain(k, l, t, d));
        // Number fields: their default if numeric, else a number for their kind.
        assert_eq!(v("boost", "Player 1 boost", FieldType::Number, ""), "45");
        assert_eq!(
            v(
                "boost",
                "Player 1 boost",
                FieldType::Number,
                "Player 1 boost"
            ),
            "45"
        );
        assert_eq!(v("n", "Anything", FieldType::Number, "7"), "7");
        // Text fields named like numbers.
        assert_eq!(
            v("blue-score", "Blue series score", FieldType::Text, ""),
            "3"
        );
        assert_eq!(v("home-goals", "Home", FieldType::Text, ""), "2");
        assert_eq!(v("pos", "Position", FieldType::Text, ""), "1");
        assert_eq!(v("score", "Score", FieldType::Text, "0"), "0");
        // Other text: its default, else the label (as before).
        assert_eq!(v("team", "Blue team", FieldType::Text, ""), "Blue team");
        assert_eq!(
            v("team", "Blue team", FieldType::Text, "Rangers"),
            "Rangers"
        );
        assert_eq!(v("logo", "Score logo", FieldType::Image, ""), "Score logo");
    }

    fn cols(keys: &[&str]) -> Vec<SourceColumn> {
        keys.iter()
            .map(|k| SourceColumn {
                key: k.to_string(),
                label: k.to_uppercase(),
                kind: Default::default(),
            })
            .collect()
    }

    fn list_def(keys: &[&str], default: &str) -> FieldDef {
        FieldDef {
            key: "rows".into(),
            label: "Rows".into(),
            kind: FieldType::List,
            default: default.into(),
            columns: cols(keys),
            format: None,
        }
    }

    fn row(pairs: &[(&str, &str)]) -> Fields {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn status_def() -> FieldDef {
        FieldDef {
            key: STATUS_FIELD.into(),
            label: "Status".into(),
            kind: FieldType::Text,
            default: String::new(),
            columns: Vec::new(),
            format: None,
        }
    }

    #[test]
    fn poll_winner_marks_follow_the_status() {
        let defs = vec![status_def(), list_def(&["label", "pct", "votes"], "")];
        let rows = r#"[{"label":"A","pct":"40","votes":"4"},{"label":"B","pct":"60%","votes":"6"},{"label":"C","pct":"x","votes":"0"}]"#;
        let mut v: Fields = [
            ("status".to_string(), "open".to_string()),
            ("rows".to_string(), rows.to_string()),
        ]
        .into();
        apply_row_marks(&defs, &mut v);
        assert_eq!(v["rows"], rows, "open: no winner, untouched");
        v.insert("status".into(), " Closed ".into());
        apply_row_marks(&defs, &mut v);
        assert_eq!(
            v["rows"],
            r#"[{"label":"A","pct":"40","votes":"4"},{"label":"B","pct":"60%","votes":"6","_winner":"true"},{"label":"C","pct":"x","votes":"0"}]"#
        );
        // Every mark passes the renderer's token rule, and the value stays canonical.
        let parsed = parse_list(&cols(&["label", "pct", "votes"]), &v["rows"], true).unwrap();
        assert_eq!(parsed.json, v["rows"]);
        // Ties all win; reopening removes the mark.
        v.insert(
            "rows".into(),
            r#"[{"label":"A","pct":"50"},{"label":"B","pct":"50.0"}]"#.into(),
        );
        apply_row_marks(&defs, &mut v);
        assert_eq!(v["rows"].matches("_winner").count(), 2);
        v.insert("status".into(), "open".into());
        apply_row_marks(&defs, &mut v);
        assert!(!v["rows"].contains("_winner"));
        // No status field in the template: never a winner.
        let mut w: Fields = [
            ("status".to_string(), "closed".to_string()),
            ("rows".to_string(), rows.to_string()),
        ]
        .into();
        apply_row_marks(&defs[1..], &mut w);
        assert_eq!(w["rows"], rows);
    }

    #[test]
    fn chat_highlight_marks() {
        let defs = vec![list_def(&["user", "text", "highlighted"], "")];
        let mut v: Fields = [(
            "rows".to_string(),
            r#"[{"user":"a","text":"hi","highlighted":"YES"},{"user":"b","text":"yo","highlighted":"","_highlight":"true"},{"user":"c","text":"1","highlighted":"1","_current":"true"}]"#
                .to_string(),
        )]
        .into();
        apply_row_marks(&defs, &mut v);
        assert_eq!(
            v["rows"],
            r#"[{"user":"a","text":"hi","highlighted":"YES","_highlight":"true"},{"user":"b","text":"yo","highlighted":""},{"user":"c","text":"1","highlighted":"1","_current":"true","_highlight":"true"}]"#
        );
        // A list without the convention's columns is untouched.
        let other = vec![list_def(&["name"], "")];
        let mut v: Fields = [("rows".to_string(), r#"[{"name":"x"}]"#.to_string())].into();
        apply_row_marks(&other, &mut v);
        assert_eq!(v["rows"], r#"[{"name":"x"}]"#);
    }

    #[test]
    fn list_values_are_canonical() {
        let c = cols(&["name", "score"]);
        let p = parse_list(
            &c,
            r#"[ {"score": 3, "name":"Ada"}, {"name": null}, {} ]"#,
            true,
        )
        .unwrap();
        assert_eq!(
            p.json,
            r#"[{"name":"Ada","score":"3"},{"name":"","score":""},{"name":"","score":""}]"#
        );
        assert!(!p.dropped);
        assert_eq!(parse_list(&c, "", true).unwrap().json, "[]");
        assert_eq!(parse_list(&c, " [ ] ", true).unwrap().json, "[]");
        // Escaping survives the round trip.
        let q = parse_list(&c, r#"[{"name":"a \"q\" <b>\n"}]"#, true).unwrap();
        let back: Value = serde_json::from_str(&q.json).unwrap();
        assert_eq!(back[0]["name"], "a \"q\" <b>\n");

        // Unknown columns: strict rejects, lenient drops.
        assert!(parse_list(&c, r#"[{"x":"1"}]"#, true).is_err());
        let l = parse_list(&c, r#"[{"x":"1","name":"B"}]"#, false).unwrap();
        assert!(l.dropped);
        assert_eq!(l.json, r#"[{"name":"B","score":""}]"#);

        for bad in [
            "{}",
            "[1]",
            r#"[{"name":["x"]}]"#,
            r#"[{"name":{"a":1}}]"#,
            "not json",
        ] {
            assert!(parse_list(&c, bad, false).is_err(), "{bad}");
        }
        let many = format!("[{}]", vec!["{}"; LIST_ROWS_MAX + 1].join(","));
        assert!(parse_list(&c, &many, false).is_err());
        let ok = format!("[{}]", vec!["{}"; LIST_ROWS_MAX].join(","));
        assert!(parse_list(&c, &ok, false).is_ok());
        let big_cell = format!(r#"[{{"name":"{}"}}]"#, "x".repeat(SOURCE_CELL_MAX + 1));
        assert!(parse_list(&c, &big_cell, false).is_err());
        let huge = format!(
            "[{}]",
            vec![format!(r#"{{"name":"{}"}}"#, "x".repeat(1000)); 70].join(",")
        );
        assert!(parse_list(&c, &huge, false).is_err(), "over 64 KiB");
    }

    #[test]
    fn reserved_row_keys_survive_canonicalization() {
        let c = cols(&["name"]);
        let p = parse_list(
            &c,
            r#"[{"_status":"now","name":"A","_current":"true","_duration":"6"}]"#,
            true,
        )
        .unwrap();
        assert_eq!(
            p.json, r#"[{"name":"A","_current":"true","_duration":"6","_status":"now"}]"#,
            "after the columns, in canonical order"
        );
        // Not a safe token: strict rejects, lenient drops.
        for bad in [r#""Now""#, r#""a b""#, "5", r#""""#, r#""x\"><script>""#] {
            let raw = format!(r#"[{{"name":"A","_status":{bad}}}]"#);
            assert!(parse_list(&c, &raw, true).is_err(), "{bad}");
            let l = parse_list(&c, &raw, false).unwrap();
            assert!(l.dropped);
            assert_eq!(l.json, r#"[{"name":"A"}]"#);
        }
        // Other `_` keys are unknown columns.
        assert!(parse_list(&c, r#"[{"_other":"x"}]"#, true).is_err());
        // From source rows: reserved keys kept (valid ones only), other extra keys dropped.
        let rows = vec![
            row(&[
                ("name", "A"),
                ("_current", "true"),
                ("_duration", "6"),
                ("x", "1"),
            ]),
            row(&[("name", "B"), ("_status", "Bad Token")]),
        ];
        let (json, _) = list_from_rows(&c, &["name".into()], &rows, None, None);
        assert_eq!(
            json,
            r#"[{"name":"A","_current":"true","_duration":"6"},{"name":"B"}]"#
        );
        assert!(is_row_token("upcoming"));
        assert!(!is_row_token(&"a".repeat(33)));
        // Image columns get no sample text.
        let mut def = list_def(&["logo", "name"], "");
        def.columns[0].kind = ColumnKind::Image;
        assert_eq!(
            sample_value(&def),
            r#"[{"logo":"","name":"NAME 1"},{"logo":"","name":"NAME 2"},{"logo":"","name":"NAME 3"}]"#
        );
    }

    #[test]
    fn list_definitions_are_validated() {
        assert!(validate_def(&list_def(&["name"], "")).is_ok());
        assert!(validate_def(&list_def(&["name"], r#"[{"name":"A"}]"#)).is_ok());
        assert!(validate_def(&list_def(&["name"], r#"[{"nope":"A"}]"#)).is_err());
        assert!(validate_def(&list_def(&["name"], "x")).is_err());
        assert!(validate_def(&list_def(&[], "")).is_err());
        assert!(validate_def(&list_def(&["a", "a"], "")).is_err());
        assert!(validate_def(&list_def(&["Bad"], "")).is_err());
        let keys: Vec<String> = (0..17).map(|i| format!("c{i}")).collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        assert!(validate_def(&list_def(&refs, "")).is_err());
        assert!(validate_def(&list_def(&refs[..16], "")).is_ok());
        let mut blank_label = list_def(&["a"], "");
        blank_label.columns[0].label = "  ".into();
        assert!(validate_def(&blank_label).is_err());
        assert_eq!(wire_default(&list_def(&["a"], "")), "[]");
        assert_eq!(
            sample_value(&list_def(&["name", "score"], "")),
            r#"[{"name":"NAME 1","score":"SCORE 1"},{"name":"NAME 2","score":"SCORE 2"},{"name":"NAME 3","score":"SCORE 3"}]"#
        );
    }

    #[test]
    fn clock_formats() {
        for ok in ["", "HH:mm:ss", "HH:mm", "HH.mm", "mm ss", "HH"] {
            assert!(validate_clock_format(ok).is_ok(), "{ok}");
        }
        for bad in [
            "hh:mm",
            "H:m",
            "HH:MM",
            "yyyy",
            "HH-mm",
            "HH:mm:ss:HH:mm:ss:HH:mm:ss:HH:mm:ss",
        ] {
            assert!(validate_clock_format(bad).is_err(), "{bad}");
        }
        let mut def = FieldDef {
            key: "clock".into(),
            label: "Clock".into(),
            kind: FieldType::Clock,
            ..Default::default()
        };
        assert_eq!(wire_default(&def), "HH:mm:ss");
        def.format = Some("HH:mm".into());
        assert_eq!(wire_default(&def), "HH:mm");
        assert_eq!(own_value(&def, "whatever").unwrap().0, "HH:mm");
        assert!(edit_value(&def, "x").is_err());
    }

    fn timer(run: bool, at: u64, base: u64, dir: TimerDir) -> TimerState {
        TimerState {
            run,
            at,
            base,
            dir,
            end: TimerEnd::Stop,
        }
    }

    #[test]
    fn timer_actions() {
        let zero = zero_timer();
        // Start, then pause after 1.5 s: shows 1.5 s.
        let s = timer_next(zero, TimerAction::Start, 10_000, zero);
        assert_eq!(s, timer(true, 10_000, 0, TimerDir::Up));
        assert_eq!(
            timer_next(s, TimerAction::Start, 99_000, zero),
            s,
            "start while running"
        );
        assert_eq!(displayed_ms(&s, 11_500), 1_500);
        let p = timer_next(s, TimerAction::Pause, 11_500, zero);
        assert_eq!(p, timer(false, 11_500, 1_500, TimerDir::Up));
        assert_eq!(
            timer_next(p, TimerAction::Pause, 20_000, zero),
            p,
            "pause while paused"
        );
        assert_eq!(displayed_ms(&p, 50_000), 1_500, "paused holds");
        // Resume continues from 1.5 s, with `at` snapped so whole seconds tick on second
        // boundaries (1.5 s shown → the next change is half a second away).
        let r = timer_next(p, TimerAction::Start, 30_000, zero);
        assert_eq!(r.at, 30_500);
        assert_eq!(displayed_ms(&r, 31_500), 2_500);
        // Set pauses at the value.
        let set = timer_next(r, TimerAction::Set { ms: 60_000 }, 31_000, zero);
        assert_eq!(set, timer(false, 31_000, 60_000, TimerDir::Up));
        assert_eq!(
            timer_next(set, TimerAction::Set { ms: u64::MAX }, 0, zero).base,
            TIMER_MS_MAX
        );
        // Countdown: never below zero.
        let down = timer(true, 1_000, 5_000, TimerDir::Down);
        assert_eq!(displayed_ms(&down, 3_000), 3_000);
        assert_eq!(displayed_ms(&down, 9_000), 0);
        let paused = timer_next(down, TimerAction::Pause, 9_000, zero);
        assert_eq!(paused.base, 0);
        // Configure while running keeps the shown time.
        let up = timer(true, 0, 0, TimerDir::Up);
        let c = timer_next(
            up,
            TimerAction::Configure {
                dir: TimerDir::Down,
                end: TimerEnd::Hide,
            },
            4_000,
            zero,
        );
        assert_eq!(
            (c.run, c.at, c.base, c.dir, c.end),
            (true, 4_000, 4_000, TimerDir::Down, TimerEnd::Hide)
        );
        assert_eq!(countdown_hide_at(&c), Some(8_000));
        assert_eq!(
            countdown_hide_at(&timer(true, 0, 5, TimerDir::Down)),
            None,
            "end stop"
        );
        // Configure while paused keeps base.
        let cp = timer_next(
            set,
            TimerAction::Configure {
                dir: TimerDir::Down,
                end: TimerEnd::Stop,
            },
            99,
            zero,
        );
        assert_eq!((cp.run, cp.base, cp.dir), (false, 60_000, TimerDir::Down));
        // Reset goes back to the default (paused).
        let def = timer(true, 5, 90_000, TimerDir::Down);
        assert_eq!(
            timer_next(r, TimerAction::Reset, 1, def),
            timer(false, 5, 90_000, TimerDir::Down)
        );
    }

    #[test]
    fn start_snaps_to_whole_seconds() {
        // Down: the zero time `at + base` is a whole second; the shift is under half a second.
        for now in [1_000_000u64, 1_000_123, 1_000_499, 1_000_501, 1_000_999] {
            for base in [0u64, 60_000, 90_250, 5_999] {
                let at = snap_at(now, base, TimerDir::Down);
                assert_eq!((at + base) % 1000, 0, "down now={now} base={base}");
                assert!(at.abs_diff(now) <= 500, "down shift now={now} base={base}");
                let up = snap_at(now, base, TimerDir::Up);
                assert_eq!((up - base) % 1000, 0, "up now={now} base={base}");
                assert!(up.abs_diff(now) <= 500, "up shift now={now} base={base}");
            }
        }
        // Nearest: 1 000 123 → 1 000 000; 1 000 501 → 1 001 000.
        assert_eq!(snap_at(1_000_123, 0, TimerDir::Down), 1_000_000);
        assert_eq!(snap_at(1_000_501, 0, TimerDir::Up), 1_001_000);
        // A started countdown with end: hide reaches zero on a whole second.
        let cd = TimerState {
            run: false,
            at: 0,
            base: 10_250,
            dir: TimerDir::Down,
            end: TimerEnd::Hide,
        };
        let s = timer_next(cd, TimerAction::Start, 5_000_321, zero_timer());
        assert_eq!(countdown_hide_at(&s).unwrap() % 1000, 0);
        assert!(s.at.abs_diff(5_000_321) <= 500);
    }

    #[test]
    fn timer_values() {
        let mut def = FieldDef {
            key: "t".into(),
            label: "T".into(),
            kind: FieldType::Timer,
            ..Default::default()
        };
        assert!(validate_def(&def).is_ok());
        assert_eq!(
            wire_default(&def),
            r#"{"run":false,"at":0,"base":0,"dir":"up","end":"stop"}"#
        );
        def.default = r#"{"run":true,"base":60000,"dir":"down"}"#.into();
        assert!(validate_def(&def).is_ok());
        assert_eq!(
            wire_default(&def),
            r#"{"run":false,"at":0,"base":60000,"dir":"down","end":"stop"}"#,
            "a default never runs"
        );
        def.default = "nope".into();
        assert!(validate_def(&def).is_err());
        def.default.clear();
        assert_eq!(own_value(&def, "").unwrap().0, wire_default(&def));
        assert!(own_value(&def, "{").is_err());
        assert_eq!(
            edit_value(&def, r#"{"base":5,"run":false}"#).unwrap(),
            r#"{"run":false,"at":0,"base":5,"dir":"up","end":"stop"}"#
        );
        assert_eq!(edit_value(&def, "").unwrap(), "");
        assert!(edit_value(&def, "[]").is_err());
    }

    #[test]
    fn list_from_source_rows_sorts_and_limits() {
        let c = cols(&["name", "score"]);
        let src_cols = vec!["name".to_string(), "score".to_string(), "team".to_string()];
        let rows = vec![
            row(&[("name", "ada"), ("score", "10"), ("team", "x")]),
            row(&[("name", "Bob"), ("score", "n/a")]),
            row(&[("name", "cy"), ("score", "30")]),
            row(&[("name", "Dee"), ("score", "10")]),
            row(&[("name", "eve"), ("score", " 2.5 ")]),
        ];
        let names = |json: &str| -> Vec<String> {
            let v: Value = serde_json::from_str(json).unwrap();
            v.as_array()
                .unwrap()
                .iter()
                .map(|r| r["name"].as_str().unwrap().to_string())
                .collect()
        };
        let (all, notes) = list_from_rows(&c, &src_cols, &rows, None, None);
        assert!(notes.is_empty());
        assert_eq!(names(&all), ["ada", "Bob", "cy", "Dee", "eve"]);
        assert!(
            all.starts_with(r#"[{"name":"ada","score":"10"}"#),
            "only the list's columns"
        );

        let desc = ListSort {
            column: "score".into(),
            numeric: true,
            desc: true,
        };
        let (top, _) = list_from_rows(&c, &src_cols, &rows, Some(&desc), Some(3));
        assert_eq!(
            names(&top),
            ["cy", "ada", "Dee"],
            "numeric desc, stable ties"
        );
        let asc = ListSort {
            desc: false,
            ..desc.clone()
        };
        let (all_asc, _) = list_from_rows(&c, &src_cols, &rows, Some(&asc), None);
        assert_eq!(
            names(&all_asc),
            ["eve", "ada", "Dee", "cy", "Bob"],
            "non-numbers last"
        );
        let (all_desc, _) = list_from_rows(&c, &src_cols, &rows, Some(&desc), None);
        assert_eq!(
            names(&all_desc).last().unwrap(),
            "Bob",
            "non-numbers last in desc too"
        );

        let text = ListSort {
            column: "name".into(),
            numeric: false,
            desc: false,
        };
        let (t, _) = list_from_rows(&c, &src_cols, &rows, Some(&text), None);
        assert_eq!(
            names(&t),
            ["ada", "Bob", "cy", "Dee", "eve"],
            "case-insensitive"
        );
        let (td, _) = list_from_rows(
            &c,
            &src_cols,
            &rows,
            Some(&ListSort { desc: true, ..text }),
            Some(2),
        );
        assert_eq!(names(&td), ["eve", "Dee"]);

        let missing = ListSort {
            column: "nope".into(),
            numeric: false,
            desc: false,
        };
        let (m, notes) = list_from_rows(&c, &src_cols, &rows, Some(&missing), None);
        assert_eq!(names(&m), names(&all));
        assert_eq!(notes.len(), 1);

        let many: Vec<Fields> = (0..150).map(|i| row(&[("name", &i.to_string())])).collect();
        let (cut, notes) = list_from_rows(&c, &src_cols, &many, None, None);
        assert_eq!(names(&cut).len(), LIST_ROWS_MAX);
        assert_eq!(notes.len(), 1);
        let (lim, notes) = list_from_rows(&c, &src_cols, &many, None, Some(10));
        assert_eq!(names(&lim).len(), 10);
        assert!(notes.is_empty());
        let (empty, _) = list_from_rows(&c, &src_cols, &[], None, None);
        assert_eq!(empty, "[]");
    }
}

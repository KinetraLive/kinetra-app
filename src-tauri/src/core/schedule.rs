//! The schedule engine, pure part: statuses, effective times, binding rows, the next boundary
//! and operator actions. No I/O, no clock (callers pass [`Now`]). The live part (re-planning at
//! boundaries, applying bound screens, rules) is in `core::rules`.
//!
//! # Effective times
//! - A planned start/end is a local date-time (`YYYY-MM-DDTHH:MM`), converted to epoch ms with
//!   the zone of [`Now`] per item, so each item gets the UTC offset of its own date.
//! - `delayMin` shifts start and end. With `ripple`, an item is also shifted by the delays of
//!   every item planned before it (cumulative; cancelled items' delays don't count).
//! - `startedAt` (Start now) replaces the start; the end then follows the planned duration.
//!   `endedAt` (End, or starting another item) replaces the end.
//!
//! # Status (first match wins)
//! 1. `cancelled` → CANCELLED.
//! 2. `done`, or the effective end has passed → PREVIOUS.
//! 3. `now`, or the effective start has passed (`startedAt` included) → NOW.
//! 4. `delayMin > 0` (not started) → DELAYED.
//! 5. the item `nextOverride` names (if it is still upcoming), else the upcoming item with the
//!    earliest effective start → NEXT. A delayed item picked as next stays DELAYED but is the
//!    view's `nextId`.
//! 6. otherwise UPCOMING.
//!
//! The NOW item of the view (`nowId`, the `now` row of single bindings) is the NOW item that
//! started last.

use std::collections::HashMap;

use time::PrimitiveDateTime;

use crate::core::fieldvals;
use crate::core::localtime::{self, Now};
use crate::core::model::{
    ItemState, ItemStatus, ScheduleAction, ScheduleItem, ScheduleItemView, ScheduleLabels,
    ScheduleView, SCHEDULE_COLUMNS, SCHEDULE_ITEMS_MAX,
};
use crate::core::protocol::{Fields, TimerDir, TimerEnd, TimerState};
use crate::core::templates::is_id;

/// Longest item title, subtitle and location, in characters.
pub const ITEM_TEXT_MAX: usize = 200;
/// Longest status label, in characters.
pub const LABEL_MAX: usize = 32;
/// Largest delay of one item, in minutes (a week).
pub const DELAY_MAX_MIN: u32 = 7 * 24 * 60;

const MIN_MS: i64 = 60_000;

/// One item with its computed status and effective times.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eff {
    /// Index into the schedule's items.
    pub index: usize,
    pub status: ItemStatus,
    pub start_ms: i64,
    pub end_ms: i64,
    pub planned_start_ms: i64,
    pub moved: bool,
}

/// A computed schedule: items ordered by effective start (then by their order in the file).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Computed {
    pub items: Vec<Eff>,
    pub now_id: Option<String>,
    pub next_id: Option<String>,
}

pub fn status_name(s: ItemStatus) -> &'static str {
    match s {
        ItemStatus::Previous => "previous",
        ItemStatus::Now => "now",
        ItemStatus::Next => "next",
        ItemStatus::Delayed => "delayed",
        ItemStatus::Upcoming => "upcoming",
        ItemStatus::Cancelled => "cancelled",
    }
}

fn label(labels: &ScheduleLabels, s: ItemStatus) -> &str {
    match s {
        ItemStatus::Previous => &labels.previous,
        ItemStatus::Now => &labels.now,
        ItemStatus::Next => &labels.next,
        ItemStatus::Delayed => &labels.delayed,
        ItemStatus::Upcoming => &labels.upcoming,
        ItemStatus::Cancelled => &labels.cancelled,
    }
}

fn epoch(now: &Now, s: &str) -> Option<i64> {
    localtime::parse_local(s).map(|dt| now.zone.to_epoch(dt))
}

/// Computes every item's status and effective times at `now` (see the module docs). Items whose
/// times don't parse (never the case after validation) are left out.
pub fn compute(
    items: &[ScheduleItem],
    ripple: bool,
    next_override: Option<&str>,
    now: &Now,
) -> Computed {
    struct Planned {
        index: usize,
        start: i64,
        end: i64,
    }
    let mut planned: Vec<Planned> = items
        .iter()
        .enumerate()
        .filter_map(|(index, it)| {
            Some(Planned {
                index,
                start: epoch(now, &it.start)?,
                end: epoch(now, &it.end)?,
            })
        })
        .collect();
    planned.sort_by_key(|p| (p.start, p.index));

    let mut cum: i64 = 0;
    let mut effs = Vec::with_capacity(planned.len());
    for p in &planned {
        let it = &items[p.index];
        let own = if it.state == ItemState::Cancelled {
            0
        } else {
            i64::from(it.delay_min)
        };
        cum += own;
        let shift = if ripple { cum } else { own } * MIN_MS;
        let duration = (p.end - p.start).max(0);
        let started = it.started_at.as_deref().and_then(|s| epoch(now, s));
        let start = started.unwrap_or(p.start + shift);
        let end = it
            .ended_at
            .as_deref()
            .and_then(|s| epoch(now, s))
            .unwrap_or(match started {
                Some(s) => s + duration,
                None => p.end + shift,
            });
        let status = if it.state == ItemState::Cancelled {
            ItemStatus::Cancelled
        } else if it.state == ItemState::Done || end <= now.ms {
            ItemStatus::Previous
        } else if it.state == ItemState::Now || start <= now.ms {
            ItemStatus::Now
        } else if it.delay_min > 0 {
            ItemStatus::Delayed
        } else {
            ItemStatus::Upcoming
        };
        effs.push(Eff {
            index: p.index,
            status,
            start_ms: start,
            end_ms: end,
            planned_start_ms: p.start,
            moved: start != p.start,
        });
    }
    // Stable: equal starts keep their planned order.
    effs.sort_by_key(|e| e.start_ms);

    let upcoming = |e: &Eff| matches!(e.status, ItemStatus::Upcoming | ItemStatus::Delayed);
    let next_pos = next_override
        .and_then(|id| {
            effs.iter()
                .position(|e| items[e.index].id == id && upcoming(e))
        })
        .or_else(|| effs.iter().position(upcoming));
    if let Some(i) = next_pos {
        if effs[i].status == ItemStatus::Upcoming {
            effs[i].status = ItemStatus::Next;
        }
    }
    let now_pos = effs
        .iter()
        .enumerate()
        .filter(|(_, e)| e.status == ItemStatus::Now)
        .max_by_key(|(i, e)| (e.start_ms, *i))
        .map(|(i, _)| i);
    Computed {
        now_id: now_pos.map(|i| items[effs[i].index].id.clone()),
        next_id: next_pos.map(|i| items[effs[i].index].id.clone()),
        items: effs,
    }
}

/// The operator's view of a computed schedule.
pub fn view(
    source_id: &str,
    items: &[ScheduleItem],
    c: &Computed,
    now: &Now,
    can_undo: bool,
) -> ScheduleView {
    let ms = |v: i64| u64::try_from(v).unwrap_or(0);
    ScheduleView {
        source_id: source_id.to_string(),
        items: c
            .items
            .iter()
            .map(|e| ScheduleItemView {
                id: items[e.index].id.clone(),
                title: items[e.index].title.clone(),
                location: items[e.index].location.clone(),
                status: e.status,
                start: localtime::format_local(now.zone.to_local(e.start_ms)),
                end: localtime::format_local(now.zone.to_local(e.end_ms)),
                start_ms: ms(e.start_ms),
                end_ms: ms(e.end_ms),
                moved: e.moved,
            })
            .collect(),
        now_id: c.now_id.clone(),
        next_id: c.next_id.clone(),
        computed_at: ms(now.ms),
        can_undo,
    }
}

/// A stopped countdown at zero (past and NOW items, or nothing to count down to).
pub fn countdown_zero() -> String {
    fieldvals::timer_json(&TimerState {
        run: false,
        at: 0,
        base: 0,
        dir: TimerDir::Down,
        end: TimerEnd::Stop,
    })
}

/// The `countdown` column: a running countdown reaching zero exactly at `start_ms` (schedule
/// frame). Deterministic, so re-planning at another moment yields the same value (no spurious
/// `fields` messages or staged changes): `at` = zero time − `TIMER_MS_MAX`, `base` =
/// `TIMER_MS_MAX`. Zero lands on a whole second because starts are whole minutes. A start more
/// than `TIMER_MS_MAX` (1000 h) away shows the maximum, paused.
pub fn countdown(start_ms: i64, now: &Now) -> String {
    let max = fieldvals::TIMER_MS_MAX;
    if start_ms <= now.ms {
        return countdown_zero();
    }
    let zero_real = now.to_real(start_ms);
    let at = zero_real - i64::try_from(max).unwrap_or(i64::MAX);
    let state = if at >= 0 && at <= now.real_ms {
        TimerState {
            run: true,
            at: u64::try_from(at).unwrap_or(0),
            base: max,
            dir: TimerDir::Down,
            end: TimerEnd::Stop,
        }
    } else {
        TimerState {
            run: false,
            at: 0,
            base: max,
            dir: TimerDir::Down,
            end: TimerEnd::Stop,
        }
    };
    fieldvals::timer_json(&state)
}

/// Binding rows: one per item in view order, every `SCHEDULE_COLUMNS` key plus `_status`.
pub fn rows(
    items: &[ScheduleItem],
    labels: &ScheduleLabels,
    c: &Computed,
    now: &Now,
) -> Vec<Fields> {
    c.items
        .iter()
        .map(|e| row(&items[e.index], labels, e, now))
        .collect()
}

fn row(it: &ScheduleItem, labels: &ScheduleLabels, e: &Eff, now: &Now) -> Fields {
    let start = now.zone.to_local(e.start_ms);
    let end = now.zone.to_local(e.end_ms);
    let status = status_name(e.status);
    let counting = matches!(
        e.status,
        ItemStatus::Next | ItemStatus::Delayed | ItemStatus::Upcoming
    );
    let mut f = Fields::new();
    for (key, _) in SCHEDULE_COLUMNS {
        let v = match *key {
            "title" => it.title.clone(),
            "subtitle" => it.subtitle.clone(),
            "location" => it.location.clone(),
            "start" => localtime::hhmm(start),
            "end" => localtime::hhmm(end),
            "day" => localtime::day_label(start),
            "new_start" => {
                if e.moved {
                    localtime::hhmm(start)
                } else {
                    String::new()
                }
            }
            "label" => label(labels, e.status).to_string(),
            "status" => status.to_string(),
            "countdown" => {
                if counting {
                    countdown(e.start_ms, now)
                } else {
                    countdown_zero()
                }
            }
            _ => String::new(),
        };
        f.insert((*key).to_string(), v);
    }
    f.insert("_status".into(), status.to_string());
    f
}

/// The next moment (schedule frame, after `now`) at which a status can change: the earliest
/// effective start or end still ahead.
pub fn next_boundary(c: &Computed, now: &Now) -> Option<i64> {
    c.items
        .iter()
        .filter(|e| e.status != ItemStatus::Cancelled)
        .flat_map(|e| [e.start_ms, e.end_ms])
        .filter(|t| *t > now.ms)
        .min()
}

/// Checks a schedule (see `sources::validate`) and normalizes it: texts trimmed, a dangling
/// `nextOverride` cleared.
pub fn validate(
    items: &mut [ScheduleItem],
    labels: &mut ScheduleLabels,
    next_override: &mut Option<String>,
) -> Result<(), String> {
    if items.len() > SCHEDULE_ITEMS_MAX {
        return Err(format!(
            "Too many items ({}, at most {SCHEDULE_ITEMS_MAX})",
            items.len()
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for (i, it) in items.iter_mut().enumerate() {
        let n = i + 1;
        if !is_id(&it.id) {
            return Err(format!("Item {n}: invalid id \"{}\"", it.id));
        }
        if !ids.insert(it.id.clone()) {
            return Err(format!("Item {n}: duplicate id \"{}\"", it.id));
        }
        it.title = it.title.trim().to_string();
        let tl = it.title.chars().count();
        if tl == 0 || tl > ITEM_TEXT_MAX {
            return Err(format!(
                "Item {n}: the title must be 1 to {ITEM_TEXT_MAX} characters"
            ));
        }
        for (what, v) in [
            ("subtitle", &mut it.subtitle),
            ("location", &mut it.location),
        ] {
            *v = v.trim().to_string();
            if v.chars().count() > ITEM_TEXT_MAX {
                return Err(format!(
                    "Item {n}: the {what} must be at most {ITEM_TEXT_MAX} characters"
                ));
            }
        }
        let time = |what: &str, v: &str| {
            localtime::parse_local(v).ok_or_else(|| {
                format!("Item {n}: the {what} must be a date and time like 2026-10-05T14:30")
            })
        };
        let start = time("start", &it.start)?;
        let end = time("end", &it.end)?;
        if end <= start {
            return Err(format!("Item {n}: the end must be after the start"));
        }
        if let Some(s) = &it.started_at {
            time("actual start", s)?;
        }
        if let Some(s) = &it.ended_at {
            time("actual end", s)?;
        }
        if it.delay_min > DELAY_MAX_MIN {
            return Err(format!(
                "Item {n}: the delay must be at most {DELAY_MAX_MIN} minutes"
            ));
        }
    }
    for l in [
        &labels.now,
        &labels.next,
        &labels.delayed,
        &labels.previous,
        &labels.cancelled,
        &labels.upcoming,
    ] {
        if l.chars().count() > LABEL_MAX {
            return Err(format!(
                "Status labels must be at most {LABEL_MAX} characters"
            ));
        }
    }
    if next_override
        .as_ref()
        .is_some_and(|id| !items.iter().any(|i| &i.id == id))
    {
        *next_override = None;
    }
    Ok(())
}

/// The editable part of a schedule an action changes (and Undo restores).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub items: Vec<ScheduleItem>,
    pub ripple: bool,
    pub next_override: Option<String>,
}

fn now_local(now: &Now) -> String {
    localtime::format_local(localtime::floor_minute(now.local()))
}

/// Applies an operator action (not `Undo`, which the caller handles) at `now`. `Err` with a
/// message for an unknown item or an invalid time.
pub fn apply_action(plan: &mut Plan, action: &ScheduleAction, now: &Now) -> Result<(), String> {
    let pos = |plan: &Plan, id: &str| {
        plan.items
            .iter()
            .position(|i| i.id == id)
            .ok_or_else(|| format!("Schedule item not found: {id}"))
    };
    let unpoint = |plan: &mut Plan, id: &str| {
        if plan.next_override.as_deref() == Some(id) {
            plan.next_override = None;
        }
    };
    match action {
        ScheduleAction::StartNow { item_id } => {
            let i = pos(plan, item_id)?;
            let c = compute(&plan.items, plan.ripple, plan.next_override.as_deref(), now);
            let stamp = now_local(now);
            let others: Vec<usize> = c
                .items
                .iter()
                .filter(|e| e.status == ItemStatus::Now && e.index != i)
                .map(|e| e.index)
                .collect();
            for o in others {
                let it = &mut plan.items[o];
                if it.ended_at.is_none() {
                    it.ended_at = Some(stamp.clone());
                }
                if it.state == ItemState::Now {
                    it.state = ItemState::Done;
                }
            }
            let it = &mut plan.items[i];
            it.started_at = Some(stamp);
            it.ended_at = None;
            if matches!(it.state, ItemState::Done | ItemState::Cancelled) {
                it.state = ItemState::Auto;
            }
            unpoint(plan, item_id);
        }
        ScheduleAction::End { item_id } => {
            let i = pos(plan, item_id)?;
            let it = &mut plan.items[i];
            it.ended_at = Some(now_local(now));
            it.state = ItemState::Done;
            unpoint(plan, item_id);
        }
        ScheduleAction::SetNext { item_id, start } => {
            let i = pos(plan, item_id)?;
            if let Some(s) = start {
                set_start(plan, i, s, now)?;
            }
            plan.next_override = Some(item_id.clone());
        }
        ScheduleAction::Delay { item_id, minutes } => {
            let i = pos(plan, item_id)?;
            let it = &mut plan.items[i];
            let d = (i64::from(it.delay_min) + i64::from(*minutes)).clamp(0, DELAY_MAX_MIN.into());
            it.delay_min = u32::try_from(d).unwrap_or(0);
        }
        ScheduleAction::SetStart { item_id, start } => {
            let i = pos(plan, item_id)?;
            set_start(plan, i, start, now)?;
        }
        ScheduleAction::Cancel { item_id } => {
            let i = pos(plan, item_id)?;
            plan.items[i].state = ItemState::Cancelled;
            unpoint(plan, item_id);
        }
        ScheduleAction::Restore { item_id } => {
            let i = pos(plan, item_id)?;
            let it = &mut plan.items[i];
            if it.state == ItemState::Cancelled {
                it.state = ItemState::Auto;
            }
        }
        ScheduleAction::Auto { item_id } => {
            let i = pos(plan, item_id)?;
            let it = &mut plan.items[i];
            it.state = ItemState::Auto;
            it.delay_min = 0;
            it.started_at = None;
            it.ended_at = None;
            unpoint(plan, item_id);
        }
        ScheduleAction::Undo => return Err("Undo is handled by the caller".into()),
    }
    Ok(())
}

/// A new effective start for item `i`, keeping its duration: its delay is cleared and its planned
/// start set so that (with the ripple of earlier delays) it starts exactly at `start`.
fn set_start(plan: &mut Plan, i: usize, start: &str, now: &Now) -> Result<(), String> {
    let new = localtime::parse_local(start).ok_or_else(|| {
        format!("Invalid start \"{start}\": use a date and time like 2026-10-05T14:30")
    })?;
    let new = localtime::floor_minute(new);
    let earlier_shift_min: i64 = if plan.ripple {
        let me = epoch(now, &plan.items[i].start);
        plan.items
            .iter()
            .enumerate()
            .filter(|(j, it)| {
                *j != i
                    && it.state != ItemState::Cancelled
                    && match (epoch(now, &it.start), me) {
                        (Some(a), Some(b)) => (a, *j) < (b, i),
                        _ => false,
                    }
            })
            .map(|(_, it)| i64::from(it.delay_min))
            .sum()
    } else {
        0
    };
    let it = &mut plan.items[i];
    let old_start = localtime::parse_local(&it.start).ok_or("Invalid planned start")?;
    let old_end = localtime::parse_local(&it.end).ok_or("Invalid planned end")?;
    let duration = old_end - old_start;
    let planned: PrimitiveDateTime = new - time::Duration::minutes(earlier_shift_min);
    it.start = localtime::format_local(planned);
    it.end = localtime::format_local(planned + duration);
    it.delay_min = 0;
    it.started_at = None;
    it.ended_at = None;
    if it.state == ItemState::Done {
        it.state = ItemState::Auto;
    }
    Ok(())
}

/// Statuses by item id (for transition tracking).
pub fn statuses(items: &[ScheduleItem], c: &Computed) -> HashMap<String, ItemStatus> {
    c.items
        .iter()
        .map(|e| (items[e.index].id.clone(), e.status))
        .collect()
}

#[cfg(test)]
mod tests;

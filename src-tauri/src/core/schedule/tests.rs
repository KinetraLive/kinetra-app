use super::*;
use crate::core::localtime::{parse_local, Zone};

const Z: Zone = Zone::Fixed(0);

fn t(s: &str) -> i64 {
    Z.to_epoch(parse_local(&format!("2026-10-05T{s}")).unwrap())
}

fn at(s: &str) -> Now {
    Now::at(t(s), Z)
}

fn item(id: &str, start: &str, end: &str) -> ScheduleItem {
    ScheduleItem {
        id: id.into(),
        title: format!("Item {id}"),
        subtitle: String::new(),
        location: format!("Room {id}"),
        start: format!("2026-10-05T{start}"),
        end: format!("2026-10-05T{end}"),
        delay_min: 0,
        state: ItemState::Auto,
        started_at: None,
        ended_at: None,
    }
}

/// A 13:00 / 13:30–14:30 / 14:30 / 15:00–16:00 run of show.
fn run() -> Vec<ScheduleItem> {
    vec![
        item("a", "13:00", "13:30"),
        item("b", "13:30", "14:30"),
        item("c", "14:30", "15:00"),
        item("d", "15:00", "16:00"),
    ]
}

fn statuses_at(
    items: &[ScheduleItem],
    ripple: bool,
    ovr: Option<&str>,
    now: &Now,
) -> Vec<(String, ItemStatus)> {
    let c = compute(items, ripple, ovr, now);
    c.items
        .iter()
        .map(|e| (items[e.index].id.clone(), e.status))
        .collect()
}

fn st(list: &[(&str, ItemStatus)]) -> Vec<(String, ItemStatus)> {
    list.iter().map(|(i, s)| (i.to_string(), *s)).collect()
}

use ItemStatus::{Cancelled, Delayed, Next, Previous, Upcoming};
const NOW_: ItemStatus = ItemStatus::Now;

#[test]
fn statuses_follow_the_clock() {
    let items = run();
    let c = compute(&items, false, None, &at("14:00"));
    assert_eq!(
        statuses_at(&items, false, None, &at("14:00")),
        st(&[("a", Previous), ("b", NOW_), ("c", Next), ("d", Upcoming)])
    );
    assert_eq!(c.now_id.as_deref(), Some("b"));
    assert_eq!(c.next_id.as_deref(), Some("c"));
    // Boundaries: an end at `now` is past, a start at `now` has begun.
    assert_eq!(
        statuses_at(&items, false, None, &at("14:30")),
        st(&[("a", Previous), ("b", Previous), ("c", NOW_), ("d", Next)])
    );
    // Before everything: the first is next; after everything: all previous, no next.
    assert_eq!(
        statuses_at(&items, false, None, &at("12:00")),
        st(&[
            ("a", Next),
            ("b", Upcoming),
            ("c", Upcoming),
            ("d", Upcoming)
        ])
    );
    let late = compute(&items, false, None, &at("17:00"));
    assert!(late.items.iter().all(|e| e.status == Previous));
    assert_eq!((late.now_id.clone(), late.next_id.clone()), (None, None));
    // The next boundary: B's end.
    assert_eq!(next_boundary(&c, &at("14:00")), Some(t("14:30")));
    assert_eq!(next_boundary(&late, &at("17:00")), None);
}

#[test]
fn operator_states_override_the_clock() {
    let mut items = run();
    items[2].state = ItemState::Cancelled;
    items[1].state = ItemState::Done;
    items[3].state = ItemState::Now;
    let c = compute(&items, false, None, &at("14:00"));
    assert_eq!(
        statuses_at(&items, false, None, &at("14:00")),
        st(&[
            ("a", Previous),
            ("b", Previous),
            ("c", Cancelled),
            ("d", NOW_)
        ])
    );
    assert_eq!(c.now_id.as_deref(), Some("d"));
    assert_eq!(c.next_id, None, "nothing upcoming is left");
    // A cancelled item is never next and has no boundary.
    let mut items = run();
    items[2].state = ItemState::Cancelled;
    let c = compute(&items, false, None, &at("14:00"));
    assert_eq!(c.next_id.as_deref(), Some("d"));
    assert_eq!(next_boundary(&c, &at("14:29")), Some(t("14:30")));
}

#[test]
fn actual_start_and_end_override_the_plan() {
    let mut items = run();
    // C started early at 14:10: NOW, its end follows its duration (14:40), and it moved.
    items[2].started_at = Some("2026-10-05T14:10".into());
    let c = compute(&items, false, None, &at("14:15"));
    let ce = c.items.iter().find(|e| items[e.index].id == "c").unwrap();
    assert_eq!(
        (ce.status, ce.start_ms, ce.end_ms),
        (NOW_, t("14:10"), t("14:40"))
    );
    assert!(ce.moved);
    // Two NOW items: the view's NOW is the one that started last.
    assert_eq!(c.now_id.as_deref(), Some("c"));
    // B ended at 14:12.
    items[1].ended_at = Some("2026-10-05T14:12".into());
    assert_eq!(
        statuses_at(&items, false, None, &at("14:15")),
        st(&[("a", Previous), ("b", Previous), ("c", NOW_), ("d", Next)])
    );
    // An item started late that has passed its planned end is still on.
    let mut items = run();
    items[0].started_at = Some("2026-10-05T13:20".into());
    let c = compute(&items, false, None, &at("13:40"));
    let ae = c.items.iter().find(|e| items[e.index].id == "a").unwrap();
    assert_eq!((ae.status, ae.end_ms), (NOW_, t("13:50")));
}

#[test]
fn delays_shift_one_item_or_ripple() {
    let mut items = run();
    items[2].delay_min = 10;
    let c = compute(&items, false, None, &at("14:00"));
    let ce = c.items.iter().find(|e| items[e.index].id == "c").unwrap();
    assert_eq!(
        (ce.status, ce.start_ms, ce.end_ms),
        (Delayed, t("14:40"), t("15:10"))
    );
    assert!(ce.moved);
    assert_eq!(
        c.next_id.as_deref(),
        Some("c"),
        "a delayed next stays DELAYED but is next"
    );
    let de = c.items.iter().find(|e| items[e.index].id == "d").unwrap();
    assert_eq!(
        (de.status, de.start_ms, de.moved),
        (Upcoming, t("15:00"), false)
    );

    // Ripple: later items shift too, cumulatively; they are moved, not DELAYED.
    items[1].delay_min = 5;
    let c = compute(&items, true, None, &at("14:00"));
    let get = |id: &str| {
        c.items
            .iter()
            .find(|e| items[e.index].id == id)
            .unwrap()
            .clone()
    };
    assert_eq!(get("b").end_ms, t("14:35"));
    assert_eq!(get("c").start_ms, t("14:45"));
    assert_eq!(
        (get("d").status, get("d").start_ms, get("d").moved),
        (Upcoming, t("15:15"), true)
    );
    assert_eq!(get("a").start_ms, t("13:00"), "earlier items don't move");
    // A cancelled item's delay doesn't ripple.
    items[1].state = ItemState::Cancelled;
    let c = compute(&items, true, None, &at("14:00"));
    let d = c.items.iter().find(|e| items[e.index].id == "d").unwrap();
    assert_eq!(d.start_ms, t("15:10"));
    // Items are ordered by effective start.
    let mut items = run();
    items[2].delay_min = 45;
    let ids: Vec<String> = compute(&items, false, None, &at("14:00"))
        .items
        .iter()
        .map(|e| items[e.index].id.clone())
        .collect();
    assert_eq!(ids, ["a", "b", "d", "c"]);
}

#[test]
fn next_override_picks_up_next() {
    let items = run();
    assert_eq!(
        statuses_at(&items, false, Some("d"), &at("14:00")),
        st(&[("a", Previous), ("b", NOW_), ("c", Upcoming), ("d", Next)])
    );
    // Pointing at an item that is not upcoming: ignored.
    for id in ["a", "b", "nope"] {
        let c = compute(&items, false, Some(id), &at("14:00"));
        assert_eq!(c.next_id.as_deref(), Some("c"), "{id}");
    }
}

#[test]
fn multi_day_schedules() {
    let mut items = run();
    items.push(ScheduleItem {
        start: "2026-10-06T10:00".into(),
        end: "2026-10-06T11:00".into(),
        ..item("e", "10:00", "11:00")
    });
    let now = at("23:00");
    let c = compute(&items, false, None, &now);
    assert_eq!(c.next_id.as_deref(), Some("e"));
    let rows = rows(&items, &ScheduleLabels::default(), &c, &now);
    assert_eq!(rows[0]["day"], "Mon 5 Oct");
    assert_eq!(rows[4]["day"], "Tue 6 Oct");
    let v = view("src", &items, &c, &now, false);
    assert_eq!(v.items[4].start, "2026-10-06T10:00");
    assert_eq!(v.items[4].title, "Item e");
    assert_eq!(v.items[4].location, "Room e");
    // A non-UTC zone: local times stay local.
    let z = Zone::Fixed(2 * 3600);
    let now = Now::at(z.to_epoch(parse_local("2026-10-05T14:00").unwrap()), z);
    let c = compute(&items, false, None, &now);
    assert_eq!(c.now_id.as_deref(), Some("b"));
    let r = super::rows(&items, &ScheduleLabels::default(), &c, &now);
    assert_eq!(r[1]["start"], "13:30");
}

#[test]
fn binding_columns() {
    let mut items = run();
    items[2].delay_min = 10;
    let now = at("14:00");
    let labels = ScheduleLabels {
        upcoming: "LATER".into(),
        ..Default::default()
    };
    let c = compute(&items, false, None, &now);
    let r = rows(&items, &labels, &c, &now);
    assert_eq!(r.len(), 4);
    let keys: Vec<&str> = SCHEDULE_COLUMNS.iter().map(|(k, _)| *k).collect();
    for row in &r {
        for k in &keys {
            assert!(row.contains_key(*k), "{k}");
        }
        assert_eq!(row["_status"], row["status"]);
    }
    assert_eq!(r[1]["title"], "Item b");
    assert_eq!(r[1]["location"], "Room b");
    assert_eq!(r[1]["start"], "13:30");
    assert_eq!(r[1]["end"], "14:30");
    assert_eq!(r[1]["new_start"], "");
    assert_eq!(r[1]["label"], "NOW");
    assert_eq!(r[1]["status"], "now");
    assert_eq!(r[2]["status"], "delayed");
    assert_eq!(r[2]["label"], "DELAYED");
    assert_eq!(r[2]["start"], "14:40");
    assert_eq!(r[2]["new_start"], "14:40");
    assert_eq!(r[3]["label"], "LATER");
    assert_eq!(r[0]["label"], "PREVIOUS");
    // Countdown: running to the effective start, zero exactly then; past/now stopped at zero.
    let cd = fieldvals::parse_timer(&r[2]["countdown"]).unwrap();
    assert!(cd.run && cd.dir == TimerDir::Down && cd.end == TimerEnd::Stop);
    assert_eq!(cd.at + cd.base, t("14:40") as u64);
    assert!(cd.at as i64 <= now.real_ms);
    assert_eq!(
        fieldvals::displayed_ms(&cd, now.real_ms as u64),
        40 * 60_000
    );
    assert_eq!((cd.at + cd.base) % 1000, 0, "zero on a whole second");
    for i in [0, 1] {
        let z = fieldvals::parse_timer(&r[i]["countdown"]).unwrap();
        assert!(!z.run && z.base == 0);
    }
    // Deterministic: the same value at another moment (no spurious updates).
    assert_eq!(
        countdown(t("14:40"), &at("14:01")),
        countdown(t("14:40"), &at("14:05"))
    );
    // A skewed clock: the zero is in real (renderer) time.
    let skewed = Now {
        ms: t("14:00"),
        real_ms: t("14:00") - 3_600_000,
        zone: Z,
    };
    let cd = fieldvals::parse_timer(&countdown(t("14:40"), &skewed)).unwrap();
    assert_eq!(cd.at + cd.base, (t("14:40") - 3_600_000) as u64);
}

fn plan() -> Plan {
    Plan {
        items: run(),
        ripple: false,
        next_override: None,
    }
}

fn find<'a>(p: &'a Plan, id: &str) -> &'a ScheduleItem {
    p.items.iter().find(|i| i.id == id).unwrap()
}

fn act(p: &mut Plan, a: ScheduleAction, now: &str) {
    apply_action(p, &a, &at(now)).unwrap();
}

#[test]
fn start_now_and_end() {
    let mut p = plan();
    p.next_override = Some("c".into());
    act(
        &mut p,
        ScheduleAction::StartNow {
            item_id: "c".into(),
        },
        "14:10",
    );
    assert_eq!(
        find(&p, "c").started_at.as_deref(),
        Some("2026-10-05T14:10")
    );
    assert_eq!(
        find(&p, "b").ended_at.as_deref(),
        Some("2026-10-05T14:10"),
        "the previous NOW ends"
    );
    assert_eq!(p.next_override, None, "the override pointed at it");
    let c = compute(&p.items, false, None, &Now::at(t("14:10") + 30_000, Z));
    assert_eq!(c.now_id.as_deref(), Some("c"));
    assert_eq!(c.next_id.as_deref(), Some("d"));
    let ce = c.items.iter().find(|e| p.items[e.index].id == "c").unwrap();
    assert_eq!(ce.end_ms, t("14:40"), "duration kept");
    // End: PREVIOUS, done.
    act(
        &mut p,
        ScheduleAction::End {
            item_id: "c".into(),
        },
        "14:20",
    );
    assert_eq!(find(&p, "c").state, ItemState::Done);
    assert_eq!(find(&p, "c").ended_at.as_deref(), Some("2026-10-05T14:20"));
    let c = compute(&p.items, false, None, &at("14:20"));
    assert_eq!(c.now_id, None);
    // Starting a done or cancelled item brings it back.
    act(
        &mut p,
        ScheduleAction::StartNow {
            item_id: "c".into(),
        },
        "14:21",
    );
    assert_eq!(find(&p, "c").state, ItemState::Auto);
    assert_eq!(find(&p, "c").ended_at, None);
    // Unknown item.
    assert!(apply_action(
        &mut p,
        &ScheduleAction::End {
            item_id: "zz".into()
        },
        &at("14:00")
    )
    .is_err());
}

#[test]
fn set_next_delay_set_start_cancel_restore_auto() {
    let mut p = plan();
    act(
        &mut p,
        ScheduleAction::SetNext {
            item_id: "d".into(),
            start: Some("2026-10-05T14:50".into()),
        },
        "14:00",
    );
    assert_eq!(p.next_override.as_deref(), Some("d"));
    assert_eq!(
        (find(&p, "d").start.as_str(), find(&p, "d").end.as_str()),
        ("2026-10-05T14:50", "2026-10-05T15:50")
    );
    act(
        &mut p,
        ScheduleAction::SetNext {
            item_id: "c".into(),
            start: None,
        },
        "14:00",
    );
    assert_eq!(p.next_override.as_deref(), Some("c"));
    assert_eq!(find(&p, "c").start, "2026-10-05T14:30");

    act(
        &mut p,
        ScheduleAction::Delay {
            item_id: "c".into(),
            minutes: 5,
        },
        "14:00",
    );
    act(
        &mut p,
        ScheduleAction::Delay {
            item_id: "c".into(),
            minutes: 10,
        },
        "14:00",
    );
    assert_eq!(find(&p, "c").delay_min, 15);
    act(
        &mut p,
        ScheduleAction::Delay {
            item_id: "c".into(),
            minutes: -30,
        },
        "14:00",
    );
    assert_eq!(find(&p, "c").delay_min, 0, "never below zero");

    // SetStart keeps the duration and clears the delay.
    act(
        &mut p,
        ScheduleAction::Delay {
            item_id: "c".into(),
            minutes: 5,
        },
        "14:00",
    );
    act(
        &mut p,
        ScheduleAction::SetStart {
            item_id: "c".into(),
            start: "2026-10-05T15:10".into(),
        },
        "14:00",
    );
    let c = find(&p, "c");
    assert_eq!(
        (c.start.as_str(), c.end.as_str(), c.delay_min),
        ("2026-10-05T15:10", "2026-10-05T15:40", 0)
    );
    assert!(apply_action(
        &mut p,
        &ScheduleAction::SetStart {
            item_id: "c".into(),
            start: "tomorrow".into()
        },
        &at("14:00")
    )
    .is_err());
    // With ripple, SetStart lands exactly on the time despite earlier delays.
    let mut r = plan();
    r.ripple = true;
    r.items[1].delay_min = 10;
    act(
        &mut r,
        ScheduleAction::SetStart {
            item_id: "d".into(),
            start: "2026-10-05T15:30".into(),
        },
        "14:00",
    );
    let cmp = compute(&r.items, true, None, &at("14:00"));
    let d = cmp
        .items
        .iter()
        .find(|e| r.items[e.index].id == "d")
        .unwrap();
    assert_eq!(d.start_ms, t("15:30"));

    act(
        &mut p,
        ScheduleAction::Cancel {
            item_id: "c".into(),
        },
        "14:00",
    );
    assert_eq!(find(&p, "c").state, ItemState::Cancelled);
    assert_eq!(p.next_override, None, "a cancelled item is not next");
    act(
        &mut p,
        ScheduleAction::Restore {
            item_id: "c".into(),
        },
        "14:00",
    );
    assert_eq!(find(&p, "c").state, ItemState::Auto);

    // Back to automatic clears every override of the item.
    act(
        &mut p,
        ScheduleAction::Delay {
            item_id: "b".into(),
            minutes: 5,
        },
        "14:00",
    );
    act(
        &mut p,
        ScheduleAction::End {
            item_id: "b".into(),
        },
        "14:00",
    );
    p.next_override = Some("b".into());
    act(
        &mut p,
        ScheduleAction::Auto {
            item_id: "b".into(),
        },
        "14:00",
    );
    let b = find(&p, "b");
    assert_eq!(
        (
            b.state,
            b.delay_min,
            b.started_at.clone(),
            b.ended_at.clone()
        ),
        (ItemState::Auto, 0, None, None)
    );
    assert_eq!(p.next_override, None);
    assert!(
        apply_action(&mut p, &ScheduleAction::Undo, &at("14:00")).is_err(),
        "undo is the caller's"
    );
}

#[test]
fn validation() {
    let ok = || (run(), ScheduleLabels::default(), None::<String>);
    let check = |items: &mut Vec<ScheduleItem>,
                 labels: &mut ScheduleLabels,
                 o: &mut Option<String>| { validate(items, labels, o) };
    let (mut i, mut l, mut o) = ok();
    assert!(check(&mut i, &mut l, &mut o).is_ok());
    // Trimmed; a dangling override is cleared.
    i[0].title = "  Spaced  ".into();
    o = Some("gone".into());
    check(&mut i, &mut l, &mut o).unwrap();
    assert_eq!(i[0].title, "Spaced");
    assert_eq!(o, None);
    type Bad = Box<dyn Fn(&mut Vec<ScheduleItem>, &mut ScheduleLabels)>;
    let bads: Vec<(&str, Bad)> = vec![
        ("empty title", Box::new(|i, _| i[0].title = " ".into())),
        ("long title", Box::new(|i, _| i[0].title = "x".repeat(201))),
        (
            "long location",
            Box::new(|i, _| i[0].location = "x".repeat(201)),
        ),
        ("bad id", Box::new(|i, _| i[0].id = "../x".into())),
        ("dup id", Box::new(|i, _| i[1].id = "a".into())),
        ("bad start", Box::new(|i, _| i[0].start = "14:00".into())),
        (
            "end before start",
            Box::new(|i, _| i[0].end = "2026-10-05T12:00".into()),
        ),
        (
            "end = start",
            Box::new(|i, _| i[0].end = i[0].start.clone()),
        ),
        (
            "bad started",
            Box::new(|i, _| i[0].started_at = Some("x".into())),
        ),
        (
            "big delay",
            Box::new(|i, _| i[0].delay_min = DELAY_MAX_MIN + 1),
        ),
        ("long label", Box::new(|_, l| l.now = "x".repeat(33))),
        (
            "too many",
            Box::new(|i, _| {
                let x = i[0].clone();
                i.clear();
                for n in 0..=SCHEDULE_ITEMS_MAX {
                    i.push(ScheduleItem {
                        id: format!("i{n}"),
                        ..x.clone()
                    });
                }
            }),
        ),
    ];
    for (what, f) in bads {
        let (mut i, mut l, mut o) = ok();
        f(&mut i, &mut l);
        assert!(check(&mut i, &mut l, &mut o).is_err(), "{what}");
    }
}

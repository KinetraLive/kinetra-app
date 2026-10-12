//! Local date-times for schedules and time-of-day rules: parsing and formatting the
//! `YYYY-MM-DDTHH:MM` values stored in schedule items, and converting between them and server
//! epoch milliseconds in the app machine's time zone.
//!
//! # Time zone
//! The `time` crate's `local-offset` feature is not enabled (and would add a dependency path), so
//! the conversion uses the OS directly:
//! - **Windows:** `SystemTimeToTzSpecificLocalTime` / `TzSpecificLocalTimeToSystemTime` from
//!   kernel32 (always linked; no new crate). They apply the current time zone's daylight-saving
//!   rules to the date being converted, so each item is converted with the offset valid on ITS
//!   date (DST-safe per item, e.g. a festival spanning the October change).
//! - **Elsewhere:** UTC (offset 0). Kinetra ships for Windows; this keeps the code portable.
//!
//! Tests (and the e2e fixture) use [`Zone::Fixed`] and a skewed [`Clock`] instead.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, PoisonError};

use time::{Date, Month, PrimitiveDateTime, Time, Weekday};

use crate::core::fsx;

/// How local date-times map to epoch milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Zone {
    /// The app machine's time zone (see the module docs).
    #[default]
    System,
    /// A fixed offset east of UTC, in seconds (tests).
    Fixed(i32),
}

const MS_PER_DAY: i64 = 86_400_000;

fn epoch_date() -> Date {
    // 1970-01-01 always exists.
    Date::from_calendar_date(1970, Month::January, 1).unwrap_or(Date::MIN)
}

/// A UTC (or offset-free) date-time as milliseconds since the epoch.
pub fn naive_to_ms(dt: PrimitiveDateTime) -> i64 {
    let days = i64::from(dt.date().to_julian_day() - epoch_date().to_julian_day());
    let t = dt.time();
    days * MS_PER_DAY
        + i64::from(t.hour()) * 3_600_000
        + i64::from(t.minute()) * 60_000
        + i64::from(t.second()) * 1000
        + i64::from(t.millisecond())
}

/// Milliseconds since the epoch as an offset-free date-time.
pub fn ms_to_naive(ms: i64) -> PrimitiveDateTime {
    let days = ms.div_euclid(MS_PER_DAY);
    let rest = ms.rem_euclid(MS_PER_DAY);
    let julian = i64::from(epoch_date().to_julian_day()) + days;
    let date = i32::try_from(julian)
        .ok()
        .and_then(|j| Date::from_julian_day(j).ok())
        .unwrap_or(epoch_date());
    let time = Time::from_hms_milli(
        (rest / 3_600_000) as u8,
        ((rest / 60_000) % 60) as u8,
        ((rest / 1000) % 60) as u8,
        (rest % 1000) as u16,
    )
    .unwrap_or(Time::MIDNIGHT);
    PrimitiveDateTime::new(date, time)
}

impl Zone {
    /// The local date-time of an instant.
    pub fn to_local(self, epoch_ms: i64) -> PrimitiveDateTime {
        match self {
            Zone::Fixed(off) => ms_to_naive(epoch_ms + i64::from(off) * 1000),
            Zone::System => ms_to_naive(sys::utc_to_local(epoch_ms).unwrap_or(epoch_ms)),
        }
    }

    /// The instant of a local date-time.
    pub fn to_epoch(self, local: PrimitiveDateTime) -> i64 {
        let naive = naive_to_ms(local);
        match self {
            Zone::Fixed(off) => naive - i64::from(off) * 1000,
            Zone::System => sys::local_to_utc(naive).unwrap_or(naive),
        }
    }
}

#[cfg(windows)]
mod sys {
    //! kernel32's time zone conversions (see the module docs).

    use std::ffi::c_void;

    use super::{ms_to_naive, naive_to_ms};
    use time::{Date, Month, PrimitiveDateTime, Time};

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn SystemTimeToTzSpecificLocalTime(
            tz: *const c_void,
            universal: *const SystemTime,
            local: *mut SystemTime,
        ) -> i32;
        fn TzSpecificLocalTimeToSystemTime(
            tz: *const c_void,
            local: *const SystemTime,
            universal: *mut SystemTime,
        ) -> i32;
    }

    fn to_st(ms: i64) -> Option<SystemTime> {
        let dt = ms_to_naive(ms);
        let year = u16::try_from(dt.year()).ok().filter(|y| *y >= 1601)?;
        Some(SystemTime {
            year,
            month: u8::from(dt.month()) as u16,
            day_of_week: 0,
            day: dt.day() as u16,
            hour: dt.hour() as u16,
            minute: dt.minute() as u16,
            second: dt.second() as u16,
            milliseconds: dt.millisecond(),
        })
    }

    fn from_st(st: &SystemTime) -> Option<i64> {
        let month = Month::try_from(u8::try_from(st.month).ok()?).ok()?;
        let date =
            Date::from_calendar_date(i32::from(st.year), month, u8::try_from(st.day).ok()?).ok()?;
        let time = Time::from_hms_milli(
            u8::try_from(st.hour).ok()?,
            u8::try_from(st.minute).ok()?,
            u8::try_from(st.second).ok()?,
            st.milliseconds,
        )
        .ok()?;
        Some(naive_to_ms(PrimitiveDateTime::new(date, time)))
    }

    /// UTC epoch ms → local wall-clock ms (offset-free).
    pub fn utc_to_local(ms: i64) -> Option<i64> {
        let utc = to_st(ms)?;
        let mut local = SystemTime::default();
        // SAFETY: both pointers are valid for the call; a null zone = the current time zone.
        let ok = unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) };
        if ok == 0 {
            return None;
        }
        from_st(&local)
    }

    /// Local wall-clock ms (offset-free) → UTC epoch ms.
    pub fn local_to_utc(ms: i64) -> Option<i64> {
        let local = to_st(ms)?;
        let mut utc = SystemTime::default();
        // SAFETY: both pointers are valid for the call; a null zone = the current time zone.
        let ok = unsafe { TzSpecificLocalTimeToSystemTime(std::ptr::null(), &local, &mut utc) };
        if ok == 0 {
            return None;
        }
        from_st(&utc)
    }
}

#[cfg(not(windows))]
mod sys {
    //! UTC fallback (see the module docs).
    pub fn utc_to_local(ms: i64) -> Option<i64> {
        Some(ms)
    }
    pub fn local_to_utc(ms: i64) -> Option<i64> {
        Some(ms)
    }
}

/// `YYYY-MM-DDTHH:MM`, optionally with `:SS`.
pub fn parse_local(s: &str) -> Option<PrimitiveDateTime> {
    let b = s.as_bytes();
    if !(b.len() == 16 || b.len() == 19) {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        let part = s.get(r)?;
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' {
        return None;
    }
    let second = if b.len() == 19 {
        if b[16] != b':' {
            return None;
        }
        digits(17..19)?
    } else {
        0
    };
    let year = i32::try_from(digits(0..4)?).ok()?;
    let month = Month::try_from(u8::try_from(digits(5..7)?).ok()?).ok()?;
    let date = Date::from_calendar_date(year, month, u8::try_from(digits(8..10)?).ok()?).ok()?;
    let time = Time::from_hms(
        u8::try_from(digits(11..13)?).ok()?,
        u8::try_from(digits(14..16)?).ok()?,
        u8::try_from(second).ok()?,
    )
    .ok()?;
    Some(PrimitiveDateTime::new(date, time))
}

/// `YYYY-MM-DDTHH:MM` (seconds dropped).
pub fn format_local(dt: PrimitiveDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        dt.year(),
        u8::from(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute()
    )
}

/// `HH:MM`.
pub fn hhmm(dt: PrimitiveDateTime) -> String {
    format!("{:02}:{:02}", dt.hour(), dt.minute())
}

/// `HH:MM` → (hour, minute).
pub fn parse_hhmm(s: &str) -> Option<(u8, u8)> {
    let (h, m) = s.split_once(':')?;
    if h.len() != 2 || m.len() != 2 || !h.bytes().chain(m.bytes()).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// `Sat 5 Oct`.
pub fn day_label(dt: PrimitiveDateTime) -> String {
    let wd = match dt.weekday() {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    };
    let month = match dt.month() {
        Month::January => "Jan",
        Month::February => "Feb",
        Month::March => "Mar",
        Month::April => "Apr",
        Month::May => "May",
        Month::June => "Jun",
        Month::July => "Jul",
        Month::August => "Aug",
        Month::September => "Sep",
        Month::October => "Oct",
        Month::November => "Nov",
        Month::December => "Dec",
    };
    format!("{wd} {} {month}", dt.day())
}

/// The date-time floored to the minute.
pub fn floor_minute(dt: PrimitiveDateTime) -> PrimitiveDateTime {
    dt.replace_second(0)
        .and_then(|d| d.replace_millisecond(0))
        .unwrap_or(dt)
}

/// The current time as schedules see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    /// Epoch ms in the schedule's frame (real time plus the test skew).
    pub ms: i64,
    /// Real server epoch ms (what renderers' clocks and timer states use).
    pub real_ms: i64,
    pub zone: Zone,
}

impl Now {
    /// A fixed instant (tests).
    pub fn at(ms: i64, zone: Zone) -> Self {
        Self {
            ms,
            real_ms: ms,
            zone,
        }
    }

    pub fn local(&self) -> PrimitiveDateTime {
        self.zone.to_local(self.ms)
    }

    /// Schedule-frame ms → real server ms.
    pub fn to_real(&self, ms: i64) -> i64 {
        ms - (self.ms - self.real_ms)
    }
}

/// Server time for schedules and time-of-day rules: real time, plus a skew and a fixed zone that
/// only tests and the e2e fixture set.
#[derive(Debug, Default)]
pub struct Clock {
    skew_ms: AtomicI64,
    zone: Mutex<Zone>,
}

impl Clock {
    pub fn now(&self) -> Now {
        let real = i64::try_from(fsx::now_epoch_ms()).unwrap_or(i64::MAX);
        Now {
            ms: real + self.skew_ms.load(Ordering::Relaxed),
            real_ms: real,
            zone: *self.zone.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }

    /// Test time: schedule time = real time + `skew_ms`, in `zone`.
    #[doc(hidden)]
    pub fn set_test(&self, skew_ms: i64, zone: Zone) {
        self.skew_ms.store(skew_ms, Ordering::Relaxed);
        *self.zone.lock().unwrap_or_else(PoisonError::into_inner) = zone;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_local_date_times() {
        let dt = parse_local("2026-10-05T14:30").unwrap();
        assert_eq!(format_local(dt), "2026-10-05T14:30");
        assert_eq!(hhmm(dt), "14:30");
        assert_eq!(day_label(dt), "Mon 5 Oct");
        assert_eq!(
            format_local(parse_local("2026-10-05T14:30:59").unwrap()),
            "2026-10-05T14:30"
        );
        for bad in [
            "",
            "2026-10-05 14:30",
            "2026-13-05T14:30",
            "2026-02-30T14:30",
            "2026-10-05T24:00",
            "2026-10-05T14:60",
            "2026-10-05T14:3",
            "+026-10-05T14:30",
            "2026-10-05T14:30:61",
        ] {
            assert!(parse_local(bad).is_none(), "{bad}");
        }
        assert_eq!(parse_hhmm("07:05"), Some((7, 5)));
        for bad in ["7:05", "24:00", "12:60", "ab:cd", "1205"] {
            assert!(parse_hhmm(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn epoch_round_trips() {
        let dt = parse_local("2026-10-05T14:30").unwrap();
        let z = Zone::Fixed(7200);
        let ms = z.to_epoch(dt);
        assert_eq!(ms, naive_to_ms(dt) - 7_200_000);
        assert_eq!(z.to_local(ms), dt);
        assert_eq!(ms_to_naive(0), parse_local("1970-01-01T00:00").unwrap());
        assert_eq!(
            ms_to_naive(-60_000),
            parse_local("1969-12-31T23:59").unwrap()
        );
        // The system zone round-trips too (whatever it is).
        let s = Zone::System;
        assert_eq!(s.to_local(s.to_epoch(dt)), dt);
    }

    #[test]
    fn clock_skew_and_zone() {
        let c = Clock::default();
        c.set_test(3_600_000, Zone::Fixed(0));
        let n = c.now();
        assert_eq!(n.ms - n.real_ms, 3_600_000);
        assert_eq!(n.to_real(n.ms), n.real_ms);
        assert_eq!(n.zone, Zone::Fixed(0));
    }
}

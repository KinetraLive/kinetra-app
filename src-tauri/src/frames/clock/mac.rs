//! macOS: `mach_absolute_time` (monotonic, the same in every process), `mach_wait_until` for
//! absolute deadlines, and the user-interactive QoS class for the clock and sender threads.

use std::sync::OnceLock;

#[repr(C)]
#[derive(Default)]
struct Timebase {
    numer: u32,
    denom: u32,
}

/// `QOS_CLASS_USER_INTERACTIVE` (sys/qos.h).
const QOS_USER_INTERACTIVE: u32 = 0x21;

extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_timebase_info(info: *mut Timebase) -> i32;
    fn mach_wait_until(deadline: u64) -> i32;
    fn qos_class_self() -> u32;
    fn pthread_set_qos_class_self_np(class: u32, relative_priority: i32) -> i32;
}

/// (numer, denom): mach ticks × numer / denom = nanoseconds.
fn timebase() -> (u64, u64) {
    static T: OnceLock<(u64, u64)> = OnceLock::new();
    *T.get_or_init(|| {
        let mut t = Timebase::default();
        if unsafe { mach_timebase_info(&mut t) } != 0 || t.numer == 0 || t.denom == 0 {
            return (1, 1);
        }
        (u64::from(t.numer), u64::from(t.denom))
    })
}

/// Microseconds on the mach clock.
pub fn now_us() -> i64 {
    let (n, d) = timebase();
    let ticks = unsafe { mach_absolute_time() };
    (u128::from(ticks) * u128::from(n) / (u128::from(d) * 1000)) as i64
}

fn us_to_ticks(us: i64) -> u64 {
    let (n, d) = timebase();
    (u128::from(us.max(0) as u64) * 1000 * u128::from(d) / u128::from(n)) as u64
}

/// The calling thread at the user-interactive QoS class while this lives (the previous class is
/// restored on drop).
pub struct Boost(Option<u32>);

pub fn boost_thread() -> Boost {
    let before = unsafe { qos_class_self() };
    if unsafe { pthread_set_qos_class_self_np(QOS_USER_INTERACTIVE, 0) } == 0 {
        Boost(Some(before))
    } else {
        Boost(None)
    }
}

impl Drop for Boost {
    fn drop(&mut self) {
        if let Some(c) = self.0.take() {
            unsafe {
                pthread_set_qos_class_self_np(c, 0);
            }
        }
    }
}

/// The ticker's sleep (no state: `mach_wait_until` takes an absolute deadline).
pub struct Timer;

impl Timer {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }

    /// Sleeps until `due_us` ([`now_us`]).
    pub fn sleep_until(&mut self, due_us: i64) {
        if due_us > now_us() {
            unsafe {
                mach_wait_until(us_to_ticks(due_us));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microseconds_round_trip_through_mach_ticks() {
        let now = now_us();
        let (n, d) = timebase();
        let back = (u128::from(us_to_ticks(now)) * u128::from(n) / (u128::from(d) * 1000)) as i64;
        assert!((back - now).abs() <= 1);
    }

    #[test]
    fn a_boost_can_be_taken_and_released() {
        let before = unsafe { qos_class_self() };
        drop(boost_thread());
        assert_eq!(unsafe { qos_class_self() }, before);
    }
}

//! Converting epoch seconds to the watch's **local** civil time.
//!
//! Apps that display a stored timestamp (a saved activity, a log entry) hold a
//! UTC epoch but must render local time. Only the firmware knows the watch's
//! timezone, so this conversion cannot be done in portable Rust — a
//! civil-from-days implementation would render everything in UTC, which is
//! wrong by up to 12 hours and invisible on an emulator running UTC.
//!
//! `strftime` is deliberately NOT wrapped: it is a C varargs format-string
//! surface. Format the numeric fields of [`Time`] instead.

use crate::sys;
use crate::tick::Time;

/// Converts a Unix epoch to the watch's local civil time.
///
/// Returns `None` if the SDK cannot represent the value (`localtime` returns
/// NULL). The C code that motivated this wrapper does not check for NULL; the
/// wrapper does.
pub fn local_time(epoch: i64) -> Option<Time> {
    let t = epoch_to_time_t(epoch);
    // SAFETY: &t is a live local variable for the duration of this call, and
    // localtime does not retain the pointer after returning.
    let tm = unsafe { sys::localtime(&t as *const sys::time_t as *mut sys::time_t) };
    if tm.is_null() {
        return None;
    }
    // SAFETY: localtime returned non-NULL, so it points at its static `tm`,
    // which stays valid until the next localtime/gmtime call. We copy out of
    // it immediately and never retain the pointer.
    Some(Time::from_tm(unsafe { &*tm }))
}

/// Narrow an i64 epoch to `sys::time_t` with saturation.
///
/// `sys::time_t` is `c_long`, which is 32 bits on `thumbv7m-none-eabi` (i32)
/// and 64 bits on 64-bit hosts. Without saturation, an `epoch` larger than
/// `time_t::MAX` would wrap and render an ancient date. Saturation ensures the
/// result is clearly wrong rather than silently corrupted.
#[allow(clippy::unnecessary_cast)]
fn epoch_to_time_t(epoch: i64) -> sys::time_t {
    if epoch < sys::time_t::MIN as i64 {
        sys::time_t::MIN
    } else if epoch > sys::time_t::MAX as i64 {
        sys::time_t::MAX
    } else {
        epoch as sys::time_t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // Real-symbol SDK stub, per the persist.rs house pattern: the host linker
    // uses this definition to satisfy the bindgen `extern` declaration, so the
    // tests drive the REAL local_time wrapper. Scripted through thread_locals
    // so the multithreaded harness cannot race.
    std::thread_local! {
        static RETURN_NULL: Cell<bool> = const { Cell::new(false) };
        static ASKED_FOR: Cell<i64> = const { Cell::new(0) };
    }

    std::thread_local! {
        static FAKE_TM: std::cell::UnsafeCell<sys::tm> = const { std::cell::UnsafeCell::new(unsafe { core::mem::zeroed() }) };
    }

    #[no_mangle]
    extern "C" fn localtime(timep: *mut sys::time_t) -> *mut sys::tm {
        // sys::time_t is c_long. On 64-bit hosts, this is i64 and the cast to
        // i64 is unnecessary, but harmless. On 32-bit hosts, the cast is needed.
        #[allow(clippy::unnecessary_cast)]
        ASKED_FOR.with(|c| c.set(unsafe { *timep } as i64));
        if RETURN_NULL.with(|c| c.get()) {
            return core::ptr::null_mut();
        }
        // A fixed civil time: 2026-08-11 14:32:05, a Tuesday.
        FAKE_TM.with(|fake| unsafe {
            let tm = fake.get();
            (*tm).tm_sec = 5;
            (*tm).tm_min = 32;
            (*tm).tm_hour = 14;
            (*tm).tm_mday = 11;
            (*tm).tm_mon = 7; // 0-based: August
            (*tm).tm_year = 126; // years since 1900
            (*tm).tm_wday = 2; // Tuesday
            tm
        })
    }

    #[test]
    fn converts_an_epoch_to_civil_fields() {
        RETURN_NULL.with(|c| c.set(false));
        let t = local_time(1_786_000_000).expect("converted");
        assert_eq!(t.hour, 14);
        assert_eq!(t.min, 32);
        assert_eq!(t.mday, 11);
        assert_eq!(t.mon, 7);
        assert_eq!(t.year, 126);
    }

    #[test]
    fn passes_the_requested_epoch_to_the_sdk() {
        RETURN_NULL.with(|c| c.set(false));
        ASKED_FOR.with(|c| c.set(0));
        let _ = local_time(1_700_000_000);
        assert_eq!(ASKED_FOR.with(|c| c.get()), 1_700_000_000);
    }

    /// The C callers that motivated this wrapper dereference localtime's
    /// result unchecked. The wrapper must not.
    #[test]
    fn a_null_result_is_none_not_a_crash() {
        RETURN_NULL.with(|c| c.set(true));
        assert!(local_time(1_700_000_000).is_none());
    }

    #[test]
    fn epoch_to_time_t_ordinary_values_pass_through() {
        assert_eq!(super::epoch_to_time_t(0), 0);
        assert_eq!(super::epoch_to_time_t(1_700_000_000), 1_700_000_000);
    }

    #[test]
    fn epoch_to_time_t_saturates_below_min() {
        assert_eq!(super::epoch_to_time_t(i64::MIN), sys::time_t::MIN);
        // On 32-bit targets, time_t::MIN is i32::MIN. On 64-bit, it's i64::MIN.
        // Test with a value we know is below the limit without overflow.
        let below_min = if sys::time_t::MIN == i64::MIN as sys::time_t {
            i64::MIN
        } else {
            // 32-bit target: use a value below i32::MIN
            -3_000_000_000i64
        };
        assert_eq!(super::epoch_to_time_t(below_min), sys::time_t::MIN);
    }

    #[test]
    fn epoch_to_time_t_saturates_above_max() {
        assert_eq!(super::epoch_to_time_t(i64::MAX), sys::time_t::MAX);
        // On 32-bit targets, time_t::MAX is i32::MAX. On 64-bit, it's i64::MAX.
        // Test with a value we know is above the limit without overflow.
        let above_max = if sys::time_t::MAX == i64::MAX as sys::time_t {
            i64::MAX
        } else {
            // 32-bit target: use a value above i32::MAX
            3_000_000_000i64
        };
        assert_eq!(super::epoch_to_time_t(above_max), sys::time_t::MAX);
    }
}

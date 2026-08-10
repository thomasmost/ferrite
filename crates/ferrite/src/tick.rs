//! Tick timer service. The SDK callback carries no context pointer, so the
//! closure lives in a private static slot — sound because the platform is
//! single-threaded; re-subscribing replaces the slot.
//!
//! **Static-slot discipline:** The trampoline takes the closure out of the
//! slot (preventing self-freeing during reentry), runs it borrow-free, and
//! restores it only if the slot is still empty. This is the same take/call/restore
//! pattern as the context-based services (see `click.rs`/`canvas.rs`/`menu_layer.rs`/
//! `app_message.rs`), but here the slot is a `static` and the "context" is implicit
//! in the static's identity. A closure that re-subscribes from inside itself cannot
//! free its own executing body.

use alloc::boxed::Box;
use core::cell::UnsafeCell;

use crate::sys;
use crate::App;

pub use crate::sys::TimeUnits;

/// Wall-clock fields copied out of the SDK's `struct tm`.
#[derive(Clone, Copy, Debug)]
pub struct Time {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    /// Day of month, 1-31.
    pub mday: i32,
    /// Month, 0-11.
    pub mon: i32,
    /// Years since 1900.
    pub year: i32,
    /// Day of week, 0 = Sunday.
    pub wday: i32,
}

impl Time {
    pub(crate) fn from_tm(tm: &sys::tm) -> Time {
        Time {
            sec: tm.tm_sec,
            min: tm.tm_min,
            hour: tm.tm_hour,
            mday: tm.tm_mday,
            mon: tm.tm_mon,
            year: tm.tm_year,
            wday: tm.tm_wday,
        }
    }
}

type TickClosure = Box<dyn FnMut(&Time, TimeUnits)>;

struct TickSlot(UnsafeCell<Option<TickClosure>>);

// SAFETY: the watch runtime is single-threaded; this static is only touched
// from the app task. (Required because statics must be Sync.)
unsafe impl Sync for TickSlot {}

static TICK_SLOT: TickSlot = TickSlot(UnsafeCell::new(None));

/// Subscribes to tick events. A second call replaces the previous closure.
pub fn subscribe(_app: &mut App, units: TimeUnits, f: impl FnMut(&Time, TimeUnits) + 'static) {
    unsafe {
        *TICK_SLOT.0.get() = Some(Box::new(f));
        sys::tick_timer_service_subscribe(units, Some(on_tick));
    }
}

/// Unsubscribes and drops the stored closure.
pub fn unsubscribe() {
    unsafe {
        sys::tick_timer_service_unsubscribe();
        *TICK_SLOT.0.get() = None;
    }
}

unsafe extern "C" fn on_tick(tick_time: *mut sys::tm, units_changed: sys::TimeUnits) {
    // Take the closure out while it runs so a re-subscribe from inside the
    // closure cannot free the executing body; restore it only if the slot
    // is still empty afterwards.
    let taken = (*TICK_SLOT.0.get()).take();
    if let Some(mut f) = taken {
        let time = Time::from_tm(&*tick_time);
        f(&time, units_changed);
        let slot = &mut *TICK_SLOT.0.get();
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_from_tm_copies_fields() {
        let tm = crate::sys::tm {
            tm_sec: 7,
            tm_min: 30,
            tm_hour: 13,
            tm_mday: 5,
            tm_mon: 7,
            tm_year: 126,
            tm_wday: 3,
            tm_yday: 216,
            tm_isdst: 0,
            ..Default::default()
        };
        let t = Time::from_tm(&tm);
        assert_eq!(t.sec, 7);
        assert_eq!(t.min, 30);
        assert_eq!(t.hour, 13);
        assert_eq!(t.mday, 5);
        assert_eq!(t.mon, 7);
        assert_eq!(t.year, 126);
        assert_eq!(t.wday, 3);
    }

    /// Closure survives two tick dispatches: restore must keep the closure.
    #[test]
    fn tick_closure_survives_two_dispatches() {
        let call_count = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let c = call_count.clone();
        unsafe {
            *TICK_SLOT.0.get() = Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                c.set(c.get() + 1);
            }));
        }

        // First dispatch
        let tm = crate::sys::tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 1,
            tm_mon: 0,
            tm_year: 100,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            ..Default::default()
        };
        unsafe {
            on_tick(&mut tm.clone() as *mut _, TimeUnits::SECOND_UNIT);
        }
        assert_eq!(call_count.get(), 1);

        // Second dispatch: restore must have kept the closure
        unsafe {
            on_tick(&mut tm.clone() as *mut _, TimeUnits::SECOND_UNIT);
        }
        assert_eq!(call_count.get(), 2, "restore must keep the closure");

        // Clean up
        unsafe {
            *TICK_SLOT.0.get() = None;
        }
    }

    /// THE invariant: a closure that re-subscribes from inside itself must WIN --
    /// restore must not clobber it with the finished closure.
    #[test]
    fn reentrant_tick_replacement_wins() {
        let first = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let second = alloc::rc::Rc::new(core::cell::Cell::new(0));

        let f1 = first.clone();
        let s2 = second.clone();
        unsafe {
            *TICK_SLOT.0.get() = Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                f1.set(f1.get() + 1);
                // Reentrant replacement: our slot is empty (taken) right now, so
                // this lands in the slot and restore must NOT overwrite it.
                let s2 = s2.clone();
                unsafe {
                    *TICK_SLOT.0.get() = Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                        s2.set(s2.get() + 1);
                    }));
                }
            }));
        }

        let tm = crate::sys::tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 1,
            tm_mon: 0,
            tm_year: 100,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            ..Default::default()
        };

        unsafe {
            on_tick(&mut tm.clone() as *mut _, TimeUnits::SECOND_UNIT);
            on_tick(&mut tm.clone() as *mut _, TimeUnits::SECOND_UNIT);
        }
        assert_eq!(
            (first.get(), second.get()),
            (1, 1),
            "the reentrantly-registered closure must win over the restore"
        );

        // Clean up
        unsafe {
            *TICK_SLOT.0.get() = None;
        }
    }
}

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
//!
//! **Static ownership:** The TICK_SLOT is a never-dropped `static`. The closure and
//! all its captures live in this slot until `unsubscribe()` is called or the app
//! terminates. Care must be taken when capturing data that must be dropped at a
//! specific time.

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

// Flag set by unsubscribe() to signal that the slot should NOT be restored
// after the current dispatch (e.g., if unsubscribe was called from inside the
// closure). Cleared after dispatch or when subscribe() is called.
struct PendingUnsubscribeFlag(core::cell::Cell<bool>);
unsafe impl Sync for PendingUnsubscribeFlag {}
static PENDING_UNSUBSCRIBE: PendingUnsubscribeFlag =
    PendingUnsubscribeFlag(core::cell::Cell::new(false));

/// Subscribes to tick events. A second call replaces the previous closure.
pub fn subscribe(_app: &mut App, units: TimeUnits, f: impl FnMut(&Time, TimeUnits) + 'static) {
    unsafe {
        *TICK_SLOT.0.get() = Some(Box::new(f));
        PENDING_UNSUBSCRIBE.0.set(false);
        sys::tick_timer_service_subscribe(units, Some(on_tick));
    }
}

/// Unsubscribes and drops the stored closure.
pub fn unsubscribe() {
    unsafe {
        sys::tick_timer_service_unsubscribe();
        PENDING_UNSUBSCRIBE.0.set(true);
        *TICK_SLOT.0.get() = None;
    }
}

/// Dispatch logic shared by the trampoline and the host tests.
///
/// `slot` is a RAW pointer on purpose: every access below is statement-scoped,
/// so NO borrow of the slot is live while the user closure runs. A `&mut
/// Option<..>` parameter would be held across `f(...)` for the whole call --
/// and a reentrant `subscribe()` writing through `TICK_SLOT.0.get()` would
/// then alias an outstanding `&mut` (the Miri-proven-UB shape this codebase's
/// trampolines are required to avoid). `pending` is a `&Cell` -- shared
/// interior mutability, safe to write through from inside the closure.
///
/// 1. Take the closure out (a reentrant re-subscribe cannot free the
///    executing body).
/// 2. Run it borrow-free.
/// 3. If `pending` was set (unsubscribe from inside the closure), DROP the
///    closure instead of restoring it, and clear the flag.
/// 4. Otherwise restore only if the slot is still empty, so a reentrant
///    re-subscribe wins.
unsafe fn dispatch(
    slot: *mut Option<TickClosure>,
    time: &Time,
    units: TimeUnits,
    pending: &core::cell::Cell<bool>,
) {
    let taken = (*slot).take();
    if let Some(mut f) = taken {
        f(time, units);
        if pending.get() {
            // unsubscribe() ran inside the closure: honour it -- f drops here.
            pending.set(false);
        } else {
            let s = &mut *slot;
            if s.is_none() {
                *s = Some(f);
            }
        }
    }
}

unsafe extern "C" fn on_tick(tick_time: *mut sys::tm, units_changed: sys::TimeUnits) {
    let time = Time::from_tm(&*tick_time);
    dispatch(
        TICK_SLOT.0.get(),
        &time,
        units_changed,
        &PENDING_UNSUBSCRIBE.0,
    );
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

    /// Closure survives two tick dispatches with local slot (no static-based race).
    #[test]
    fn tick_closure_survives_two_dispatches() {
        let call_count = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let c = call_count.clone();

        let mut local_slot: Option<TickClosure> =
            Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                c.set(c.get() + 1);
            }));

        let flag = core::cell::Cell::new(false);
        let time = Time {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 1,
            mon: 0,
            year: 100,
            wday: 0,
        };

        // First dispatch
        unsafe {
            dispatch(&mut local_slot, &time, TimeUnits::SECOND_UNIT, &flag);
        }
        assert_eq!(call_count.get(), 1);
        assert!(
            local_slot.is_some(),
            "closure should be restored after dispatch"
        );

        // Second dispatch: restore must have kept the closure
        unsafe {
            dispatch(&mut local_slot, &time, TimeUnits::SECOND_UNIT, &flag);
        }
        assert_eq!(call_count.get(), 2, "restore must keep the closure");
    }

    /// THE invariant: a closure that re-subscribes from inside itself must WIN --
    /// restore must not clobber it with the finished closure.
    #[test]
    fn reentrant_tick_replacement_wins() {
        let first = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let second = alloc::rc::Rc::new(core::cell::Cell::new(0));

        let f1 = first.clone();
        let mut local_slot: Option<TickClosure> =
            Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                f1.set(f1.get() + 1);
            }));

        let flag = core::cell::Cell::new(false);
        let time = Time {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 1,
            mon: 0,
            year: 100,
            wday: 0,
        };

        // First dispatch with first closure
        unsafe {
            dispatch(&mut local_slot, &time, TimeUnits::SECOND_UNIT, &flag);
        }
        assert_eq!(first.get(), 1);

        // Simulate resubscribe from inside: set new slot content
        let s2_clone = second.clone();
        local_slot = Some(Box::new(move |_t: &Time, _u: TimeUnits| {
            s2_clone.set(s2_clone.get() + 1);
        }) as TickClosure);

        // Second dispatch with new closure
        unsafe {
            dispatch(&mut local_slot, &time, TimeUnits::SECOND_UNIT, &flag);
        }
        assert_eq!(second.get(), 1);
    }

    /// Unsubscribe from inside closure must drop the closure and not restore it.
    #[test]
    fn unsubscribe_from_inside_drops_closure() {
        use core::cell::Cell;

        let drop_count = alloc::rc::Rc::new(Cell::new(0));

        struct DropGuard(alloc::rc::Rc<Cell<i32>>);
        impl Drop for DropGuard {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        let guard = DropGuard(drop_count.clone());
        let mut local_slot: Option<TickClosure> =
            Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                // The guard is captured here and will be dropped
                let _ = &guard;
            }));

        let flag = core::cell::Cell::new(false);
        let time = Time {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 1,
            mon: 0,
            year: 100,
            wday: 0,
        };

        // First dispatch (flag is false)
        unsafe {
            dispatch(&mut local_slot, &time, TimeUnits::SECOND_UNIT, &flag);
        }

        // Check: closure should be restored (since flag was not set)
        assert!(
            local_slot.is_some(),
            "closure should be restored when flag is not set"
        );
        assert_eq!(
            drop_count.get(),
            0,
            "closure not dropped yet since it's still in slot"
        );

        // Now simulate what happens when unsubscribe is called from inside:
        // set the flag and dispatch again
        flag.set(true);
        unsafe {
            dispatch(&mut local_slot, &time, TimeUnits::SECOND_UNIT, &flag);
        }

        // After dispatch with flag set, slot should be empty (closure dropped)
        assert!(
            local_slot.is_none(),
            "closure must be dropped when flag is set"
        );
        assert_eq!(drop_count.get(), 1, "guard should have been dropped");
        assert!(
            !flag.get(),
            "dispatch must clear the flag after honouring it"
        );
    }

    /// The true I1 regression shape: the flag is set FROM INSIDE the closure
    /// (as `unsubscribe()` does on-device), through the same `&Cell` dispatch
    /// holds -- the closure must be dropped, not restored, and a later
    /// resubscribe must not be killed by a stale flag.
    #[test]
    fn unsubscribe_called_inside_the_closure_is_honoured() {
        use core::cell::Cell;

        let drops = alloc::rc::Rc::new(Cell::new(0));
        struct DropGuard(alloc::rc::Rc<Cell<i32>>);
        impl Drop for DropGuard {
            fn drop(&mut self) {
                self.0.set(self.0.get() + 1);
            }
        }

        // Shared flag via Rc: the closure (which must be 'static) holds one
        // clone, the test body passes `&*flag` to dispatch -- mirroring the
        // static PENDING_UNSUBSCRIBE on-device, without leaking.
        let flag = alloc::rc::Rc::new(Cell::new(false));
        let flag_inner = flag.clone();

        let guard = DropGuard(drops.clone());
        let mut local_slot: Option<TickClosure> =
            Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                let _ = &guard;
                flag_inner.set(true); // what unsubscribe() does, mid-dispatch
            }));

        let time = Time {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 1,
            mon: 0,
            year: 100,
            wday: 0,
        };
        unsafe { dispatch(&mut local_slot, &time, TimeUnits::SECOND_UNIT, &flag) };

        assert!(local_slot.is_none(), "in-closure unsubscribe must drop");
        assert_eq!(drops.get(), 1, "captures must be released");
        assert!(!flag.get(), "flag cleared so a later resubscribe works");
    }

    /// A closure that re-subscribes WHILE IT RUNS (writing through a raw
    /// pointer to the same slot dispatch is using -- the on-device shape of
    /// `subscribe()` from inside a tick closure) must win over the restore.
    /// This also pins that dispatch holds no borrow across the closure call:
    /// under a held `&mut Option<..>` this pattern would be aliasing UB.
    #[test]
    fn mid_dispatch_resubscribe_wins() {
        use core::cell::Cell;

        let first = alloc::rc::Rc::new(Cell::new(0));
        let second = alloc::rc::Rc::new(Cell::new(0));

        let mut local_slot: Option<TickClosure> = None;
        let slot_ptr: *mut Option<TickClosure> = &raw mut local_slot;

        let f1 = first.clone();
        let s2 = second.clone();
        unsafe {
            *slot_ptr = Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                f1.set(f1.get() + 1);
                let s2 = s2.clone();
                // Reentrant resubscribe mid-dispatch: our slot is empty
                // (taken) right now, so this lands in it and the restore
                // must NOT overwrite it. (No inner `unsafe` block -- closure
                // bodies inherit the enclosing one's context.)
                *slot_ptr = Some(Box::new(move |_t: &Time, _u: TimeUnits| {
                    s2.set(s2.get() + 1)
                }));
            }));
        }

        let flag = Cell::new(false);
        let time = Time {
            sec: 0,
            min: 0,
            hour: 0,
            mday: 1,
            mon: 0,
            year: 100,
            wday: 0,
        };
        unsafe { dispatch(slot_ptr, &time, TimeUnits::SECOND_UNIT, &flag) };
        unsafe { dispatch(slot_ptr, &time, TimeUnits::SECOND_UNIT, &flag) };
        assert_eq!(
            (first.get(), second.get()),
            (1, 1),
            "the mid-dispatch resubscribe must win over the restore"
        );
    }
}

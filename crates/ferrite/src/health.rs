//! HealthService: heart rate and activity metrics, with graceful absence —
//! on watches without a sensor, peeks return 0 and accessibility reports
//! not-supported; nothing panics.
//!
//! **Trampoline discipline:** The health event trampoline holds `&mut HealthState`
//! and must never hold a borrow of it across a user-closure call, as the closure
//! can reenter the safe API. We use the take/call/restore pattern from
//! `click.rs`/`canvas.rs`/`menu_layer.rs`/`app_message.rs`: the closure is taken
//! out of its slot (statement-scoped borrow), run borrow-free, and restored only
//! if empty (so a reentrant re-subscription wins). See `app_message.rs`'s module
//! doc for the full contract.
//!
//! **Residual contract:** Dropping a `Health` subscription from inside its own
//! callback is not supported — the state box would be freed under the executing
//! closure. `Drop` debug-asserts that `callback_depth` is zero, making this a
//! deterministic panic in debug builds.

use alloc::boxed::Box;
use core::ffi::c_void;

use crate::sys;
use crate::App;

pub use crate::sys::{HealthEventType, HealthMetric, HealthServiceAccessibilityMask};

type HealthEventCallback = Box<dyn FnMut(HealthEventType)>;

struct HealthState {
    on_event: Option<HealthEventCallback>,
    /// Number of this service's callbacks currently on the stack. Structural
    /// backstop for the documented-unsupported case (dropping a `Health`
    /// from inside its own callback): `Drop` debug-asserts this is zero, so
    /// in debug builds the use-after-free becomes a deterministic panic.
    callback_depth: u8,
}

/// Active health-event subscription; unsubscribes on drop.
pub struct Health {
    state: *mut HealthState, // Box, owned; freed in Drop
}

impl Health {
    /// Subscribes to health events. Returns `None` if the SDK refuses
    /// (out of memory or health unsupported).
    pub fn subscribe(
        _app: &mut App,
        f: impl FnMut(HealthEventType) + 'static,
    ) -> Option<Health> {
        let state = Box::into_raw(Box::new(HealthState {
            on_event: Some(Box::new(f)),
            callback_depth: 0,
        }));
        let ok = unsafe { sys::health_service_events_subscribe(Some(on_health_event), state.cast()) };
        if ok {
            Some(Health { state })
        } else {
            unsafe { drop(Box::from_raw(state)) };
            None
        }
    }

    /// Requests a heart-rate sample period (seconds); 0 restores the default.
    pub fn set_heart_rate_sample_period(&mut self, interval_secs: u16) -> bool {
        unsafe { sys::health_service_set_heart_rate_sample_period(interval_secs) }
    }
}

impl Drop for Health {
    fn drop(&mut self) {
        // Backstop for the documented-unsupported case: dropping a Health
        // from inside its own callback would free the state box under
        // the executing closure.
        debug_assert!(
            unsafe { (*self.state).callback_depth } == 0,
            "Health dropped from inside its own callback (unsupported; see health.rs)"
        );
        unsafe {
            sys::health_service_events_unsubscribe();
            drop(Box::from_raw(self.state));
        }
    }
}

/// Current value of a metric (0 when unavailable).
pub fn peek(metric: HealthMetric) -> i32 {
    unsafe { sys::health_service_peek_current_value(metric) }
}

/// Whether `metric` is available right now.
pub fn metric_available(metric: HealthMetric) -> bool {
    let now = unsafe { sys::time(core::ptr::null_mut()) };
    let mask = unsafe { sys::health_service_metric_accessible(metric, now, now) };
    // Accessibility is a bitmask — test the Available bit, don't compare
    // for equality (the SDK may OR in other reasons).
    mask.0 & sys::HealthServiceAccessibilityMask::HealthServiceAccessibilityMaskAvailable.0 != 0
}

// --- Trampoline (context = *mut HealthState) ---
//
// Same take/call/restore discipline as app_message.rs: no borrow of
// HealthState is live while the user closure runs, so the closure may safely
// reenter the safe API without aliasing UB, and a replaced closure cannot be
// freed out from under itself.

unsafe extern "C" fn on_health_event(event: sys::HealthEventType, context: *mut c_void) {
    let state = context as *mut HealthState;
    let taken = (*state).on_event.take();
    if let Some(mut f) = taken {
        (*state).callback_depth += 1;
        f(event);
        (*state).callback_depth -= 1;
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).on_event;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closure survives two health event dispatches: restore must keep the closure.
    #[test]
    fn on_health_event_closure_survives_two_dispatches() {
        let call_count = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let state = Box::into_raw(Box::new(HealthState {
            on_event: None,
            callback_depth: 0,
        }));

        let c = call_count.clone();
        unsafe { &mut *state }.on_event = Some(Box::new(move |_event: HealthEventType| {
            c.set(c.get() + 1);
        }));

        // First dispatch
        unsafe {
            on_health_event(
                sys::HealthEventType::HealthEventSignificantUpdate,
                state as *mut c_void,
            );
        }
        assert_eq!(call_count.get(), 1);

        // Second dispatch: restore must have kept the closure
        unsafe {
            on_health_event(
                sys::HealthEventType::HealthEventSignificantUpdate,
                state as *mut c_void,
            );
        }
        assert_eq!(call_count.get(), 2, "restore must keep the closure");

        unsafe { drop(Box::from_raw(state)) };
    }

    /// THE invariant: a closure that re-subscribes from inside itself must WIN --
    /// restore must not clobber it with the finished closure.
    #[test]
    fn reentrant_on_health_event_replacement_wins() {
        let first = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let second = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let state = Box::into_raw(Box::new(HealthState {
            on_event: None,
            callback_depth: 0,
        }));

        let f1 = first.clone();
        let s2 = second.clone();
        unsafe { &mut *state }.on_event = Some(Box::new(move |_event: HealthEventType| {
            f1.set(f1.get() + 1);
            // Reentrant replacement: our slot is empty (taken) right now, so
            // this lands in the slot and restore must NOT overwrite it.
            let s2 = s2.clone();
            unsafe { &mut *state }.on_event = Some(Box::new(move |_event: HealthEventType| {
                s2.set(s2.get() + 1);
            }));
        }));

        unsafe {
            on_health_event(
                sys::HealthEventType::HealthEventSignificantUpdate,
                state as *mut c_void,
            );
            on_health_event(
                sys::HealthEventType::HealthEventSignificantUpdate,
                state as *mut c_void,
            );
        }
        assert_eq!(
            (first.get(), second.get()),
            (1, 1),
            "the reentrantly-registered closure must win over the restore"
        );

        unsafe { drop(Box::from_raw(state)) };
    }
}

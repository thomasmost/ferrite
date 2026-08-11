//! Wall-clock time from the SDK.
//!
//! Exists so app code never has to write `unsafe { sys::time(null_mut()) }`.
//! Pebble apps that inject time into testable logic (rather than calling the
//! clock deep inside it) call this once per event and pass the result down.

use crate::sys;

/// Seconds since the Unix epoch, as the `u32` that app logic normally wants.
///
/// Saturates rather than wrapping: a negative clock (pre-1970, which the
/// firmware should never report) reads as 0, and a post-2106 clock pins at
/// `u32::MAX`. Both are preferable to a wrapped value that looks plausible.
pub fn now() -> u32 {
    epoch_to_u32(unsafe { sys::time(core::ptr::null_mut()) })
}

/// Split out from `now` so the narrowing is unit-testable. The `time` symbol
/// itself cannot be replaced with a `#[no_mangle]` stub on the host — it would
/// collide with libc's definition — so the house real-symbol stub pattern does
/// not apply here.
fn epoch_to_u32(t: core::ffi::c_long) -> u32 {
    if t <= 0 {
        0
    } else if (t as u64) >= u32::MAX as u64 {
        u32::MAX
    } else {
        t as u32
    }
}

#[cfg(test)]
mod tests {
    use super::epoch_to_u32;

    #[test]
    fn ordinary_epoch_seconds_pass_through() {
        assert_eq!(epoch_to_u32(1_700_000_000), 1_700_000_000);
        assert_eq!(epoch_to_u32(1), 1);
    }

    #[test]
    fn zero_and_negative_clock_read_as_zero() {
        assert_eq!(epoch_to_u32(0), 0);
        assert_eq!(epoch_to_u32(-1), 0);
        assert_eq!(epoch_to_u32(core::ffi::c_long::MIN), 0);
    }

    /// Saturating, not wrapping. A wrapped value would look like a plausible
    /// recent timestamp and silently corrupt every elapsed-time calculation
    /// downstream; a pinned maximum is at least obviously wrong.
    #[test]
    fn beyond_u32_saturates_instead_of_wrapping() {
        assert_eq!(epoch_to_u32(u32::MAX as core::ffi::c_long), u32::MAX);
        assert_eq!(epoch_to_u32(u32::MAX as core::ffi::c_long + 1), u32::MAX);
        assert_eq!(epoch_to_u32(core::ffi::c_long::MAX), u32::MAX);
    }
}

//! Inbound AppMessage: inbox open/received/dropped with context-based
//! closures, and safe dictionary reading over `dict_find`.
//!
//! **Trampoline discipline:** The inbox received/dropped trampolines hold
//! `&mut AppMessageState` and must never hold a borrow of it across a
//! user-closure call, as the closure can reenter the safe API. We use the
//! take/call/restore pattern from `click.rs`/`canvas.rs`/`menu_layer.rs`:
//! the closure is taken out of its slot (statement-scoped borrow), run
//! borrow-free, and restored only if empty (so a reentrant re-registration
//! wins). See `click.rs`'s module doc for the full contract.
//!
//! **Residual contract:** Dropping an `AppMessage` from inside one of its own
//! callbacks (on_received, on_dropped) is not supported — the state box would
//! be freed under the executing closure. `Drop` debug-asserts that
//! `callback_depth` is zero, making this a deterministic panic in debug
//! builds.

use alloc::boxed::Box;
use core::ffi::{c_void, CStr};
use core::marker::PhantomData;

use crate::sys;
use crate::App;

pub use crate::sys::TupleType;

/// Little-endian unsigned decode for AppMessage integer tuples (1/2/4 bytes).
fn decode_uint(bytes: &[u8]) -> Option<u32> {
    match bytes.len() {
        1 => Some(bytes[0] as u32),
        2 => Some(u16::from_le_bytes([bytes[0], bytes[1]]) as u32),
        4 => Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ => None,
    }
}

/// Little-endian signed decode for AppMessage integer tuples (1/2/4 bytes).
fn decode_int(bytes: &[u8]) -> Option<i32> {
    match bytes.len() {
        1 => Some(bytes[0] as i8 as i32),
        2 => Some(i16::from_le_bytes([bytes[0], bytes[1]]) as i32),
        4 => Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ => None,
    }
}

/// A received dictionary, valid only inside the receive closure.
pub struct Dict<'a> {
    iter: *mut sys::DictionaryIterator,
    _lifetime: PhantomData<&'a ()>,
}

impl Dict<'_> {
    pub fn find(&self, key: u32) -> Option<Tuple<'_>> {
        let raw = unsafe { sys::dict_find(self.iter as *const _, key) };
        if raw.is_null() {
            None
        } else {
            Some(Tuple {
                raw,
                _lifetime: PhantomData,
            })
        }
    }
}

/// One key/value entry of a received dictionary.
pub struct Tuple<'a> {
    raw: *mut sys::Tuple,
    _lifetime: PhantomData<&'a ()>,
}

impl Tuple<'_> {
    pub fn key(&self) -> u32 {
        unsafe { (*self.raw).key }
    }

    /// The tuple's wire type (int/uint/bytes/cstring).
    pub fn tuple_type(&self) -> TupleType {
        // `type` is a C bitfield; bindgen generates an accessor (named after
        // the renamed field, e.g. `type_()`).
        unsafe { (*self.raw).type_() }
    }

    /// Length in bytes of the tuple's value.
    pub fn length(&self) -> u16 {
        unsafe { (*self.raw).length }
    }

    fn value_bytes_raw(&self) -> &[u8] {
        unsafe {
            // Tuple is packed; compute the byte offset to the value field
            // and use raw pointer arithmetic to slice it.
            let len = (*self.raw).length as usize;
            // Get offset to the value field within the packed struct
            let base_ptr = self.raw as *const u8;
            let value_field_offset =
                core::mem::offset_of!(sys::Tuple, value);
            let value_ptr = base_ptr.add(value_field_offset);
            core::slice::from_raw_parts(value_ptr, len)
        }
    }

    /// Integer value (signed or unsigned tuple types), sign-extended to i32.
    pub fn value_i32(&self) -> Option<i32> {
        match self.tuple_type() {
            sys::TupleType::TUPLE_INT => decode_int(self.value_bytes_raw()),
            sys::TupleType::TUPLE_UINT => {
                decode_uint(self.value_bytes_raw()).map(|v| v as i32)
            }
            _ => None,
        }
    }

    /// Raw bytes of a byte-array tuple.
    pub fn value_bytes(&self) -> Option<&[u8]> {
        (self.tuple_type() == sys::TupleType::TUPLE_BYTE_ARRAY)
            .then(|| self.value_bytes_raw())
    }

    /// C-string value of a cstring tuple.
    pub fn value_cstr(&self) -> Option<&CStr> {
        if self.tuple_type() != sys::TupleType::TUPLE_CSTRING {
            return None;
        }
        CStr::from_bytes_until_nul(self.value_bytes_raw()).ok()
    }
}

struct AppMessageState {
    on_received: Option<Box<dyn FnMut(&Dict<'_>)>>,
    on_dropped: Option<Box<dyn FnMut(sys::AppMessageResult)>>,
    /// Number of this app's callbacks currently on the stack. Structural
    /// backstop for the documented-unsupported case (dropping an `AppMessage`
    /// from inside its own callback): `Drop` debug-asserts this is zero, so
    /// in debug builds the use-after-free becomes a deterministic panic.
    callback_depth: u8,
}

/// The AppMessage service (inbound). At most one instance should exist.
pub struct AppMessage {
    state: *mut AppMessageState, // Box, owned; freed in Drop
}

impl AppMessage {
    /// Registers callbacks and opens the inbox/outbox with the given sizes.
    pub fn open(
        _app: &mut App,
        inbox_size: u32,
        outbox_size: u32,
    ) -> Result<AppMessage, sys::AppMessageResult> {
        let state = Box::into_raw(Box::new(AppMessageState {
            on_received: None,
            on_dropped: None,
            callback_depth: 0,
        }));
        unsafe {
            sys::app_message_set_context(state.cast());
            sys::app_message_register_inbox_received(Some(on_inbox_received));
            sys::app_message_register_inbox_dropped(Some(on_inbox_dropped));
            let r = sys::app_message_open(inbox_size, outbox_size);
            if r != sys::AppMessageResult::APP_MSG_OK {
                sys::app_message_deregister_callbacks();
                sys::app_message_set_context(core::ptr::null_mut());
                drop(Box::from_raw(state));
                return Err(r);
            }
        }
        Ok(AppMessage { state })
    }

    pub fn on_received(&mut self, f: impl FnMut(&Dict<'_>) + 'static) {
        unsafe { &mut *self.state }.on_received = Some(Box::new(f));
    }

    pub fn on_dropped(&mut self, f: impl FnMut(sys::AppMessageResult) + 'static) {
        unsafe { &mut *self.state }.on_dropped = Some(Box::new(f));
    }
}

impl Drop for AppMessage {
    fn drop(&mut self) {
        // Backstop for the documented-unsupported case: dropping an AppMessage
        // from inside one of its own callbacks would free the state box under
        // the executing closure.
        debug_assert!(
            unsafe { (*self.state).callback_depth } == 0,
            "AppMessage dropped from inside its own callback (unsupported; see app_message.rs)"
        );
        unsafe {
            sys::app_message_deregister_callbacks();
            sys::app_message_set_context(core::ptr::null_mut());
            drop(Box::from_raw(self.state));
        }
    }
}

// --- Trampolines (context = *mut AppMessageState) ---
//
// Same take/call/restore discipline as the click trampolines (click.rs
// module doc): no borrow of AppMessageState is live while the user closure
// runs, so the closure may safely reenter the safe API without aliasing UB,
// and a replaced closure cannot be freed out from under itself.

unsafe extern "C" fn on_inbox_received(
    iterator: *mut sys::DictionaryIterator,
    context: *mut c_void,
) {
    let state = context as *mut AppMessageState;
    let taken = (*state).on_received.take();
    if let Some(mut f) = taken {
        let dict = Dict {
            iter: iterator,
            _lifetime: PhantomData,
        };
        (*state).callback_depth += 1;
        f(&dict);
        (*state).callback_depth -= 1;
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).on_received;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

unsafe extern "C" fn on_inbox_dropped(reason: sys::AppMessageResult, context: *mut c_void) {
    let state = context as *mut AppMessageState;
    let taken = (*state).on_dropped.take();
    if let Some(mut f) = taken {
        (*state).callback_depth += 1;
        f(reason);
        (*state).callback_depth -= 1;
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).on_dropped;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_uint_widths() {
        assert_eq!(decode_uint(&[0x2a]), Some(42));
        assert_eq!(decode_uint(&[0x34, 0x12]), Some(0x1234));
        assert_eq!(decode_uint(&[0x78, 0x56, 0x34, 0x12]), Some(0x1234_5678));
        assert_eq!(decode_uint(&[1, 2, 3]), None); // no 3-byte ints
        assert_eq!(decode_uint(&[]), None);
    }

    #[test]
    fn decode_int_sign_extends() {
        assert_eq!(decode_int(&[0xff]), Some(-1));
        assert_eq!(decode_int(&[0xfe, 0xff]), Some(-2));
        assert_eq!(decode_int(&[0xfd, 0xff, 0xff, 0xff]), Some(-3));
        assert_eq!(decode_int(&[0x2a]), Some(42));
    }

    /// Closure survives two dispatches: restore must keep the closure.
    #[test]
    fn on_received_closure_survives_two_dispatches() {
        let call_count = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let state = Box::into_raw(Box::new(AppMessageState {
            on_received: None,
            on_dropped: None,
            callback_depth: 0,
        }));

        let c = call_count.clone();
        unsafe { &mut *state }.on_received =
            Some(Box::new(move |_dict: &Dict| c.set(c.get() + 1)));

        // First dispatch
        unsafe {
            on_inbox_received(core::ptr::null_mut(), state as *mut c_void);
        }
        assert_eq!(call_count.get(), 1);

        // Second dispatch: restore must have kept the closure
        unsafe {
            on_inbox_received(core::ptr::null_mut(), state as *mut c_void);
        }
        assert_eq!(call_count.get(), 2, "restore must keep the closure");

        unsafe { drop(Box::from_raw(state)) };
    }

    /// THE invariant: a closure that re-registers a replacement during its
    /// own dispatch must WIN -- restore must not clobber it with the finished
    /// closure.
    #[test]
    fn reentrant_on_received_replacement_wins() {
        let first = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let second = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let state = Box::into_raw(Box::new(AppMessageState {
            on_received: None,
            on_dropped: None,
            callback_depth: 0,
        }));

        let f1 = first.clone();
        let s2 = second.clone();
        unsafe { &mut *state }.on_received = Some(Box::new(move |_dict: &Dict| {
            f1.set(f1.get() + 1);
            // Reentrant replacement: our slot is empty (taken) right now, so
            // this lands in the slot and restore must NOT overwrite it.
            let s2 = s2.clone();
            unsafe { &mut *state }.on_received = Some(Box::new(move |_dict: &Dict| {
                s2.set(s2.get() + 1);
            }));
        }));

        unsafe {
            on_inbox_received(core::ptr::null_mut(), state as *mut c_void);
            on_inbox_received(core::ptr::null_mut(), state as *mut c_void);
        }
        assert_eq!(
            (first.get(), second.get()),
            (1, 1),
            "the reentrantly-registered closure must win over the restore"
        );

        unsafe { drop(Box::from_raw(state)) };
    }
}

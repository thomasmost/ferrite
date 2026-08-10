//! Custom-drawing layer over `layer_create_with_data`.
//!
//! The SDK's update proc carries no context pointer, but a data layer gives
//! us per-layer storage: we keep a single pointer to the boxed closure state
//! there, so no globals are needed.
//!
//! **Trampoline discipline:** The update proc must never hold a borrow of
//! `CanvasState` across a user-closure call, as the closure can reenter the
//! safe API (e.g. `mark_dirty()` which calls `layer_mark_dirty`). We use the
//! take/call/restore pattern from `click.rs`/`window.rs`: the closure is taken
//! out of its slot (statement-scoped borrow), run borrow-free, and restored
//! only if empty (so a reentrant registration wins). See `click.rs`'s module
//! doc for the full contract.
//!
//! **Residual contract:** Dropping a `CanvasLayer` from inside its own
//! `on_draw` closure is not supported — the state box would be freed under the
//! executing closure. `Drop` debug-asserts that `callback_depth` is zero,
//! making this a deterministic panic in debug builds.
//!
//! **RFC-2229 hazard:** When destructuring in closures (e.g. with `move`),
//! capture whole variables, not disjoint fields. Example: `let screens =
//! (text_window, canvas_window); closure.on_select(move |row|
//! screens.0.push(...))` captures only the two windows, NOT the layers (text,
//! canvas) inside them. The sibling layers drop early and destroy their SDK
//! resources while the window is still visible, causing use-after-free. Fix:
//! destructure outside the closure (`let (text_win, text) = ...; let
//! (canvas_win, canvas) = ...; closure.on_select(move |row| text_win.push(
//! ...))`) and move the windows into the closure. Return children before
//! parents in the app state tuple to ensure drop order.

use alloc::boxed::Box;
use core::mem::size_of;

use crate::graphics::Graphics;
use crate::layer::AsLayer;
use crate::sys;
use crate::App;

type DrawCallback = Box<dyn FnMut(&mut Graphics<'_>)>;

struct CanvasState {
    on_draw: Option<DrawCallback>,
    /// Number of this layer's callbacks currently on the stack. Structural
    /// backstop for the documented-unsupported case (dropping a `CanvasLayer`
    /// from inside its own callback): `Drop` debug-asserts this is zero, so
    /// in debug builds the use-after-free becomes a deterministic panic.
    callback_depth: u8,
}

pub struct CanvasLayer {
    raw: *mut sys::Layer,
    state: *mut CanvasState, // Box, owned; freed in Drop
}

impl CanvasLayer {
    /// Creates a canvas layer with the given frame. Panics if the SDK
    /// returns NULL (out of memory).
    pub fn new(_app: &mut App, frame: sys::GRect) -> CanvasLayer {
        let raw = unsafe { sys::layer_create_with_data(frame, size_of::<*mut CanvasState>()) };
        assert!(!raw.is_null(), "layer_create_with_data returned NULL");
        let state = Box::into_raw(Box::new(CanvasState {
            on_draw: None,
            callback_depth: 0,
        }));
        unsafe {
            let data_ptr = sys::layer_get_data(raw) as *mut *mut CanvasState;
            // The SDK allocates Layer+data from malloc, which guarantees
            // pointer alignment. ARMv7-M (Pebble's target) tolerates unaligned
            // word access anyway, but we assert for safety in debug builds.
            debug_assert_eq!(
                (data_ptr as usize) % core::mem::align_of::<*mut CanvasState>(),
                0,
                "layer data pointer not aligned for state storage"
            );
            *data_ptr = state;
            sys::layer_set_update_proc(raw, Some(canvas_update_proc));
        }
        CanvasLayer { raw, state }
    }

    /// Sets the draw closure, called whenever the layer needs rendering.
    /// Call [`mark_dirty`](Self::mark_dirty) to request a redraw.
    pub fn on_draw(&mut self, f: impl FnMut(&mut Graphics<'_>) + 'static) {
        unsafe { &mut *self.state }.on_draw = Some(Box::new(f));
    }

    /// Requests a redraw.
    pub fn mark_dirty(&mut self) {
        unsafe { sys::layer_mark_dirty(self.raw) };
    }

    pub fn bounds(&self) -> sys::GRect {
        unsafe { sys::layer_get_bounds(self.raw) }
    }

    pub fn set_hidden(&mut self, hidden: bool) {
        unsafe { sys::layer_set_hidden(self.raw, hidden) };
    }
}

impl Drop for CanvasLayer {
    fn drop(&mut self) {
        // Backstop for the documented-unsupported case: dropping a CanvasLayer
        // from inside its own callback would free the state box under the
        // executing closure.
        debug_assert!(
            unsafe { (*self.state).callback_depth } == 0,
            "CanvasLayer dropped from inside its own callback (unsupported; see canvas.rs)"
        );
        unsafe {
            sys::layer_destroy(self.raw);
            drop(Box::from_raw(self.state));
        }
    }
}

impl AsLayer for CanvasLayer {
    fn as_layer_ptr(&self) -> *mut sys::Layer {
        self.raw
    }
}

// --- Trampoline (state via layer_get_data) ---
//
// Same take/call/restore discipline as the click trampolines (click.rs
// module doc): no borrow of CanvasState is live while the user closure runs,
// so the closure may safely reenter the safe API without aliasing UB, and a
// replaced closure cannot be freed out from under itself.

unsafe extern "C" fn canvas_update_proc(layer: *mut sys::Layer, ctx: *mut sys::GContext) {
    let state = *(sys::layer_get_data(layer) as *mut *mut CanvasState);
    let bounds = sys::layer_get_bounds(layer);
    let taken = (*state).on_draw.take();
    if let Some(mut f) = taken {
        let mut g = Graphics::new(ctx, bounds);
        (*state).callback_depth += 1;
        f(&mut g);
        (*state).callback_depth -= 1;
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).on_draw;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    thread_local! {
        static LAYER_BOUNDS: Cell<sys::GRect> = const { Cell::new(sys::GRect(0, 0, 180, 180)) };
        static MARKED_DIRTY_COUNT: Cell<usize> = const { Cell::new(0) };
        static STATE_STORAGE: Cell<*mut CanvasState> = const { Cell::new(core::ptr::null_mut()) };
    }

    #[no_mangle]
    extern "C" fn layer_get_data(_layer: *mut sys::Layer) -> *mut core::ffi::c_void {
        // For tests: return a pointer to a thread-local slot holding the state pointer.
        // Tests set the state pointer via STATE_STORAGE before calling the trampoline.
        STATE_STORAGE
            .with(|storage| storage as *const Cell<*mut CanvasState> as *mut core::ffi::c_void)
    }

    #[no_mangle]
    extern "C" fn layer_get_bounds(_layer: *mut sys::Layer) -> sys::GRect {
        LAYER_BOUNDS.with(|b| b.get())
    }

    #[no_mangle]
    extern "C" fn layer_mark_dirty(_layer: *mut sys::Layer) {
        MARKED_DIRTY_COUNT.with(|c| c.set(c.get() + 1));
    }

    #[no_mangle]
    extern "C" fn layer_destroy(_layer: *mut sys::Layer) {}

    #[no_mangle]
    extern "C" fn layer_set_update_proc(
        _layer: *mut sys::Layer,
        _proc: Option<unsafe extern "C" fn(*mut sys::Layer, *mut sys::GContext)>,
    ) {
    }

    #[test]
    fn on_draw_closure_survives_two_dispatches() {
        let call_count = Rc::new(Cell::new(0));
        let state = Box::into_raw(Box::new(CanvasState {
            on_draw: None,
            callback_depth: 0,
        }));

        let c = call_count.clone();
        unsafe { &mut *state }.on_draw = Some(Box::new(move |_g| {
            c.set(c.get() + 1);
        }));

        // First dispatch
        unsafe {
            let bounds = sys::GRect(0, 0, 180, 180);
            let g_ctx = core::ptr::null_mut();
            let taken = (*state).on_draw.take();
            if let Some(mut f) = taken {
                (*state).callback_depth += 1;
                let mut g = Graphics::new(g_ctx, bounds);
                f(&mut g);
                (*state).callback_depth -= 1;
                let slot = &mut (*state).on_draw;
                if slot.is_none() {
                    *slot = Some(f);
                }
            }
        }

        assert_eq!(call_count.get(), 1, "first dispatch should call closure");

        // Second dispatch: restore must have kept the closure
        unsafe {
            let bounds = sys::GRect(0, 0, 180, 180);
            let g_ctx = core::ptr::null_mut();
            let taken = (*state).on_draw.take();
            if let Some(mut f) = taken {
                (*state).callback_depth += 1;
                let mut g = Graphics::new(g_ctx, bounds);
                f(&mut g);
                (*state).callback_depth -= 1;
                let slot = &mut (*state).on_draw;
                if slot.is_none() {
                    *slot = Some(f);
                }
            }
        }

        assert_eq!(
            call_count.get(),
            2,
            "second dispatch should also call closure (restore works)"
        );

        unsafe {
            drop(Box::from_raw(state));
        }
    }

    #[test]
    fn on_draw_can_reenter_via_mark_dirty() {
        let state = Box::into_raw(Box::new(CanvasState {
            on_draw: None,
            callback_depth: 0,
        }));
        let raw_layer = state as *mut sys::Layer; // Dummy, won't be dereferenced

        unsafe { &mut *state }.on_draw = Some(Box::new(move |_g| {
            // Simulate reentrancy: call mark_dirty from within the closure
            unsafe { sys::layer_mark_dirty(raw_layer) };
        }));

        MARKED_DIRTY_COUNT.with(|c| c.set(0));

        // Dispatch the closure
        unsafe {
            let taken = (*state).on_draw.take();
            if let Some(mut f) = taken {
                (*state).callback_depth += 1;
                let bounds = sys::GRect(0, 0, 180, 180);
                let mut g = Graphics::new(core::ptr::null_mut(), bounds);
                f(&mut g);
                (*state).callback_depth -= 1;
                let slot = &mut (*state).on_draw;
                if slot.is_none() {
                    *slot = Some(f);
                }
            }
        }

        // Verify mark_dirty was called (no crash = reentrancy safe)
        MARKED_DIRTY_COUNT.with(|c| {
            assert_eq!(
                c.get(),
                1,
                "closure should have called mark_dirty reentrantly"
            )
        });

        unsafe {
            drop(Box::from_raw(state));
        }
    }

    #[test]
    fn canvas_update_proc_through_trampoline_survives_two_calls() {
        // This test verifies the restore logic in canvas_update_proc by:
        // 1. Creating a state and setting a closure
        // 2. Calling canvas_update_proc directly (simulating SDK's call)
        // 3. Calling it again to verify the closure survived restore
        // If restore were broken (unconditional overwrite), the second call would
        // use the dropped first closure and panic or crash.

        let call_count = Rc::new(Cell::new(0));
        let state = Box::into_raw(Box::new(CanvasState {
            on_draw: None,
            callback_depth: 0,
        }));

        let c = call_count.clone();
        unsafe { &mut *state }.on_draw = Some(Box::new(move |_g| {
            c.set(c.get() + 1);
        }));

        // Set up the state storage so layer_get_data can find our state
        STATE_STORAGE.with(|storage| storage.set(state));

        let fake_layer = core::ptr::null_mut::<sys::Layer>();

        // First call to trampoline
        unsafe {
            canvas_update_proc(fake_layer, core::ptr::null_mut());
        }
        assert_eq!(
            call_count.get(),
            1,
            "first trampoline call should execute closure"
        );

        // Second call to trampoline (would fail if restore clobbered the closure)
        unsafe {
            canvas_update_proc(fake_layer, core::ptr::null_mut());
        }
        assert_eq!(
            call_count.get(),
            2,
            "second trampoline call should also execute closure (restore works)"
        );

        unsafe {
            drop(Box::from_raw(state));
        }
    }
}

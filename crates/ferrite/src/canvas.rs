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
//! **RFC-2229 hazard:** When capturing state variables in closures (e.g. with
//! `move`), be careful to capture whole variables, not disjoint fields. A
//! closure that captures only `state.canvas` (via field-level capture) can
//! outlive the window that holds the CanvasLayer, causing use-after-free when
//! the closure later tries to access the layer. Destructure at the capture
//! boundary and move entire variables into closures; return children before
//! parents in the app state tuple.

use alloc::boxed::Box;
use core::mem::size_of;

use crate::graphics::Graphics;
use crate::layer::AsLayer;
use crate::sys;
use crate::App;

struct CanvasState {
    on_draw: Option<Box<dyn FnMut(&mut Graphics<'_>)>>,
}

pub struct CanvasLayer {
    raw: *mut sys::Layer,
    state: *mut CanvasState, // Box, owned; freed in Drop
}

impl CanvasLayer {
    /// Creates a canvas layer with the given frame. Panics if the SDK
    /// returns NULL (out of memory).
    pub fn new(_app: &mut App, frame: sys::GRect) -> CanvasLayer {
        let raw =
            unsafe { sys::layer_create_with_data(frame, size_of::<*mut CanvasState>()) };
        assert!(!raw.is_null(), "layer_create_with_data returned NULL");
        let state = Box::into_raw(Box::new(CanvasState { on_draw: None }));
        unsafe {
            *(sys::layer_get_data(raw) as *mut *mut CanvasState) = state;
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
        f(&mut g);
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).on_draw;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

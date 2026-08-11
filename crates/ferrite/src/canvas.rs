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
//! **RFC-2229 hazard:** edition-2021 `move` closures capture only the PATHS
//! they use, not whole variables. With screen tuples like
//! `let screens = (text_screen, canvas_screen);` — where each element is a
//! `(Window, Layer)` pair — a closure body using `screens.0 .0.push(true)`
//! captures ONLY that window field. The uncaptured siblings (the layers at
//! `screens.0 .1` / `screens.1 .1`) are dropped at the end of the enclosing
//! block, destroying their SDK objects while the windows still list them as
//! children — the screen then renders blank (observed on the emulator in
//! Phase 4). Fix: destructure first (`let (mut text_win, text) = ...;`),
//! have the closure use the WINDOW as a whole variable
//! (`move |row| text_win.push(true)` — a bare path captures completely),
//! and keep the layers alive in the app-state tuple, children before
//! parents.

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

    /// Drive the REAL trampoline (not a copy of its body): set up
    /// STATE_STORAGE so the layer_get_data stub hands canvas_update_proc our
    /// state, then call the extern "C" symbol itself.
    fn dispatch(state: *mut CanvasState) {
        STATE_STORAGE.with(|storage| storage.set(state));
        unsafe { canvas_update_proc(core::ptr::null_mut(), core::ptr::null_mut()) };
    }

    #[test]
    fn on_draw_closure_survives_two_dispatches() {
        let call_count = Rc::new(Cell::new(0));
        let state = Box::into_raw(Box::new(CanvasState {
            on_draw: None,
            callback_depth: 0,
        }));
        let c = call_count.clone();
        unsafe { &mut *state }.on_draw = Some(Box::new(move |_g| c.set(c.get() + 1)));

        dispatch(state);
        assert_eq!(call_count.get(), 1);
        dispatch(state);
        assert_eq!(call_count.get(), 2, "restore must keep the closure");

        unsafe { drop(Box::from_raw(state)) };
    }

    /// The draw closure may reenter the safe API mid-dispatch (its slot is
    /// empty at that point). mark_dirty is the realistic reentry; the
    /// trampoline must not hold any borrow of CanvasState across the call.
    #[test]
    fn on_draw_can_reenter_via_mark_dirty() {
        let state = Box::into_raw(Box::new(CanvasState {
            on_draw: None,
            callback_depth: 0,
        }));
        unsafe { &mut *state }.on_draw = Some(Box::new(move |_g| {
            unsafe { sys::layer_mark_dirty(core::ptr::null_mut()) };
        }));

        MARKED_DIRTY_COUNT.with(|c| c.set(0));
        dispatch(state);
        MARKED_DIRTY_COUNT.with(|c| assert_eq!(c.get(), 1));

        unsafe { drop(Box::from_raw(state)) };
    }

    /// THE invariant the module doc promises: a closure that re-registers a
    /// replacement during its own dispatch must WIN -- restore must not
    /// clobber it with the finished closure. (Mirror of
    /// click.rs::restore_does_not_clobber_a_reregistered_handler.)
    #[test]
    fn reentrant_on_draw_replacement_wins() {
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        let state = Box::into_raw(Box::new(CanvasState {
            on_draw: None,
            callback_depth: 0,
        }));

        let f1 = first.clone();
        let s2 = second.clone();
        unsafe { &mut *state }.on_draw = Some(Box::new(move |_g| {
            f1.set(f1.get() + 1);
            // Reentrant replacement: our slot is empty (taken) right now, so
            // this lands in the slot and restore must NOT overwrite it.
            let s2 = s2.clone();
            unsafe { &mut *state }.on_draw = Some(Box::new(move |_g| s2.set(s2.get() + 1)));
        }));

        dispatch(state);
        dispatch(state);
        assert_eq!(
            (first.get(), second.get()),
            (1, 1),
            "the reentrantly-registered closure must win over the restore"
        );

        unsafe { drop(Box::from_raw(state)) };
    }

    // (A former canvas_update_proc_through_trampoline_survives_two_calls test
    // was deleted: after the dispatch() helper landed it duplicated
    // on_draw_closure_survives_two_dispatches exactly — same trampoline path,
    // same mutants killed — and its comment claimed it caught the
    // unconditional-restore mutant, which measurement disproved. The
    // reentrant_on_draw_replacement_wins test is what kills that mutant.)
}

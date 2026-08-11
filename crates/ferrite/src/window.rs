//! Owned wrapper over the SDK `Window`, with closure-based handlers.
//!
//! Each window owns a boxed `WindowState` attached via
//! `window_set_user_data`; `extern "C"` trampolines recover it from the
//! window pointer (load/unload) or the click context.
//!
//! Soundness: the platform is single-threaded, but that alone is NOT the
//! justification -- the SDK does reenter (pebble.h:5213: installing a click
//! provider on a visible window invokes it synchronously), and user closures
//! can reach back into this API. The rule that actually holds is the one the
//! trampolines follow: never hold a borrow of `WindowState` across a
//! user-closure call. Closures are taken out of their slot, run borrow-free,
//! and restored only if the slot is still empty. See `click.rs`'s module doc
//! for the full contract, including the one unsupported case (dropping a
//! `Window` from inside its own callback).

use alloc::boxed::Box;

use crate::click::{self, Button, ClickHandlers, LongClick};
use crate::sys;
use crate::App;

pub(crate) struct WindowState {
    pub(crate) on_load: Option<Box<dyn FnMut()>>,
    pub(crate) on_unload: Option<Box<dyn FnMut()>>,
    pub(crate) clicks: ClickHandlers,
    /// Number of this window's callbacks currently on the stack. Structural
    /// backstop for the documented-unsupported case (dropping a `Window`
    /// from inside its own callback): `Drop` debug-asserts this is zero, so
    /// in debug builds the use-after-free becomes a deterministic panic.
    pub(crate) callback_depth: u8,
}

pub struct Window {
    raw: *mut sys::Window,
    state: *mut WindowState, // Box, owned; freed in Drop after window_destroy
}

impl Window {
    /// Creates a new window. Panics if the SDK returns NULL (out of memory).
    pub fn new(_app: &mut App) -> Window {
        let raw = unsafe { sys::window_create() };
        assert!(!raw.is_null(), "window_create returned NULL");
        let state = Box::into_raw(Box::new(WindowState {
            on_load: None,
            on_unload: None,
            clicks: ClickHandlers::new(),
            callback_depth: 0,
        }));
        unsafe {
            sys::window_set_user_data(raw, state.cast());
            sys::window_set_window_handlers(
                raw,
                sys::WindowHandlers {
                    load: Some(on_window_load),
                    appear: None,
                    disappear: None,
                    unload: Some(on_window_unload),
                },
            );
        }
        Window { raw, state }
    }

    fn state_mut(&mut self) -> &mut WindowState {
        unsafe { &mut *self.state }
    }

    /// Runs when the SDK loads the window (each time it enters the stack).
    pub fn on_load(&mut self, f: impl FnMut() + 'static) {
        self.state_mut().on_load = Some(Box::new(f));
    }

    /// Runs when the SDK unloads the window.
    pub fn on_unload(&mut self, f: impl FnMut() + 'static) {
        self.state_mut().on_unload = Some(Box::new(f));
    }

    /// Registers a single-click handler for a button.
    ///
    /// Takes effect immediately even if the window is already on-screen: the
    /// SDK re-runs the click config provider synchronously when it is
    /// (re)installed on a visible window (pebble.h:5213), which is why this
    /// method re-installs it on every registration.
    ///
    /// **Mutually exclusive with `MenuLayer::attach_clicks`:** If this window
    /// has a menu layer with `attach_clicks` wired to it, calling `on_click` or
    /// `on_long_click` will re-install ferrite's click provider, silently
    /// replacing the menu's UP/DOWN/SELECT navigation with ferrite's handlers.
    /// Similarly, calling `attach_clicks` after `on_click` re-installs the
    /// menu's provider. Use one or the other, not both on the same window.
    pub fn on_click(&mut self, button: Button, f: impl FnMut() + 'static) {
        self.state_mut().clicks.set_single(button, Box::new(f));
        self.install_click_provider();
    }

    /// Registers a long-press handler (fires on press after `delay_ms`).
    ///
    /// **Mutually exclusive with `MenuLayer::attach_clicks`:** see `on_click`'s
    /// documentation for the clobbering behavior.
    pub fn on_long_click(&mut self, button: Button, delay_ms: u16, f: impl FnMut() + 'static) {
        let entry =
            self.state_mut().clicks.long[button as usize].get_or_insert_with(|| LongClick {
                delay_ms,
                down: None,
                up: None,
            });
        entry.delay_ms = delay_ms;
        entry.down = Some(Box::new(f));
        self.install_click_provider();
    }

    /// Registers a long-press release handler (fires when the button is
    /// released after a long press of `delay_ms`).
    pub fn on_long_click_up(&mut self, button: Button, delay_ms: u16, f: impl FnMut() + 'static) {
        let entry =
            self.state_mut().clicks.long[button as usize].get_or_insert_with(|| LongClick {
                delay_ms,
                down: None,
                up: None,
            });
        entry.delay_ms = delay_ms;
        entry.up = Some(Box::new(f));
        self.install_click_provider();
    }

    fn install_click_provider(&mut self) {
        unsafe {
            sys::window_set_click_config_provider_with_context(
                self.raw,
                Some(click::click_config_provider),
                self.state.cast(),
            );
        }
    }

    pub fn set_background_color(&mut self, color: sys::GColor8) {
        unsafe { sys::window_set_background_color(self.raw, color) };
    }

    /// Bounds of the window's root layer.
    pub fn bounds(&self) -> sys::GRect {
        unsafe { sys::layer_get_bounds(sys::window_get_root_layer(self.raw)) }
    }

    /// Adds a layer wrapper as a child of the window's root layer. The child
    /// must outlive its time in the window (keep both in your app state).
    ///
    /// See `crate::layer` for why children are borrowed (not owned by the
    /// window): post-add mutation (`set_text`, `mark_dirty`, `reload`) is
    /// ubiquitous, and ownership would require a handle/index API. The borrow
    /// contract is enforced by the app's return-tuple order (children before
    /// parents, so Rust's tuple drop order maintains correctness) and the
    /// trampoline discipline (take/call/restore).
    pub fn add_child(&mut self, child: &impl crate::layer::AsLayer) {
        unsafe {
            sys::layer_add_child(sys::window_get_root_layer(self.raw), child.as_layer_ptr());
        }
    }

    pub(crate) fn as_window_ptr(&mut self) -> *mut sys::Window {
        self.raw
    }

    /// Pushes the window onto the window stack, making it visible.
    pub fn push(&mut self, animated: bool) {
        unsafe { sys::window_stack_push(self.raw, animated) };
    }

    /// Removes this window from the stack (visible or not).
    ///
    /// Note (pebble.h:5730): if this leaves no windows on the stack, the
    /// system kills the app shortly afterwards.
    pub fn remove_from_stack(&mut self, animated: bool) -> bool {
        unsafe { sys::window_stack_remove(self.raw, animated) }
    }

    pub fn is_loaded(&self) -> bool {
        unsafe { sys::window_is_loaded(self.raw) }
    }

    /// Whether this window is currently on the window stack (visible or
    /// covered). Distinct from `is_loaded`, which reports SDK load state.
    ///
    /// Intended for idempotent stack management: a caller that maps
    /// application state onto the window stack every tick needs to know
    /// whether a push would be a no-op.
    pub fn is_on_stack(&self) -> bool {
        unsafe { sys::window_stack_contains_window(self.raw) }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // Backstop for the documented-unsupported case: dropping a Window
        // from inside one of its own callbacks would free the state box
        // under the executing closure. Checked before window_destroy, which
        // legitimately raises the depth itself when it fires unload.
        debug_assert!(
            unsafe { (*self.state).callback_depth } == 0,
            "Window dropped from inside its own callback (unsupported; see click.rs)"
        );
        unsafe {
            // Destroy first: it can fire the unload handler, which reads state.
            sys::window_destroy(self.raw);
            drop(Box::from_raw(self.state));
        }
    }
}

/// Pops the topmost window off the stack. Returns `false` if the stack was
/// empty (nothing to pop).
///
/// Note (pebble.h:5730): if this leaves no windows on the stack, the system
/// kills the app shortly afterwards.
pub fn stack_pop(animated: bool) -> bool {
    !unsafe { sys::window_stack_pop(animated) }.is_null()
}

// --- Window handler trampolines (state via window_get_user_data) ---
//
// Same take/call/restore discipline as the click trampolines (click.rs
// module doc): no borrow of WindowState is live while the user closure runs,
// so a load/unload closure may reenter the safe API without aliasing UB, and
// a replaced closure cannot be freed out from under itself.

unsafe extern "C" fn on_window_load(window: *mut sys::Window) {
    let state = sys::window_get_user_data(window) as *mut WindowState;
    let taken = (*state).on_load.take();
    if let Some(mut f) = taken {
        (*state).callback_depth += 1;
        f();
        (*state).callback_depth -= 1;
        let slot = &mut (*state).on_load;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

unsafe extern "C" fn on_window_unload(window: *mut sys::Window) {
    let state = sys::window_get_user_data(window) as *mut WindowState;
    let taken = (*state).on_unload.take();
    if let Some(mut f) = taken {
        (*state).callback_depth += 1;
        f();
        (*state).callback_depth -= 1;
        let slot = &mut (*state).on_unload;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::click::ClickHandlers;
    use core::mem::ManuallyDrop;
    use std::cell::Cell;

    // Real-symbol SDK stub: the host linker uses this #[no_mangle] definition
    // to satisfy the bindgen `extern` declaration, so the tests below exercise
    // the REAL is_on_stack wrapper -- not a parallel mock API. House pattern
    // per persist.rs / click.rs::provider_tests. Behaviour is scripted through
    // thread_locals so the multithreaded harness cannot race.
    std::thread_local! {
        static ON_STACK: Cell<bool> = const { Cell::new(false) };
        static ASKED_ABOUT: Cell<usize> = const { Cell::new(0) };
    }

    #[no_mangle]
    extern "C" fn window_stack_contains_window(window: *mut sys::Window) -> bool {
        ASKED_ABOUT.with(|c| c.set(window as usize));
        ON_STACK.with(|c| c.get())
    }

    fn empty_state() -> WindowState {
        WindowState {
            on_load: None,
            on_unload: None,
            clicks: ClickHandlers::new(),
            callback_depth: 0,
        }
    }

    /// Wraps a fabricated raw pointer in a `Window` without calling the SDK.
    /// `ManuallyDrop` because the real `Drop` would call `window_destroy` on a
    /// bogus pointer and then `Box::from_raw` a pointer that was never boxed.
    fn fake_window(raw: usize, state: &mut WindowState) -> ManuallyDrop<Window> {
        ManuallyDrop::new(Window {
            raw: raw as *mut sys::Window,
            state: state as *mut WindowState,
        })
    }

    #[test]
    fn is_on_stack_reports_true_when_the_sdk_says_so() {
        let mut state = empty_state();
        let w = fake_window(0xBEEF, &mut state);
        ON_STACK.with(|c| c.set(true));
        assert!(w.is_on_stack());
    }

    #[test]
    fn is_on_stack_reports_false_when_the_sdk_says_so() {
        let mut state = empty_state();
        let w = fake_window(0xBEEF, &mut state);
        ON_STACK.with(|c| c.set(false));
        assert!(!w.is_on_stack());
    }

    /// The wrapper must ask about ITS OWN window. Passing the wrong pointer
    /// would make fitter's router see another window's membership and
    /// push/pop in a 1 Hz loop.
    #[test]
    fn is_on_stack_asks_the_sdk_about_this_window() {
        let mut state = empty_state();
        let w = fake_window(0xD00D, &mut state);
        ASKED_ABOUT.with(|c| c.set(0));
        let _ = w.is_on_stack();
        assert_eq!(ASKED_ABOUT.with(|c| c.get()), 0xD00D);
    }
}

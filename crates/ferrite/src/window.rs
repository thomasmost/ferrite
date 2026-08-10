//! Owned wrapper over the SDK `Window`, with closure-based handlers.
//!
//! Each window owns a boxed `WindowState` attached via
//! `window_set_user_data`; `extern "C"` trampolines recover it from the
//! window pointer (load/unload) or the click context. Sound because the
//! platform is single-threaded and the SDK never nests these callbacks.

use alloc::boxed::Box;

use crate::click::{self, Button, ClickHandlers, LongClick};
use crate::sys;
use crate::App;

pub(crate) struct WindowState {
    pub(crate) on_load: Option<Box<dyn FnMut()>>,
    pub(crate) on_unload: Option<Box<dyn FnMut()>>,
    pub(crate) clicks: ClickHandlers,
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
    pub fn on_click(&mut self, button: Button, f: impl FnMut() + 'static) {
        self.state_mut().clicks.single[button as usize] = Some(Box::new(f));
        self.install_click_provider();
    }

    /// Registers a long-press handler (fires on press after `delay_ms`).
    pub fn on_long_click(&mut self, button: Button, delay_ms: u16, f: impl FnMut() + 'static) {
        let entry = self.state_mut().clicks.long[button as usize]
            .get_or_insert_with(|| LongClick {
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
        let entry = self.state_mut().clicks.long[button as usize]
            .get_or_insert_with(|| LongClick {
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

    /// Adds a text layer as a child of the window's root layer. The child
    /// must outlive its time in the window (keep both in your app state).
    pub fn add_child(&mut self, child: &crate::text_layer::TextLayer) {
        unsafe {
            sys::layer_add_child(
                sys::window_get_root_layer(self.raw),
                child.as_layer_ptr(),
            );
        }
    }

    /// Pushes the window onto the window stack, making it visible.
    pub fn push(&mut self, animated: bool) {
        unsafe { sys::window_stack_push(self.raw, animated) };
    }

    /// Removes this window from the stack (visible or not).
    pub fn remove_from_stack(&mut self, animated: bool) -> bool {
        unsafe { sys::window_stack_remove(self.raw, animated) }
    }

    pub fn is_loaded(&self) -> bool {
        unsafe { sys::window_is_loaded(self.raw) }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            // Destroy first: it can fire the unload handler, which reads state.
            sys::window_destroy(self.raw);
            drop(Box::from_raw(self.state));
        }
    }
}

/// Pops the topmost window off the stack.
pub fn stack_pop(animated: bool) {
    unsafe {
        sys::window_stack_pop(animated);
    }
}

// --- Window handler trampolines (state via window_get_user_data) ---

unsafe extern "C" fn on_window_load(window: *mut sys::Window) {
    let state = sys::window_get_user_data(window) as *mut WindowState;
    if let Some(f) = (*state).on_load.as_mut() {
        f();
    }
}

unsafe extern "C" fn on_window_unload(window: *mut sys::Window) {
    let state = sys::window_get_user_data(window) as *mut WindowState;
    if let Some(f) = (*state).on_unload.as_mut() {
        f();
    }
}

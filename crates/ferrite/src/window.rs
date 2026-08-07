//! Owned wrapper over the SDK `Window`.

use crate::sys;
use crate::App;

pub struct Window {
    raw: *mut sys::Window,
}

impl Window {
    /// Creates a new window. Panics if the SDK returns NULL (out of memory).
    pub fn new(_app: &mut App) -> Window {
        let raw = unsafe { sys::window_create() };
        assert!(!raw.is_null(), "window_create returned NULL");
        Window { raw }
    }

    /// Bounds of the window's root layer.
    pub fn bounds(&self) -> sys::GRect {
        unsafe { sys::layer_get_bounds(sys::window_get_root_layer(self.raw)) }
    }

    /// Adds a text layer as a child of the window's root layer.
    ///
    /// Note (phase-1 contract): the SDK does not take ownership — the child
    /// must stay alive as long as the window shows it. Keep both in the
    /// state returned from your `app!` setup block.
    pub fn add_child(&mut self, child: &crate::text_layer::TextLayer) {
        unsafe {
            sys::layer_add_child(
                sys::window_get_root_layer(self.raw),
                child.as_layer_ptr(),
            );
        }
    }

    /// Pushes the window onto the window stack, making it visible.
    pub fn push(&mut self) {
        unsafe { sys::window_stack_push(self.raw, false) };
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe { sys::window_destroy(self.raw) };
    }
}

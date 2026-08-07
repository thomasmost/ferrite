//! Owned wrapper over the SDK `TextLayer`.

use core::ffi::CStr;

use crate::sys;
use crate::App;

pub struct TextLayer {
    raw: *mut sys::TextLayer,
}

impl TextLayer {
    /// Creates a text layer with the given frame. Panics if the SDK returns
    /// NULL (out of memory).
    pub fn new(_app: &mut App, frame: sys::GRect) -> TextLayer {
        let raw = unsafe { sys::text_layer_create(frame) };
        assert!(!raw.is_null(), "text_layer_create returned NULL");
        TextLayer { raw }
    }

    /// Sets the displayed text. `&'static` because the SDK stores the pointer
    /// without copying; `c"..."` literals satisfy this.
    pub fn set_text(&mut self, text: &'static CStr) {
        unsafe { sys::text_layer_set_text(self.raw, text.as_ptr()) };
    }

    pub(crate) fn as_layer_ptr(&self) -> *mut sys::Layer {
        unsafe { sys::text_layer_get_layer(self.raw) }
    }
}

impl Drop for TextLayer {
    fn drop(&mut self) {
        unsafe { sys::text_layer_destroy(self.raw) };
    }
}

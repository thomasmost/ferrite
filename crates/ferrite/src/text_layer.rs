//! Owned wrapper over the SDK `TextLayer`.

use core::ffi::CStr;

use crate::sys;
use crate::text_buf::TextBuf;
use crate::App;

/// A system font handle (Copy; fonts are owned by the system).
#[derive(Clone, Copy)]
pub struct Font(pub(crate) sys::GFont);

/// Looks up a system font by key. Pass one of the `sys::FONT_KEY_*` byte
/// strings (they are NUL-terminated).
///
/// Panics if `key` is not NUL-terminated. With the intended inputs -- the
/// bindgen-generated `sys::FONT_KEY_*` constants, which always carry the
/// trailing NUL -- the panic is unreachable; it exists to catch hand-built
/// keys. (The parameter is `&'static [u8]` rather than `&CStr` because that
/// is the type bindgen emits for the `#define` string constants; a
/// compile-time check would require wrapping every constant in a macro or
/// post-processing the generated bindings, which is not worth the surface.)
pub fn system_font(key: &'static [u8]) -> Font {
    assert!(
        matches!(key.last(), Some(0)),
        "font key must be a NUL-terminated sys::FONT_KEY_* constant"
    );
    Font(unsafe { sys::fonts_get_system_font(key.as_ptr().cast()) })
}

pub struct TextLayer {
    raw: *mut sys::TextLayer,
    text: TextBuf,
}

impl TextLayer {
    /// Creates a text layer with the given frame. Panics if the SDK returns
    /// NULL (out of memory).
    pub fn new(_app: &mut App, frame: sys::GRect) -> TextLayer {
        let raw = unsafe { sys::text_layer_create(frame) };
        assert!(!raw.is_null(), "text_layer_create returned NULL");
        TextLayer {
            raw,
            text: TextBuf::new(),
        }
    }

    /// Sets static text (no copy; the SDK stores the pointer).
    pub fn set_text(&mut self, text: &'static CStr) {
        let ptr = self.text.set_static(text);
        unsafe { sys::text_layer_set_text(self.raw, ptr) };
    }

    /// Sets dynamic text: copied into a buffer owned by this wrapper, so the
    /// SDK's stored pointer stays valid for the layer's lifetime.
    pub fn set_text_owned(&mut self, text: &str) {
        let ptr = self.text.set_owned(text);
        unsafe { sys::text_layer_set_text(self.raw, ptr) };
    }

    pub fn set_text_color(&mut self, color: sys::GColor8) {
        unsafe { sys::text_layer_set_text_color(self.raw, color) };
    }

    pub fn set_background_color(&mut self, color: sys::GColor8) {
        unsafe { sys::text_layer_set_background_color(self.raw, color) };
    }

    pub fn set_alignment(&mut self, alignment: sys::GTextAlignment) {
        unsafe { sys::text_layer_set_text_alignment(self.raw, alignment) };
    }

    pub fn set_font(&mut self, font: Font) {
        unsafe { sys::text_layer_set_font(self.raw, font.0) };
    }
}

impl Drop for TextLayer {
    fn drop(&mut self) {
        unsafe { sys::text_layer_destroy(self.raw) };
    }
}

impl crate::layer::AsLayer for TextLayer {
    fn as_layer_ptr(&self) -> *mut sys::Layer {
        unsafe { sys::text_layer_get_layer(self.raw) }
    }
}

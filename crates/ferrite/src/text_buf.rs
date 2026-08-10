//! Ownership bookkeeping for text the SDK stores by raw pointer: the SDK
//! keeps a `const char*` without copying, so the wrapper must own the
//! storage for as long as the SDK might read it.

use alloc::ffi::CString;
use core::ffi::{c_char, CStr};

enum TextSource {
    None,
    Static(&'static CStr),
    Owned(CString),
}

pub(crate) struct TextBuf {
    source: TextSource,
}

impl TextBuf {
    pub(crate) fn new() -> TextBuf {
        TextBuf {
            source: TextSource::None,
        }
    }

    /// Point at a static C string; returns the pointer to hand to the SDK.
    pub(crate) fn set_static(&mut self, s: &'static CStr) -> *const c_char {
        self.source = TextSource::Static(s);
        s.as_ptr()
    }

    /// Copy `s` into owned storage; returns a pointer that stays valid until
    /// the next `set_*` call. Interior NUL bytes are not representable in a
    /// C string; such input is replaced with a marker rather than panicking.
    ///
    /// Lifetime note: the assignment to `self.source` FREES the previous
    /// buffer while the SDK may still hold its pointer (the SDK stores
    /// `const char*` without copying). The gap is closed only because the
    /// platform is single-threaded and nothing can render between this
    /// assignment and the caller immediately handing the SDK the new pointer
    /// (`text_layer_set_text` in `TextLayer::set_text_owned`). Do not insert
    /// work between a `set_*` call and the SDK update that consumes it.
    pub(crate) fn set_owned(&mut self, s: &str) -> *const c_char {
        let c = CString::new(s).unwrap_or_else(|_| CString::new("<text contained NUL>").unwrap());
        self.source = TextSource::Owned(c);
        self.as_ptr()
    }

    pub(crate) fn as_ptr(&self) -> *const c_char {
        match &self.source {
            TextSource::None => core::ptr::null(),
            TextSource::Static(s) => s.as_ptr(),
            TextSource::Owned(c) => c.as_ptr(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ffi::CStr;

    fn cstr_at(ptr: *const core::ffi::c_char) -> &'static str {
        unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()
    }

    #[test]
    fn static_text_returns_same_pointer() {
        let mut buf = TextBuf::new();
        let ptr = buf.set_static(c"hello");
        assert_eq!(ptr, c"hello".as_ptr());
        assert_eq!(cstr_at(buf.as_ptr()), "hello");
    }

    #[test]
    fn owned_text_is_copied_and_nul_terminated() {
        let mut buf = TextBuf::new();
        let s = String::from("dynamic");
        let ptr = buf.set_owned(&s);
        drop(s); // wrapper owns its own copy
        assert_eq!(cstr_at(ptr), "dynamic");
        assert_eq!(cstr_at(buf.as_ptr()), "dynamic");
    }

    #[test]
    fn replacing_owned_text_keeps_new_contents() {
        let mut buf = TextBuf::new();
        buf.set_owned("first");
        let ptr = buf.set_owned("second");
        assert_eq!(cstr_at(ptr), "second");
    }

    #[test]
    fn interior_nul_is_replaced_not_panicking() {
        let mut buf = TextBuf::new();
        let ptr = buf.set_owned("a\0b");
        // must produce *some* valid C string without panicking
        assert!(!cstr_at(ptr).is_empty());
    }

    /// The None state is the only path that can hand `text_layer_set_text`
    /// a NULL, which the SDK documents as "clear the text".
    #[test]
    fn empty_buf_yields_null() {
        let buf = TextBuf::new();
        assert!(buf.as_ptr().is_null());
    }

    #[test]
    fn owned_to_static_transition() {
        let mut buf = TextBuf::new();
        buf.set_owned("owned");
        let ptr = buf.set_static(c"static");
        assert_eq!(ptr, c"static".as_ptr());
        assert_eq!(cstr_at(buf.as_ptr()), "static");
    }

    #[test]
    fn static_to_owned_transition() {
        let mut buf = TextBuf::new();
        buf.set_static(c"static");
        let ptr = buf.set_owned("owned");
        assert_eq!(cstr_at(ptr), "owned");
        assert_eq!(cstr_at(buf.as_ptr()), "owned");
    }
}

//! Common interface for wrappers that are backed by an SDK `Layer`.

use crate::sys;

pub trait AsLayer {
    /// Raw layer pointer — used by `Window::add_child`. Not for user code.
    #[doc(hidden)]
    fn as_layer_ptr(&self) -> *mut sys::Layer;
}

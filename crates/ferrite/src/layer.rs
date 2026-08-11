//! Common interface for wrappers that are backed by an SDK `Layer`.
//!
//! **Child ownership model (rejected):** Phase 4's review proposed making
//! `Window` own its children (taking layers by value into `WindowState`, with
//! `Drop` destroying children before `window_destroy`), to make the lifetime
//! ordering structurally impossible to violate. This was rejected in Phase 5's
//! layer design (which added `AsLayer` and two new layer types), because
//! post-add mutation is ubiquitous: text layers need `set_text()`, canvas layers
//! need `mark_dirty()`, menu layers need `reload()` — all taking `&mut self`
//! AFTER `add_child()`. Under ownership, these would require a handle/index API
//! into `WindowState` to retrieve the layer mutably. Instead, the borrow model
//! is retained and enforced by the app's return-tuple contract: children are
//! listed before parents (tuple order = drop order), and the app's state is
//! moved into closures, not scattered across fields (whole-variable capture,
//! not RFC-2229 disjoint fields). The trampoline discipline
//! (take/call/restore) is the other guardrail. See `click.rs`/`canvas.rs`/
//! `menu_layer.rs` for the full contract.

use crate::sys;

pub trait AsLayer {
    /// Raw layer pointer — used by `Window::add_child`. Not for user code.
    #[doc(hidden)]
    fn as_layer_ptr(&self) -> *mut sys::Layer;
}

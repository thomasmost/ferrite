//! List UI over the SDK `MenuLayer` — single-section, closure callbacks.
//!
//! The SDK carries one `callback_context` for all menu callbacks
//! (`menu_layer_set_callbacks`); we pass the boxed `MenuState` pointer.
//!
//! **Trampoline discipline:** The callbacks must never hold a borrow of
//! `MenuState` across a user-closure call, as the closure can reenter the
//! safe API (e.g. `reload()` which calls `menu_layer_reload_data`). We use the
//! take/call/restore pattern from `click.rs`/`canvas.rs`: the closure is taken
//! out of its slot (statement-scoped borrow), run borrow-free, and restored
//! only if empty (so a reentrant registration wins). For `cb_get_num_rows`
//! which returns a value, we capture the return value before restoring.
//! See `click.rs`'s module doc for the full contract.
//!
//! **Residual contract:** Dropping a `MenuLayer` from inside one of its own
//! callbacks (select_click, rows, on_draw_row) is not supported — the state
//! box would be freed under the executing closure. `Drop` debug-asserts that
//! `callback_depth` is zero, making this a deterministic panic in debug
//! builds.

use alloc::boxed::Box;
use core::ffi::c_void;
use core::ffi::CStr;
use core::marker::PhantomData;
use core::ptr;

use crate::layer::AsLayer;
use crate::sys;
use crate::window::Window;
use crate::App;

/// Copies `s` into `buf` with a NUL terminator, truncating by byte to fit,
/// and returns it as a `&CStr`. Mirrors `snprintf`'s contract.
///
/// Truncation is by byte, not character: the SDK renders bytes, and a
/// half-written multi-byte character renders as a replacement glyph rather
/// than panicking. `buf` must be non-empty.
pub(crate) fn str_to_cstr<'a>(buf: &'a mut [u8], s: &str) -> &'a CStr {
    assert!(!buf.is_empty(), "row-text buffer must have room for a NUL");
    let limit = buf.len() - 1;
    let n = core::cmp::min(s.len(), limit);
    buf[..n].copy_from_slice(&s.as_bytes()[..n]);
    buf[n] = 0;
    // SAFETY: bytes [0, n) are copied from a &str so contain no interior NUL,
    // and buf[n] is the terminator.
    unsafe { CStr::from_bytes_with_nul_unchecked(&buf[..=n]) }
}

/// A menu row cell being drawn — draw helpers over the SDK cell renderers.
pub struct RowCell<'a> {
    ctx: *mut sys::GContext,
    cell_layer: *const sys::Layer,
    _lifetime: PhantomData<&'a mut ()>,
}

impl RowCell<'_> {
    /// Standard cell: title only.
    pub fn basic_draw(&mut self, title: &CStr) {
        unsafe {
            sys::menu_cell_basic_draw(
                self.ctx,
                self.cell_layer,
                title.as_ptr(),
                ptr::null(),
                ptr::null_mut(),
            );
        }
    }

    /// Standard cell: title and subtitle.
    pub fn basic_draw_with_subtitle(&mut self, title: &CStr, subtitle: &CStr) {
        unsafe {
            sys::menu_cell_basic_draw(
                self.ctx,
                self.cell_layer,
                title.as_ptr(),
                subtitle.as_ptr(),
                ptr::null_mut(),
            );
        }
    }

    /// Like [`basic_draw`](Self::basic_draw) but takes a `&str`, copying it
    /// into a stack buffer. For dynamic row text, which is the common case.
    ///
    /// Text longer than 63 bytes is truncated — menu cells cannot render more
    /// than a couple of dozen characters at any system font anyway.
    pub fn basic_draw_str(&mut self, title: &str) {
        let mut tb = [0u8; 64];
        self.basic_draw(str_to_cstr(&mut tb, title));
    }

    /// Like [`basic_draw_with_subtitle`](Self::basic_draw_with_subtitle) but
    /// takes `&str`s. Each is truncated at 63 bytes.
    pub fn basic_draw_with_subtitle_str(&mut self, title: &str, subtitle: &str) {
        let mut tb = [0u8; 64];
        let mut sb = [0u8; 64];
        // Two separate buffers: the SDK reads both pointers during the call.
        let t = str_to_cstr(&mut tb, title);
        let s = str_to_cstr(&mut sb, subtitle);
        self.basic_draw_with_subtitle(t, s);
    }
}

type GetNumRowsCallback = Box<dyn FnMut() -> u16>;
type DrawRowCallback = Box<dyn FnMut(&mut RowCell<'_>, u16)>;
type SelectClickCallback = Box<dyn FnMut(u16)>;

struct MenuState {
    num_rows: Option<GetNumRowsCallback>,
    draw_row: Option<DrawRowCallback>,
    on_select: Option<SelectClickCallback>,
    /// Number of this menu's callbacks currently on the stack. Structural
    /// backstop for the documented-unsupported case (dropping a `MenuLayer`
    /// from inside its own callback): `Drop` debug-asserts this is zero, so
    /// in debug builds the use-after-free becomes a deterministic panic.
    callback_depth: u8,
}

pub struct MenuLayer {
    raw: *mut sys::MenuLayer,
    state: *mut MenuState, // Box, owned; freed in Drop
}

impl MenuLayer {
    /// Creates a menu layer. Panics if the SDK returns NULL (out of memory).
    ///
    /// Note: a menu without `rows()` and `on_draw_row()` set will render empty.
    /// Set both closures before pushing the window containing this menu.
    pub fn new(_app: &mut App, frame: sys::GRect) -> MenuLayer {
        let raw = unsafe { sys::menu_layer_create(frame) };
        assert!(!raw.is_null(), "menu_layer_create returned NULL");
        let state = Box::into_raw(Box::new(MenuState {
            num_rows: None,
            draw_row: None,
            on_select: None,
            callback_depth: 0,
        }));
        unsafe {
            sys::menu_layer_set_callbacks(
                raw,
                state.cast(),
                sys::MenuLayerCallbacks {
                    get_num_rows: Some(cb_get_num_rows),
                    draw_row: Some(cb_draw_row),
                    select_click: Some(cb_select_click),
                    ..Default::default()
                },
            );
        }
        MenuLayer { raw, state }
    }

    fn state_mut(&mut self) -> &mut MenuState {
        unsafe { &mut *self.state }
    }

    /// Sets the row-count closure (single section).
    pub fn rows(&mut self, f: impl FnMut() -> u16 + 'static) {
        self.state_mut().num_rows = Some(Box::new(f));
    }

    /// Sets the row-draw closure.
    pub fn on_draw_row(&mut self, f: impl FnMut(&mut RowCell<'_>, u16) + 'static) {
        self.state_mut().draw_row = Some(Box::new(f));
    }

    /// Sets the SELECT-click closure (receives the selected row).
    pub fn on_select(&mut self, f: impl FnMut(u16) + 'static) {
        self.state_mut().on_select = Some(Box::new(f));
    }

    /// Binds UP/DOWN/SELECT on the window to this menu (the SDK's standard
    /// menu navigation — replaces any window click config).
    ///
    /// **Mutually exclusive with `Window::on_click` and `on_long_click`:**
    /// If either has been called on this window, calling `attach_clicks` will
    /// re-install the menu's provider, silently replacing ferrite's handlers
    /// with menu navigation. Similarly, calling `on_click` or `on_long_click`
    /// after `attach_clicks` re-installs ferrite's provider. Use one or the
    /// other, not both on the same window.
    pub fn attach_clicks(&mut self, window: &mut Window) {
        unsafe {
            sys::menu_layer_set_click_config_onto_window(self.raw, window.as_window_ptr());
        }
    }

    pub fn set_normal_colors(&mut self, background: sys::GColor8, foreground: sys::GColor8) {
        unsafe { sys::menu_layer_set_normal_colors(self.raw, background, foreground) };
    }

    pub fn set_highlight_colors(&mut self, background: sys::GColor8, foreground: sys::GColor8) {
        unsafe { sys::menu_layer_set_highlight_colors(self.raw, background, foreground) };
    }

    /// Re-reads row count and contents (call after your data changes).
    pub fn reload(&mut self) {
        unsafe { sys::menu_layer_reload_data(self.raw) };
    }
}

impl Drop for MenuLayer {
    fn drop(&mut self) {
        // Backstop for the documented-unsupported case: dropping a MenuLayer
        // from inside one of its own callbacks would free the state box under
        // the executing closure.
        debug_assert!(
            unsafe { (*self.state).callback_depth } == 0,
            "MenuLayer dropped from inside its own callback (unsupported; see menu_layer.rs)"
        );
        unsafe {
            sys::menu_layer_destroy(self.raw);
            drop(Box::from_raw(self.state));
        }
    }
}

impl AsLayer for MenuLayer {
    fn as_layer_ptr(&self) -> *mut sys::Layer {
        unsafe { sys::menu_layer_get_layer(self.raw) }
    }
}

// --- Trampolines (context = *mut MenuState) ---
//
// Same take/call/restore discipline as the click trampolines (click.rs
// module doc): no borrow of MenuState is live while the user closure runs,
// so the closure may safely reenter the safe API without aliasing UB, and a
// replaced closure cannot be freed out from under itself.

unsafe extern "C" fn cb_get_num_rows(
    _menu: *mut sys::MenuLayer,
    _section: u16,
    context: *mut c_void,
) -> u16 {
    let state = context as *mut MenuState;
    let taken = (*state).num_rows.take();
    if let Some(mut f) = taken {
        (*state).callback_depth += 1;
        let val = f();
        (*state).callback_depth -= 1;
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).num_rows;
        if slot.is_none() {
            *slot = Some(f);
        }
        val
    } else {
        0
    }
}

unsafe extern "C" fn cb_draw_row(
    ctx: *mut sys::GContext,
    cell_layer: *const sys::Layer,
    cell_index: *mut sys::MenuIndex,
    context: *mut c_void,
) {
    let state = context as *mut MenuState;
    let row = (*cell_index).row;
    let taken = (*state).draw_row.take();
    if let Some(mut f) = taken {
        let mut cell = RowCell {
            ctx,
            cell_layer,
            _lifetime: PhantomData,
        };
        (*state).callback_depth += 1;
        f(&mut cell, row);
        (*state).callback_depth -= 1;
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).draw_row;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

unsafe extern "C" fn cb_select_click(
    _menu: *mut sys::MenuLayer,
    cell_index: *mut sys::MenuIndex,
    context: *mut c_void,
) {
    let state = context as *mut MenuState;
    let row = (*cell_index).row;
    let taken = (*state).on_select.take();
    if let Some(mut f) = taken {
        (*state).callback_depth += 1;
        f(row);
        (*state).callback_depth -= 1;
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).on_select;
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

    #[test]
    fn cb_get_num_rows_closure_survives_two_calls() {
        let call_count = Rc::new(Cell::new(0));
        let state = Box::into_raw(Box::new(MenuState {
            num_rows: None,
            draw_row: None,
            on_select: None,
            callback_depth: 0,
        }));

        let c = call_count.clone();
        unsafe { &mut *state }.num_rows = Some(Box::new(move || {
            c.set(c.get() + 1);
            5
        }));

        // First call
        let result1 = unsafe { cb_get_num_rows(core::ptr::null_mut(), 0, state as *mut c_void) };
        assert_eq!(result1, 5);
        assert_eq!(call_count.get(), 1);

        // Second call: restore must have kept the closure
        let result2 = unsafe { cb_get_num_rows(core::ptr::null_mut(), 0, state as *mut c_void) };
        assert_eq!(result2, 5);
        assert_eq!(call_count.get(), 2, "closure should survive restore");

        unsafe {
            drop(Box::from_raw(state));
        }
    }

    #[test]
    fn cb_select_click_closure_survives_two_calls() {
        let call_count = Rc::new(Cell::new(0));
        let last_row = Rc::new(Cell::new(0u16));

        let state = Box::into_raw(Box::new(MenuState {
            num_rows: None,
            draw_row: None,
            on_select: None,
            callback_depth: 0,
        }));

        let c = call_count.clone();
        let r = last_row.clone();
        unsafe { &mut *state }.on_select = Some(Box::new(move |row| {
            c.set(c.get() + 1);
            r.set(row);
        }));

        // First call
        let mut cell_index = sys::MenuIndex { section: 0, row: 3 };
        unsafe {
            cb_select_click(core::ptr::null_mut(), &mut cell_index, state as *mut c_void);
        }
        assert_eq!(call_count.get(), 1);
        assert_eq!(last_row.get(), 3);

        // Second call: restore must have kept the closure
        cell_index.row = 7;
        unsafe {
            cb_select_click(core::ptr::null_mut(), &mut cell_index, state as *mut c_void);
        }
        assert_eq!(call_count.get(), 2, "closure should survive restore");
        assert_eq!(last_row.get(), 7);

        unsafe {
            drop(Box::from_raw(state));
        }
    }

    #[test]
    fn cb_draw_row_closure_survives_two_calls() {
        let call_count = Rc::new(Cell::new(0));
        let state = Box::into_raw(Box::new(MenuState {
            num_rows: None,
            draw_row: None,
            on_select: None,
            callback_depth: 0,
        }));

        let c = call_count.clone();
        unsafe { &mut *state }.draw_row = Some(Box::new(move |_cell: &mut RowCell, _row: u16| {
            c.set(c.get() + 1);
        }));

        // First call
        let mut cell_index = sys::MenuIndex { section: 0, row: 2 };
        unsafe {
            cb_draw_row(
                core::ptr::null_mut(),
                core::ptr::null(),
                &mut cell_index,
                state as *mut c_void,
            );
        }
        assert_eq!(call_count.get(), 1);

        // Second call: restore must have kept the closure
        cell_index.row = 3;
        unsafe {
            cb_draw_row(
                core::ptr::null_mut(),
                core::ptr::null(),
                &mut cell_index,
                state as *mut c_void,
            );
        }
        assert_eq!(call_count.get(), 2, "closure should survive restore");

        unsafe {
            drop(Box::from_raw(state));
        }
    }

    /// THE invariant the module doc promises: a closure that re-registers a
    /// replacement during its own dispatch must WIN -- restore must not
    /// clobber it with the finished closure. (Mirror of
    /// click.rs::restore_does_not_clobber_a_reregistered_handler; supersedes
    /// a duplicate survives-two-calls test that added no coverage.)
    #[test]
    fn reentrant_on_select_replacement_wins() {
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        let state = Box::into_raw(Box::new(MenuState {
            num_rows: None,
            draw_row: None,
            on_select: None,
            callback_depth: 0,
        }));

        let f1 = first.clone();
        let s2 = second.clone();
        unsafe { &mut *state }.on_select = Some(Box::new(move |_row| {
            f1.set(f1.get() + 1);
            // Reentrant replacement: our slot is empty (taken) right now, so
            // this lands in the slot and restore must NOT overwrite it.
            let s2 = s2.clone();
            unsafe { &mut *state }.on_select = Some(Box::new(move |_row| s2.set(s2.get() + 1)));
        }));

        let mut cell_index = sys::MenuIndex { section: 0, row: 0 };
        unsafe {
            cb_select_click(core::ptr::null_mut(), &mut cell_index, state as *mut c_void);
            cb_select_click(core::ptr::null_mut(), &mut cell_index, state as *mut c_void);
        }
        assert_eq!(
            (first.get(), second.get()),
            (1, 1),
            "the reentrantly-registered closure must win over the restore"
        );

        unsafe {
            drop(Box::from_raw(state));
        }
    }

    /// Dynamic row text is the common case -- fitter formats a timestamp and a
    /// stats line per row. Forcing every caller to hand-roll NUL termination
    /// in a hot draw callback invites a panic path where there should be none.
    #[test]
    fn str_overload_nul_terminates_and_truncates() {
        let mut buf = [0u8; 8];
        let c = super::str_to_cstr(&mut buf, "abcdefghij");
        assert_eq!(c.to_bytes(), b"abcdefg", "must truncate to fit the NUL");
    }

    #[test]
    fn str_overload_handles_exact_fit_and_empty() {
        let mut buf = [0u8; 4];
        assert_eq!(super::str_to_cstr(&mut buf, "abc").to_bytes(), b"abc");
        assert_eq!(super::str_to_cstr(&mut buf, "").to_bytes(), b"");
    }

    /// Truncation is by BYTE, matching snprintf. A multi-byte character cut in
    /// half must not produce an invalid CStr or panic -- the SDK renders bytes.
    #[test]
    fn str_overload_truncating_mid_character_does_not_panic() {
        let mut buf = [0u8; 4];
        let c = super::str_to_cstr(&mut buf, "aé…");
        assert!(c.to_bytes().len() <= 3);
    }
}

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

use alloc::boxed::Box;
use core::ffi::c_void;
use core::ffi::CStr;
use core::marker::PhantomData;
use core::ptr;

use crate::layer::AsLayer;
use crate::sys;
use crate::window::Window;
use crate::App;

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
}

type GetNumRowsCallback = Box<dyn FnMut() -> u16>;
type DrawRowCallback = Box<dyn FnMut(&mut RowCell<'_>, u16)>;
type SelectClickCallback = Box<dyn FnMut(u16)>;

struct MenuState {
    num_rows: Option<GetNumRowsCallback>,
    draw_row: Option<DrawRowCallback>,
    on_select: Option<SelectClickCallback>,
}

pub struct MenuLayer {
    raw: *mut sys::MenuLayer,
    state: *mut MenuState, // Box, owned; freed in Drop
}

impl MenuLayer {
    /// Creates a menu layer. Panics if the SDK returns NULL (out of memory).
    pub fn new(_app: &mut App, frame: sys::GRect) -> MenuLayer {
        let raw = unsafe { sys::menu_layer_create(frame) };
        assert!(!raw.is_null(), "menu_layer_create returned NULL");
        let state = Box::into_raw(Box::new(MenuState {
            num_rows: None,
            draw_row: None,
            on_select: None,
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
        let val = f();
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
        f(&mut cell, row);
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
        f(row);
        // Restore only if empty (reentrancy-safe).
        let slot = &mut (*state).on_select;
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}

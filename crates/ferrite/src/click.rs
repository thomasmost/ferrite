//! Click configuration: closures per button, wired through the SDK's
//! context-carrying click config provider.
//!
//! Contract (verified against pebble.h): each window has ONE click context;
//! we use the window's `WindowState` pointer. The provider trampoline runs
//! when the window becomes topmost and subscribes a trampoline per
//! registered button; handlers receive the same context back.

use alloc::boxed::Box;
use core::ffi::c_void;

use crate::sys;
use crate::window::WindowState;

/// The four physical buttons. Discriminants match the SDK's `ButtonId`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Back = 0,
    Up = 1,
    Select = 2,
    Down = 3,
}

pub(crate) const NUM_BUTTONS: usize = 4;

impl Button {
    pub(crate) fn raw(self) -> sys::ButtonId {
        sys::ButtonId(self as u8)
    }

    pub(crate) fn from_raw(id: sys::ButtonId) -> Option<Button> {
        match id.0 {
            0 => Some(Button::Back),
            1 => Some(Button::Up),
            2 => Some(Button::Select),
            3 => Some(Button::Down),
            _ => None,
        }
    }

    const ALL: [Button; NUM_BUTTONS] =
        [Button::Back, Button::Up, Button::Select, Button::Down];
}

type Callback = Box<dyn FnMut() + 'static>;

pub(crate) struct LongClick {
    pub(crate) delay_ms: u16,
    pub(crate) down: Option<Callback>,
    pub(crate) up: Option<Callback>,
}

pub(crate) struct ClickHandlers {
    pub(crate) single: [Option<Callback>; NUM_BUTTONS],
    pub(crate) long: [Option<LongClick>; NUM_BUTTONS],
}

impl ClickHandlers {
    pub(crate) fn new() -> ClickHandlers {
        ClickHandlers {
            single: [None, None, None, None],
            long: [None, None, None, None],
        }
    }

    pub(crate) fn dispatch_single(&mut self, button: Button) {
        if let Some(f) = self.single[button as usize].as_mut() {
            f();
        }
    }

    pub(crate) fn dispatch_long_down(&mut self, button: Button) {
        if let Some(lc) = self.long[button as usize].as_mut() {
            if let Some(f) = lc.down.as_mut() {
                f();
            }
        }
    }

    pub(crate) fn dispatch_long_up(&mut self, button: Button) {
        if let Some(lc) = self.long[button as usize].as_mut() {
            if let Some(f) = lc.up.as_mut() {
                f();
            }
        }
    }
}

// --- Trampolines (context = *mut WindowState) ---

/// Registered via `window_set_click_config_provider_with_context`; the SDK
/// calls it (with our context) whenever the window needs its click config.
pub(crate) unsafe extern "C" fn click_config_provider(context: *mut c_void) {
    let state = &mut *(context as *mut WindowState);
    for button in Button::ALL {
        if state.clicks.single[button as usize].is_some() {
            sys::window_single_click_subscribe(button.raw(), Some(on_single_click));
        }
        if let Some(lc) = state.clicks.long[button as usize].as_ref() {
            sys::window_long_click_subscribe(
                button.raw(),
                lc.delay_ms,
                Some(on_long_down),
                Some(on_long_up),
            );
        }
    }
}

unsafe extern "C" fn on_single_click(rec: sys::ClickRecognizerRef, context: *mut c_void) {
    let state = &mut *(context as *mut WindowState);
    if let Some(b) = Button::from_raw(sys::click_recognizer_get_button_id(rec)) {
        state.clicks.dispatch_single(b);
    }
}

unsafe extern "C" fn on_long_down(rec: sys::ClickRecognizerRef, context: *mut c_void) {
    let state = &mut *(context as *mut WindowState);
    if let Some(b) = Button::from_raw(sys::click_recognizer_get_button_id(rec)) {
        state.clicks.dispatch_long_down(b);
    }
}

unsafe extern "C" fn on_long_up(rec: sys::ClickRecognizerRef, context: *mut c_void) {
    let state = &mut *(context as *mut WindowState);
    if let Some(b) = Button::from_raw(sys::click_recognizer_get_button_id(rec)) {
        state.clicks.dispatch_long_up(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn button_raw_roundtrip() {
        for b in [Button::Back, Button::Up, Button::Select, Button::Down] {
            assert_eq!(Button::from_raw(b.raw()), Some(b));
        }
        assert_eq!(Button::from_raw(crate::sys::ButtonId(9)), None);
    }

    #[test]
    fn dispatch_runs_only_the_registered_button() {
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let mut ch = ClickHandlers::new();
        ch.single[Button::Select as usize] = Some(Box::new(move || h.set(h.get() + 1)));

        ch.dispatch_single(Button::Up); // unregistered: no-op
        assert_eq!(hits.get(), 0);
        ch.dispatch_single(Button::Select);
        ch.dispatch_single(Button::Select);
        assert_eq!(hits.get(), 2);
    }

    #[test]
    fn long_click_dispatches_down_and_up_separately() {
        let downs = Rc::new(Cell::new(0));
        let ups = Rc::new(Cell::new(0));
        let (d, u) = (downs.clone(), ups.clone());
        let mut ch = ClickHandlers::new();
        ch.long[Button::Select as usize] = Some(LongClick {
            delay_ms: 500,
            down: Some(Box::new(move || d.set(d.get() + 1))),
            up: Some(Box::new(move || u.set(u.get() + 1))),
        });

        ch.dispatch_long_down(Button::Select);
        assert_eq!((downs.get(), ups.get()), (1, 0));
        ch.dispatch_long_up(Button::Select);
        assert_eq!((downs.get(), ups.get()), (1, 1));
        ch.dispatch_long_down(Button::Back); // unregistered: no-op
        assert_eq!(downs.get(), 1);
    }
}

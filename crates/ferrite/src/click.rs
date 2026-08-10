//! Click configuration: closures per button, wired through the SDK's
//! context-carrying click config provider.
//!
//! Contract (verified against pebble.h): each window has ONE click context;
//! we use the window's `WindowState` pointer. The provider trampoline runs
//! every time the window becomes visible AND -- per pebble.h:5213 -- runs
//! synchronously whenever `window_set_click_config_provider(_with_context)`
//! is called on an already-visible window. That second half is load-bearing:
//! it is why `Window::on_click` re-installing the provider on every
//! registration makes handlers registered AFTER `push()` live immediately
//! instead of inert until the next push. Do not "deduplicate" the install.
//!
//! Soundness: the platform is single-threaded, but the SDK DOES reenter
//! (the provider case above), and user closures can reach back into this
//! API (e.g. re-registering a handler from inside a click handler, via
//! `Rc<RefCell<Window>>`). Trampolines therefore never hold a borrow of
//! `WindowState` across a user-closure call: they take the closure out of
//! its slot (ending the borrow), run it borrow-free, and restore it only if
//! the slot is still empty -- so a reentrant re-registration wins and the
//! replaced closure (the one currently executing, owned by the trampoline's
//! stack frame) is dropped safely when the call returns.
//!
//! Residual contract: dropping a `Window` from inside one of its own
//! callbacks is not supported -- the state box would be freed under the
//! executing closure. This cannot be written without deliberate
//! `Rc`-into-own-closure gymnastics. As a backstop, every trampoline tracks
//! `WindowState::callback_depth` and `Drop for Window` debug-asserts it is
//! zero, turning the use-after-free into a deterministic panic in debug
//! builds. (Caveat: `pebble build` compiles `--release`, so today the assert
//! is active only in host/debug builds -- it documents the contract and
//! guards any future debug-on-device workflow, not the shipped `.pbw`.)

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

    const ALL: [Button; NUM_BUTTONS] = [Button::Back, Button::Up, Button::Select, Button::Down];
}

type Callback = Box<dyn FnMut() + 'static>;

/// A registered single-click handler.
///
/// The wrapper exists so REGISTRATION is a different fact from the closure
/// being momentarily out of its slot: while a handler executes, `take_*` has
/// emptied the inner `cb`, but the outer `Option<SingleClick>` stays `Some`.
/// The provider reads the outer option, and the SDK clears every
/// subscription before each provider run -- if presence were derived from
/// the closure slot, a reentrant registration (which re-runs the provider
/// synchronously, pebble.h:5213) would see the running button as
/// unregistered and permanently unsubscribe it.
pub(crate) struct SingleClick {
    pub(crate) cb: Option<Callback>,
}

pub(crate) struct LongClick {
    pub(crate) delay_ms: u16,
    pub(crate) down: Option<Callback>,
    pub(crate) up: Option<Callback>,
}

pub(crate) struct ClickHandlers {
    pub(crate) single: [Option<SingleClick>; NUM_BUTTONS],
    pub(crate) long: [Option<LongClick>; NUM_BUTTONS],
}

impl ClickHandlers {
    pub(crate) fn new() -> ClickHandlers {
        ClickHandlers {
            single: [None, None, None, None],
            long: [None, None, None, None],
        }
    }

    // take_*/restore_* are the reentrancy-safe dispatch primitives (see the
    // module doc). The trampolines call them in SEPARATE borrow scopes so no
    // &mut WindowState is live while the user closure runs; restore_* only
    // writes into an empty inner slot so a reentrant re-registration wins.
    // Both tables keep their OUTER option occupied during dispatch, so the
    // provider's registration predicate stays truthful throughout.

    pub(crate) fn set_single(&mut self, button: Button, cb: Callback) {
        self.single[button as usize] = Some(SingleClick { cb: Some(cb) });
    }

    pub(crate) fn take_single(&mut self, button: Button) -> Option<Callback> {
        self.single[button as usize].as_mut()?.cb.take()
    }

    pub(crate) fn restore_single(&mut self, button: Button, cb: Callback) {
        if let Some(sc) = self.single[button as usize].as_mut() {
            if sc.cb.is_none() {
                sc.cb = Some(cb);
            }
        }
    }

    pub(crate) fn take_long_down(&mut self, button: Button) -> Option<Callback> {
        self.long[button as usize].as_mut()?.down.take()
    }

    pub(crate) fn restore_long_down(&mut self, button: Button, cb: Callback) {
        if let Some(lc) = self.long[button as usize].as_mut() {
            if lc.down.is_none() {
                lc.down = Some(cb);
            }
        }
    }

    pub(crate) fn take_long_up(&mut self, button: Button) -> Option<Callback> {
        self.long[button as usize].as_mut()?.up.take()
    }

    pub(crate) fn restore_long_up(&mut self, button: Button, cb: Callback) {
        if let Some(lc) = self.long[button as usize].as_mut() {
            if lc.up.is_none() {
                lc.up = Some(cb);
            }
        }
    }

    // Safe-code equivalents of the trampolines' take/call/restore sequence,
    // for host tests. The trampolines cannot use these directly: a method
    // holds `&mut self` across the closure call, which is exactly the borrow
    // the raw-pointer reentry path must not overlap with.
    #[cfg(test)]
    pub(crate) fn dispatch_single(&mut self, button: Button) {
        if let Some(mut f) = self.take_single(button) {
            f();
            self.restore_single(button, f);
        }
    }

    #[cfg(test)]
    pub(crate) fn dispatch_long_down(&mut self, button: Button) {
        if let Some(mut f) = self.take_long_down(button) {
            f();
            self.restore_long_down(button, f);
        }
    }

    #[cfg(test)]
    pub(crate) fn dispatch_long_up(&mut self, button: Button) {
        if let Some(mut f) = self.take_long_up(button) {
            f();
            self.restore_long_up(button, f);
        }
    }
}

// --- Trampolines (context = *mut WindowState) ---

/// Registered via `window_set_click_config_provider_with_context`; the SDK
/// calls it (with our context) whenever the window needs its click config --
/// including synchronously from the install call itself when the window is
/// already visible (pebble.h:5213). The SDK clears all previous
/// subscriptions before each run, so this must subscribe every button that
/// is REGISTERED -- which is why registration is tracked by the outer
/// options, not by whether a closure currently sits in its slot.
///
/// Shared reference because this function only reads; it needs no
/// exclusivity, so taking `&mut` here would claim more than it uses.
pub(crate) unsafe extern "C" fn click_config_provider(context: *mut c_void) {
    let state = &*(context as *const WindowState);
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

// Each handler trampoline runs the take/call/restore sequence with every
// borrow confined to its own statement: no reference into WindowState exists
// while the user closure runs, so the closure may safely reenter the safe
// API (re-register handlers, push windows) without aliasing UB, and a
// replaced closure cannot be freed while it is executing (the trampoline's
// local owns it until the call returns).

unsafe extern "C" fn on_single_click(rec: sys::ClickRecognizerRef, context: *mut c_void) {
    let state = context as *mut WindowState;
    if let Some(b) = Button::from_raw(sys::click_recognizer_get_button_id(rec)) {
        let taken = (*state).clicks.take_single(b);
        if let Some(mut f) = taken {
            (*state).callback_depth += 1;
            f();
            (*state).callback_depth -= 1;
            (*state).clicks.restore_single(b, f);
        }
    }
}

unsafe extern "C" fn on_long_down(rec: sys::ClickRecognizerRef, context: *mut c_void) {
    let state = context as *mut WindowState;
    if let Some(b) = Button::from_raw(sys::click_recognizer_get_button_id(rec)) {
        let taken = (*state).clicks.take_long_down(b);
        if let Some(mut f) = taken {
            (*state).callback_depth += 1;
            f();
            (*state).callback_depth -= 1;
            (*state).clicks.restore_long_down(b, f);
        }
    }
}

unsafe extern "C" fn on_long_up(rec: sys::ClickRecognizerRef, context: *mut c_void) {
    let state = context as *mut WindowState;
    if let Some(b) = Button::from_raw(sys::click_recognizer_get_button_id(rec)) {
        let taken = (*state).clicks.take_long_up(b);
        if let Some(mut f) = taken {
            (*state).callback_depth += 1;
            f();
            (*state).callback_depth -= 1;
            (*state).clicks.restore_long_up(b, f);
        }
    }
}

/// Host tests for the provider trampoline itself: the three SDK symbols it
/// (and the handler trampolines it references) need are stubbed with
/// `#[no_mangle]` definitions, which the host linker uses to satisfy the
/// bindgen `extern` declarations. This is the only place the coupling
/// between the registration predicate and the provider is machine-checked.
#[cfg(test)]
mod provider_tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static SUBSCRIBED_SINGLE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static SUBSCRIBED_LONG: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    #[no_mangle]
    extern "C" fn window_single_click_subscribe(button: sys::ButtonId, _h: sys::ClickHandler) {
        SUBSCRIBED_SINGLE.with(|s| s.borrow_mut().push(button.0));
    }

    #[no_mangle]
    extern "C" fn window_long_click_subscribe(
        button: sys::ButtonId,
        _delay_ms: u16,
        _down: sys::ClickHandler,
        _up: sys::ClickHandler,
    ) {
        SUBSCRIBED_LONG.with(|s| s.borrow_mut().push(button.0));
    }

    #[no_mangle]
    extern "C" fn click_recognizer_get_button_id(_r: sys::ClickRecognizerRef) -> sys::ButtonId {
        sys::ButtonId(0)
    }

    fn fresh_state() -> WindowState {
        WindowState {
            on_load: None,
            on_unload: None,
            clicks: ClickHandlers::new(),
            callback_depth: 0,
        }
    }

    /// THE regression test for the cycle-2 Critical: the provider must
    /// subscribe a button whose closure is momentarily taken out for
    /// dispatch. The SDK clears all subscriptions before each provider run,
    /// so if the provider's predicate saw the taken slot as unregistered, a
    /// reentrant registration mid-dispatch would permanently unsubscribe
    /// the running button (reproduced on the emulator: press 1 works,
    /// press 2 dead).
    #[test]
    fn provider_subscribes_button_whose_handler_is_taken() {
        let mut state = fresh_state();
        state.clicks.set_single(Button::Select, Box::new(|| {}));

        let taken = state
            .clicks
            .take_single(Button::Select)
            .expect("registered");
        SUBSCRIBED_SINGLE.with(|s| s.borrow_mut().clear());
        unsafe { click_config_provider(&mut state as *mut WindowState as *mut core::ffi::c_void) };
        assert!(
            SUBSCRIBED_SINGLE.with(|s| s.borrow().contains(&(Button::Select as u8))),
            "provider must still subscribe a button whose handler is mid-dispatch"
        );
        state.clicks.restore_single(Button::Select, taken);
    }

    /// The provider mirrors registration exactly: registered buttons are
    /// subscribed (single and long independently), unregistered ones are not.
    #[test]
    fn provider_subscribes_exactly_the_registered_buttons() {
        let mut state = fresh_state();
        state.clicks.set_single(Button::Up, Box::new(|| {}));
        state.clicks.long[Button::Down as usize] = Some(LongClick {
            delay_ms: 700,
            down: Some(Box::new(|| {})),
            up: None,
        });

        SUBSCRIBED_SINGLE.with(|s| s.borrow_mut().clear());
        SUBSCRIBED_LONG.with(|s| s.borrow_mut().clear());
        unsafe { click_config_provider(&mut state as *mut WindowState as *mut core::ffi::c_void) };
        SUBSCRIBED_SINGLE.with(|s| {
            assert_eq!(*s.borrow(), vec![Button::Up as u8]);
        });
        SUBSCRIBED_LONG.with(|s| {
            assert_eq!(*s.borrow(), vec![Button::Down as u8]);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn button_raw_roundtrip() {
        // Iterate ALL (the array the provider trampoline walks), not a
        // hand-copied list: a typo in ALL would silently skip a button's
        // subscription, and this is the test that must catch it.
        for b in Button::ALL {
            assert_eq!(Button::from_raw(b.raw()), Some(b));
        }
        assert_eq!(Button::from_raw(crate::sys::ButtonId(9)), None);
    }

    #[test]
    fn all_covers_every_button_exactly_once() {
        let mut seen = [false; NUM_BUTTONS];
        for b in Button::ALL {
            let i = b as usize;
            assert!(!seen[i], "{b:?} appears twice in Button::ALL");
            seen[i] = true;
        }
        assert_eq!(seen, [true; NUM_BUTTONS], "Button::ALL is missing a button");
    }

    #[test]
    fn single_dispatch_ignores_long_only_button() {
        let downs = Rc::new(Cell::new(0));
        let d = downs.clone();
        let mut ch = ClickHandlers::new();
        ch.long[Button::Up as usize] = Some(LongClick {
            delay_ms: 500,
            down: Some(Box::new(move || d.set(d.get() + 1))),
            up: None,
        });
        ch.dispatch_single(Button::Up); // only a long handler registered
        assert_eq!(downs.get(), 0);
    }

    /// The reentrancy-safe primitives: take empties the inner slot, restore
    /// refills an empty inner slot, and restore NEVER clobbers an occupied
    /// one (a reentrant re-registration must win over the finished closure).
    #[test]
    fn restore_does_not_clobber_a_reregistered_handler() {
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        let mut ch = ClickHandlers::new();

        let f1 = first.clone();
        ch.set_single(Button::Select, Box::new(move || f1.set(f1.get() + 1)));

        // Simulate the trampoline sequence with a reentrant registration
        // happening while the closure is out of its slot.
        let mut taken = ch.take_single(Button::Select).expect("registered");
        taken();
        let s2 = second.clone();
        ch.set_single(Button::Select, Box::new(move || s2.set(s2.get() + 1)));
        ch.restore_single(Button::Select, taken); // must NOT clobber

        ch.dispatch_single(Button::Select);
        assert_eq!(
            (first.get(), second.get()),
            (1, 1),
            "the re-registered handler must win after restore"
        );
    }

    /// While a handler executes (taken out of its slot), the button must
    /// still read as REGISTERED: the provider re-runs synchronously on any
    /// reentrant registration, the SDK clears all subscriptions first, and
    /// its predicate is the outer option. If take emptied the outer option,
    /// a reentrant registration would permanently unsubscribe the running
    /// button (observed on the emulator before this shape existed).
    #[test]
    fn registration_stays_visible_while_handler_is_taken() {
        let mut ch = ClickHandlers::new();
        ch.set_single(Button::Select, Box::new(|| {}));

        let taken = ch.take_single(Button::Select).expect("registered");
        assert!(
            ch.single[Button::Select as usize].is_some(),
            "provider's registration predicate went false during dispatch"
        );
        ch.restore_single(Button::Select, taken);
        assert!(ch.single[Button::Select as usize]
            .as_ref()
            .is_some_and(|sc| sc.cb.is_some()));
    }

    /// The long-click clobber guards, mirror of the single-click test: a
    /// reentrant re-registration while the down (or up) closure is out of
    /// its slot must win over the restore.
    #[test]
    fn restore_long_does_not_clobber_reregistered_handlers() {
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        let mut ch = ClickHandlers::new();

        let f1 = first.clone();
        ch.long[Button::Select as usize] = Some(LongClick {
            delay_ms: 500,
            down: Some(Box::new(move || f1.set(f1.get() + 1))),
            up: None,
        });

        let mut taken = ch.take_long_down(Button::Select).expect("registered");
        taken();
        let s2 = second.clone();
        ch.long[Button::Select as usize].as_mut().unwrap().down =
            Some(Box::new(move || s2.set(s2.get() + 1)));
        ch.restore_long_down(Button::Select, taken); // must NOT clobber
        ch.dispatch_long_down(Button::Select);
        assert_eq!((first.get(), second.get()), (1, 1));

        // Same for the up slot.
        let third = Rc::new(Cell::new(0));
        let fourth = Rc::new(Cell::new(0));
        let t3 = third.clone();
        ch.long[Button::Select as usize].as_mut().unwrap().up =
            Some(Box::new(move || t3.set(t3.get() + 1)));
        let mut taken_up = ch.take_long_up(Button::Select).expect("registered");
        taken_up();
        let f4 = fourth.clone();
        ch.long[Button::Select as usize].as_mut().unwrap().up =
            Some(Box::new(move || f4.set(f4.get() + 1)));
        ch.restore_long_up(Button::Select, taken_up); // must NOT clobber
        ch.dispatch_long_up(Button::Select);
        assert_eq!((third.get(), fourth.get()), (1, 1));
    }

    #[test]
    fn dispatch_runs_only_the_registered_button() {
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let mut ch = ClickHandlers::new();
        ch.set_single(Button::Select, Box::new(move || h.set(h.get() + 1)));

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

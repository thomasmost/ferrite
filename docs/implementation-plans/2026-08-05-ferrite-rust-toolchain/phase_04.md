# Ferrite Rust Toolchain Implementation Plan — Phase 4: Safe UI core

**Goal:** Windows, text, and buttons as safe closure-based APIs — enough to build multi-screen button-driven apps; hello upgraded to two windows with SELECT navigation.

**Architecture:** Each `Window` wrapper owns a boxed `WindowState` (load/unload closures + per-button click closures) attached via `window_set_user_data`; private `extern "C"` trampolines recover the state from the SDK's window pointer / click context and dispatch. Click config uses `window_set_click_config_provider_with_context` — the SDK passes one context pointer per window to both the provider and (by default) every `ClickHandler`. `TextLayer` gains setters, system fonts, and structural text-buffer ownership via an internal `TextBuf`. Pure bookkeeping types get host unit tests (`cargo test -p ferrite`).

**Tech Stack:** Rust `alloc` (Box, CString) over the Phase 3 allocator; host tests via `cfg_attr(not(test), no_std)`.

**Scope:** Phase 4 of 7 from `docs/design-plans/2026-08-05-ferrite-rust-toolchain.md`.

**Codebase verified:** 2026-08-05. Verified `pebble.h` signatures (line numbers from SDK 4.17): `window_set_click_config_provider_with_context(Window*, ClickConfigProvider, void*)` (5575) — the context defaults to the window and is what `ClickHandler`s receive as their second arg; subscribe calls (`window_single_click_subscribe` 5638, `window_long_click_subscribe(ButtonId, uint16_t delay_ms, ClickHandler down, ClickHandler up)` 5669) take **no window argument** and must be called from inside the provider; `ClickConfigProvider` = `void (*)(void *context)` (5224); `ClickHandler` = `void (*)(ClickRecognizerRef, void *context)` (5211); `click_recognizer_get_button_id` (5237); `ButtonId` BACK=0, UP=1, SELECT=2, DOWN=3. `window_set_user_data`/`window_get_user_data` (5620/5626). `text_layer_set_font/text_color/background_color/text_alignment` (6720/6708/6702/6726); `GFont fonts_get_system_font(const char*)` (4858); `FONT_KEY_*` are `#define` string constants in `pebble_fonts.h` (bindgen emits them as `&[u8; N]` byte strings with trailing NUL). `window_stack_pop(bool)` returns the popped window; `window_stack_remove(Window*, bool)` returns bool.

---

## Context for the implementing engineer (read first)

- **Click context flow (verified in SDK docs):** one context pointer per window. We pass the `WindowState` pointer; the provider trampoline runs inside the SDK when the window becomes topmost and calls `window_single_click_subscribe`/`window_long_click_subscribe` for every button that has a registered closure; those handlers then receive the same `WindowState` pointer and dispatch by `click_recognizer_get_button_id`.
- **Watchface → watchapp.** Watchfaces don't receive button clicks (the system owns the buttons). This phase flips `examples/hello`'s `package.json` to `"watchface": false`. On a watchapp, BACK on the root window exits the app and BACK on pushed windows pops them automatically — so two-window navigation needs only a SELECT handler.
- **Host tests.** `ferrite` switches to `#![cfg_attr(not(test), no_std)]` so `cargo test -p ferrite` runs the unit tests on macOS with std's test harness (the panic handler and allocator are already gated on `target_os = "none"`, so they don't clash). Tests cover pure bookkeeping only (no SDK calls): color packing, text-buffer ownership, click dispatch tables.
- **Reentrancy note (document, don't "fix"):** trampolines take `&mut WindowState` from a raw pointer. This is sound because the platform is single-threaded and the SDK never nests these callbacks; do not add locking.
- **Drop order in `Window::drop`:** `window_destroy` first (it may fire the unload handler, which touches the state), then free the state box.
- The `app!` state-keeping contract is unchanged: everything built in setup must be returned from the block (closures capturing windows keep them alive too, since the closure lives inside another window's state).

---

<!-- START_SUBCOMPONENT_A (tasks 1-3) -->
<!-- START_TASK_1 -->
### Task 1: Crate groundwork — testable no_std, alloc, module skeleton

**Files:**
- Modify: `crates/ferrite/src/lib.rs`
- Create: `crates/ferrite/src/types.rs` (placeholder)
- Create: `crates/ferrite/src/text_buf.rs` (placeholder)
- Create: `crates/ferrite/src/click.rs` (placeholder)

**Step 1: Update the crate root**

In `crates/ferrite/src/lib.rs`:

1. Replace the `#![no_std]` line with:
```rust
#![cfg_attr(not(test), no_std)]
```
   **Already done in Phase 1** (commit 5e2c34c): this was pulled forward so
   `FixedBuf`'s UTF-8 truncation arithmetic in `panic.rs` could be unit-tested
   on the host after it regressed twice. `panic.rs` is now declared
   `#[cfg(any(target_os = "none", test))]` with only `#[panic_handler]` gated
   on `target_os = "none"`. Verify the line is present and move on — no edit
   needed.
2. Add below the attribute:
```rust
extern crate alloc;
```
3. Add the new modules alongside the existing declarations (`text_buf`, `click`, `types` are created in the next tasks — add all three declarations now, with placeholder files so the crate compiles):
```rust
pub mod click;
mod text_buf;
pub mod types;
```

Create placeholder files:

`crates/ferrite/src/types.rs`:
```rust
//! Safe value types (populated in this phase).
```

`crates/ferrite/src/text_buf.rs`:
```rust
//! Text-buffer ownership bookkeeping (populated in this phase).
```

`crates/ferrite/src/click.rs`:
```rust
//! Click configuration (populated in this phase).
```

**Step 2: Verify**

```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo test -p ferrite
```
Expected: check passes; `cargo test` compiles and runs 0 tests successfully.

**Step 3: Commit**

```bash
git add crates/ferrite/src
git commit -m "chore(ferrite): host-testable no_std setup and module skeleton"
```
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: types module with host tests

**Files:**
- Modify: `crates/ferrite/src/types.rs`

**Step 1: Write the failing test**

`crates/ferrite/src/types.rs` (replace placeholder):
```rust
//! Safe value types: re-exports of the `#[repr(C)]` SDK value types and
//! their const constructors. They cross the FFI boundary untranslated.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_from_rgb_packs_two_bit_channels() {
        // argb bit layout: a[7:6] r[5:4] g[3:2] b[1:0]
        assert_eq!(unsafe { GColorFromRGB(255, 0, 0).argb }, 0b1111_0000);
        assert_eq!(unsafe { GColorFromRGB(0, 255, 0).argb }, 0b1100_1100);
        assert_eq!(unsafe { GColorFromRGB(0, 0, 255).argb }, 0b1100_0011);
        assert_eq!(unsafe { GColorFromHEX(0xFFFFFF).argb }, 0b1111_1111);
        assert_eq!(unsafe { GColorFromRGBA(0, 0, 0, 0).argb }, 0b0000_0000);
    }

    #[test]
    fn grect_constructor_and_layout() {
        let r = GRect(1, 2, 3, 4);
        assert_eq!(r.origin.x, 1);
        assert_eq!(r.origin.y, 2);
        assert_eq!(r.size.w, 3);
        assert_eq!(r.size.h, 4);
        assert_eq!(core::mem::size_of::<GRect>(), 8);
    }
}
```

**Step 2: Run the test to verify it fails**

```bash
cargo test -p ferrite
```
Expected: FAIL to compile — `GColorFromRGB` etc. not in scope (no re-exports yet).

**Step 3: Add the re-exports**

Insert above the `#[cfg(test)]` module:
```rust
pub use crate::sys::{
    GColor, GColor8, GColorClear, GColorBlack, GColorWhite, GColorFromHEX,
    GColorFromRGB, GColorFromRGBA, GPoint, GRect, GSize,
};
```
(The `use` of `GRect`/`GPoint`/`GSize` re-exports both the type and the same-named const constructor fn — one path covers both namespaces. The full 65-color table stays reachable as `ferrite::sys::GColor*`.)

**Step 4: Run the test to verify it passes**

```bash
cargo test -p ferrite
```
Expected: PASS (2 tests).

**Step 5: Commit**

```bash
git add crates/ferrite/src/types.rs
git commit -m "feat(ferrite): types module re-exporting safe value types"
```
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: TextBuf — text-buffer ownership bookkeeping

**Files:**
- Modify: `crates/ferrite/src/text_buf.rs`

**Step 1: Write the failing tests**

`crates/ferrite/src/text_buf.rs` (replace placeholder):
```rust
//! Ownership bookkeeping for text the SDK stores by raw pointer: the SDK
//! keeps a `const char*` without copying, so the wrapper must own the
//! storage for as long as the SDK might read it.

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
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p ferrite
```
Expected: FAIL to compile — `TextBuf` not defined.

**Step 3: Implement TextBuf**

Insert above the test module:
```rust
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
    pub(crate) fn set_owned(&mut self, s: &str) -> *const c_char {
        let c = CString::new(s)
            .unwrap_or_else(|_| CString::new("<text contained NUL>").unwrap());
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
```

**Step 4: Run tests to verify they pass**

```bash
cargo test -p ferrite
```
Expected: PASS (6 tests total with Task 2's).

**Step 5: Commit**

```bash
git add crates/ferrite/src/text_buf.rs
git commit -m "feat(ferrite): TextBuf ownership bookkeeping with host tests"
```
<!-- END_TASK_3 -->
<!-- END_SUBCOMPONENT_A -->

<!-- START_SUBCOMPONENT_B (tasks 4-6) -->
<!-- START_TASK_4 -->
### Task 4: click module — Button, handler tables, trampolines

**Files:**
- Modify: `crates/ferrite/src/click.rs`
- Modify: `crates/ferrite/src/window.rs` (add the `WindowState` struct the trampolines dereference)

**Step 1: Write the failing tests**

`crates/ferrite/src/click.rs` (replace placeholder):
```rust
//! Click configuration: closures per button, wired through the SDK's
//! context-carrying click config provider.
//!
//! Contract (verified against pebble.h): each window has ONE click context;
//! we use the window's `WindowState` pointer. The provider trampoline runs
//! when the window becomes topmost and subscribes a trampoline per
//! registered button; handlers receive the same context back.

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
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p ferrite
```
Expected: FAIL to compile — `Button`, `ClickHandlers`, `LongClick` undefined.

**Step 3: Implement the module**

Insert above the test module:
```rust
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
```

The trampolines reference `crate::window::WindowState`, which does not exist yet. Add it to `crates/ferrite/src/window.rs` now (top of the file, below the existing `use` items) — this is the FINAL struct Task 5's rewrite keeps, not throwaway code:
```rust
use alloc::boxed::Box;

pub(crate) struct WindowState {
    pub(crate) on_load: Option<Box<dyn FnMut()>>,
    pub(crate) on_unload: Option<Box<dyn FnMut()>>,
    pub(crate) clicks: crate::click::ClickHandlers,
}
```
(The struct is unused by the Phase 1-era `Window` code still in that file; that's fine — Task 5 replaces the file and wires it up. If the compiler warns about the unused fields, that warning disappears in Task 5.)

**Step 4: Run tests to verify they pass**

```bash
cargo test -p ferrite
```
Expected: PASS (9 tests total).

**Step 5: Commit**

```bash
git add crates/ferrite/src/click.rs crates/ferrite/src/window.rs
git commit -m "feat(ferrite): click module with per-button closures and trampolines"
```
<!-- END_TASK_4 -->

<!-- START_TASK_5 -->
### Task 5: Window rewrite — load/unload closures, clicks, stack API

**Files:**
- Modify: `crates/ferrite/src/window.rs` (replace entirely)

**Step 1: Rewrite the module**

`crates/ferrite/src/window.rs`:
```rust
//! Owned wrapper over the SDK `Window`, with closure-based handlers.
//!
//! Each window owns a boxed `WindowState` attached via
//! `window_set_user_data`; `extern "C"` trampolines recover it from the
//! window pointer (load/unload) or the click context. Sound because the
//! platform is single-threaded and the SDK never nests these callbacks.

use alloc::boxed::Box;

use crate::click::{self, Button, ClickHandlers, LongClick};
use crate::sys;
use crate::App;

pub(crate) struct WindowState {
    pub(crate) on_load: Option<Box<dyn FnMut()>>,
    pub(crate) on_unload: Option<Box<dyn FnMut()>>,
    pub(crate) clicks: ClickHandlers,
}

pub struct Window {
    raw: *mut sys::Window,
    state: *mut WindowState, // Box, owned; freed in Drop after window_destroy
}

impl Window {
    /// Creates a new window. Panics if the SDK returns NULL (out of memory).
    pub fn new(_app: &mut App) -> Window {
        let raw = unsafe { sys::window_create() };
        assert!(!raw.is_null(), "window_create returned NULL");
        let state = Box::into_raw(Box::new(WindowState {
            on_load: None,
            on_unload: None,
            clicks: ClickHandlers::new(),
        }));
        unsafe {
            sys::window_set_user_data(raw, state.cast());
            sys::window_set_window_handlers(
                raw,
                sys::WindowHandlers {
                    load: Some(on_window_load),
                    appear: None,
                    disappear: None,
                    unload: Some(on_window_unload),
                },
            );
        }
        Window { raw, state }
    }

    fn state_mut(&mut self) -> &mut WindowState {
        unsafe { &mut *self.state }
    }

    /// Runs when the SDK loads the window (each time it enters the stack).
    pub fn on_load(&mut self, f: impl FnMut() + 'static) {
        self.state_mut().on_load = Some(Box::new(f));
    }

    /// Runs when the SDK unloads the window.
    pub fn on_unload(&mut self, f: impl FnMut() + 'static) {
        self.state_mut().on_unload = Some(Box::new(f));
    }

    /// Registers a single-click handler for a button.
    pub fn on_click(&mut self, button: Button, f: impl FnMut() + 'static) {
        self.state_mut().clicks.single[button as usize] = Some(Box::new(f));
        self.install_click_provider();
    }

    /// Registers a long-press handler (fires on press after `delay_ms`).
    pub fn on_long_click(&mut self, button: Button, delay_ms: u16, f: impl FnMut() + 'static) {
        let entry = self.state_mut().clicks.long[button as usize]
            .get_or_insert_with(|| LongClick {
                delay_ms,
                down: None,
                up: None,
            });
        entry.delay_ms = delay_ms;
        entry.down = Some(Box::new(f));
        self.install_click_provider();
    }

    /// Registers a long-press release handler (fires when the button is
    /// released after a long press of `delay_ms`).
    pub fn on_long_click_up(&mut self, button: Button, delay_ms: u16, f: impl FnMut() + 'static) {
        let entry = self.state_mut().clicks.long[button as usize]
            .get_or_insert_with(|| LongClick {
                delay_ms,
                down: None,
                up: None,
            });
        entry.delay_ms = delay_ms;
        entry.up = Some(Box::new(f));
        self.install_click_provider();
    }

    fn install_click_provider(&mut self) {
        unsafe {
            sys::window_set_click_config_provider_with_context(
                self.raw,
                Some(click::click_config_provider),
                self.state.cast(),
            );
        }
    }

    pub fn set_background_color(&mut self, color: sys::GColor8) {
        unsafe { sys::window_set_background_color(self.raw, color) };
    }

    /// Bounds of the window's root layer.
    pub fn bounds(&self) -> sys::GRect {
        unsafe { sys::layer_get_bounds(sys::window_get_root_layer(self.raw)) }
    }

    /// Adds a text layer as a child of the window's root layer. The child
    /// must outlive its time in the window (keep both in your app state).
    pub fn add_child(&mut self, child: &crate::text_layer::TextLayer) {
        unsafe {
            sys::layer_add_child(
                sys::window_get_root_layer(self.raw),
                child.as_layer_ptr(),
            );
        }
    }

    /// Pushes the window onto the window stack, making it visible.
    pub fn push(&mut self, animated: bool) {
        unsafe { sys::window_stack_push(self.raw, animated) };
    }

    /// Removes this window from the stack (visible or not).
    pub fn remove_from_stack(&mut self, animated: bool) -> bool {
        unsafe { sys::window_stack_remove(self.raw, animated) }
    }

    pub fn is_loaded(&self) -> bool {
        unsafe { sys::window_is_loaded(self.raw) }
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            // Destroy first: it can fire the unload handler, which reads state.
            sys::window_destroy(self.raw);
            drop(Box::from_raw(self.state));
        }
    }
}

/// Pops the topmost window off the stack.
pub fn stack_pop(animated: bool) {
    unsafe {
        sys::window_stack_pop(animated);
    }
}

// --- Window handler trampolines (state via window_get_user_data) ---

unsafe extern "C" fn on_window_load(window: *mut sys::Window) {
    let state = sys::window_get_user_data(window) as *mut WindowState;
    if let Some(f) = (*state).on_load.as_mut() {
        f();
    }
}

unsafe extern "C" fn on_window_unload(window: *mut sys::Window) {
    let state = sys::window_get_user_data(window) as *mut WindowState;
    if let Some(f) = (*state).on_unload.as_mut() {
        f();
    }
}
```

(If Task 4 already added the temporary `WindowState` definition, this rewrite replaces the whole file, keeping that struct — no duplicate.)

**Step 2: Verify**

```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo test -p ferrite
```
Expected: both pass. (`examples/hello` still calls `push()` with no argument — fixed in Task 7.)

**Step 3: Commit**

```bash
git add crates/ferrite/src/window.rs
git commit -m "feat(ferrite): window handlers, click wiring, and stack API"
```
<!-- END_TASK_5 -->

<!-- START_TASK_6 -->
### Task 6: TextLayer upgrade — setters, fonts, owned text

**Files:**
- Modify: `crates/ferrite/src/text_layer.rs` (replace entirely)

**Step 1: Rewrite the module**

`crates/ferrite/src/text_layer.rs`:
```rust
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
/// Panics if `key` is not NUL-terminated.
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

    pub(crate) fn as_layer_ptr(&self) -> *mut sys::Layer {
        unsafe { sys::text_layer_get_layer(self.raw) }
    }
}

impl Drop for TextLayer {
    fn drop(&mut self) {
        unsafe { sys::text_layer_destroy(self.raw) };
    }
}
```

**Step 2: Verify**

```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo test -p ferrite
```
Expected: both pass.

**Step 3: Commit**

```bash
git add crates/ferrite/src/text_layer.rs
git commit -m "feat(ferrite): text layer setters, system fonts, owned text"
```
<!-- END_TASK_6 -->
<!-- END_SUBCOMPONENT_B -->

<!-- START_TASK_7 -->
### Task 7: hello upgrade — watchapp with two windows and button navigation

**Files:**
- Modify: `examples/hello/package.json` (watchface → watchapp)
- Modify: `examples/hello/src/lib.rs` (replace entirely)

**Step 1: Flip to watchapp**

In `examples/hello/package.json`, change:
```json
    "watchapp": { "watchface": false },
```
(Watchfaces never receive button events; the design's end-state — Fitter — is a watchapp anyway.)

**Step 2: Rewrite the example**

> **CORRECTED during Phase 4 — two defects in the listing below; the notes
> win over the code:**
>
> 1. **Drop order, again (third recurrence).** The listing returns
>    `(window, text)` and captures `let mut screen2 = (win2, text2);` — both
>    are parent-before-child, the exact use-after-free Phase 1 flagged as
>    Critical. Children first in BOTH tuples: return `(text, window)` and
>    capture `(text2, win2)` (push via `screen2.1`).
>    **Further corrected after emulator verification:** the tuple-in-closure
>    pattern is broken EITHER way. Edition-2021 closures capture only the
>    paths they use (RFC 2229), so `move || screen2.1.push(..)` moves in just
>    the window and silently DROPS the tuple's other field at the end of the
>    setup block — destroying the second window's text layer before it is
>    ever shown (observed: window 2 rendered blank white; root-caused via a
>    single-variable emulator experiment). The shipped fix captures `win2`
>    as a whole variable in the closure and keeps `text2` alive in the
>    returned app state: `(text2, text, window)`.
> 2. **The heartbeat must keep the `u64` box and log field.** Phase 3's review
>    added a `Box<u64>` round-trip (the only on-device exercise of the
>    allocator's over-alignment shim) and `scripts/check.sh` now counts a
>    heartbeat line as complete ONLY if it matches
>    `heap_free=[0-9]+ u64=[0-9]+`. The listing's `on_tick` reverts to the
>    old format, which would make check.sh time out. Keep Phase 3's on_tick
>    body: both boxes, log line `"HEARTBEAT {} heap_free={} u64={}"`.
```rust
//! Two-window demo: SELECT pushes the second window, BACK pops it
//! (automatic for watchapps). Heartbeat log retained for scripts/check.sh.

#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

use ferrite::click::Button;
use ferrite::text_layer::{system_font, TextLayer};
use ferrite::window::Window;
use ferrite::{sys, App};

static TICKS: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_tick(_tick_time: *mut sys::tm, _units_changed: sys::TimeUnits) {
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    let boxed = alloc::boxed::Box::new(n);
    let free = ferrite::heap::heap_bytes_free();
    ferrite::info!("HEARTBEAT {} heap_free={}", *boxed, free);
}

ferrite::app! {
    fn main(app: &mut App) {
        ferrite::log::info(c"hello starting");

        // Second screen, built up front, moved into the SELECT closure.
        let mut win2 = Window::new(app);
        let b2 = win2.bounds();
        let mut text2 = TextLayer::new(
            app,
            sys::GRect(0, b2.size.h / 2 - 20, b2.size.w, 40),
        );
        text2.set_text(c"Second window");
        text2.set_alignment(sys::GTextAlignment::GTextAlignmentCenter);
        text2.set_font(system_font(sys::FONT_KEY_GOTHIC_24));
        win2.add_child(&text2);
        win2.on_load(|| ferrite::log::info(c"window 2 loaded"));
        win2.on_unload(|| ferrite::log::info(c"window 2 unloaded"));

        // Main screen.
        let mut window = Window::new(app);
        let bounds = window.bounds();
        let mut text = TextLayer::new(
            app,
            sys::GRect(0, bounds.size.h / 2 - 30, bounds.size.w, 60),
        );
        text.set_text(c"Hello from Rust\nSELECT: next");
        text.set_alignment(sys::GTextAlignment::GTextAlignmentCenter);
        window.add_child(&text);
        window.on_load(|| ferrite::log::info(c"window 1 loaded"));

        let mut screen2 = (win2, text2);
        window.on_click(Button::Select, move || {
            ferrite::log::info(c"SELECT pressed");
            screen2.0.push(true);
        });

        window.push(true);

        unsafe {
            sys::tick_timer_service_subscribe(sys::TimeUnits::SECOND_UNIT, Some(on_tick));
        }

        (window, text)
    }
}
```

(Note `FONT_KEY_GOTHIC_24` is a `&[u8; N]` constant in the generated bindings. If the compiler reports it as `&[u8; N]` vs `&[u8]` mismatch, the `system_font` parameter accepts it via unsized coercion — pass it directly as written.)

**Step 3: Build and verify in the emulator**

```bash
cd examples/hello && pebble build && pebble install --emulator emery --logs
```
Expected in logs: `window 1 loaded`, heartbeats. Leave running; from a second terminal:
```bash
cd examples/hello
pebble emu-button --emulator emery click select
```
After sending SELECT, the first terminal must show `SELECT pressed` and `window 2 loaded`. Screenshot both screens:
```bash
pebble screenshot --emulator emery --no-open screen2.png
```
View `screen2.png` — must show "Second window". Send BACK (`pebble emu-button --emulator emery click back`), confirm `window 2 unloaded` appears. Clean up: `rm -f screen2.png`, Ctrl-C the log stream.

**Step 4: Commit**

```bash
cd ../.. && git add examples/hello
git commit -m "feat(examples): two-window button navigation demo"
```
<!-- END_TASK_7 -->

<!-- START_TASK_8 -->
### Task 8: Phase verification

**Files:** none.

**Step 1: Full test + smoke pass**

Run (from repo root):
```bash
cargo test -p ferrite
./scripts/check.sh
```
Expected: all host tests pass; check.sh prints PASS.

**Step 2: Commit any stragglers**

```bash
git status --short
```
If anything is uncommitted, commit it with an appropriate message.

**Phase complete when:** the upgraded example navigates between windows via buttons in the emulator (verified in Task 7 Step 3), `cargo test -p ferrite` passes, and `./scripts/check.sh` still passes.
<!-- END_TASK_8 -->

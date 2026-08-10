# Ferrite Rust Toolchain Implementation Plan — Phase 5: Canvas graphics and menus

**Goal:** Custom drawing (`CanvasLayer` + safe `Graphics` context) and list UI (`MenuLayer` with closure callbacks) — the rendering surface Fitter's map and history screens need. Hello becomes a menu-driven demo with a text screen and a drawing screen.

**Architecture:** `CanvasLayer` wraps `layer_create_with_data`, storing a pointer to a boxed closure-state in the layer's data area; the update-proc trampoline recovers it and hands the closure a borrowed `Graphics<'_>` (raw `GContext` + bounds, lifetime-bound so it can't be stashed). `MenuLayer` uses the SDK's own context slot (`menu_layer_set_callbacks`'s `callback_context` parameter) to carry its boxed state; single-section rows/draw-row/select closures. A small `AsLayer` trait lets `Window::add_child` accept any layer wrapper.

**Tech Stack:** Same as Phase 4 (alloc closures, trampolines); no new dependencies.

**Scope:** Phase 5 of 7 from `docs/design-plans/2026-08-05-ferrite-rust-toolchain.md`.

**Codebase verified:** 2026-08-05. Verified `pebble.h` signatures (SDK 4.17): `Layer* layer_create_with_data(GRect, size_t)` (5323), `void* layer_get_data(const Layer*)` (5483), `LayerUpdateProc` = `void (*)(struct Layer*, GContext*)` (5296), `layer_set_update_proc` (5346), `layer_mark_dirty` (5337). `GContext` fully opaque (4030). Graphics: `graphics_context_set_stroke_color/fill_color` (4124/4129), `set_antialiased(GContext*, bool)` (4152), `set_stroke_width(GContext*, uint8_t)` (4161), `graphics_draw_pixel` (4213), `draw_line(GContext*, GPoint, GPoint)` (4219), `draw_rect` (4224), `fill_rect(GContext*, GRect, uint16_t, GCornerMask)` (4233), `draw_circle(GContext*, GPoint, uint16_t)` (4240), `fill_circle` (4246). Menu: `menu_layer_set_callbacks(MenuLayer*, void *callback_context, MenuLayerCallbacks)` (7415, struct **by value**, 13 `Option` fn-pointer fields; every callback's last param is the shared `callback_context`), `MenuIndex { uint16_t section; uint16_t row; }` (7109), `menu_layer_set_click_config_onto_window` (7433 — wires UP/DOWN/SELECT; "deviation from the usual click config pattern"), `menu_layer_set_normal_colors/highlight_colors` (7515/7526), `menu_layer_reload_data` (7495), `menu_cell_basic_draw(GContext*, const Layer*, const char *title, const char *subtitle, GBitmap *icon)` (7078).

---

## Context for the implementing engineer (read first)

- **Docketed from the Phase 4 review (design item, decide during this phase's
  layer work):** parent/child layer lifetime is currently a doc-only contract
  ("list children before parents in the returned tuple") that has failed three
  times in four phases — twice via tuple ordering, once via RFC-2229 disjoint
  capture, where no ordering fixes it. The Phase 4 reviewer's proposal: make
  `Window` own its children (`add_child` taking the layer by value into
  `WindowState`, `Drop` destroying children before `window_destroy`), which
  makes the ordering structurally impossible to get wrong. This phase adds the
  `AsLayer` trait and two new layer types, so it is the natural place to
  decide: either adopt ownership as part of the `AsLayer` design, or record
  why borrowing is kept (e.g. post-add mutation needs, like updating a text
  layer every tick, would then require a handle API). Do not silently keep the
  status quo — decide and document.

- **Two context mechanisms, per the design.** Canvas layers have no SDK context parameter on the update proc — but `layer_create_with_data` gives us per-layer storage, where we keep a pointer to the boxed closure state. Menus DO have an SDK context (`callback_context`) — we pass the boxed state pointer there. No globals either way.
- **`Graphics<'_>` is lifetime-bound on purpose.** The raw `GContext*` is only valid during the update proc. The lifetime parameter stops safe code from smuggling it out of the closure. Don't remove it.
- **Menu simplification (deliberate, matches Fitter's usage):** single section, `u16` row indices. `get_num_sections: None` defaults to 1 section in the SDK. Multi-section support is future surface (design: "grows on demand").
- **`MenuLayerCallbacks` field names in the generated bindings** match the C struct: `get_num_sections`, `get_num_rows`, `get_cell_height`, `get_header_height`, `draw_row`, `draw_header`, `select_click`, `select_long_click`, `selection_changed`, `get_separator_height`, `draw_separator`, `selection_will_change`, `draw_background`. Use `..Default::default()` (bindgen derives `Default`) and set only the three we implement.
- **BACK auto-pops** pushed windows in a watchapp — the menu demo needs no BACK handling.

---

<!-- START_SUBCOMPONENT_A (tasks 1-3) -->
<!-- START_TASK_1 -->
### Task 1: graphics module — safe Graphics context

**Files:**
- Create: `crates/ferrite/src/graphics.rs`
- Modify: `crates/ferrite/src/lib.rs` (add `pub mod graphics;`)

**Step 1: Write the module**

`crates/ferrite/src/graphics.rs`:
```rust
//! Safe drawing context, handed to canvas update closures.
//!
//! Wraps the SDK `GContext`, which is only valid for the duration of an
//! update proc — the lifetime parameter enforces that borrow structurally.

use core::marker::PhantomData;

use crate::sys;

pub struct Graphics<'a> {
    ctx: *mut sys::GContext,
    bounds: sys::GRect,
    _lifetime: PhantomData<&'a mut ()>,
}

impl<'a> Graphics<'a> {
    /// Internal: constructed by trampolines only.
    pub(crate) fn new(ctx: *mut sys::GContext, bounds: sys::GRect) -> Graphics<'a> {
        Graphics {
            ctx,
            bounds,
            _lifetime: PhantomData,
        }
    }

    /// Bounds of the layer being drawn.
    pub fn bounds(&self) -> sys::GRect {
        self.bounds
    }

    pub fn set_stroke_color(&mut self, color: sys::GColor8) {
        unsafe { sys::graphics_context_set_stroke_color(self.ctx, color) };
    }

    pub fn set_fill_color(&mut self, color: sys::GColor8) {
        unsafe { sys::graphics_context_set_fill_color(self.ctx, color) };
    }

    pub fn set_stroke_width(&mut self, width: u8) {
        unsafe { sys::graphics_context_set_stroke_width(self.ctx, width) };
    }

    pub fn set_antialiased(&mut self, enabled: bool) {
        unsafe { sys::graphics_context_set_antialiased(self.ctx, enabled) };
    }

    pub fn draw_pixel(&mut self, point: sys::GPoint) {
        unsafe { sys::graphics_draw_pixel(self.ctx, point) };
    }

    pub fn draw_line(&mut self, from: sys::GPoint, to: sys::GPoint) {
        unsafe { sys::graphics_draw_line(self.ctx, from, to) };
    }

    pub fn draw_rect(&mut self, rect: sys::GRect) {
        unsafe { sys::graphics_draw_rect(self.ctx, rect) };
    }

    /// Fills a rectangle; `corner_radius` 0 and `GCornerNone` for square.
    pub fn fill_rect(&mut self, rect: sys::GRect, corner_radius: u16, mask: sys::GCornerMask) {
        unsafe { sys::graphics_fill_rect(self.ctx, rect, corner_radius, mask) };
    }

    pub fn draw_circle(&mut self, center: sys::GPoint, radius: u16) {
        unsafe { sys::graphics_draw_circle(self.ctx, center, radius) };
    }

    pub fn fill_circle(&mut self, center: sys::GPoint, radius: u16) {
        unsafe { sys::graphics_fill_circle(self.ctx, center, radius) };
    }
}
```

In `crates/ferrite/src/lib.rs`, add to the module list:
```rust
pub mod graphics;
```

**Step 2: Verify**

```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo test -p ferrite
```
Expected: both pass.

**Step 3: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): safe Graphics drawing context"
```
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: AsLayer trait — generalize add_child

**Files:**
- Create: `crates/ferrite/src/layer.rs`
- Modify: `crates/ferrite/src/lib.rs` (add `pub mod layer;`)
- Modify: `crates/ferrite/src/window.rs` (generalize `add_child`, expose raw window ptr crate-internally)
- Modify: `crates/ferrite/src/text_layer.rs` (implement `AsLayer`)

**Step 1: Create the trait**

`crates/ferrite/src/layer.rs`:
```rust
//! Common interface for wrappers that are backed by an SDK `Layer`.

use crate::sys;

pub trait AsLayer {
    /// Raw layer pointer — used by `Window::add_child`. Not for user code.
    #[doc(hidden)]
    fn as_layer_ptr(&self) -> *mut sys::Layer;
}
```

In `crates/ferrite/src/lib.rs`, add:
```rust
pub mod layer;
```

**Step 2: Generalize `Window::add_child` in `crates/ferrite/src/window.rs`**

Replace the existing `add_child` method with:
```rust
    /// Adds a layer wrapper as a child of the window's root layer. The child
    /// must outlive its time in the window (keep both in your app state).
    pub fn add_child(&mut self, child: &impl crate::layer::AsLayer) {
        unsafe {
            sys::layer_add_child(
                sys::window_get_root_layer(self.raw),
                child.as_layer_ptr(),
            );
        }
    }
```
And add this method to `impl Window` (the menu layer needs the raw window to bind its click config):
```rust
    pub(crate) fn as_window_ptr(&mut self) -> *mut sys::Window {
        self.raw
    }
```

**Step 3: Implement the trait for TextLayer**

In `crates/ferrite/src/text_layer.rs`, replace the inherent `as_layer_ptr` method:
```rust
    pub(crate) fn as_layer_ptr(&self) -> *mut sys::Layer {
        unsafe { sys::text_layer_get_layer(self.raw) }
    }
```
with a trait impl (delete the inherent method from the `impl TextLayer` block and add at the bottom of the file):
```rust
impl crate::layer::AsLayer for TextLayer {
    fn as_layer_ptr(&self) -> *mut sys::Layer {
        unsafe { sys::text_layer_get_layer(self.raw) }
    }
}
```

**Step 4: Verify**

```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo test -p ferrite
```
Expected: both pass (hello still compiles against this API in Task 5's rebuild — `add_child(&text)` call sites are source-compatible).

**Step 5: Commit**

```bash
git add crates/ferrite/src
git commit -m "refactor(ferrite): AsLayer trait generalizing Window::add_child"
```
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: canvas module — CanvasLayer with draw closure

**Files:**
- Create: `crates/ferrite/src/canvas.rs`
- Modify: `crates/ferrite/src/lib.rs` (add `pub mod canvas;`)

**Step 1: Write the module**

`crates/ferrite/src/canvas.rs`:
```rust
//! Custom-drawing layer over `layer_create_with_data`.
//!
//! The SDK's update proc carries no context pointer, but a data layer gives
//! us per-layer storage: we keep a single pointer to the boxed closure state
//! there, so no globals are needed.

use alloc::boxed::Box;
use core::mem::size_of;

use crate::graphics::Graphics;
use crate::layer::AsLayer;
use crate::sys;
use crate::App;

struct CanvasState {
    on_draw: Option<Box<dyn FnMut(&mut Graphics<'_>)>>,
}

pub struct CanvasLayer {
    raw: *mut sys::Layer,
    state: *mut CanvasState, // Box, owned; freed in Drop
}

impl CanvasLayer {
    /// Creates a canvas layer with the given frame. Panics if the SDK
    /// returns NULL (out of memory).
    pub fn new(_app: &mut App, frame: sys::GRect) -> CanvasLayer {
        let raw =
            unsafe { sys::layer_create_with_data(frame, size_of::<*mut CanvasState>()) };
        assert!(!raw.is_null(), "layer_create_with_data returned NULL");
        let state = Box::into_raw(Box::new(CanvasState { on_draw: None }));
        unsafe {
            *(sys::layer_get_data(raw) as *mut *mut CanvasState) = state;
            sys::layer_set_update_proc(raw, Some(canvas_update_proc));
        }
        CanvasLayer { raw, state }
    }

    /// Sets the draw closure, called whenever the layer needs rendering.
    /// Call [`mark_dirty`](Self::mark_dirty) to request a redraw.
    pub fn on_draw(&mut self, f: impl FnMut(&mut Graphics<'_>) + 'static) {
        unsafe { &mut *self.state }.on_draw = Some(Box::new(f));
    }

    /// Requests a redraw.
    pub fn mark_dirty(&mut self) {
        unsafe { sys::layer_mark_dirty(self.raw) };
    }

    pub fn bounds(&self) -> sys::GRect {
        unsafe { sys::layer_get_bounds(self.raw) }
    }

    pub fn set_hidden(&mut self, hidden: bool) {
        unsafe { sys::layer_set_hidden(self.raw, hidden) };
    }
}

impl Drop for CanvasLayer {
    fn drop(&mut self) {
        unsafe {
            sys::layer_destroy(self.raw);
            drop(Box::from_raw(self.state));
        }
    }
}

impl AsLayer for CanvasLayer {
    fn as_layer_ptr(&self) -> *mut sys::Layer {
        self.raw
    }
}

unsafe extern "C" fn canvas_update_proc(layer: *mut sys::Layer, ctx: *mut sys::GContext) {
    let state = *(sys::layer_get_data(layer) as *mut *mut CanvasState);
    let bounds = sys::layer_get_bounds(layer);
    let mut g = Graphics::new(ctx, bounds);
    if let Some(f) = (*state).on_draw.as_mut() {
        f(&mut g);
    }
}
```

In `crates/ferrite/src/lib.rs`, add:
```rust
pub mod canvas;
```

**Step 2: Verify**

```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo test -p ferrite
```
Expected: both pass.

**Step 3: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): CanvasLayer with safe draw closure"
```
<!-- END_TASK_3 -->
<!-- END_SUBCOMPONENT_A -->

<!-- START_TASK_4 -->
### Task 4: menu_layer module — MenuLayer with closure callbacks

**Files:**
- Create: `crates/ferrite/src/menu_layer.rs`
- Modify: `crates/ferrite/src/lib.rs` (add `pub mod menu_layer;`)

**Step 1: Write the module**

`crates/ferrite/src/menu_layer.rs`:
```rust
//! List UI over the SDK `MenuLayer` — single-section, closure callbacks.
//!
//! The SDK carries one `callback_context` for all menu callbacks
//! (`menu_layer_set_callbacks`); we pass the boxed `MenuState` pointer.

use alloc::boxed::Box;
use core::ffi::{c_void, CStr};
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

struct MenuState {
    num_rows: Option<Box<dyn FnMut() -> u16>>,
    draw_row: Option<Box<dyn FnMut(&mut RowCell<'_>, u16)>>,
    on_select: Option<Box<dyn FnMut(u16)>>,
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

unsafe extern "C" fn cb_get_num_rows(
    _menu: *mut sys::MenuLayer,
    _section: u16,
    context: *mut c_void,
) -> u16 {
    let state = &mut *(context as *mut MenuState);
    state.num_rows.as_mut().map(|f| f()).unwrap_or(0)
}

unsafe extern "C" fn cb_draw_row(
    ctx: *mut sys::GContext,
    cell_layer: *const sys::Layer,
    cell_index: *mut sys::MenuIndex,
    context: *mut c_void,
) {
    let state = &mut *(context as *mut MenuState);
    let row = (*cell_index).row;
    let mut cell = RowCell {
        ctx,
        cell_layer,
        _lifetime: PhantomData,
    };
    if let Some(f) = state.draw_row.as_mut() {
        f(&mut cell, row);
    }
}

unsafe extern "C" fn cb_select_click(
    _menu: *mut sys::MenuLayer,
    cell_index: *mut sys::MenuIndex,
    context: *mut c_void,
) {
    let state = &mut *(context as *mut MenuState);
    let row = (*cell_index).row;
    if let Some(f) = state.on_select.as_mut() {
        f(row);
    }
}
```

In `crates/ferrite/src/lib.rs`, add:
```rust
pub mod menu_layer;
```

**Step 2: Verify**

```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo test -p ferrite
```
Expected: both pass. If field names in the `MenuLayerCallbacks` literal don't match, check the generated struct in `crates/ferrite-sys/src/bindings_emery.rs` (`grep -A 20 "pub struct MenuLayerCallbacks" crates/ferrite-sys/src/bindings_emery.rs`) — the generated names are the source of truth.

**Step 3: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): MenuLayer with closure callbacks"
```
<!-- END_TASK_4 -->

<!-- START_TASK_5 -->
### Task 5: hello upgrade — menu home screen, text screen, canvas screen

**Files:**
- Modify: `examples/hello/src/lib.rs` (replace entirely)

**Step 1: Rewrite the example**

`examples/hello/src/lib.rs`:
```rust
//! Menu-driven demo: home menu with two entries — a text screen and a
//! custom-drawn canvas screen. Heartbeat log retained for scripts/check.sh.

#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

use ferrite::canvas::CanvasLayer;
use ferrite::menu_layer::MenuLayer;
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

fn make_text_screen(app: &mut App) -> (Window, TextLayer) {
    let mut win = Window::new(app);
    let b = win.bounds();
    let mut text = TextLayer::new(app, sys::GRect(0, b.size.h / 2 - 20, b.size.w, 40));
    text.set_text(c"Hello from Rust");
    text.set_alignment(sys::GTextAlignment::GTextAlignmentCenter);
    text.set_font(system_font(sys::FONT_KEY_GOTHIC_24));
    win.add_child(&text);
    win.on_load(|| ferrite::log::info(c"text screen loaded"));
    (win, text)
}

fn make_canvas_screen(app: &mut App) -> (Window, CanvasLayer) {
    let mut win = Window::new(app);
    let b = win.bounds();
    let mut canvas = CanvasLayer::new(app, sys::GRect(0, 0, b.size.w, b.size.h));
    canvas.on_draw(|g| {
        let b = g.bounds();
        g.set_antialiased(true);
        // Diagonal cross
        g.set_stroke_color(sys::GColorBlack);
        g.set_stroke_width(3);
        g.draw_line(
            sys::GPoint(0, 0),
            sys::GPoint(b.size.w - 1, b.size.h - 1),
        );
        g.draw_line(
            sys::GPoint(b.size.w - 1, 0),
            sys::GPoint(0, b.size.h - 1),
        );
        // Filled circle in the center
        g.set_fill_color(sys::GColorFromRGB(255, 0, 0));
        g.fill_circle(sys::GPoint(b.size.w / 2, b.size.h / 2), 30);
    });
    win.add_child(&canvas);
    win.on_load(|| ferrite::log::info(c"canvas screen loaded"));
    (win, canvas)
}

ferrite::app! {
    fn main(app: &mut App) {
        ferrite::log::info(c"hello starting");

        let text_screen = make_text_screen(app);
        let canvas_screen = make_canvas_screen(app);

        // Home: a menu filling the root window.
        let mut home = Window::new(app);
        let hb = home.bounds();
        let mut menu = MenuLayer::new(app, sys::GRect(0, 0, hb.size.w, hb.size.h));
        menu.rows(|| 2);
        menu.on_draw_row(|cell, row| match row {
            0 => cell.basic_draw_with_subtitle(c"Text", c"TextLayer demo"),
            _ => cell.basic_draw_with_subtitle(c"Canvas", c"custom drawing"),
        });

        let mut screens = (text_screen, canvas_screen);
        menu.on_select(move |row| {
            ferrite::info!("menu select row={}", row);
            match row {
                0 => screens.0 .0.push(true),
                _ => screens.1 .0.push(true),
            }
        });
        menu.attach_clicks(&mut home);
        home.add_child(&menu);
        home.on_load(|| ferrite::log::info(c"home menu loaded"));
        home.push(true);

        unsafe {
            sys::tick_timer_service_subscribe(sys::TimeUnits::SECOND_UNIT, Some(on_tick));
        }

        (home, menu)
    }
}
```

**Step 2: Build and verify in the emulator**

```bash
cd examples/hello && pebble build && pebble install --emulator emery --logs
```
Expected in logs: `home menu loaded`, heartbeats. From a second terminal in `examples/hello` (button presses via `pebble emu-button --emulator emery click <button>`):

1. Screenshot the home screen: `pebble screenshot --emulator emery --no-open menu.png` — must show the two menu rows ("Text", "Canvas").
2. `pebble emu-button --emulator emery click select` — logs show `menu select row=0`, `text screen loaded`; screenshot shows "Hello from Rust".
3. `click back`, then `click down`, then `click select` — logs show `menu select row=1`, `canvas screen loaded`; screenshot `canvas.png` must show the diagonal cross and red center circle.
4. `click back` to return to the menu.

Clean up screenshots (`rm -f menu.png canvas.png`), Ctrl-C the log stream.

**Step 3: Commit**

```bash
cd ../.. && git add examples/hello/src/lib.rs
git commit -m "feat(examples): menu home with text and canvas screens"
```
<!-- END_TASK_5 -->

<!-- START_TASK_6 -->
### Task 6: Phase verification

**Files:** none.

**Step 1: Full pass**

Run (from repo root):
```bash
cargo test -p ferrite
./scripts/check.sh
```
Expected: host tests pass; check.sh prints PASS.

**Step 2: Commit stragglers if any**

```bash
git status --short
```
Commit anything outstanding.

**Phase complete when:** the example renders custom-drawn content and a navigable menu in the emulator (Task 5 Step 2 screenshots), and `./scripts/check.sh` still passes.
<!-- END_TASK_6 -->

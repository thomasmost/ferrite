# Ferrite Rust Toolchain Implementation Plan — Phase 1: Build-integration proof

**Goal:** A Rust watchface builds via `pebble build` and runs in the Emery emulator — proving the cargo-staticlib-into-waf integration end to end.

**Architecture:** A Cargo workspace (`crates/ferrite-sys` raw FFI + `crates/ferrite` safe runtime) plus a standalone Pebble project at `examples/hello` whose customized `wscript` runs `cargo build --release --target thumbv7m-none-eabi` and links the resulting staticlib into the app ELF via the SDK's `stlib`/`stlibpath` kwargs passthrough. The SDK's linker script, jump table, and metadata injection run unchanged.

**Tech Stack:** Stable Rust (1.97, edition 2021), `thumbv7m-none-eabi` target, rePebble SDK 4.17 / pebble-tool 5.0.39 (installed at `/Users/thomas/Library/Application Support/Pebble SDK/SDKs/4.17/`), waf build system.

**Scope:** Phase 1 of 7 from `docs/design-plans/2026-08-05-ferrite-rust-toolchain.md`.

**Codebase verified:** 2026-08-05. Repo is greenfield (only `docs/design-plans/` exists). Verified on this machine: `pebble` CLI at `/Users/thomas/.local/bin/pebble` (v5.0.39, active SDK 4.17), Emery header at `$SDK/sdk-core/pebble/emery/include/pebble.h`, `libpebble.a` at `$SDK/sdk-core/pebble/emery/lib/`, SDK compiles everything `-mcpu=cortex-m3 -mthumb -fPIE -Os`. The `thumbv7m-none-eabi` rustup target is **NOT** installed yet (Task 1 installs it).

---

## Context for the implementing engineer (read first)

- **How Pebble apps link.** The app is an ELF linked by `arm-none-eabi-gcc` with the SDK's generated linker script (`ENTRY(main)`, one 128 KB `APP` memory region). Apps call firmware through `libpebble.a` jump-table trampolines — you never link firmware code directly. After linking, the SDK's `inject_metadata.py` post-processes the binary. All of this is driven by waf when you run `pebble build`.
- **The integration hook (verified in SDK source).** `ctx.pbl_build(...)` in a project's `wscript` forwards extra kwargs to the waf task generator, and the SDK's `setup_pebble_cprogram` only ever *appends* to `stlib`/`stlibpath`/`linkflags`. So passing `stlib=['hello'], stlibpath=[<cargo out dir>]` links our Rust staticlib, and our entries come before `-lpebble` on the link line (correct order).
- **`main` comes from Rust.** The linker script declares `ENTRY(main)`. Because no C object references `main`, we also pass `-Wl,-u,main` so the linker pulls it from `libhello.a` (an archive member is only extracted if some undefined symbol demands it).
- **PIC flags.** The SDK compiles C with `-fPIE`. The Rust side mirrors this with `-C relocation-model=pic` (in `examples/hello/.cargo/config.toml`), plus `-C force-unwind-tables=no` and `panic = "abort"` so no `.ARM.extab`/`.ARM.exidx` sections grow (the one identified binary-format hazard for `inject_metadata.py`).
- **The `pebble` CLI runs waf under the SDK's own Python venv.** Your `wscript` code executes on that interpreter, but `PATH` is inherited, so `cargo` resolves normally (we also fall back to `~/.cargo/bin/cargo` explicitly).
- **Editions and hygiene.** All crates use edition 2021 (avoids edition-2024 `unsafe extern` / `unsafe(no_mangle)` churn inside the `app!` macro). The panic handler is gated `#[cfg(target_os = "none")]` so host `cargo check`/`cargo test` (used in later phases) doesn't clash with std's panic handler.
- **`GRect`/`GPoint`/`GSize` are C macros** (compound literals) in `pebble.h`, so Rust gets const fns of the same names. `layer_get_bounds` returns `GRect` **by value** and `text_layer_create` takes it **by value** — the `#[repr(C)]` structs below match the AAPCS ABI for these 8-byte aggregates.
- **Verified C signatures** (from `$SDK/sdk-core/pebble/emery/include/pebble.h`; line numbers as of SDK 4.17): `Window* window_create(void)` (5554), `void window_destroy(Window*)` (5557), `struct Layer* window_get_root_layer(const Window*)` (5598), `void window_stack_push(Window*, bool)` (5717), `GRect layer_get_bounds(const Layer*)` (5376), `void layer_add_child(Layer*, Layer*)` (5427), `TextLayer* text_layer_create(GRect)` (6672), `void text_layer_destroy(TextLayer*)` (6675), `Layer* text_layer_get_layer(TextLayer*)` (6681), `void text_layer_set_text(TextLayer*, const char*)` (6691), `void app_event_loop(void)` (2854), `void app_log(uint8_t, const char*, int, const char*, ...)` (1631). Log levels: ERROR=1, WARNING=50, INFO=100, DEBUG=200.
- **Design note — `app!` lifetime contract for this phase:** the user's setup block evaluates to a value (their windows/layers) which the macro keeps alive across `app_event_loop()` and drops afterward. This gives correct create → run → destroy ordering without needing an allocator (which doesn't exist until Phase 3).

Shell note: the SDK path contains a space — always quote it. In commands below, `$SDK` means `/Users/thomas/Library/Application Support/Pebble SDK/SDKs/4.17`.

---

<!-- START_TASK_1 -->
### Task 1: Install the Rust cross-compilation target

**Files:** none (toolchain state only).

**Step 1: Install the target**

Run:
```bash
rustup target add thumbv7m-none-eabi
```

**Step 2: Verify**

Run:
```bash
rustup target list --installed
```
Expected: output includes `thumbv7m-none-eabi`.

No commit (no repo changes).
<!-- END_TASK_1 -->

<!-- START_SUBCOMPONENT_A (tasks 2-4) -->
<!-- START_TASK_2 -->
### Task 2: Workspace scaffolding

**Files:**
- Create: `Cargo.toml`
- Create: `.gitignore`
- Create: `README.md`
- Create: `LICENSE-MIT`
- Create: `LICENSE-APACHE`
- Create: `crates/ferrite-sys/Cargo.toml`
- Create: `crates/ferrite-sys/src/lib.rs` (stub — real content in Task 3)
- Create: `crates/ferrite/Cargo.toml`
- Create: `crates/ferrite/src/lib.rs` (stub — real content in Task 4)

**Step 1: Create the workspace root files**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/ferrite-sys", "crates/ferrite"]
exclude = ["examples/hello"]
```

(`examples/hello` is deliberately **excluded**: it is a standalone Pebble project with its own `.cargo/config.toml` and release profile, which only take effect at a workspace root.)

`.gitignore`:
```gitignore
/target
/examples/hello/target
/examples/hello/build
.lock-waf_*_build
.DS_Store
```

`README.md`:
```markdown
# Ferrite

A Rust toolchain for writing PebbleOS watchapps — targeting the Pebble Time 2
("Emery") with the community-maintained rePebble SDK (pebble-tool 5.0.x /
SDK 4.17).

Rust code compiles to a static library with Cargo; the SDK's own `pebble build`
links it into the app binary. See `examples/hello` for the project template.

**Status:** Phase 1 — build-integration proof.

## Prerequisites

- rePebble SDK: pebble-tool 5.0.x with SDK 4.17 installed
- Rust (stable) with the ARM target: `rustup target add thumbv7m-none-eabi`

## License

Licensed under either of Apache License, Version 2.0 (`LICENSE-APACHE`) or
MIT license (`LICENSE-MIT`) at your option.
```

`LICENSE-MIT`:
```text
MIT License

Copyright (c) 2026 Ferrite contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

`LICENSE-APACHE` — fetch the canonical text:
```bash
curl -fsSL -o LICENSE-APACHE https://www.apache.org/licenses/LICENSE-2.0.txt
```

**Step 2: Create the two crate skeletons**

`crates/ferrite-sys/Cargo.toml`:
```toml
[package]
name = "ferrite-sys"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
description = "Raw FFI bindings to the rePebble SDK (Emery, SDK core 4.17)"
```

`crates/ferrite-sys/src/lib.rs` (stub):
```rust
#![no_std]
```

`crates/ferrite/Cargo.toml`:
```toml
[package]
name = "ferrite"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
description = "Safe Rust API and runtime for PebbleOS watchapps"

[dependencies]
ferrite-sys = { path = "../ferrite-sys" }
```

`crates/ferrite/src/lib.rs` (stub):
```rust
#![no_std]

pub use ferrite_sys as sys;
```

**Step 3: Verify**

Run (from repo root):
```bash
cargo check
cargo check --target thumbv7m-none-eabi
```
Expected: both succeed (two empty `no_std` crates).

Run:
```bash
ls LICENSE-APACHE LICENSE-MIT README.md .gitignore
head -1 LICENSE-APACHE
```
Expected: files exist; first line of `LICENSE-APACHE` contains "Apache License".

**Step 4: Commit**

```bash
git add Cargo.toml .gitignore README.md LICENSE-MIT LICENSE-APACHE crates
git commit -m "chore: scaffold ferrite cargo workspace"
```
<!-- END_TASK_2 -->

<!-- START_TASK_3 -->
### Task 3: Hand-written FFI declarations in ferrite-sys

Only the surface hello-world needs; generated bindings replace this file in Phase 2.

**Files:**
- Modify: `crates/ferrite-sys/src/lib.rs` (replace stub entirely)

**Step 1: Write the declarations**

`crates/ferrite-sys/src/lib.rs`:
```rust
//! Raw FFI bindings to the rePebble SDK (Emery, SDK core 4.17).
//!
//! Phase 1: hand-written declarations for the hello-world surface only.
//! Signatures transcribed from
//! `$SDK/sdk-core/pebble/emery/include/pebble.h` (SDK 4.17).
//! Replaced by generated bindings in Phase 2.

#![no_std]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use core::ffi::{c_char, c_int};

// --- Value types (match C layout exactly; passed/returned by value) ---

/// 8-bit ARGB (2 bits per channel). C: `union GColor8 { uint8_t argb; ... }`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GColor8 {
    pub argb: u8,
}

pub type GColor = GColor8;

pub const GColorBlack: GColor8 = GColor8 { argb: 0b1100_0000 };
pub const GColorWhite: GColor8 = GColor8 { argb: 0b1111_1111 };
pub const GColorClear: GColor8 = GColor8 { argb: 0b0000_0000 };

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GPoint {
    pub x: i16,
    pub y: i16,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GSize {
    pub w: i16,
    pub h: i16,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GRect {
    pub origin: GPoint,
    pub size: GSize,
}

// Ports of the C constructor macros of the same names.
pub const fn GPoint(x: i16, y: i16) -> GPoint {
    GPoint { x, y }
}

pub const fn GSize(w: i16, h: i16) -> GSize {
    GSize { w, h }
}

pub const fn GRect(x: i16, y: i16, w: i16, h: i16) -> GRect {
    GRect {
        origin: GPoint(x, y),
        size: GSize(w, h),
    }
}

// --- Log levels (C enum AppLogLevel; app_log takes uint8_t) ---

pub const APP_LOG_LEVEL_ERROR: u8 = 1;
pub const APP_LOG_LEVEL_WARNING: u8 = 50;
pub const APP_LOG_LEVEL_INFO: u8 = 100;
pub const APP_LOG_LEVEL_DEBUG: u8 = 200;

// --- Opaque SDK object types ---

#[repr(C)]
pub struct Window {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Layer {
    _private: [u8; 0],
}

#[repr(C)]
pub struct TextLayer {
    _private: [u8; 0],
}

extern "C" {
    // Window
    pub fn window_create() -> *mut Window;
    pub fn window_destroy(window: *mut Window);
    pub fn window_get_root_layer(window: *const Window) -> *mut Layer;
    pub fn window_stack_push(window: *mut Window, animated: bool);

    // Layer
    pub fn layer_get_bounds(layer: *const Layer) -> GRect;
    pub fn layer_add_child(parent: *mut Layer, child: *mut Layer);

    // TextLayer
    pub fn text_layer_create(frame: GRect) -> *mut TextLayer;
    pub fn text_layer_destroy(text_layer: *mut TextLayer);
    pub fn text_layer_get_layer(text_layer: *mut TextLayer) -> *mut Layer;
    pub fn text_layer_set_text(text_layer: *mut TextLayer, text: *const c_char);

    // App
    pub fn app_event_loop();
    pub fn app_log(
        log_level: u8,
        src_filename: *const c_char,
        src_line_number: c_int,
        fmt: *const c_char,
        ...
    );
}
```

**Step 2: Verify**

Run (from repo root):
```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys
cargo check -p ferrite-sys
```
Expected: both pass with no warnings about the declarations themselves.

**Step 3: Commit**

```bash
git add crates/ferrite-sys/src/lib.rs
git commit -m "feat(sys): hand-written FFI declarations for hello-world surface"
```
<!-- END_TASK_3 -->

<!-- START_TASK_4 -->
### Task 4: ferrite runtime — panic handler, `app!` macro, thin wrappers

**Files:**
- Modify: `crates/ferrite/src/lib.rs` (replace stub entirely)
- Create: `crates/ferrite/src/panic.rs`
- Create: `crates/ferrite/src/window.rs`
- Create: `crates/ferrite/src/text_layer.rs`

**Step 1: Write the crate root**

`crates/ferrite/src/lib.rs`:
```rust
//! Safe Rust API and runtime for PebbleOS watchapps.

#![no_std]

pub use ferrite_sys as sys;

#[cfg(target_os = "none")]
mod panic;
pub mod text_layer;
pub mod window;

use core::ffi::CStr;

/// Capability token proving the app runtime is initialized.
///
/// A `&mut App` is handed to your setup code by [`app!`]; it cannot be
/// constructed by user code. Wrapper constructors take it so SDK calls are
/// structurally impossible before `main` runs.
pub struct App {
    _private: (),
}

impl App {
    /// Internal constructor used by the [`app!`] macro. Do not call directly.
    #[doc(hidden)]
    pub unsafe fn __new() -> App {
        App { _private: () }
    }
}

/// Log a message at INFO level via the SDK's `app_log` (shows in `pebble logs`).
pub fn log_info(msg: &CStr) {
    unsafe {
        sys::app_log(
            sys::APP_LOG_LEVEL_INFO,
            c"rust".as_ptr(),
            0,
            c"%s".as_ptr(),
            msg.as_ptr(),
        );
    }
}

/// Declares the app entry point.
///
/// Expands to `#[no_mangle] extern "C" fn main()`, which constructs the
/// [`App`] token, runs your setup block, keeps the block's resulting value
/// alive while the SDK event loop runs, and drops it (running destructors)
/// when the app exits.
///
/// The setup block's final expression is the state kept alive for the app's
/// lifetime — return every window/layer you create from the block:
///
/// ```ignore
/// ferrite::app! {
///     fn main(app: &mut App) {
///         let window = Window::new(app);
///         // ... configure, push ...
///         window // kept alive until the app exits
///     }
/// }
/// ```
#[macro_export]
macro_rules! app {
    (fn main($app:ident: &mut App) $body:block) => {
        #[no_mangle]
        pub extern "C" fn main() -> i32 {
            let mut __token = unsafe { $crate::App::__new() };
            let $app: &mut $crate::App = &mut __token;
            let __state = $body;
            unsafe { $crate::sys::app_event_loop() };
            ::core::mem::drop(__state);
            0
        }
    };
}
```

**Step 2: Write the panic handler**

`crates/ferrite/src/panic.rs`:
```rust
//! Panic = log via app_log, then trap so the firmware's app-fault path
//! terminates the app. There is no unwinding (panic = "abort").

use core::fmt::Write;

struct FixedBuf {
    buf: [u8; 128],
    len: usize,
}

impl Write for FixedBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let space = self.buf.len() - 1 - self.len; // reserve NUL byte
        let n = s.len().min(space);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut out = FixedBuf {
        buf: [0; 128],
        len: 0,
    };
    let _ = write!(out, "{}", info);
    out.buf[out.len] = 0;
    unsafe {
        crate::sys::app_log(
            crate::sys::APP_LOG_LEVEL_ERROR,
            c"rust".as_ptr(),
            0,
            c"%s".as_ptr(),
            out.buf.as_ptr(),
        );
    }
    // Undefined instruction: firmware kills the app through its fault handler.
    loop {
        unsafe { core::arch::asm!("udf #255") };
    }
}
```

**Step 3: Write the Window wrapper**

`crates/ferrite/src/window.rs`:
```rust
//! Owned wrapper over the SDK `Window`.

use crate::sys;
use crate::App;

pub struct Window {
    raw: *mut sys::Window,
}

impl Window {
    /// Creates a new window. Panics if the SDK returns NULL (out of memory).
    pub fn new(_app: &mut App) -> Window {
        let raw = unsafe { sys::window_create() };
        assert!(!raw.is_null(), "window_create returned NULL");
        Window { raw }
    }

    /// Bounds of the window's root layer.
    pub fn bounds(&self) -> sys::GRect {
        unsafe { sys::layer_get_bounds(sys::window_get_root_layer(self.raw)) }
    }

    /// Adds a text layer as a child of the window's root layer.
    ///
    /// Note (phase-1 contract): the SDK does not take ownership — the child
    /// must stay alive as long as the window shows it. Keep both in the
    /// state returned from your `app!` setup block.
    pub fn add_child(&mut self, child: &crate::text_layer::TextLayer) {
        unsafe {
            sys::layer_add_child(
                sys::window_get_root_layer(self.raw),
                child.as_layer_ptr(),
            );
        }
    }

    /// Pushes the window onto the window stack, making it visible.
    pub fn push(&mut self) {
        unsafe { sys::window_stack_push(self.raw, false) };
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe { sys::window_destroy(self.raw) };
    }
}
```

**Step 4: Write the TextLayer wrapper**

`crates/ferrite/src/text_layer.rs`:
```rust
//! Owned wrapper over the SDK `TextLayer`.

use core::ffi::CStr;

use crate::sys;
use crate::App;

pub struct TextLayer {
    raw: *mut sys::TextLayer,
}

impl TextLayer {
    /// Creates a text layer with the given frame. Panics if the SDK returns
    /// NULL (out of memory).
    pub fn new(_app: &mut App, frame: sys::GRect) -> TextLayer {
        let raw = unsafe { sys::text_layer_create(frame) };
        assert!(!raw.is_null(), "text_layer_create returned NULL");
        TextLayer { raw }
    }

    /// Sets the displayed text. `&'static` because the SDK stores the pointer
    /// without copying; `c"..."` literals satisfy this.
    pub fn set_text(&mut self, text: &'static CStr) {
        unsafe { sys::text_layer_set_text(self.raw, text.as_ptr()) };
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

**Step 5: Verify**

Run (from repo root):
```bash
cargo check --target thumbv7m-none-eabi
cargo check
```
Expected: both pass. (Host check works because `panic.rs` is gated on `target_os = "none"`.)

**Step 6: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): panic handler, app! macro, window/text-layer wrappers"
```
<!-- END_TASK_4 -->
<!-- END_SUBCOMPONENT_A -->

<!-- START_TASK_5 -->
### Task 5: The hello example — a full Pebble project

**Files:**
- Create: `examples/hello/package.json`
- Create: `examples/hello/wscript`
- Create: `examples/hello/Cargo.toml`
- Create: `examples/hello/.cargo/config.toml`
- Create: `examples/hello/src/lib.rs`

**Step 1: Create the Pebble project metadata**

`examples/hello/package.json`:
```json
{
  "name": "hello",
  "version": "1.0.0",
  "private": true,
  "keywords": ["pebble-app"],
  "dependencies": {},
  "pebble": {
    "displayName": "Hello Rust",
    "uuid": "9a1caaa4-92f2-4b46-9dd0-72eb7b0e3b3b",
    "sdkVersion": "3",
    "enableMultiJS": true,
    "targetPlatforms": ["emery"],
    "watchapp": { "watchface": true },
    "messageKeys": [],
    "resources": { "media": [] },
    "capabilities": []
  }
}
```

**Step 2: Create the customized wscript**

Based on the SDK's default template (its header comment says "Feel free to customize this to your needs"); changes: run cargo before the platform loop, inject the staticlib via `stlib`/`stlibpath`, force `main` extraction with `-Wl,-u,main`, drop the unused worker branch.

`examples/hello/wscript`:
```python
#
# Pebble app build script, customized for Ferrite: the app's code is Rust,
# compiled by cargo into a static library that is linked into the app ELF
# via the SDK's stlib mechanism. See the repo README.
#
import os
import subprocess

top = '.'
out = 'build'

CARGO_TARGET = 'thumbv7m-none-eabi'
# Cargo crate name = staticlib name (libhello.a). Template users: change
# this alongside [package] name in Cargo.toml.
RUST_CRATE = 'hello'


def options(ctx):
    ctx.load('pebble_sdk')


def configure(ctx):
    ctx.load('pebble_sdk')


def _cargo_path():
    cargo = os.path.expanduser('~/.cargo/bin/cargo')
    return cargo if os.path.exists(cargo) else 'cargo'


def build(ctx):
    ctx.load('pebble_sdk')

    # Compile the Rust staticlib (target/thumbv7m-none-eabi/release/libhello.a).
    subprocess.check_call(
        [_cargo_path(), 'build', '--release', '--target', CARGO_TARGET],
        cwd=ctx.path.abspath())
    rust_lib_dir = os.path.join(
        ctx.path.abspath(), 'target', CARGO_TARGET, 'release')
    rust_lib = os.path.join(rust_lib_dir, 'lib{}.a'.format(RUST_CRATE))

    binaries = []
    cached_env = ctx.env
    for platform in ctx.env.TARGET_PLATFORMS:
        ctx.env = ctx.all_envs[platform]
        ctx.set_group(ctx.env.PLATFORM_NAME)
        app_elf = '{}/pebble-app.elf'.format(ctx.env.BUILD_DIR)
        # source glob matches nothing: all app code is Rust. The SDK still
        # appends its generated appinfo.auto.c / resource_ids.auto.c.
        # -u main forces extraction of the Rust-provided entry point from
        # the archive (nothing else references it).
        ctx.pbl_build(source=ctx.path.ant_glob('src/c/**/*.c'),
                      target=app_elf,
                      bin_type='app',
                      stlib=[RUST_CRATE],
                      stlibpath=[rust_lib_dir],
                      linkflags=['-Wl,-u,main'])
        # waf only sees stlib as -L/-l flags, not as a link input: without
        # this, editing Rust source can silently produce a stale .pbw.
        ctx.add_manual_dependency(
            ctx.path.find_or_declare(app_elf),
            ctx.root.make_node(rust_lib))
        binaries.append({'platform': platform, 'app_elf': app_elf})
    ctx.env = cached_env

    ctx.set_group('bundle')
    ctx.pbl_bundle(binaries=binaries,
                   js=ctx.path.ant_glob(['src/pkjs/**/*.js',
                                         'src/pkjs/**/*.json',
                                         'src/common/**/*.js']),
                   js_entry_file='src/pkjs/index.js')
```

**Step 3: Create the app crate**

`examples/hello/Cargo.toml`:
```toml
[package]
name = "hello"
version = "0.1.0"
edition = "2021"
publish = false

[lib]
crate-type = ["staticlib"]

[dependencies]
ferrite = { path = "../../crates/ferrite" }

[profile.release]
opt-level = "z"
lto = true
panic = "abort"
codegen-units = 1
```

`examples/hello/.cargo/config.toml`:
```toml
[target.thumbv7m-none-eabi]
rustflags = ["-C", "relocation-model=pic", "-C", "force-unwind-tables=no"]
```

`examples/hello/src/lib.rs`:
```rust
//! Hello-world watchface: proves the Ferrite build integration.

#![no_std]

use ferrite::text_layer::TextLayer;
use ferrite::window::Window;
use ferrite::App;

ferrite::app! {
    fn main(app: &mut App) {
        ferrite::log_info(c"Hello from Rust");

        let mut window = Window::new(app);
        let bounds = window.bounds();
        let mut text = TextLayer::new(
            app,
            ferrite::sys::GRect(0, bounds.size.h / 2 - 20, bounds.size.w, 40),
        );
        text.set_text(c"Hello from Rust");
        window.add_child(&text);
        window.push();

        // Returned state stays alive while the event loop runs.
        (window, text)
    }
}
```

**Step 4: Verify the Rust half builds standalone**

Run:
```bash
cd examples/hello && cargo build --release --target thumbv7m-none-eabi
ls target/thumbv7m-none-eabi/release/libhello.a
```
Expected: build succeeds; `libhello.a` exists.

**Step 5: Commit**

```bash
cd ../..   # back to repo root
git add examples/hello
git commit -m "feat(examples): hello watchface pebble project with cargo-in-waf build"
```
<!-- END_TASK_5 -->

<!-- START_TASK_6 -->
### Task 6: End-to-end verification — pebble build, emulator, logs

**Files:** none (verification only; plus one commit if fixes were needed).

**Step 1: Full SDK build**

Run:
```bash
cd examples/hello && pebble build
```
Expected: waf configure + build succeed, ending with `'build' finished successfully`. The bundle exists:
```bash
ls build/hello.pbw
```

If the link fails with duplicate `__aeabi_*` symbols (Rust's compiler-builtins vs GCC's libgcc), add `-Wl,--allow-multiple-definition` to the `linkflags` list in `examples/hello/wscript` and rebuild. If it fails with *undefined* `__aeabi_*` symbols, the SDK's linker script discards `libgcc.a` — Rust's compiler-builtins (already in `libhello.a`) should provide them; check the map file `build/emery/pebble-app.map` to see which object pulled the symbol.

**Step 2: Install into the Emery emulator and watch logs**

Run:
```bash
pebble install --emulator emery --logs
```
Expected: the QEMU emulator boots, the app installs, and the streamed logs contain the line logged from Rust, e.g.:
```
[INFO] rust:0: Hello from Rust
```
Leave it running for ~10 seconds to confirm no crash/fault appears, then Ctrl-C.

If install fails during metadata injection or the app faults immediately on launch, inspect sections:
```bash
"/Users/thomas/Library/Application Support/Pebble SDK/SDKs/4.17/toolchain/arm-none-eabi/bin/arm-none-eabi-readelf" -S build/emery/pebble-app.elf
```
`.ARM.extab` must be absent and `.ARM.exidx` absent-or-tiny (the C SDK build tolerates a small orphan one). If they contain entries, confirm `panic = "abort"` is in `examples/hello/Cargo.toml` and `force-unwind-tables=no` is in `examples/hello/.cargo/config.toml`.

**Known latent risk — prebuilt `core` and relocations.** `-C relocation-model=pic` applies to code we compile (the app crate and its path dependencies), but the `core`/`compiler_builtins` rlibs shipped by rustup are prebuilt without it, and rebuilding them (`build-std`) is explicitly out of scope per the design. If the app installs but faults at runtime inside `core` code paths (formatting, slice/`memcpy` helpers) while our own code runs fine, suspect absolute relocations from those prebuilt objects that the Pebble loader doesn't fix up. Diagnose with:
```bash
"/Users/thomas/Library/Application Support/Pebble SDK/SDKs/4.17/toolchain/arm-none-eabi/bin/arm-none-eabi-readelf" -r build/emery/pebble-app.elf
```
and look for `R_ARM_ABS32` entries against `.text` symbols that live outside `.got`/`.data`. If this occurs, STOP and surface it to the user — the candidate mitigations (accept-and-avoid the affected core paths, or revisit the stable-only constraint) are a design decision, not an implementation detail.

**Step 3: Screenshot proof**

Run:
```bash
pebble screenshot --emulator emery --no-open hello.png
```
Then view `hello.png` (Read tool / open it): the screen must show "Hello from Rust". Delete the screenshot afterwards (`rm hello.png`) — it is not a repo artifact.

**Step 4: Commit (only if fixes were made)**

If Steps 1–3 required changes (e.g. wscript linkflags), commit them:
```bash
cd ../.. && git add -A && git commit -m "fix: adjust link flags for rust staticlib integration"
```

**Phase complete when:** `pebble build` succeeds, the emulator shows "Hello from Rust", and the Rust log line appears in the streamed logs.
<!-- END_TASK_6 -->

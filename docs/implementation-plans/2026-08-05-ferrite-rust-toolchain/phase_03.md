# Ferrite Rust Toolchain Implementation Plan — Phase 3: Runtime completion and smoke test

**Goal:** Complete the runtime layer (SDK-backed global allocator, log module) and add a repeatable end-to-end emulator smoke test (`scripts/check.sh`).

**Architecture:** `#[global_allocator]` over the SDK's `malloc`/`free` (firmware-trampoline symbols confirmed present in `libpebble.a`), with an over-alignment shim since the firmware heap's alignment guarantee is conservatively assumed to be 4 bytes. A `log` module wraps `app_log` levels with both `&CStr` and `format_args!` entry points sharing the panic handler's fixed-buffer formatter. The hello example gains a per-second heartbeat log that exercises the allocator and tick service; `check.sh` builds, installs to the Emery emulator, and greps streamed logs for the heartbeat.

**Tech Stack:** Rust `alloc` crate over SDK heap; bash + pebble-tool CLI for the smoke test.

**Scope:** Phase 3 of 7 from `docs/design-plans/2026-08-05-ferrite-rust-toolchain.md`.

**Codebase verified:** 2026-08-05. Verified in `libpebble.a` (via bundled `arm-none-eabi-nm`): `malloc`, `free`, `calloc`, `realloc`, `heap_bytes_free`, `heap_bytes_used`, `memcpy`, `memset`, `strlen` all defined (`T`). Verified in `pebble.h`: `typedef void (*TickHandler)(struct tm *tick_time, TimeUnits units_changed)` (line 903), `void tick_timer_service_subscribe(TimeUnits, TickHandler)` (911). `struct tm` is newlib's nine-`int` struct (no gmtoff/zone fields on this toolchain). Verified pebble CLI: `pebble install --logs` exists; `pebble logs` streams forever (no timeout flag); `pebble kill [--force]` and `pebble screenshot <filename> --no-open` exist.

---

## Context for the implementing engineer (read first)

- **Allocator alignment.** The SDK's `malloc` is a jump-table trampoline into the firmware heap. Its alignment guarantee is not documented; we conservatively assume 4 bytes (ARM word). Rust's `u64`/`f64` want 8-byte alignment on this target, so allocations with `align > 4` go through an over-allocate-and-store-original-pointer shim. This is a correctness decision — do not simplify it away.
- **OOM path.** On stable Rust, a failed allocation in a `no_std` + `alloc` program routes through the default alloc-error handler, which panics — landing in our panic handler (logs, then traps). No extra code needed; just don't add a custom `#[alloc_error_handler]` (that's unstable).
- **Design deviation, intentional:** the design sketch says "heartbeat log on a minute tick". The example subscribes to `SECOND_UNIT` instead so `check.sh` completes in seconds rather than blocking ≥60 s waiting for a minute boundary. The point of the heartbeat — exercising allocator + tick service — is unchanged; the design's own "Done when" for this phase is just that `check.sh` passes.
- **Bindgen names used below** (all present in `bindings_emery.rs` after Phase 2): `sys::tm` (transitively generated from newlib `time.h` because `TickHandler` references it), `sys::TimeUnits::SECOND_UNIT` (newtype associated const), `sys::TickHandler` = `Option<unsafe extern "C" fn(*mut tm, TimeUnits)>`, `sys::heap_bytes_free()`. Safe `extern "C" fn`s coerce to the `unsafe extern "C" fn` pointer type when wrapped in `Some(...)`.
- **`pebble logs` exits only when killed** — the smoke script runs `pebble install --logs` in the background, polls the captured output for the heartbeat marker, then kills the process. `pebble logs --help` exits 1 (CLI quirk); don't use `--help` probes in scripts.

---

<!-- START_SUBCOMPONENT_A (tasks 1-2) -->
<!-- START_TASK_1 -->
### Task 1: Global allocator over the SDK heap

**Files:**
- Create: `crates/ferrite/src/heap.rs`
- Modify: `crates/ferrite/src/lib.rs` (register the module)

(The module is named `heap`, NOT `alloc` — Phase 4 adds `extern crate alloc;` to this crate, and a sibling `mod alloc` would collide with it: `error[E0260]: the name 'alloc' is defined multiple times`.)

**Step 1: Write the allocator**

`crates/ferrite/src/heap.rs`:
```rust
//! Global allocator backed by the SDK heap (`malloc`/`free` jump-table
//! trampolines in libpebble.a). Rust and C code share one heap, so the SDK's
//! `heap_bytes_free()` stays meaningful.
//!
//! The firmware heap's alignment guarantee is undocumented; we assume 4
//! (ARM word). Stricter alignments over-allocate and stash the original
//! pointer just below the aligned block.

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use core::mem::size_of;

extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

/// Alignment we trust the SDK heap to provide.
const ASSUMED_ALIGN: usize = 4;

struct PebbleHeap;

unsafe impl GlobalAlloc for PebbleHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        if align <= ASSUMED_ALIGN {
            return malloc(layout.size()).cast();
        }
        // [ raw ... | original ptr | aligned block ... ]
        let total = layout.size() + align + size_of::<usize>();
        let raw = malloc(total) as usize;
        if raw == 0 {
            return core::ptr::null_mut();
        }
        let aligned = (raw + size_of::<usize>() + align - 1) & !(align - 1);
        *((aligned - size_of::<usize>()) as *mut usize) = raw;
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if layout.align() <= ASSUMED_ALIGN {
            free(ptr.cast());
            return;
        }
        let raw = *((ptr as usize - size_of::<usize>()) as *const usize);
        free(raw as *mut c_void);
    }
}

// Registered only for the watch target: host `cargo test` (Phase 4+) must
// keep std's allocator.
#[cfg(target_os = "none")]
#[global_allocator]
static ALLOCATOR: PebbleHeap = PebbleHeap;

/// Free bytes remaining on the app heap (SDK `heap_bytes_free`).
pub fn heap_bytes_free() -> usize {
    unsafe { crate::sys::heap_bytes_free() }
}

/// Bytes currently allocated on the app heap (SDK `heap_bytes_used`).
pub fn heap_bytes_used() -> usize {
    unsafe { crate::sys::heap_bytes_used() }
}
```

**Step 2: Register the module in `crates/ferrite/src/lib.rs`**

Add alongside the existing module declarations (ungated — only the `#[global_allocator]` static inside is target-gated, so the safe `heap_bytes_*` wrappers exist everywhere):
```rust
pub mod heap;
```

**Step 3: Verify**

Run (from repo root):
```bash
cargo check --target thumbv7m-none-eabi -p ferrite
cargo check -p ferrite
```
Expected: both pass.

**Step 4: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): global allocator over SDK heap"
```
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: Log module and shared fixed-buffer formatter

**Files:**
- Create: `crates/ferrite/src/fmt_buf.rs`
- Create: `crates/ferrite/src/log.rs`
- Modify: `crates/ferrite/src/panic.rs` (use the shared buffer)
- Modify: `crates/ferrite/src/lib.rs` (register modules, remove `log_info`)

**Step 1: Extract the fixed-size formatter**

`crates/ferrite/src/fmt_buf.rs`:
```rust
//! Fixed-size, NUL-terminated format buffer — the no-allocation path for
//! turning `format_args!` into a C string for `app_log`.

pub(crate) struct FixedBuf {
    buf: [u8; 128],
    len: usize,
}

impl FixedBuf {
    pub(crate) fn new() -> FixedBuf {
        FixedBuf {
            buf: [0; 128],
            len: 0,
        }
    }

    /// NUL-terminated view; valid C string.
    pub(crate) fn as_cstr_ptr(&mut self) -> *const core::ffi::c_char {
        self.buf[self.len] = 0;
        self.buf.as_ptr().cast()
    }
}

impl core::fmt::Write for FixedBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let space = self.buf.len() - 1 - self.len; // reserve NUL
        let n = s.len().min(space);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}
```

**Step 2: Write the log module**

`crates/ferrite/src/log.rs`:
```rust
//! Logging via the SDK's `app_log` — lines appear in `pebble logs`.
//!
//! Two entry points per level: `&CStr` (zero formatting cost — prefer for
//! fixed messages) and the `error!`/`warn!`/`info!`/`debug!` macros
//! (`format_args!`-based, truncated at 127 bytes).

use core::ffi::CStr;
use core::fmt::Write;

use crate::fmt_buf::FixedBuf;
use crate::sys;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    fn raw(self) -> u8 {
        match self {
            Level::Error => sys::AppLogLevel::APP_LOG_LEVEL_ERROR.0,
            Level::Warn => sys::AppLogLevel::APP_LOG_LEVEL_WARNING.0,
            Level::Info => sys::AppLogLevel::APP_LOG_LEVEL_INFO.0,
            Level::Debug => sys::AppLogLevel::APP_LOG_LEVEL_DEBUG.0,
        }
    }
}

/// Log a fixed C-string message at the given level.
pub fn log(level: Level, msg: &CStr) {
    unsafe {
        sys::app_log(level.raw(), c"rust".as_ptr(), 0, c"%s".as_ptr(), msg.as_ptr());
    }
}

pub fn error(msg: &CStr) {
    log(Level::Error, msg);
}

pub fn warn(msg: &CStr) {
    log(Level::Warn, msg);
}

pub fn info(msg: &CStr) {
    log(Level::Info, msg);
}

pub fn debug(msg: &CStr) {
    log(Level::Debug, msg);
}

/// Format-and-log backend for the level macros. Truncates at 127 bytes.
pub fn log_fmt(level: Level, args: core::fmt::Arguments) {
    let mut buf = FixedBuf::new();
    let _ = buf.write_fmt(args);
    unsafe {
        sys::app_log(
            level.raw(),
            c"rust".as_ptr(),
            0,
            c"%s".as_ptr(),
            buf.as_cstr_ptr(),
        );
    }
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::Level::Error, ::core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::Level::Warn, ::core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::Level::Info, ::core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::Level::Debug, ::core::format_args!($($arg)*))
    };
}
```

**Step 3: Rewrite the panic handler over the shared buffer**

`crates/ferrite/src/panic.rs` (replace entirely):
```rust
//! Panic = log via app_log, then trap so the firmware's app-fault path
//! terminates the app. There is no unwinding (panic = "abort").

use core::fmt::Write;

use crate::fmt_buf::FixedBuf;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let mut out = FixedBuf::new();
    let _ = write!(out, "{}", info);
    unsafe {
        crate::sys::app_log(
            crate::sys::AppLogLevel::APP_LOG_LEVEL_ERROR.0,
            c"rust".as_ptr(),
            0,
            c"%s".as_ptr(),
            out.as_cstr_ptr(),
        );
    }
    // Undefined instruction: firmware kills the app through its fault handler.
    loop {
        unsafe { core::arch::asm!("udf #255") };
    }
}
```

**Step 4: Update `crates/ferrite/src/lib.rs`**

- Add module declarations (`fmt_buf` and `log` compile for host too — `log`'s `app_log` call links only on target, but type-checks everywhere):
  ```rust
  mod fmt_buf;
  pub mod log;
  ```
- **Delete** the old `pub fn log_info` function and the now-unused `use core::ffi::CStr;` import (the `App` struct, `app!` macro, and module list stay).
- **Finalize the `app!` cleanup semantics** (design Phase 3 item): the Phase 1 contract — the setup block's value is kept alive across `app_event_loop()` and dropped afterward, running all wrapper destructors — IS the final contract; no macro code change is needed. Extend the macro's doc comment to state this explicitly by appending to it:
  ```rust
  /// Cleanup: when the event loop returns (user exits the app), the kept
  /// state is dropped in reverse declaration order, running every wrapper's
  /// `Drop` (windows destroy their SDK objects, services unsubscribe).
  /// This is the finalized lifecycle — apps never manage teardown manually.
  ```

**Step 5: Verify**

Run (from repo root):
```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo check -p ferrite-sys -p ferrite
```
Expected: both pass. (`examples/hello` still references `ferrite::log_info` — it's fixed in Task 3; hello is outside the workspace so these checks don't touch it.)

**Step 6: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): log module with level macros; panic handler shares fixed buffer"
```
<!-- END_TASK_2 -->
<!-- END_SUBCOMPONENT_A -->

<!-- START_TASK_3 -->
### Task 3: Heartbeat in the hello example

**Files:**
- Modify: `examples/hello/src/lib.rs` (replace entirely)

**Step 1: Rewrite the example with a heartbeat tick**

`examples/hello/src/lib.rs`:
```rust
//! Hello-world watchface with a heartbeat log: exercises the allocator
//! (Box) and the tick service, and is the target of scripts/check.sh.

#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

use ferrite::text_layer::TextLayer;
use ferrite::window::Window;
use ferrite::{sys, App};

static TICKS: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_tick(_tick_time: *mut sys::tm, _units_changed: sys::TimeUnits) {
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    // Round-trip through the SDK heap so the smoke test exercises the
    // allocator on every heartbeat.
    let boxed = alloc::boxed::Box::new(n);
    let free = ferrite::heap::heap_bytes_free();
    ferrite::info!("HEARTBEAT {} heap_free={}", *boxed, free);
}

ferrite::app! {
    fn main(app: &mut App) {
        ferrite::log::info(c"hello starting");

        let mut window = Window::new(app);
        let bounds = window.bounds();
        let mut text = TextLayer::new(
            app,
            ferrite::sys::GRect(0, bounds.size.h / 2 - 20, bounds.size.w, 40),
        );
        text.set_text(c"Hello from Rust");
        window.add_child(&text);
        window.push();

        // Tick service is wrapped safely in Phase 6; raw sys is fine here.
        unsafe {
            sys::tick_timer_service_subscribe(sys::TimeUnits::SECOND_UNIT, Some(on_tick));
        }

        (window, text)
    }
}
```

**Step 2: Verify it builds and beats**

Run:
```bash
cd examples/hello && pebble build
pebble install --emulator emery --logs
```
Expected: within a few seconds the log stream shows increasing heartbeats, e.g.
```
[INFO] rust:0: HEARTBEAT 1 heap_free=...
[INFO] rust:0: HEARTBEAT 2 heap_free=...
```
`heap_free` should stabilize (not shrink monotonically — that would mean the Box leaks). Ctrl-C.

**Step 3: Commit**

```bash
cd ../.. && git add examples/hello/src/lib.rs
git commit -m "feat(examples): heartbeat log exercising allocator and tick service"
```
<!-- END_TASK_3 -->

<!-- START_TASK_4 -->
### Task 4: scripts/check.sh — repeatable smoke test

**Files:**
- Create: `scripts/check.sh` (mode 755)

**Note:** the script body below already includes the two test suites Phase 1
checked in (`cargo test -p ferrite` and the Python guardrail tests). They have
no runner until this script exists, so keep them when you copy the block.

**Step 1: Write the script**

`scripts/check.sh`:
```bash
#!/usr/bin/env bash
# End-to-end smoke test: build hello, install into the Emery emulator,
# verify the Rust heartbeat appears in the logs.
#
# Prerequisites: pebble-tool 5.0.x with SDK 4.17, and
# `rustup target add thumbv7m-none-eabi`.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELLO_DIR="$REPO_ROOT/examples/hello"
MARKER="HEARTBEAT"
TIMEOUT_SECS=120   # first emulator boot can be slow

echo "==> cargo checks"
(cd "$REPO_ROOT" && cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite)

echo "==> unit tests"
# Added in Phase 1. These guard logic that regressed more than once: the
# wscript relocation guardrail (which catches -C relocation-model=pic failing
# to reach rustc) and FixedBuf's UTF-8 truncation in the panic handler.
(cd "$REPO_ROOT" && cargo test -p ferrite)
(cd "$REPO_ROOT" && python3 -m unittest discover -s examples/hello/tests)

echo "==> pebble build"
(cd "$HELLO_DIR" && pebble build)

echo "==> install to emery emulator and watch logs (up to ${TIMEOUT_SECS}s)"
LOG_FILE="$(mktemp)"
cleanup() {
    if [[ -n "${INSTALL_PID:-}" ]] && kill -0 "$INSTALL_PID" 2>/dev/null; then
        kill "$INSTALL_PID" 2>/dev/null || true
        wait "$INSTALL_PID" 2>/dev/null || true
    fi
    rm -f "$LOG_FILE"
}
trap cleanup EXIT

(cd "$HELLO_DIR" && pebble install --emulator emery --logs >"$LOG_FILE" 2>&1) &
INSTALL_PID=$!

for _ in $(seq 1 "$TIMEOUT_SECS"); do
    if grep -q "panicked" "$LOG_FILE"; then
        echo "FAIL: app panicked:"
        grep "panicked" "$LOG_FILE"
        exit 1
    fi
    if grep -q "$MARKER" "$LOG_FILE"; then
        echo "PASS: heartbeat observed:"
        grep -m 3 "$MARKER" "$LOG_FILE"
        exit 0
    fi
    if ! kill -0 "$INSTALL_PID" 2>/dev/null; then
        echo "FAIL: pebble install exited early:"
        tail -20 "$LOG_FILE"
        exit 1
    fi
    sleep 1
done

echo "FAIL: no '$MARKER' within ${TIMEOUT_SECS}s. Last log lines:"
tail -20 "$LOG_FILE"
exit 1
```

Make it executable:
```bash
chmod +x scripts/check.sh
```

**Step 2: Run it**

Run (from repo root):
```bash
./scripts/check.sh
```
Expected: ends with `PASS: heartbeat observed:` and three heartbeat lines, exit code 0.

Run it twice in a row — the second run (emulator already booted) should pass faster. If the second run hangs at install, run `pebble kill --force` and retry once; if that was needed, add a `pebble kill --force || true` line before the install block in the script and re-verify.

**Step 3: Commit**

```bash
git add scripts/check.sh
git commit -m "feat(scripts): end-to-end emulator smoke test"
```

**Phase complete when:** `./scripts/check.sh` prints PASS from a clean state (`cd examples/hello && cargo clean && rm -rf build` beforehand proves the from-scratch path).
<!-- END_TASK_4 -->

# Ferrite Rust Toolchain Implementation Plan — Phase 6: Services — AppMessage, persist, health, tick

**Goal:** Safe wrappers for the non-UI SDK services Fitter depends on: inbound AppMessage with dictionary reading, persist storage returning `Result`, HealthService with graceful absence, and the tick service via the static-slot mechanism. The example receives a message from a PKJS stub and persists a launch counter across relaunches.

**Architecture:** AppMessage and Health are context-carrying services (boxed state via `app_message_set_context` / the subscribe `context` arg, trampolines dispatch). Tick is context-less (bare fn pointer), so `ferrite` holds the closure in a private static slot — sound on this single-threaded platform. Dictionary reading wraps `dict_find` with lifetime-bound `Dict`/`Tuple` views; integer decoding is a pure, host-tested helper. Persist maps the SDK's mixed `status_t`/`int` returns into `Result`.

**Tech Stack:** Same as prior phases; adds a PKJS JavaScript stub to the example (already wired in the wscript's `pbl_bundle` glob).

**Scope:** Phase 6 of 7 from `docs/design-plans/2026-08-05-ferrite-rust-toolchain.md`.

**Codebase verified:** 2026-08-05. Verified `pebble.h` (SDK 4.17): `app_message_open(uint32_t, uint32_t) -> AppMessageResult` (2347), `app_message_set_context(void*) -> void*` returns the previous context (2424), register fns return the previous callback (2436–2474), `AppMessageInboxReceived = void (*)(DictionaryIterator*, void*)` (2364), `AppMessageInboxDropped = void (*)(AppMessageResult reason, void*)` (2378), `Tuple* dict_find(const DictionaryIterator*, uint32_t)` (2091). `Tuple` is packed: `u32 key; TupleType type:8; u16 length;` + flexible-array `value[]` union (1781). Persist: `write_bool`/`write_int`/`delete` return `status_t` (i32); `read_data`/`read_string`/`write_data`/`write_string`/`get_size` return plain `int` (byte count or negative StatusCode); `E_DOES_NOT_EXIST = -9`; `PERSIST_DATA_MAX_LENGTH = 256` (3081). Health: `health_service_events_subscribe(HealthEventHandler, void *context) -> bool` (1300; allocates ~2 KB, false on OOM), `HealthEventHandler = void (*)(HealthEventType, void*)` (1291), `health_service_peek_current_value(HealthMetric) -> HealthValue(i32)` (1018), `health_service_metric_accessible(HealthMetric, time_t, time_t)` (1222), `health_service_set_heart_rate_sample_period(uint16_t) -> bool` (1329). Tick: `tick_timer_service_subscribe(TimeUnits, TickHandler)` (911) — no context. **Message keys** (verified from Fitter's generated build files): `package.json` `messageKeys` array entries are assigned values sequentially **starting at 10000, in array order**; the SDK generates them as `uint32_t` globals compiled into the app — Ferrite apps just declare matching `const u32`s in Rust.

---

## Context for the implementing engineer (read first)

- **CORRECTED during Phase 6 (before execution) — the listings below predate
  the Phase 4/5 review fixes. The notes win over the code:**
  1. **Trampoline discipline (Tasks 1 and 4).** `on_inbox_received`,
     `on_inbox_dropped` and `on_health_event` as listed hold
     `&mut AppMessageState`/`&mut HealthState` across user-closure calls —
     the Miri-proven-UB shape. Use take/call/restore (house pattern in
     click.rs/canvas.rs/menu_layer.rs): take the closure out
     (statement-scoped borrow), run borrow-free, restore only into an empty
     slot. `HealthState.on_event` must become `Option<Box<...>>` for this.
     Document the discipline + residual contract (dropping the service
     wrapper from inside its own callback unsupported) as the other modules
     do; a callback_depth backstop matches house style.
  2. **Task 5's return tuple is wrong twice.** `(home, menu, messages)`
     silently DROPS `text` and `canvas` (their screens go blank — the
     Phase 4 bug class — and check.sh's navigation/canvas legs fail), and
     orders `home` before `menu` (parent before child). Keep all Phase 5
     state, children before parents, services anywhere before home:
     `(text, canvas, menu, messages, home)`.
  3. **Task 5's tick closure drops the u64 box and log field (third
     recurrence).** check.sh counts a heartbeat line as complete only when
     it matches `heap_free=[0-9]+ u64=[0-9]+`, and the Box<u64> is the only
     on-device exercise of the allocator's over-alignment shim. The safe
     tick closure must keep both boxes and the exact format
     `"HEARTBEAT {} heap_free={} u64={}"`.

- **AppMessage has ONE global context and four callback slots.** We register inbox received/dropped trampolines and set the context to a boxed `AppMessageState`. The wrapper's `Drop` deregisters and clears the context. Outbound messaging is out of scope (Fitter is inbound-only); the raw sys API remains available.
- **Reading `Tuple` fields must respect packing.** The struct is `#[repr(C, packed)]` in the bindings; the `type` bitfield is read via the generated accessor (check its exact name — bindgen renames the C field `type` to `type_`), and `length`/`key` reads from a packed struct copy by value (allowed). The value bytes start at the flexible-array member; decode integers by copying bytes (no unaligned typed loads).
- **Static slot pattern for tick** (design-mandated): a `static` `UnsafeCell<Option<Box<closure>>>` with a manual `Sync` impl, justified by the single-threaded platform. The trampoline *takes* the closure out, calls it, and restores it only if the slot is still empty — so a closure that resubscribes from inside itself can't free its own executing body.
- **Health "graceful absence":** `subscribe` returns `Option` (SDK returns false on OOM/unsupported); `peek`/`accessible` are plain calls that return 0 / not-supported values on watches without the sensor. Don't panic on absence.
- **PKJS**: `require('message_keys')` in JS resolves key names to the same auto-assigned numbers; `Pebble.sendAppMessage(dict, ok, err)` sends on the phone side. The example uses one key `PING` → 10000. PKJS `console.log` lines appear in `pebble logs` prefixed with the JS filename.

---

<!-- START_SUBCOMPONENT_A (tasks 1-2) -->
<!-- START_TASK_1 -->
### Task 1: app_message module — inbox with dictionary reading

**Files:**
- Create: `crates/ferrite/src/app_message.rs`
- Modify: `crates/ferrite/src/lib.rs` (add `pub mod app_message;`)

**Step 1: Write the failing tests (pure decode helpers)**

`crates/ferrite/src/app_message.rs`:
```rust
//! Inbound AppMessage: inbox open/received/dropped with context-based
//! closures, and safe dictionary reading over `dict_find`.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_uint_widths() {
        assert_eq!(decode_uint(&[0x2a]), Some(42));
        assert_eq!(decode_uint(&[0x34, 0x12]), Some(0x1234));
        assert_eq!(decode_uint(&[0x78, 0x56, 0x34, 0x12]), Some(0x1234_5678));
        assert_eq!(decode_uint(&[1, 2, 3]), None); // no 3-byte ints
        assert_eq!(decode_uint(&[]), None);
    }

    #[test]
    fn decode_int_sign_extends() {
        assert_eq!(decode_int(&[0xff]), Some(-1));
        assert_eq!(decode_int(&[0xfe, 0xff]), Some(-2));
        assert_eq!(decode_int(&[0xfd, 0xff, 0xff, 0xff]), Some(-3));
        assert_eq!(decode_int(&[0x2a]), Some(42));
    }
}
```

**Step 2: Run tests to verify they fail**

```bash
cargo test -p ferrite
```
Expected: FAIL to compile — `decode_uint`/`decode_int` undefined.

**Step 3: Implement the module**

Insert above the test module:
```rust
use alloc::boxed::Box;
use core::ffi::{c_void, CStr};
use core::marker::PhantomData;

use crate::sys;
use crate::App;

pub use crate::sys::TupleType;

/// Little-endian unsigned decode for AppMessage integer tuples (1/2/4 bytes).
fn decode_uint(bytes: &[u8]) -> Option<u32> {
    match bytes.len() {
        1 => Some(bytes[0] as u32),
        2 => Some(u16::from_le_bytes([bytes[0], bytes[1]]) as u32),
        4 => Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ => None,
    }
}

/// Little-endian signed decode for AppMessage integer tuples (1/2/4 bytes).
fn decode_int(bytes: &[u8]) -> Option<i32> {
    match bytes.len() {
        1 => Some(bytes[0] as i8 as i32),
        2 => Some(i16::from_le_bytes([bytes[0], bytes[1]]) as i32),
        4 => Some(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
        _ => None,
    }
}

/// A received dictionary, valid only inside the receive closure.
pub struct Dict<'a> {
    iter: *mut sys::DictionaryIterator,
    _lifetime: PhantomData<&'a ()>,
}

impl Dict<'_> {
    pub fn find(&self, key: u32) -> Option<Tuple<'_>> {
        let raw = unsafe { sys::dict_find(self.iter, key) };
        if raw.is_null() {
            None
        } else {
            Some(Tuple {
                raw,
                _lifetime: PhantomData,
            })
        }
    }
}

/// One key/value entry of a received dictionary.
pub struct Tuple<'a> {
    raw: *mut sys::Tuple,
    _lifetime: PhantomData<&'a ()>,
}

impl Tuple<'_> {
    pub fn key(&self) -> u32 {
        unsafe { (*self.raw).key }
    }

    /// The tuple's wire type (int/uint/bytes/cstring).
    pub fn tuple_type(&self) -> TupleType {
        // `type` is a C bitfield; bindgen generates an accessor (named after
        // the renamed field, e.g. `type_()`).
        unsafe { (*self.raw).type_() }
    }

    /// Length in bytes of the tuple's value.
    pub fn length(&self) -> u16 {
        unsafe { (*self.raw).length }
    }

    fn value_bytes_raw(&self) -> &[u8] {
        unsafe {
            let len = (*self.raw).length as usize;
            let ptr = (*self.raw).value.as_ptr() as *const u8;
            core::slice::from_raw_parts(ptr, len)
        }
    }

    /// Integer value (signed or unsigned tuple types), sign-extended to i32.
    pub fn value_i32(&self) -> Option<i32> {
        match self.tuple_type() {
            sys::TupleType::TUPLE_INT => decode_int(self.value_bytes_raw()),
            sys::TupleType::TUPLE_UINT => {
                decode_uint(self.value_bytes_raw()).map(|v| v as i32)
            }
            _ => None,
        }
    }

    /// Raw bytes of a byte-array tuple.
    pub fn value_bytes(&self) -> Option<&[u8]> {
        (self.tuple_type() == sys::TupleType::TUPLE_BYTE_ARRAY)
            .then(|| self.value_bytes_raw())
    }

    /// C-string value of a cstring tuple.
    pub fn value_cstr(&self) -> Option<&CStr> {
        if self.tuple_type() != sys::TupleType::TUPLE_CSTRING {
            return None;
        }
        CStr::from_bytes_until_nul(self.value_bytes_raw()).ok()
    }
}

struct AppMessageState {
    on_received: Option<Box<dyn FnMut(&Dict<'_>)>>,
    on_dropped: Option<Box<dyn FnMut(sys::AppMessageResult)>>,
}

/// The AppMessage service (inbound). At most one instance should exist.
pub struct AppMessage {
    state: *mut AppMessageState, // Box, owned; freed in Drop
}

impl AppMessage {
    /// Registers callbacks and opens the inbox/outbox with the given sizes.
    pub fn open(
        _app: &mut App,
        inbox_size: u32,
        outbox_size: u32,
    ) -> Result<AppMessage, sys::AppMessageResult> {
        let state = Box::into_raw(Box::new(AppMessageState {
            on_received: None,
            on_dropped: None,
        }));
        unsafe {
            sys::app_message_set_context(state.cast());
            sys::app_message_register_inbox_received(Some(on_inbox_received));
            sys::app_message_register_inbox_dropped(Some(on_inbox_dropped));
            let r = sys::app_message_open(inbox_size, outbox_size);
            if r != sys::AppMessageResult::APP_MSG_OK {
                sys::app_message_deregister_callbacks();
                sys::app_message_set_context(core::ptr::null_mut());
                drop(Box::from_raw(state));
                return Err(r);
            }
        }
        Ok(AppMessage { state })
    }

    pub fn on_received(&mut self, f: impl FnMut(&Dict<'_>) + 'static) {
        unsafe { &mut *self.state }.on_received = Some(Box::new(f));
    }

    pub fn on_dropped(&mut self, f: impl FnMut(sys::AppMessageResult) + 'static) {
        unsafe { &mut *self.state }.on_dropped = Some(Box::new(f));
    }
}

impl Drop for AppMessage {
    fn drop(&mut self) {
        unsafe {
            sys::app_message_deregister_callbacks();
            sys::app_message_set_context(core::ptr::null_mut());
            drop(Box::from_raw(self.state));
        }
    }
}

unsafe extern "C" fn on_inbox_received(
    iterator: *mut sys::DictionaryIterator,
    context: *mut c_void,
) {
    let state = &mut *(context as *mut AppMessageState);
    let dict = Dict {
        iter: iterator,
        _lifetime: PhantomData,
    };
    if let Some(f) = state.on_received.as_mut() {
        f(&dict);
    }
}

unsafe extern "C" fn on_inbox_dropped(reason: sys::AppMessageResult, context: *mut c_void) {
    let state = &mut *(context as *mut AppMessageState);
    if let Some(f) = state.on_dropped.as_mut() {
        f(reason);
    }
}
```

In `crates/ferrite/src/lib.rs`, add:
```rust
pub mod app_message;
```

Adjustments that may be needed against the generated bindings (the bindings are the source of truth — check `bindings_emery.rs`):
- The bitfield accessor name for `Tuple.type` (`grep -B 2 -A 10 "pub struct Tuple" crates/ferrite-sys/src/bindings_emery.rs`).
- `dict_find` takes `*const DictionaryIterator` — pass `self.iter` (auto-coerces) or `.cast_const()` as the compiler requires.

**Step 4: Run tests to verify they pass**

```bash
cargo test -p ferrite
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
```
Expected: tests pass; target check passes.

**Step 5: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): AppMessage inbox with safe dictionary reading"
```
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: persist module

**Files:**
- Create: `crates/ferrite/src/persist.rs`
- Modify: `crates/ferrite/src/lib.rs` (add `pub mod persist;`)

**Step 1: Write the failing test (error mapping)**

`crates/ferrite/src/persist.rs`:
```rust
//! Persistent key-value storage, wrapping the SDK `persist_*` API with
//! `Result`s. Values are capped at `PERSIST_DATA_MAX_LENGTH` (256) bytes.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_mapping_covers_sdk_codes() {
        assert_eq!(Error::from_code(-9), Error::DoesNotExist);
        assert_eq!(Error::from_code(-6), Error::OutOfStorage);
        assert_eq!(Error::from_code(-8), Error::Range);
        assert_eq!(Error::from_code(-4), Error::InvalidArgument);
        assert_eq!(Error::from_code(-1), Error::Other(-1));
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p ferrite
```
Expected: FAIL to compile.

**Step 3: Implement the module**

Insert above the test module:
```rust
use crate::sys;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// No value stored under this key (`E_DOES_NOT_EXIST`).
    DoesNotExist,
    /// Storage quota exhausted (`E_OUT_OF_STORAGE`).
    OutOfStorage,
    /// Value or buffer out of range (`E_RANGE`).
    Range,
    /// Bad argument (`E_INVALID_ARGUMENT`).
    InvalidArgument,
    /// Any other negative SDK status code.
    Other(i32),
}

impl Error {
    fn from_code(code: i32) -> Error {
        match code {
            -9 => Error::DoesNotExist,
            -6 => Error::OutOfStorage,
            -8 => Error::Range,
            -4 => Error::InvalidArgument,
            other => Error::Other(other),
        }
    }
}

pub type Result<T> = core::result::Result<T, Error>;

fn check(code: i32) -> Result<i32> {
    if code < 0 {
        Err(Error::from_code(code))
    } else {
        Ok(code)
    }
}

pub fn exists(key: u32) -> bool {
    unsafe { sys::persist_exists(key) }
}

/// Reads a blob into `buf`; returns the number of bytes read.
pub fn read_data(key: u32, buf: &mut [u8]) -> Result<usize> {
    check(unsafe { sys::persist_read_data(key, buf.as_mut_ptr().cast(), buf.len()) })
        .map(|n| n as usize)
}

/// Writes a blob (max 256 bytes); returns the number of bytes written.
pub fn write_data(key: u32, data: &[u8]) -> Result<usize> {
    check(unsafe { sys::persist_write_data(key, data.as_ptr().cast(), data.len()) })
        .map(|n| n as usize)
}

pub fn read_int(key: u32) -> Result<i32> {
    if !exists(key) {
        return Err(Error::DoesNotExist);
    }
    Ok(unsafe { sys::persist_read_int(key) })
}

pub fn write_int(key: u32, value: i32) -> Result<()> {
    check(unsafe { sys::persist_write_int(key, value) }).map(|_| ())
}

pub fn read_bool(key: u32) -> Result<bool> {
    if !exists(key) {
        return Err(Error::DoesNotExist);
    }
    Ok(unsafe { sys::persist_read_bool(key) })
}

pub fn write_bool(key: u32, value: bool) -> Result<()> {
    check(unsafe { sys::persist_write_bool(key, value) }).map(|_| ())
}

/// Size in bytes of the stored value, if any.
pub fn size(key: u32) -> Result<usize> {
    check(unsafe { sys::persist_get_size(key) }).map(|n| n as usize)
}

pub fn delete(key: u32) -> Result<()> {
    check(unsafe { sys::persist_delete(key) }).map(|_| ())
}
```

Note: `persist_write_bool`/`persist_write_int`/`persist_delete` return `status_t` (i32 in the bindings) and the data variants return `c_int` — if the compiler complains about a type mismatch in `check(...)`, add `as i32` to the call argument; the values are the same width.

**Step 4: Run tests to verify they pass**

```bash
cargo test -p ferrite
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
```
Expected: pass.

**Step 5: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): persist storage with Result-based API"
```
<!-- END_TASK_2 -->
<!-- END_SUBCOMPONENT_A -->

<!-- START_SUBCOMPONENT_B (tasks 3-4) -->
<!-- START_TASK_3 -->
### Task 3: tick module — static-slot closure

**Files:**
- Create: `crates/ferrite/src/tick.rs`
- Modify: `crates/ferrite/src/lib.rs` (add `pub mod tick;`)

**Step 1: Write the failing test (tm conversion)**

`crates/ferrite/src/tick.rs`:
```rust
//! Tick timer service. The SDK callback carries no context pointer, so the
//! closure lives in a private static slot — sound because the platform is
//! single-threaded; re-subscribing replaces the slot.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_from_tm_copies_fields() {
        let tm = crate::sys::tm {
            tm_sec: 7,
            tm_min: 30,
            tm_hour: 13,
            tm_mday: 5,
            tm_mon: 7,
            tm_year: 126,
            tm_wday: 3,
            tm_yday: 216,
            tm_isdst: 0,
        };
        let t = Time::from_tm(&tm);
        assert_eq!(t.sec, 7);
        assert_eq!(t.min, 30);
        assert_eq!(t.hour, 13);
        assert_eq!(t.mday, 5);
        assert_eq!(t.mon, 7);
        assert_eq!(t.year, 126);
        assert_eq!(t.wday, 3);
    }
}
```

(**Confirmed during Phase 2:** the generated `sys::tm` *does* have extra fields — it is Pebble's own definition, not newlib's, so beyond the nine standard fields it also carries `tm_gmtoff: c_int` and `tm_zone: [c_char; 6]` (48 bytes total, `tm_gmtoff` at offset 36). Construct with `..Default::default()` for the remainder. See the corrected note at the top of `phase_03.md` for why.)

**Step 2: Run test to verify it fails**

```bash
cargo test -p ferrite
```
Expected: FAIL to compile.

**Step 3: Implement the module**

Insert above the test module:
```rust
use alloc::boxed::Box;
use core::cell::UnsafeCell;

use crate::sys;
use crate::App;

pub use crate::sys::TimeUnits;

/// Wall-clock fields copied out of the SDK's `struct tm`.
#[derive(Clone, Copy, Debug)]
pub struct Time {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    /// Day of month, 1-31.
    pub mday: i32,
    /// Month, 0-11.
    pub mon: i32,
    /// Years since 1900.
    pub year: i32,
    /// Day of week, 0 = Sunday.
    pub wday: i32,
}

impl Time {
    pub(crate) fn from_tm(tm: &sys::tm) -> Time {
        Time {
            sec: tm.tm_sec,
            min: tm.tm_min,
            hour: tm.tm_hour,
            mday: tm.tm_mday,
            mon: tm.tm_mon,
            year: tm.tm_year,
            wday: tm.tm_wday,
        }
    }
}

type TickClosure = Box<dyn FnMut(&Time, TimeUnits)>;

struct TickSlot(UnsafeCell<Option<TickClosure>>);

// SAFETY: the watch runtime is single-threaded; this static is only touched
// from the app task. (Required because statics must be Sync.)
unsafe impl Sync for TickSlot {}

static TICK_SLOT: TickSlot = TickSlot(UnsafeCell::new(None));

/// Subscribes to tick events. A second call replaces the previous closure.
pub fn subscribe(_app: &mut App, units: TimeUnits, f: impl FnMut(&Time, TimeUnits) + 'static) {
    unsafe {
        *TICK_SLOT.0.get() = Some(Box::new(f));
        sys::tick_timer_service_subscribe(units, Some(on_tick));
    }
}

/// Unsubscribes and drops the stored closure.
pub fn unsubscribe() {
    unsafe {
        sys::tick_timer_service_unsubscribe();
        *TICK_SLOT.0.get() = None;
    }
}

unsafe extern "C" fn on_tick(tick_time: *mut sys::tm, units_changed: sys::TimeUnits) {
    // Take the closure out while it runs so a re-subscribe from inside the
    // closure cannot free the executing body; restore it only if the slot
    // is still empty afterwards.
    let taken = (*TICK_SLOT.0.get()).take();
    if let Some(mut f) = taken {
        let time = Time::from_tm(&*tick_time);
        f(&time, units_changed);
        let slot = &mut *TICK_SLOT.0.get();
        if slot.is_none() {
            *slot = Some(f);
        }
    }
}
```

**Step 4: Run tests to verify they pass**

```bash
cargo test -p ferrite
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
```
Expected: pass.

**Step 5: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): tick service via static-slot closure"
```
<!-- END_TASK_3 -->

<!-- START_TASK_4 -->
### Task 4: health module

**Files:**
- Create: `crates/ferrite/src/health.rs`
- Modify: `crates/ferrite/src/lib.rs` (add `pub mod health;`)

**Step 1: Write the module**

`crates/ferrite/src/health.rs`:
```rust
//! HealthService: heart rate and activity metrics, with graceful absence —
//! on watches without a sensor, peeks return 0 and accessibility reports
//! not-supported; nothing panics.

use alloc::boxed::Box;
use core::ffi::c_void;

use crate::sys;
use crate::App;

pub use crate::sys::{HealthEventType, HealthMetric, HealthServiceAccessibilityMask};

struct HealthState {
    on_event: Box<dyn FnMut(HealthEventType)>,
}

/// Active health-event subscription; unsubscribes on drop.
pub struct Health {
    state: *mut HealthState, // Box, owned; freed in Drop
}

impl Health {
    /// Subscribes to health events. Returns `None` if the SDK refuses
    /// (out of memory or health unsupported).
    pub fn subscribe(
        _app: &mut App,
        f: impl FnMut(HealthEventType) + 'static,
    ) -> Option<Health> {
        let state = Box::into_raw(Box::new(HealthState {
            on_event: Box::new(f),
        }));
        let ok = unsafe { sys::health_service_events_subscribe(Some(on_health_event), state.cast()) };
        if ok {
            Some(Health { state })
        } else {
            unsafe { drop(Box::from_raw(state)) };
            None
        }
    }

    /// Requests a heart-rate sample period (seconds); 0 restores the default.
    pub fn set_heart_rate_sample_period(&mut self, interval_secs: u16) -> bool {
        unsafe { sys::health_service_set_heart_rate_sample_period(interval_secs) }
    }
}

impl Drop for Health {
    fn drop(&mut self) {
        unsafe {
            // Reset any HR sample-period request, then unsubscribe.
            sys::health_service_set_heart_rate_sample_period(0);
            sys::health_service_events_unsubscribe();
            drop(Box::from_raw(self.state));
        }
    }
}

/// Current value of a metric (0 when unavailable).
pub fn peek(metric: HealthMetric) -> i32 {
    unsafe { sys::health_service_peek_current_value(metric) }
}

/// Whether `metric` is available right now.
pub fn metric_available(metric: HealthMetric) -> bool {
    let now = unsafe { sys::time(core::ptr::null_mut()) };
    let mask = unsafe { sys::health_service_metric_accessible(metric, now, now) };
    // Accessibility is a bitmask — test the Available bit, don't compare
    // for equality (the SDK may OR in other reasons).
    mask.0 & sys::HealthServiceAccessibilityMask::HealthServiceAccessibilityMaskAvailable.0 != 0
}

unsafe extern "C" fn on_health_event(event: sys::HealthEventType, context: *mut c_void) {
    let state = &mut *(context as *mut HealthState);
    (state.on_event)(event);
}
```

In `crates/ferrite/src/lib.rs`, add:
```rust
pub mod health;
```

**Step 2: Verify**

```bash
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo test -p ferrite
```
Expected: pass.

**Step 3: Commit**

```bash
git add crates/ferrite/src
git commit -m "feat(ferrite): health service wrapper with graceful absence"
```
<!-- END_TASK_4 -->
<!-- END_SUBCOMPONENT_B -->

<!-- START_TASK_5 -->
### Task 5: Example — PKJS stub, message keys, persist counter, safe tick

**Files:**
- Modify: `examples/hello/package.json` (add a message key)
- Create: `examples/hello/src/pkjs/index.js`
- Modify: `examples/hello/src/lib.rs` (services demo)

**Step 1: Declare the message key**

In `examples/hello/package.json`, change the `messageKeys` line to:
```json
    "messageKeys": ["PING"],
```
(The SDK assigns values sequentially from 10000 in array order → `PING` = 10000. The Rust side declares the same constant; keep the two in sync by order.)

**Step 2: Create the PKJS stub**

`examples/hello/src/pkjs/index.js`:
```javascript
// PKJS stub: sends one PING message to the watch when the JS runtime is
// ready. Used to verify the inbound AppMessage path end to end.
var keys = require('message_keys');

Pebble.addEventListener('ready', function () {
    console.log('pkjs ready, sending PING');
    var msg = {};
    msg[keys.PING] = 42;
    Pebble.sendAppMessage(
        msg,
        function () { console.log('pkjs PING sent'); },
        function (e) { console.log('pkjs PING failed'); }
    );
});
```
(The wscript's `pbl_bundle` already globs `src/pkjs/**/*.js` — no build changes needed.)

**Step 3: Update the app**

In `examples/hello/src/lib.rs`:

1. Add near the top (below the `use` items):
```rust
use ferrite::app_message::AppMessage;

/// Must match the position of "PING" in package.json's messageKeys
/// (values are assigned sequentially from 10000 in array order).
const MESSAGE_KEY_PING: u32 = 10000;
const PERSIST_KEY_LAUNCHES: u32 = 1;
```
2. **Delete** the `static TICKS` atomic and the `extern "C" fn on_tick` — the safe tick module replaces them (`core::sync::atomic` import goes too).
3. Inside the `app!` block, replace the raw `tick_timer_service_subscribe` call with the safe subscription, and add the services, so the block body ends like this (menu/screens setup from Phase 5 stays unchanged above):
```rust
        // Persist: launch counter surviving relaunches.
        let launches = ferrite::persist::read_int(PERSIST_KEY_LAUNCHES).unwrap_or(0) + 1;
        let _ = ferrite::persist::write_int(PERSIST_KEY_LAUNCHES, launches);
        ferrite::info!("LAUNCH {}", launches);

        // AppMessage: log the PING sent by the PKJS stub.
        let mut messages = AppMessage::open(app, 256, 64).expect("app_message_open failed");
        messages.on_received(|dict| match dict.find(MESSAGE_KEY_PING) {
            Some(t) => ferrite::info!("PING received: {:?}", t.value_i32()),
            None => ferrite::log::warn(c"message without PING key"),
        });
        messages.on_dropped(|reason| ferrite::info!("inbox dropped: {}", reason.0));

        // Health: log availability once (graceful on emulator).
        let hr_ok = ferrite::health::metric_available(
            ferrite::health::HealthMetric::HealthMetricHeartRateBPM,
        );
        ferrite::info!(
            "hr available={} bpm={}",
            hr_ok,
            ferrite::health::peek(ferrite::health::HealthMetric::HealthMetricHeartRateBPM)
        );

        // Tick: heartbeat via the safe static-slot subscription.
        let mut ticks: u32 = 0;
        ferrite::tick::subscribe(app, ferrite::tick::TimeUnits::SECOND_UNIT, move |_t, _u| {
            ticks += 1;
            let boxed = alloc::boxed::Box::new(ticks);
            let free = ferrite::heap::heap_bytes_free();
            ferrite::info!("HEARTBEAT {} heap_free={}", *boxed, free);
        });

        (home, menu, messages)
```
(The returned state tuple now includes `messages` so the AppMessage subscription lives for the app's lifetime. `home` and `menu` are the Phase 5 variables.)

**Step 4: Build and verify in the emulator**

```bash
cd examples/hello && pebble build && pebble install --emulator emery --logs
```
Expected log sequence (order may interleave):
- `LAUNCH 1`
- `pkjs ready, sending PING` (JS side)
- `PING received: Some(42)` (Rust side) — **this is the DoD line**
- `hr available=... bpm=...`
- heartbeats

Then Ctrl-C and reinstall to prove the persist round-trip:
```bash
pebble install --emulator emery --logs
```
Expected: `LAUNCH 2` (counter survived the relaunch), and another `PING received: Some(42)`.

**Step 5: Commit**

```bash
cd ../.. && git add examples/hello
git commit -m "feat(examples): PKJS message, persist counter, safe tick heartbeat"
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
Expected: all host tests pass (decode helpers, persist error mapping, tm conversion, plus earlier phases'); check.sh prints PASS.

**Step 2: Commit stragglers if any**

```bash
git status --short
```
Commit anything outstanding.

**Phase complete when:** the example logs a PKJS-sent message in the emulator, the persist counter increments across relaunch (both verified in Task 5 Step 4), and host tests pass.
<!-- END_TASK_6 -->

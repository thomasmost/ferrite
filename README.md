# Ferrite

A Rust toolchain for writing PebbleOS watchapps — targeting the Pebble Time 2
("Emery") with the community-maintained rePebble SDK (pebble-tool 5.0.x /
SDK core 4.17).

Your app is a `no_std` Rust crate compiled by Cargo into a static library;
the SDK's own `pebble build` links it into the final app binary through the
SDK's prebuilt-library hook. Everything downstream — linker script, firmware
jump table, metadata injection — runs unchanged.

## Prerequisites

- **rePebble SDK**: `pebble` tool 5.0.x with SDK 4.17 installed and working
  (`pebble --version` shows `active SDK: v4.17`).
- **Rust** (stable) with the watch's target:
  `rustup target add thumbv7m-none-eabi`.

## Start an app from the template

1. Copy `examples/hello` to a new directory (anywhere).
2. In the copy's `package.json`: change `name`, `pebble.displayName`, and
   set a fresh `pebble.uuid` (run `uuidgen | tr 'A-Z' 'a-z'`).
3. In the copy's `Cargo.toml`: change `name` (this names the staticlib) and
   point the `ferrite` path dependency at this checkout, e.g.
   `ferrite = { path = "/path/to/ferrite/crates/ferrite" }`.
4. In the copy's `wscript`: set `RUST_CRATE = 'yourname'` to match the
   crate name from step 3.
5. Build and run:

   ```sh
   pebble build
   pebble install --emulator emery --logs
   ```

The app code lives in `src/lib.rs`; start from the `ferrite::app!` block.
The template is a watchapp (button-driven); for a watchface, set
`"watchapp": { "watchface": true }` in `package.json` and drop the click
handlers — the build pipeline is identical (that configuration was proven
in Ferrite's phase 1).

## Mandatory Rustflags

The Pebble loader requires specific code-generation flags that **must not be bypassed**:

- **`-C relocation-model=pic`**: Pebble apps are loaded at addresses chosen at runtime. Without PIC, every `&'static` pointer becomes garbage at runtime because code-section relocations are not fixed up by the loader.

- **`-C force-unwind-tables=no`**: Keeps `.ARM.extab` and `.ARM.exidx` sections from growing (the metadata injector does not expect them).

These flags are enforced in `examples/hello/.cargo/config.toml` and injected by the `wscript` build script to guard against environment-variable overrides. The custom `wscript` also verifies the linked ELF has no absolute relocations and fails the build immediately if any are found.

If you copy the hello template to a new project, preserve these flags and the relocation check in `wscript`.

## Size budget

Emery gives an app 128 KB of memory for code, data, heap, and stack
combined. Report your app's static footprint with:

```sh
cargo xtask size [path/to/pebble-app.elf]
```

Baseline for `examples/hello` (Ferrite 6fd6c18):

```
examples/hello/build/emery/pebble-app.elf
  .text (code+rodata):   12441 bytes
  .data (init data):       444 bytes
  .bss  (zeroed data):       8 bytes
  static footprint:      12893 / 131072 bytes (9% of app memory)
  left for heap+stack:  118179 bytes
```

Keep an eye on `.text`: heavy use of `core::fmt` is the usual cause of
sudden growth (the `ferrite::info!`-style macros are fine; formatting
floats or using `{:?}` on large types is what costs).

## Troubleshooting

**`error[E0463]: can't find crate for 'core'` (or "the `thumbv7m-none-eabi`
target may not be installed")** — the Rust cross target is missing:

```sh
rustup target add thumbv7m-none-eabi
```

**`pebble: command not found`, or the build stops with a missing-SDK error**
— install the rePebble tooling (see prerequisites above) and confirm
`pebble sdk list` shows 4.17 installed. If the SDK lives somewhere
non-standard, set `PEBBLE_SDK_ROOT=/path/to/SDKs/4.17` for `cargo xtask`
commands (the `pebble` tool itself manages its own paths).

**`cargo: command not found` during `pebble build`** — the wscript uses
`~/.cargo/bin/cargo` when it exists and otherwise looks up `cargo` on
`PATH`; if your Rust lives elsewhere, ensure `cargo` is on `PATH` in the
shell that runs `pebble build`.

## Regenerating the SDK bindings

`crates/ferrite-sys/src/bindings_emery.rs` is committed, so normal builds
never need bindgen. To regenerate (e.g. after an SDK update):

```sh
cargo xtask bindgen
```

Requires libclang (Apple Command Line Tools suffice). Bindings are pinned
to SDK 4.17/Emery — the firmware call table is index-based, so bindings and
SDK versions must move together.

## Use of `unsafe`

App code written against this toolchain is safe Rust: `examples/hello` — the
template you copy — contains **zero** `unsafe` blocks. (The one exception is
invisible: the `ferrite::app!` macro expands to a single `unsafe` call that
constructs the `App` token before handing it to your setup code.) All of the
`unsafe` lives inside `crates/ferrite` and `crates/ferrite-sys`, and this
section explains why it is there, what keeps it honest, and what could shrink
it.

### Why it is unavoidable

- **The SDK is a C API.** Every firmware call crosses an FFI boundary, and
  calling an `extern "C"` function is `unsafe` by definition in Rust. This is
  the irreducible floor: a Pebble toolchain that never says `unsafe` is not
  possible.
- **Callbacks carry `void*` context.** The SDK invokes our `extern "C"`
  trampolines with a raw context pointer; recovering the typed closure state
  from it requires raw-pointer dereferences.
- **The tick service has no context parameter at all**, so its closure lives
  in a private `static` slot — which needs a manual `unsafe impl Sync`,
  justified by the platform being single-threaded (three such statics exist,
  each with a `// SAFETY:` comment stating exactly that invariant).
- **`Tuple` (AppMessage) is a packed C struct with a flexible array member.**
  Reading its value bytes requires `offset_of!`-based raw-pointer arithmetic;
  taking a reference to a packed field would itself be undefined behavior.
- **The global allocator** implements `GlobalAlloc` over the SDK's
  `malloc`/`free` (an inherently `unsafe` trait), including an
  over-allocate-and-stash shim for alignments above the firmware heap's
  assumed 4 bytes.
- **The panic handler** ends in an `asm!("udf #255")` trap so the firmware's
  fault path terminates the app.

### What keeps it honest

- **Miri, under both aliasing models.** The full host test suite runs clean
  under `cargo +nightly miri test -p ferrite` (Stacked Borrows and Tree
  Borrows). This is not a formality: the first draft of the callback
  trampolines held `&mut` state across user-closure calls, and Miri proved
  that shape undefined behavior during review — the SDK genuinely reenters
  (installing a click provider on a visible window invokes it synchronously),
  and user closures can reenter the safe API. The project treats a Miri run
  as mandatory after any trampoline change.
- **One audited discipline across all fourteen trampolines** — the thirteen
  that dispatch user closures follow the same take/call/restore sequence
  (take the closure out of its slot, run it with no borrow of the state live,
  restore only if the slot is still empty), and the fourteenth (the click
  config provider) takes only a shared reference because it only reads. The
  discipline is documented at each site, and its invariants are pinned by
  host tests that drive the *real* `extern "C"` trampolines through
  `#[no_mangle]` stub symbols — with mutation testing used during review to
  prove each test fails when its invariant is broken.
- **Compile-time ABI assertions.** `ferrite-sys` `const`-asserts the struct
  sizes and field offsets the unsafe code relies on (packed `Tuple` at 7
  bytes, `struct tm` at 48 with `tm_gmtoff` at offset 36, one-byte short
  enums, …) against ground truth measured with the SDK's own
  `arm-none-eabi-gcc`. If a future bindings regeneration changes an ABI fact,
  the target build fails instead of the pointer math silently going wrong.
- **Debug backstops for the one documented-unsupported case.** Dropping a
  wrapper from inside its own callback would free state under the executing
  closure; every state-owning type counts callback depth and `debug_assert!`s
  it is zero in `Drop`, converting that misuse into a deterministic panic in
  debug builds.
- **Defense at the ABI layer too:** the wscript's fail-closed relocation
  guardrail (unit-tested, 29 cases) rejects any binary whose static pointers
  the Pebble loader could not fix up — a class of corruption Rust's type
  system cannot see.

### Shrinking it further

- **Centralize the trampoline pattern.** The take/call/restore sequence is
  repeated (deliberately — each site is small and independently auditable);
  a generic, once-audited callback-slot type could reduce twelve unsafe sites
  to one at the cost of less obvious per-site reasoning. Worth revisiting if
  the surface keeps growing.
- **Lint hardening.** `#![deny(unsafe_op_in_unsafe_fn)]` would force explicit
  `unsafe {}` blocks (with room for per-operation `// SAFETY:` comments)
  inside the `unsafe extern "C"` trampolines, and `#![forbid(unsafe_code)]`
  can be stamped on the modules that need none (`layer` today) to keep them
  that way.
- **Upstream changes could delete whole categories.** A tick API with a
  context parameter would eliminate the `static` slot and two of the three
  manual `Sync` impls; nothing else here is removable without giving up the
  safe-closure API itself.

What cannot shrink: the FFI calls. Past that floor, the goal of this codebase
is not zero `unsafe` but zero *unaudited* `unsafe` — every block sits behind
a documented invariant, and the invariants are machine-checked where a
machine can check them.

## Repository layout

- `crates/ferrite` — the crate apps depend on: safe API + runtime (entry
  macro, panic handler, allocator over the SDK heap).
- `crates/ferrite-sys` — raw generated FFI bindings (committed).
- `crates/xtask` — maintenance commands (`bindgen`, `size`).
- `examples/hello` — the copyable app template; also the integration test
  (`scripts/check.sh` builds it and verifies it in the Emery emulator).

## License

Licensed under either of Apache License, Version 2.0 (`LICENSE-APACHE`) or
MIT license (`LICENSE-MIT`) at your option.

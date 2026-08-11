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

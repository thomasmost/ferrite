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

## Mandatory Rustflags

The Pebble loader requires specific code-generation flags that **must not be bypassed**:

- **`-C relocation-model=pic`**: Pebble apps are loaded at addresses chosen at runtime. The firmware fixes up data-section pointers using relocations from `inject_metadata.py`, but it does not fix up code (.text). Without PIC, LLVM materializes addresses of statics as Thumb instructions (movw/movt ABS), which are never relocated: every `&'static` pointer becomes garbage at runtime, causing the app to fault on first use.

- **`-C force-unwind-tables=no`**: Keeps `.ARM.extab` and `.ARM.exidx` sections from growing (the metadata injector does not expect them and can malfunction if they are present).

These flags are enforced in `examples/hello/.cargo/config.toml` and also injected by the `wscript` build script to guard against environment-variable overrides. The custom `wscript` also checks the linked ELF for any absolute relocations that escaped the flags; if found, the build fails immediately rather than producing a broken `.pbw`.

If you copy the hello template to a new project, preserve these flags and the relocation check in `wscript`.

## Linking Mechanism

The app crate compiles to a static library (`.cargo/config.toml` sets `crate-type = ["staticlib"]`). The `wscript` build script:

1. Runs `cargo build --release --target thumbv7m-none-eabi` to produce `libhello.a`
2. Passes `stlib=[RUST_CRATE]` and `stlibpath=[rust_lib_dir]` to the SDK's `pbl_build` task, which tells the linker where to find and link the Rust archive
3. Passes `-Wl,-u,main` to force the linker to extract the Rust-provided `main` entry point from the archive (without this, the linker might not pull it in)
4. Verifies the resulting ELF has no absolute relocations in code sections before bundling the `.pbw`

See `examples/hello/wscript` for implementation details.

## License

Licensed under either of Apache License, Version 2.0 (`LICENSE-APACHE`) or
MIT license (`LICENSE-MIT`) at your option.

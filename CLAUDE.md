# Ferrite

Last verified: 2026-08-11

Rust toolchain for PebbleOS watchapps, targeting Pebble Time 2 ("Emery") on the
rePebble SDK. An app is a `no_std` Rust crate compiled to a staticlib that the
SDK's own `pebble build` links into the app ELF.

User-facing docs (prerequisites, template setup, troubleshooting) live in
`README.md` — do not duplicate them here. This file is the working context.

## Tech Stack
- Rust stable, edition 2021, `no_std` on target
- Target triple: `thumbv7m-none-eabi` (`rustup target add` required)
- SDK: pebble-tool 5.0.x / SDK core 4.17, Emery platform only
- Build glue: waf (`examples/hello/wscript`), invoked by `pebble build`

## Project Structure
- `crates/ferrite-sys/` — generated FFI bindings (committed). See its CLAUDE.md.
- `crates/ferrite/` — safe API + runtime. See its CLAUDE.md.
- `crates/xtask/` — `bindgen`, `size`, and `messagekeys` commands. Locates the SDK via
  `PEBBLE_SDK_ROOT` or the default macOS install path (`sdk.rs`).
- `examples/hello/` — copyable app template AND the integration test. See its
  CLAUDE.md. **Excluded from the workspace.**
- `scripts/check.sh` — the full gate (see below).
- `docs/implementation-plans/` — build history. Listings inside are annotated
  where later phases superseded them; **the code is always the source of
  truth**, never a plan listing.

## Commands
- `scripts/check.sh` — the whole gate: cargo checks, fmt, clippy, host tests,
  Python tests, `pebble build`, size budget, then emulator smoke + button
  navigation. Requires a working SDK and emulator.
- `cargo test -p ferrite` — host unit tests (`ferrite` is `no_std` except under
  `cfg(test)`; the panic handler and allocator are gated on
  `target_os = "none"` so they do not clash with std).
- `cargo +nightly miri test -p ferrite` — **run this whenever you touch a
  trampoline.** Miri is nightly-only (`rustup +nightly component add miri`
  once); builds themselves stay on stable, so do NOT pin a toolchain file.
  Miri is what proved the naive "hold `&mut State` across the user closure"
  shape to be UB in Phase 4. It is deliberately NOT in `check.sh` (too slow for
  the loop); running it is your job when editing callback dispatch.
- `cargo xtask bindgen` — regenerate `ferrite-sys` bindings (needs libclang).
- `cargo xtask size [elf]` — static footprint vs Emery's 128 KB app-memory cap.
  **Exits nonzero when over budget** — it is a gate, not a report.
- `cargo xtask messagekeys [--check] <package.json> <out.rs>` — generate message
  ID assignments from phone-side JSON keys. Values are assigned sequentially from
  10000 in `messageKeys` array order, which **IS the wire contract with JS**
  (appending is safe; inserting or reordering silently corrupts the protocol).
  **Exits nonzero on drift with --check** — stale generated files fail the gate.
- `python3 -m unittest discover -s examples/hello/tests` — tests the wscript
  relocation guardrail's parser.

## Critical: the workspace excludes examples/hello

`examples/hello` is in `[workspace] exclude` (it needs its own release profile
and `.cargo/config.toml`). Consequences that bite every time:

- `--all` / `--workspace` **never reach the example.** Every gate must run a
  second time with `cwd = examples/hello`, or the example is silently
  unchecked. `check.sh` does this; match it if you add a gate.
- Target-scoped commands must be `-p` scoped: use
  `cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite`. An
  unscoped `--target` build pulls in `xtask`, which is a host binary depending
  on bindgen and cannot cross-compile.
- The example has its own `Cargo.lock`.

## Critical: RUSTFLAGS can silently break the build

`-C relocation-model=pic` and `-C force-unwind-tables=no` are mandatory.
Without PIC, LLVM bakes absolute addresses of statics into Thumb instructions;
the Pebble loader only fixes up `.rel.data` and `.got`, so every `&'static`
pointer is garbage at runtime and the app faults on first use.

A `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS` environment variable **overrides
`[target.*] rustflags` in `.cargo/config.toml` entirely** — this actually
happened and produced the broken-pointer failure. Hence the belt-and-braces:

1. `examples/hello/.cargo/config.toml` covers bare `cargo build` in a clean env.
2. `wscript`'s `_cargo_env()` appends the flags to whatever is inherited, so
   they win even when the env is set.
3. `wscript`'s `_check_relocations()` reads the linked ELF with `readelf -r`
   and fails the build on any non-relative relocation in a loaded section.

The relocation guard is an **allow-list and fails closed** — unknown or
unparseable relocation types are treated as unsafe, and any failure to complete
the check deletes the `.pbw` and `pebble-app.bin` so an unverified binary can
never be installed. Do not soften this into a deny-list.

## Emulator etiquette (scripts/check.sh)
- `pebble install --emulator emery --logs` **holds the connection** — run it
  backgrounded with its PID captured (no subshell, or `$!` is the subshell),
  tail its log file, and kill it on exit.
- Assert on **log lines**, not screenshots — screenshots flake.
- A heartbeat line only counts as complete when the trailing `u64=` field is
  present; a half-flushed line would corrupt the leak check.
- Insert `sleep` between button presses so a press cannot land mid push
  animation and mis-route.
- Delete stale `build/*.pbw` before building: `pebble install` just reads the
  bundle off disk, so a stale one would mask a failed build.

## Conventions
- Bindings and SDK version move together — the firmware jump table is
  index-based, so a version mismatch is silent corruption, not a link error.
- Wrapper constructors take `&mut App`, a capability token proving the runtime
  is initialized.
- `unsafe` gets a comment naming the invariant it relies on.

## Boundaries
- Never hand-edit `crates/ferrite-sys/src/bindings_emery.rs` — regenerate it.
- Never edit `docs/implementation-plans/` listings to match new code; they are
  a historical record and are already annotated where superseded.

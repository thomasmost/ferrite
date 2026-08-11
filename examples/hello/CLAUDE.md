# examples/hello — app template and integration test

Last verified: 2026-08-11

## Purpose
Serves two masters: it is the directory users copy to start an app (README
documents that flow), and it is Ferrite's only on-device test. Changes here are
changes to both. Keep it exemplary — it is what people copy.

## Contracts
- **Guarantees** to `scripts/check.sh`: it emits the log markers the gate greps
  for, and its menu/button layout matches the presses the gate sends.
- **Expects**: `ferrite` by relative path; a working Emery emulator.

## Boundary: excluded from the root workspace
Declared in the root `[workspace] exclude` because it needs its own release
profile (`opt-level = "z"`, `lto`, `panic = "abort"`, `codegen-units = 1`) and
its own `.cargo/config.toml`. So it has its own `Cargo.lock`, and **`--all` /
`--workspace` from the repo root never reach it** — fmt, clippy and check must
be run a second time with `cwd` here or this crate is silently unchecked.

## Invariant: log markers the gate depends on
`scripts/check.sh` asserts on these exact strings. Renaming one breaks the gate
with a confusing timeout, not a clear failure:
- `HEARTBEAT <n> heap_free=<n> u64=<n>` — the trailing `u64=` field is what
  proves the line is fully flushed; the gate compares `heap_free` across
  heartbeats 2–4 for stability (1 is warm-up) as the allocator leak check. The
  tick closure allocates two `Box`es per tick precisely to exercise the heap.
- `text screen loaded` / `text screen select` / `canvas screen loaded` — each
  handler must be dispatched **twice** by the gate, because a broken
  take/call/restore works exactly once and is dead thereafter.
- Menu row order is load-bearing: row 0 = Text, row 1 = Canvas (the gate sends
  `down` then `select` to reach the canvas).

The canvas screen is the **only** on-device exercise of
`layer_create_with_data` and the data-slot round trip — host tests stub
`layer_get_data`, so they cannot prove the real path. Do not drop it from the
example on size grounds.

## Invariant: the wscript is not a stock SDK wscript
It compiles the Rust staticlib and links it via `stlib`. Load-bearing details:
- `REQUIRED_RUSTFLAGS` are appended to inherited flags in `_cargo_env()`
  because a `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS` env var overrides
  `.cargo/config.toml` outright (see the root CLAUDE.md).
- `linkflags=['-Wl,-u,main']` forces extraction of the Rust entry point from
  the archive — nothing else references it.
- `add_manual_dependency` on the `.a`: waf sees `stlib` only as `-L`/`-l`
  flags, so without it, editing Rust source silently yields a stale `.pbw`.
- `RUST_CRATE` must match `[package] name` in `Cargo.toml` (it names
  `lib<name>.a`). Template users change both.
- The post-build relocation check fails closed and deletes artifacts. Its
  parser is unit-tested in `tests/test_relocation_check.py` (29 tests) — that
  file is a real gate, not scaffolding. Note the readelf `%-17.17s` truncation:
  entries in `RELATIVE_RELOC_TYPES` use the truncated spelling.

## Gotchas
- `MESSAGE_KEY_PING = 10000` in `src/lib.rs` mirrors the position of `"PING"`
  in `package.json`'s `messageKeys` (values assigned sequentially from 10000 in
  array order). Reordering that array silently breaks the binding.
- `"watchface": false` — watchfaces receive no button events, so the button
  navigation gate needs a watchapp.
- `src/c/**/*.c` glob matches nothing by design; all app code is Rust. The SDK
  still appends its generated `appinfo.auto.c` / `resource_ids.auto.c`.
- `src/lib.rs` demonstrates the RFC-2229 fix and children-before-parents drop
  order in its final tuple. Both are the shape users will copy — see
  `crates/ferrite/CLAUDE.md` before restructuring the setup block.

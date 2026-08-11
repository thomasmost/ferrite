# Ferrite Rust Toolchain Implementation Plan — Phase 7: Size report and template polish

**Goal:** `cargo xtask size` reports the built app's `.text`/`.data`/`.bss` against Emery's 128 KB caps, and the README walks a newcomer from a fresh copy of the template to a running emulator app.

**Architecture:** `cargo xtask size` shells out to the SDK's bundled `arm-none-eabi-size` (Berkeley output — a format stable for decades) and compares against `MAX_APP_MEMORY_SIZE = 0x20000` (131072 bytes; the linker's whole APP region — code+data+bss share it with heap and stack). The SDK-locating logic moves to a small shared module so `bindgen` and `size` use the same resolution (default install path, `PEBBLE_SDK_ROOT` override).

**Tech Stack:** No new dependencies; std `process::Command` in xtask.

**Scope:** Phase 7 of 7 from `docs/design-plans/2026-08-05-ferrite-rust-toolchain.md`.

**Codebase verified:** 2026-08-05 (earlier investigations this session). The built ELF is at `examples/hello/build/emery/pebble-app.elf`; the size tool at `$SDK/toolchain/arm-none-eabi/bin/arm-none-eabi-size`; Emery's `MAX_APP_BINARY_SIZE` and `MAX_APP_MEMORY_SIZE` are both `0x20000` (from the SDK's `pebble_sdk_platform.py`). Apps run from RAM: load size ≈ text+data, RAM footprint = text+data+bss.

---

## Context for the implementing engineer (read first)

- **Berkeley `size` output** looks like:
  ```
     text	   data	    bss	    dec	    hex	filename
    23456	    712	    104	  24272	   5ed0	pebble-app.elf
  ```
  Parse line 2 by whitespace; fields 1–3 are text/data/bss in bytes.
- **What counts against 128 KB:** the whole binary is copied into the app's RAM region, so static footprint = text + data + bss; everything left over serves heap + stack. Report both the footprint and the remainder.
- **README numbers must be measured, not invented.** Where the README template below says `<MEASURED>`, run the command and paste the real output from this machine.

---

<!-- START_SUBCOMPONENT_A (tasks 1-2) -->
<!-- START_TASK_1 -->
### Task 1: Shared SDK locator in xtask

**Files:**
- Create: `crates/xtask/src/sdk.rs`
- Modify: `crates/xtask/src/main.rs`
- Modify: `crates/xtask/src/gen_bindings.rs`

**Step 1: Extract the locator**

`crates/xtask/src/sdk.rs`:
```rust
//! Locating the installed Pebble SDK.

use std::path::PathBuf;

/// SDK version the toolchain is pinned to. The firmware jump table is
/// index-based, so bindings must match the SDK the app links against.
pub const SDK_VERSION: &str = "4.17";

pub fn sdk_root() -> PathBuf {
    if let Ok(p) = std::env::var("PEBBLE_SDK_ROOT") {
        return PathBuf::from(p);
    }
    std::env::home_dir()
        .expect("cannot determine home directory")
        .join("Library/Application Support/Pebble SDK/SDKs")
        .join(SDK_VERSION)
}
```

In `crates/xtask/src/main.rs`, add the module and a `size` subcommand:
```rust
//! Project automation: `cargo xtask <command>`.

use std::process::ExitCode;

mod gen_bindings;
mod sdk;
mod size;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bindgen") => gen_bindings::run(),
        Some("size") => size::run(&args[1..]),
        _ => {
            eprintln!("usage: cargo xtask <bindgen|size> [args]");
            ExitCode::FAILURE
        }
    }
}
```

In `crates/xtask/src/gen_bindings.rs`: delete its private `SDK_VERSION` const and `sdk_root` fn, and use the shared ones instead:
```rust
use crate::sdk::{sdk_root, SDK_VERSION};
```
(The `raw_line` format string and error messages that reference `SDK_VERSION` keep working unchanged.)

Create a placeholder so the crate compiles (implemented in Task 2):

`crates/xtask/src/size.rs`:
```rust
use std::process::ExitCode;

pub fn run(_args: &[String]) -> ExitCode {
    eprintln!("not yet implemented");
    ExitCode::FAILURE
}
```

**Step 2: Verify**

```bash
cargo check -p xtask
cargo xtask bindgen
git diff --stat crates/ferrite-sys/src/bindings_emery.rs
```
Expected: check passes; regenerating produces **no diff** in the committed bindings (proves the refactor changed nothing).

**Step 3: Commit**

```bash
git add crates/xtask
git commit -m "refactor(xtask): shared SDK locator; size subcommand skeleton"
```
<!-- END_TASK_1 -->

<!-- START_TASK_2 -->
### Task 2: `cargo xtask size`

**Files:**
- Modify: `crates/xtask/src/size.rs` (replace skeleton entirely)

**Step 1: Implement**

`crates/xtask/src/size.rs`:
```rust
//! `cargo xtask size [path/to/app.elf]`: static-size report against the
//! Emery app-memory budget. Defaults to the hello example's built ELF.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use crate::sdk::sdk_root;

/// Emery MAX_APP_MEMORY_SIZE (the linker's whole APP region; code, data,
/// bss, heap and stack all share it).
const APP_MEMORY_CAP: u64 = 0x20000;

fn workspace_root() -> PathBuf {
    // crates/xtask -> crates -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

pub fn run(args: &[String]) -> ExitCode {
    let elf = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("examples/hello/build/emery/pebble-app.elf"));
    if !elf.exists() {
        eprintln!(
            "error: {} not found — run `pebble build` in the app directory first",
            elf.display()
        );
        return ExitCode::FAILURE;
    }

    let size_tool = sdk_root().join("toolchain/arm-none-eabi/bin/arm-none-eabi-size");
    let output = Command::new(&size_tool)
        .arg(&elf)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", size_tool.display()));
    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        return ExitCode::FAILURE;
    }

    // Berkeley format: header line, then "  text  data  bss  dec  hex  file".
    let stdout = String::from_utf8_lossy(&output.stdout);
    let fields: Vec<u64> = match stdout.lines().nth(1) {
        Some(line) => line
            .split_whitespace()
            .take(3)
            .filter_map(|f| f.parse().ok())
            .collect(),
        None => Vec::new(),
    };
    let [text, data, bss] = fields[..] else {
        eprintln!("error: unexpected `size` output:\n{stdout}");
        return ExitCode::FAILURE;
    };

    let footprint = text + data + bss;
    let percent = footprint * 100 / APP_MEMORY_CAP;
    println!("{}", elf.display());
    println!("  .text (code+rodata): {text:>7} bytes");
    println!("  .data (init data):   {data:>7} bytes");
    println!("  .bss  (zeroed data): {bss:>7} bytes");
    println!(
        "  static footprint:    {footprint:>7} / {APP_MEMORY_CAP} bytes ({percent}% of app memory)"
    );
    println!(
        "  left for heap+stack: {:>7} bytes",
        APP_MEMORY_CAP - footprint.min(APP_MEMORY_CAP)
    );

    if footprint > APP_MEMORY_CAP {
        eprintln!("ERROR: exceeds the {APP_MEMORY_CAP}-byte app memory cap");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
```

**Step 2: Verify**

```bash
cd examples/hello && pebble build && cd ../..
cargo xtask size
```
Expected: a report with plausible numbers (text in the tens of KB, footprint well under 131072), exit code 0. Also verify the explicit-path form:
```bash
cargo xtask size examples/hello/build/emery/pebble-app.elf
```
And the error path:
```bash
cargo xtask size /nonexistent.elf; echo "exit: $?"
```
Expected: clear error message, exit 1.

**Step 3: Commit**

```bash
git add crates/xtask/src
git commit -m "feat(xtask): cargo xtask size budget report"
```
<!-- END_TASK_2 -->
<!-- END_SUBCOMPONENT_A -->

<!-- START_TASK_3 -->
### Task 3: README — template walkthrough and troubleshooting

**Files:**
- Modify: `README.md` (replace entirely)

**Step 1: Rewrite the README**

Replace `README.md` with the following, filling every `<MEASURED>` with real output from running the commands on this machine (Task 2's `cargo xtask size` run gives the numbers):

````markdown
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

## Size budget

Emery gives an app 128 KB of memory for code, data, heap, and stack
combined. Report your app's static footprint with:

```sh
cargo xtask size [path/to/pebble-app.elf]
```

Baseline for `examples/hello` (Ferrite <MEASURED: git describe/commit>):

```
<MEASURED: paste the cargo xtask size output block here>
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

**`cargo: command not found` during `pebble build`** — the wscript falls
back to `~/.cargo/bin/cargo`; if your Rust lives elsewhere, ensure `cargo`
is on `PATH` in the shell that runs `pebble build`.

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
````

**Step 2: Verify the walkthrough by following it verbatim**

In the session scratchpad directory (not the repo):
```bash
SCRATCH=/private/tmp/claude-501/-Users-thomas-vibes-ferrite/d3b18456-1ae4-4957-9ce8-0423c4e57bf5/scratchpad
cp -R examples/hello "$SCRATCH/myapp" && cd "$SCRATCH/myapp"
rm -rf build target .lock-waf_darwin_build
```
Then follow README steps 2–5 exactly: set name `myapp`/displayName "My App"/fresh uuid in `package.json`; in `Cargo.toml` set `name = "myapp"` and `ferrite = { path = "/Users/thomas/vibes/ferrite/crates/ferrite" }` (ferrite-sys resolves transitively); in `wscript` set `RUST_CRATE = 'myapp'`; run `pebble build`.

Expected: `'build' finished successfully` and `build/myapp.pbw` exists. Install it (`pebble install --emulator emery --logs`) and confirm it runs. If any README step was wrong or missing, fix the README (not just the copy) and redo this verification from a fresh copy. Clean up: `rm -rf "$SCRATCH/myapp"`.

**Step 3: Commit**

```bash
cd /Users/thomas/vibes/ferrite
git add README.md
git commit -m "docs: template walkthrough, size budget, troubleshooting"
```
<!-- END_TASK_3 -->

<!-- START_TASK_4 -->
### Task 4: Final verification

**Files:** none.

**Step 1: Whole-toolchain pass**

Run (from repo root):
```bash
cargo test -p ferrite
cargo check --target thumbv7m-none-eabi -p ferrite-sys -p ferrite
cargo xtask size
./scripts/check.sh
```
Expected: all pass; size report well under budget.

**Step 2: Repo hygiene**

```bash
git status --short
```
Expected: clean tree. Commit anything outstanding.

**Phase complete when:** `cargo xtask size` reports against the caps, the README walkthrough has been executed verbatim from a fresh copy and produced a running app, and all prior verification (host tests, check.sh) still passes.
<!-- END_TASK_4 -->

# Ferrite — Rust Toolchain for PebbleOS Watchapps Design

## Summary

Ferrite is a Rust toolchain for writing PebbleOS smartwatch apps — specifically for the
Pebble Time 2 ("Emery") using the community-maintained rePebble SDK, whose official
toolchain is C-only. Rather than replacing the SDK's build system, Ferrite slots into it:
the app's Rust code is compiled by Cargo into a static library for the watch's ARM CPU,
and the SDK's existing build (`pebble build`) links that library into the final app
binary using the same hook the SDK already provides for prebuilt libraries. Everything
downstream — the SDK's linker script, firmware call-table plumbing, and binary metadata
injection — runs completely unchanged, so the developer experience stays the familiar
`pebble build` / `pebble install`.

The toolchain is three crates in one Cargo workspace: `ferrite-sys` holds
machine-generated raw bindings to the SDK's C API (committed to the repo, so normal
builds never need the binding generator); `ferrite` is the crate apps actually use,
providing the runtime glue a bare-metal Rust app needs (entry-point macro, panic
handler, an allocator backed by the SDK's heap) plus safe, idiomatic wrappers over
windows, layers, graphics, messaging, storage, and sensors; and `xtask` automates
maintenance chores like regenerating bindings and reporting binary size against the
watch's 128 KB app limit. Work is sequenced riskiest-first: phase 1 proves the whole
build-and-link integration with a hello-world watchface running in the emulator, and
later phases grow the safe API surface until it covers everything needed to rewrite the
existing C-based Fitter run-tracking app in Rust (the rewrite itself is a separate
future project).

## Definition of Done

1. A Rust toolchain (Cargo workspace in this repository) that builds PebbleOS watchapps against the rePebble SDK (pebble-tool 5.0.x / SDK 4.17): raw bindings generated from `pebble.h`, a `no_std` runtime layer (entry point, panic handler, SDK-backed allocator), a safe idiomatic API crate, and build integration that produces installable `.pbw` files.
2. Stable Rust only, `thumbv7m-none-eabi`, Emery-first (nothing designed to block other platforms, but only Emery generated/tested/documented for now). Licensed MIT OR Apache-2.0.
3. Near-term success: a hello-world watchface written in Rust builds through the toolchain and runs in the Emery emulator (this design's implementation phase 1).
4. End-state success: API coverage sufficient to rewrite the Fitter app in Rust — windows, text/menu layers, custom layer drawing + graphics primitives, click handlers (incl. long-press), AppMessage, HealthService heart rate, persist storage, timers/ticks, logging.

**Out of scope:** crate naming / publishing to crates.io, non-Emery platform testing, nightly `build-std` mode, and the actual Fitter rewrite itself (separate future project).

## Glossary

**Watch/Pebble domain**

- **PebbleOS / rePebble SDK**: The operating system of Pebble smartwatches and its revived developer kit (pebble-tool 5.0.x / SDK core 4.17). The SDK officially supports only C apps — Ferrite adds Rust on top of it.
- **Emery**: The SDK's platform codename for the Pebble Time 2 — 200x228 color display, heart-rate sensor, 128 KB app memory limit. The only platform Ferrite generates and tests bindings for initially.
- **Watchapp / watchface**: The two kinds of Pebble programs — interactive apps vs. always-on clock faces. Both build the same way; hello-world here is a watchface.
- **Fitter**: The existing C run-tracking watchapp (in a sibling repository) whose SDK usage defines Ferrite's API coverage target and eventual rewrite goal.
- **waf / `wscript`**: The Python-based build system the Pebble SDK uses; `wscript` is a project's build script and the SDK's documented customization point — where Ferrite adds its "run cargo, then link the result" rule.
- **`.pbw`**: The installable Pebble app bundle that a successful build produces.
- **Jump table / `libpebble.a` / `pbl_table_addr`**: How Pebble apps call the firmware — not by direct linking, but through an index-based table of function addresses the firmware provides at load time. This is why bindings are pinned to an exact SDK version: indices shift between versions.
- **`inject_metadata.py`**: SDK build step that post-processes the linked app binary, harvesting relocation info and writing the header the firmware loader needs. Ferrite's output must keep the binary shape this script expects.
- **AppMessage**: The SDK's message-passing service between watch and phone; data arrives as a dictionary of key-value tuples read via `dict_find`.
- **PKJS (PebbleKit JS)**: JavaScript that runs on the paired phone as the app's companion; the message-passing counterpart used to test AppMessage.
- **Message keys**: Per-project integer constants for AppMessage dictionary entries, normally generated into a C header from `package.json`; Ferrite apps declare them in Rust instead.
- **Persist storage**: The SDK's small key-value store for app data that survives app exits.
- **HealthService**: The SDK service exposing sensor data such as heart rate.
- **Tick timer service**: SDK service that invokes a callback every second/minute/etc.; notable here because its C API accepts no context pointer, forcing a different callback strategy.

**Rust / toolchain concepts**

- **`no_std`**: Rust mode without the standard library (no OS assumed) — required on embedded targets. The app must supply its own entry point, panic handler, and allocator, which is exactly what Ferrite's runtime layer provides.
- **bindgen**: Tool that machine-generates Rust FFI declarations from C headers (here, `pebble.h`). Its output is committed so building an app never requires libclang.
- **`-sys` crate**: Rust convention of splitting raw, unsafe C bindings (`ferrite-sys`) from the safe, ergonomic wrapper crate (`ferrite`).
- **staticlib**: Cargo crate type producing a C-compatible static library (`.a`) — the artifact handed to the SDK's linker.
- **`thumbv7m-none-eabi`**: The stock stable-Rust compilation target matching the watch's ARM Cortex-M CPU; using it (rather than a custom target definition) keeps plain `rustup` workflows.
- **PIC / GOT**: Position-independent code — compiled so it runs at any memory address, with global accesses routed through a Global Offset Table the loader fixes up. Pebble apps require it; `-C relocation-model=pic` mirrors the C toolchain's `-fPIE`.
- **`panic = "abort"` / unwind tables / `.ARM.exidx`**: Configuring Rust panics to abort rather than unwind the stack, so the binary carries no ARM exception-handling sections — the one binary-format hazard identified, since extra sections could confuse `inject_metadata.py`.
- **`#[global_allocator]`**: Rust hook designating what backs heap allocation — here the SDK's own `malloc`/`free`, so Rust and C apps share one heap and the SDK's `heap_bytes_free()` diagnostic stays accurate.
- **Trampoline**: Small `extern "C"` function registered with the SDK as a callback; it recovers a boxed Rust closure from the SDK's context pointer and invokes it, letting users write closures instead of C-style function pointers.
- **Capability token (`&mut App`)**: A zero-sized value handed to user code only after app initialization, so the type system prevents SDK calls before the runtime is ready.
- **`Drop`**: Rust's destructor mechanism; wrapper types call the SDK's `_destroy` functions automatically when they go out of scope.
- **`macro_rules!` vs. proc-macro**: Rust's two macro systems; `app!` uses the simpler declarative kind to avoid an extra compiler-plugin crate.
- **`xtask`**: Convention of putting project automation in a workspace member run as `cargo xtask <cmd>`, instead of shell scripts or Makefiles.
- **`build-std`**: Nightly-only Cargo mode that recompiles the standard library with custom flags; explicitly out of scope since Ferrite is stable-Rust only.
- **`#[repr(C)]`**: Attribute forcing a Rust struct's memory layout to match C, so value types like `GRect` cross the FFI boundary untranslated.
- **LTO / `opt-level = "z"`**: Link-time optimization and size-first optimization — release-profile settings that keep binaries within the 128 KB app limit.

## Architecture

**Approach: cargo-in-waf.** An app built with Ferrite is a standard rePebble SDK project whose `wscript` gains a pre-link rule: run `cargo build --release --target thumbv7m-none-eabi` for the app crate, then link the resulting staticlib into the app ELF via the SDK's own `stlib` kwargs-passthrough (`ctx.pbl_build(..., stlib=..., stlibpath=...)` — the same mechanism the SDK uses for "pebble package" prebuilt libraries). The stock pipeline — SDK linker script, `libpebble.a` jump-table trampolines, `inject_metadata.py` — runs unchanged; it only requires the symbols `main` (now provided by Rust) and `pbl_table_addr` (from `libpebble.a`). UX stays `pebble build` / `pebble install --emulator emery`.

### Workspace layout

```
ferrite/
├── Cargo.toml            # workspace
├── crates/
│   ├── ferrite-sys/      # raw bindings, committed bindgen output for emery / SDK 4.17
│   │   └── src/
│   │       ├── bindings_emery.rs   # generated — never hand-edited
│   │       └── lib.rs              # + hand-written ports of function-like C macros:
│   │                               #   GRect()/GPoint()/GSize() const fns,
│   │                               #   GColorFromRGB/HEX, named color consts
│   ├── ferrite/          # safe API + runtime; the crate apps depend on
│   │   └── src/          # app! macro, panic handler, allocator, and wrapper
│   │                     #   modules: window, layer, text_layer, menu_layer,
│   │                     #   graphics, app_message, persist, health, tick, log
│   └── xtask/            # cargo xtask bindgen (regenerate ferrite-sys from the
│                         #   installed SDK), cargo xtask size (budget report)
├── examples/
│   └── hello/            # template + integration test: full pebble project
│       ├── package.json  # normal pebble metadata (emery target)
│       ├── wscript       # customized: cargo pre-link rule + stlib injection
│       ├── Cargo.toml    # app crate; crate-type = ["staticlib"]; ferrite via path
│       ├── .cargo/config.toml
│       └── src/lib.rs    # the watchface, in Rust
├── scripts/check.sh      # end-to-end emulator smoke test
└── docs/design-plans/
```

Division of responsibility: `ferrite-sys` is mechanical (generated FFI + macro ports, no policy). `ferrite` owns all safety and ergonomics. `xtask` keeps toolchain chores inside cargo — libclang is needed only when regenerating bindings, never for normal builds. `examples/hello` doubles as the copyable project template and the integration test vehicle.

### Rust codegen contract

Set by the template's `Cargo.toml` and `.cargo/config.toml`; all flags are stable-Rust:

```toml
[profile.release]
opt-level = "z"
lto = true
panic = "abort"
codegen-units = 1

# .cargo/config.toml
[target.thumbv7m-none-eabi]
rustflags = ["-C", "relocation-model=pic", "-C", "force-unwind-tables=no"]
```

`relocation-model=pic` matches the SDK's `-fPIE`, so relocations land in `.got`/`.rel.data` where `inject_metadata.py` harvests them. `panic=abort` + no unwind tables keeps `.ARM.extab`/`.ARM.exidx` empty — the one identified binary-format hazard.

### Runtime layer (in `ferrite`)

- **Entry:** the `ferrite::app!` macro (declarative `macro_rules!`, no proc-macro crate) expands to `#[no_mangle] pub extern "C" fn main()`: runs the user's setup function with an `&mut App` (a zero-sized capability token gating SDK calls on app initialization), calls `app_event_loop()`, then cleanup. Contract:

  ```rust
  ferrite::app! {
      fn main(app: &mut App) { /* build windows, subscribe services */ }
  }
  ```

- **Panic handler:** logs message + location via `APP_LOG` (visible in `pebble logs`), then raises an undefined-instruction trap so the firmware's app-fault path terminates the app. Total, since no unwinding exists.
- **Allocator:** `#[global_allocator]` over SDK `malloc`/`free` — same heap as C apps, `heap_bytes_free()` stays meaningful. OOM routes through Rust's stable default alloc-error handler into the panic path.
- **Statics:** ordinary Rust statics work; PIC access goes through the GOT, which the firmware loader relocates. Convention (enforced by the size report): avoid `core::fmt`-heavy paths; small formatting helpers instead.

### Safe API & callback model

SDK objects are owned wrappers (`Window`, `TextLayer`, `MenuLayer`, `CanvasLayer` over `layer_create_with_data`): create in `new()`, destroy in `Drop`; parenting borrows. Value types (`GRect`, `GPoint`, `GSize`, `GColor`) are `#[repr(C)]` copies with const constructors, crossing FFI untranslated.

Callbacks use two mechanisms, chosen per service by whether the SDK carries a context pointer:

- **Context-carrying** (window handlers via `window_set_user_data`, click config via `_with_context`, AppMessage via `app_message_set_context`, health's `context` arg): boxed Rust closure, its pointer passed as SDK context, private `extern "C"` trampoline recovers and calls it. No globals in user code.
- **Context-less** (`tick_timer_service_subscribe` takes a bare fn pointer): a private `static` slot inside `ferrite` holds the closure — sound because the platform is single-threaded and re-subscribing replaces the slot.

Strings: text APIs take `&'static CStr` (`c"..."` literals) for the common case, or an owned buffer held by the layer wrapper so the lifetime is structural.

Error philosophy: soft-failure SDK calls (persist writes, message parsing) return `Result`; programming errors panic (loud, log-visible) and are prevented structurally where possible.

### API coverage target

Priority order is exactly the surface the Fitter app uses (from investigation of `/Users/thomas/vibes/fitter/src/c/`): window + window stack + click config (incl. long-press), `Layer`/`TextLayer`/`MenuLayer` + `layer_set_update_proc`, graphics (`draw_line`, `fill_circle`, stroke/fill/antialias state, system fonts), inbound AppMessage + `dict_find`, blob `persist_*`, HealthService heart rate, `tick_timer_service`, `APP_LOG`, `heap_bytes_free`. Everything else remains reachable via `ferrite-sys` (unsafe) until wrapped.

## Existing Patterns

The repository is greenfield; the design follows established external patterns rather than inventing:

- **SDK precedent:** static-library injection mirrors the SDK's own pebble-package mechanism (`setup_pebble_cprogram` in `pebble_sdk_common.py` appends `stlib` entries); the customized `wscript` is the SDK's documented extension point ("Feel free to customize this to your needs").
- **Prior art:** the staticlib-into-waf shape is the proven approach of pebble.rs (2015), pebble-rust (2019), and the Embedded Swift on Pebble demo (2026).
- **Rust ecosystem conventions:** `-sys` crate split (raw FFI separate from safe wrapper), committed-bindings-with-regeneration, the cargo `xtask` pattern, workspace layout, MIT OR Apache-2.0 dual license.

One deliberate divergence from prior art: pebble.rs used a custom target JSON; Ferrite uses the stock stable `thumbv7m-none-eabi` target with `-C relocation-model=pic`, keeping plain-rustup builds.

## Implementation Phases

<!-- START_PHASE_1 -->
### Phase 1: Build-integration proof (hello world in the emulator)
**Goal:** A Rust watchface builds via `pebble build` and runs in the Emery emulator — the riskiest integration proven first.

**Components:**
- Workspace `Cargo.toml`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore`, minimal `README.md`
- `crates/ferrite-sys/` — minimal **hand-written** `extern "C"` declarations only for what hello-world needs (window create/destroy/stack, text layer basics, `app_event_loop`, `app_log_trace`); generated bindings come in Phase 2
- `crates/ferrite/` — panic handler, `app!` macro, thin wrappers for the hello-world surface
- `examples/hello/` — pebble project (`package.json`, customized `wscript` with cargo rule + stlib injection, `Cargo.toml`, `.cargo/config.toml`, `src/lib.rs` showing a static "Hello from Rust" text layer)

**Dependencies:** None (first phase). Requires local pebble-tool 5.0.x / SDK 4.17 and `rustup target add thumbv7m-none-eabi`.

**Done when:** `pebble build` succeeds in `examples/hello`; `pebble install --emulator emery` shows the text on screen; a log line from Rust appears in `pebble logs`.
<!-- END_PHASE_1 -->

<!-- START_PHASE_2 -->
### Phase 2: Generated bindings
**Goal:** Replace hand-written FFI with committed bindgen output covering the full Emery SDK.

**Components:**
- `crates/xtask/` — `cargo xtask bindgen`: locates the installed SDK, runs bindgen over `sdk-core/pebble/emery/include/pebble.h` with the platform defines (`PBL_PLATFORM_EMERY`, `PBL_COLOR`, `PBL_HEALTH`, `PBL_DISPLAY_WIDTH=200`, etc.), `--use-core`, clang target `thumbv7m-none-eabi`; stubs for the project-generated headers (`message_keys.auto.h`, `resource_ids.auto.h`)
- `crates/ferrite-sys/src/bindings_emery.rs` — committed output
- `crates/ferrite-sys/src/lib.rs` — hand-written macro ports (`GRect`/`GPoint`/`GSize` const fns, `GColorFromRGB`/`GColorFromHEX`, named color constants) and const layout assertions for hazard structs (`GColor8` == 1 byte, packed `Tuple`, `AccelData`), evaluated when compiling for the target

**Dependencies:** Phase 1.

**Done when:** `cargo check --target thumbv7m-none-eabi` passes with layout assertions; `examples/hello` builds and runs on the generated bindings with the hand-written declarations deleted.
<!-- END_PHASE_2 -->

<!-- START_PHASE_3 -->
### Phase 3: Runtime completion and smoke test
**Goal:** Full runtime layer plus repeatable end-to-end verification.

**Components:**
- `crates/ferrite/` — `#[global_allocator]` over SDK malloc/free, OOM-to-panic path, `log` module wrapping `APP_LOG` levels, finalized `app!` cleanup semantics
- `examples/hello/src/lib.rs` — heartbeat log on a minute tick (exercises allocator + tick service minimally)
- `scripts/check.sh` — builds hello, installs to the Emery emulator, greps `pebble logs` for the heartbeat line

**Dependencies:** Phase 2.

**Done when:** `./scripts/check.sh` passes from a clean checkout (with SDK + Rust target installed).
<!-- END_PHASE_3 -->

<!-- START_PHASE_4 -->
### Phase 4: Safe UI core
**Goal:** Windows, text, and buttons — enough to build multi-screen button-driven apps.

**Components:**
- `crates/ferrite/src/window.rs` — `Window` wrapper, load/unload closures via `window_set_user_data` trampolines, window-stack API
- `crates/ferrite/src/click.rs` — click config provider with context; single and long click recognizers mapped to closures
- `crates/ferrite/src/text_layer.rs` — `TextLayer` with structural text-buffer ownership; system font lookup; alignment/color setters
- `crates/ferrite/src/types.rs` — `GRect`/`GPoint`/`GSize`/`GColor` safe value types
- Host unit tests for pure logic (types, buffer ownership bookkeeping); `examples/hello` upgraded to two windows with button navigation

**Dependencies:** Phase 3.

**Done when:** upgraded example navigates between windows via buttons in the emulator; `cargo test` (host) passes; `./scripts/check.sh` still passes.
<!-- END_PHASE_4 -->

<!-- START_PHASE_5 -->
### Phase 5: Canvas graphics and menus
**Goal:** Custom drawing and list UI — the rendering surface Fitter's map and history screens need.

**Components:**
- `crates/ferrite/src/canvas.rs` — `CanvasLayer` over `layer_create_with_data` with an update-proc closure receiving a safe `Graphics` context
- `crates/ferrite/src/graphics.rs` — stroke/fill/antialias state, `draw_line`, `fill_circle` (surface grows on demand)
- `crates/ferrite/src/menu_layer.rs` — `MenuLayer` with rows/draw-row/select callbacks, normal/highlight colors, click-config binding
- `examples/hello` (or a second example) gains a drawing screen and a menu screen

**Dependencies:** Phase 4.

**Done when:** example renders custom-drawn content and a navigable menu in the emulator; `./scripts/check.sh` still passes.
<!-- END_PHASE_5 -->

<!-- START_PHASE_6 -->
### Phase 6: Services — AppMessage, persist, health, tick
**Goal:** The non-UI SDK services Fitter depends on.

**Components:**
- `crates/ferrite/src/app_message.rs` — inbox open/received/dropped with context-based closures; safe `Dict`/`Tuple` reading (`dict_find`, int32/length/type access)
- `crates/ferrite/src/persist.rs` — blob read/write/exists/delete returning `Result` with SDK status codes
- `crates/ferrite/src/health.rs` — event subscription (context-based), metric peek, accessibility check, HR sample-period control, graceful absence
- `crates/ferrite/src/tick.rs` — tick service via the static-slot mechanism
- Host tests for dict/persist argument marshalling logic; example logs received AppMessage values from a PKJS stub

**Dependencies:** Phase 3 (runtime); independent of Phases 4–5 UI surface but lands after them.

**Done when:** example receives and logs a message sent from PKJS in the emulator; persist round-trip survives app relaunch in the emulator; host tests pass.
<!-- END_PHASE_6 -->

<!-- START_PHASE_7 -->
### Phase 7: Size report and template polish
**Goal:** Toolchain is usable by someone who isn't its author.

**Components:**
- `crates/xtask/` — `cargo xtask size`: `.text`/`.data`/`.bss` from the built ELF vs Emery's 128 KB caps; hello-world baseline recorded in `README.md`
- `README.md` — template usage walkthrough (copy `examples/hello`, rename, build), prerequisites, troubleshooting the two known failure modes (missing rustup target, missing SDK)

**Dependencies:** Phases 1–6.

**Done when:** `cargo xtask size` reports against the caps; README walkthrough works when followed verbatim in a fresh copy.
<!-- END_PHASE_7 -->

## Additional Considerations

**SDK-version coupling:** committed bindings target SDK 4.17/Emery exactly. The firmware's jump table is index-based — apps built against a newer SDK crash on older firmware — so `ferrite-sys` records the SDK version it was generated from, and `cargo xtask bindgen` is the only sanctioned way to move it. Multi-platform support later means one committed bindings module per platform behind cargo features; nothing in the design assumes Emery beyond which module is generated today.

**`.ARM.exidx` watch item:** the C build already emits a small orphan `.ARM.exidx` that `inject_metadata.py` tolerates; Rust must not grow it (`force-unwind-tables=no`). `scripts/check.sh` is the tripwire — if metadata injection miscounts sections, install fails visibly.

**Message keys:** `MESSAGE_KEY_*` constants are generated per-project into `build/include/message_keys.auto.h` by the SDK build. The sys crate stubs them; apps that use AppMessage declare their key values in Rust (matching `package.json` `messageKeys` order). A generator can be added later if hand-matching proves error-prone — for the Fitter port it's 7 keys.

**Future extensibility:** the pebble-package distribution route (shipping `ferrite` as an npm dep with prebuilt `.a`) remains open and composes with this design; crate naming/publishing decisions are deferred per the DoD.

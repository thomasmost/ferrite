# ferrite — safe API and runtime

Last verified: 2026-08-11

## Purpose
Turns the SDK's raw C callback model into safe Rust closures. Every hard
problem here is the same one: the SDK calls back into us through `extern "C"`
trampolines that recover state from a raw context pointer, and the user closure
we then invoke can reenter this API.

## Contracts
- **Exposes**: `app!` entry macro, `App` capability token, and one module per
  SDK surface (`window`, `click`, `text_layer`, `canvas`, `menu_layer`,
  `graphics`, `layer`, `tick`, `health`, `persist`, `app_message`, `log`,
  `heap`, `types`). `pub use ferrite_sys as sys` for the escape hatch.
- **Guarantees**: no UB under SDK reentrancy; `Drop` releases SDK objects;
  nothing panics on a missing sensor (`health` degrades gracefully).
- **Expects**: single-threaded platform; `&mut App` for constructors; the app's
  return tuple lists **children before parents**.

## Invariant: the take/call/restore trampoline discipline
**A trampoline must never hold a borrow of its state across a user-closure
call.** Take the closure out of its slot (borrow ends at the statement), run it
borrow-free, restore it **only if the slot is still empty** so a reentrant
re-registration wins and the replaced closure drops safely on the
trampoline's stack frame.

This is not theoretical. The naive `&mut *(context as *mut State)` shape was
written first and **Miri proved it UB** in Phase 4 review. The SDK genuinely
nests: pebble.h:5213 — installing a click config provider on an already-visible
window invokes the provider *synchronously*. Run
`cargo +nightly miri test -p ferrite` (miri is nightly-only;
`rustup +nightly component add miri` once)
after any trampoline change.

Every callback-bearing module follows it: `click`, `window`, `canvas`,
`menu_layer`, `app_message`, `health` (context-pointer based) and `tick`
(private `static` slot — same pattern, the "context" is the static's identity).
`click.rs`'s module doc is the canonical write-up.

Corollaries:
- **Registration is a separate fact from occupancy.** `single` is
  `Option<SingleClick { cb: Option<Callback> }>`: while a handler runs the
  inner slot is empty but the outer stays `Some`. The SDK clears all
  subscriptions before each provider run, so the provider's predicate reads the
  *outer* option — otherwise a reentrant registration permanently unsubscribes
  the running button.
- `Window::on_click` **re-installs the provider on every registration** so
  handlers added after `push()` go live immediately. Do not "deduplicate" it.
- For callbacks that return a value (`cb_get_num_rows`), capture the return
  value before restoring.
- **Unsupported**: dropping a `Window`/`CanvasLayer`/`MenuLayer`/`AppMessage`/
  `Health` from inside its own callback — the state box would be freed under
  the executing closure. Backstopped by `callback_depth` + a `debug_assert` in
  `Drop`. Note `pebble build` is `--release`, so on-device this documents the
  contract rather than enforcing it.
  The supported alternative for a self-dismissing window (a launch dialog, a
  confirmation) is `remove_from_stack()` inside the handler, with the `Window`
  kept in the app-state tuple so it drops at exit. Costs one allocation for
  the app's life; correct at every optimization level.

## Invariant: drop order and RFC-2229 capture
Children must drop before parents, or a child's `Drop` unlinks from a freed
parent. The `app!` macro keeps the setup block's final expression alive for the
event loop and drops it at exit — tuple fields drop left-to-right, so **list
children before parents in the return tuple**.

**RFC-2229 hazard** (edition 2021): `move` closures capture the *paths* they
use, not whole variables. A closure body writing `screens.0 .0.push(true)`
captures only that window field; the uncaptured sibling layers drop at the end
of the enclosing block, destroying their SDK objects while the windows still
list them as children — the screen renders blank. This was observed on the
emulator. Fix: destructure first, have closures use whole variables (a bare
path captures completely), and keep layers alive in the app-state tuple.
`canvas.rs` carries the full explanation.

## Key Decisions
- **Window does NOT own its children** (proposed in Phase 4 review, rejected in
  Phase 5). Post-add mutation is ubiquitous — `set_text`, `mark_dirty`,
  `reload` all take `&mut self` after `add_child` — so ownership would force a
  handle/index API. The borrow model plus the return-tuple contract is the
  chosen guardrail. Rationale is preserved in `layer.rs`.
- **One shared heap.** `heap.rs` routes the global allocator through the SDK's
  `malloc`/`free` so `heap_bytes_free()` stays meaningful (the smoke test's
  leak check depends on this). Firmware alignment guarantee is undocumented;
  we assume 4 and over-allocate for stricter requests.
- **128-byte `FixedBuf`, shared with the panic handler.** Costs ~160 bytes of
  caller stack (measured on-target); the panic handler may run from a deep
  stack. Growing it requires measurement, not a guess.
- Text is stored by the SDK as a bare `const char*` with no copy, so
  `text_buf.rs` owns the storage for as long as the SDK may read it.

## Gotchas
- `#![cfg_attr(not(test), no_std)]` — host tests get std. `panic.rs` is gated
  on `target_os = "none"`; layout assertions in `ferrite-sys` on
  `target_arch = "arm"`. A host-only test run does not exercise either.
- `log` macros truncate at 127 bytes. `core::fmt` is the usual cause of sudden
  `.text` growth — `{:?}` on large types and float formatting are the costly
  ones.
- `graphics::Graphics` carries a lifetime because the SDK `GContext` is only
  valid inside an update proc.

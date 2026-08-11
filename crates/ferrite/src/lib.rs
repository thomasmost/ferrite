//! Safe Rust API and runtime for PebbleOS watchapps.

// `no_std` except under `cargo test`, where the host test harness needs std.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub use ferrite_sys as sys;

mod fmt_buf;

pub mod app_message;
pub mod canvas;
pub mod click;
pub mod graphics;
pub mod health;
pub mod heap;
pub mod layer;
pub mod log;
pub mod menu_layer;
pub mod persist;
// Panic handler gated on target_os = "none" to avoid clashing with std's
// panic handler during host testing. The handler uses FixedBuf for logging.
#[cfg(target_os = "none")]
mod panic;
pub mod text_layer;
pub mod tick;
pub mod time;
pub mod types;
pub mod window;

mod text_buf;

/// Capability token proving the app runtime is initialized.
///
/// A `&mut App` is handed to your setup code by [`app!`] and proves that the
/// SDK event loop is ready. Wrapper constructors require it to enforce that
/// SDK calls only run after `main` initialization. The token is created via
/// `unsafe fn __new()`, so the contract is maintained by user discipline
/// (user code should not call `__new` directly -- the [`app!`] macro is the
/// only intended caller).
pub struct App {
    _private: (),
}

impl App {
    /// Internal constructor used by the [`app!`] macro. Do not call directly.
    #[doc(hidden)]
    pub unsafe fn __new() -> App {
        App { _private: () }
    }
}

/// Declares the app entry point.
///
/// Expands to `#[no_mangle] extern "C" fn main()`, which constructs the
/// [`App`] token, runs your setup block, keeps the block's resulting value
/// alive while the SDK event loop runs, and drops it (running destructors)
/// when the app exits.
///
/// The setup block's final expression is the state kept alive for the app's
/// lifetime — return every window/layer you create from the block. When
/// returning multiple objects in a tuple, list children *before* their
/// parents: Rust drops tuple fields left-to-right, so children drop first.
/// This prevents use-after-free in child drop impls that unlink from parent:
///
/// ```ignore
/// ferrite::app! {
///     fn main(app: &mut App) {
///         let window = Window::new(app);
///         let text = TextLayer::new(app, bounds);
///         // ... configure, add child ...
///         window.add_child(&text);
///         // Children must be listed before parents in the return tuple.
///         (text, window) // text drops first, then window
///     }
/// }
/// ```
///
/// **Capture guidance (RFC-2229):** When moving windows/layers into closures
/// (e.g. menu handlers), destructure and move entire variables, not tuple
/// fields. Example: `let (window, layer) = ...; closure(move || window.push(
/// ...))` captures the whole window. Do NOT do `let screens = (window,
/// layer); closure(move || screens.0.push(...))` — the uncaptured layer
/// (screens.1) drops early and destroys its SDK resources while the window
/// is still visible. See `canvas.rs` for the full RFC-2229 hazard explanation.
///
/// Cleanup: when the event loop returns (user exits the app), the kept
/// state is dropped in tuple order — children first, exactly as listed in
/// the example above — running every wrapper's `Drop` (windows destroy their
/// SDK objects, services unsubscribe). This is the finalized lifecycle — apps
/// never manage teardown manually.
#[macro_export]
macro_rules! app {
    (fn main($app:ident: &mut App) $body:block) => {
        #[no_mangle]
        pub extern "C" fn main() -> i32 {
            let mut __token = unsafe { $crate::App::__new() };
            let $app: &mut $crate::App = &mut __token;
            let __state = $body;
            unsafe { $crate::sys::app_event_loop() };
            ::core::mem::drop(__state);
            0
        }
    };
}

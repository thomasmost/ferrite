//! Safe Rust API and runtime for PebbleOS watchapps.

// `no_std` except under `cargo test`, where the host test harness needs std.
#![cfg_attr(not(test), no_std)]

pub use ferrite_sys as sys;

// Compiled on the host only under `cfg(test)`, so `FixedBuf`'s truncation
// arithmetic is unit-testable. The `#[panic_handler]` itself stays gated on
// `target_os = "none"` so it never clashes with std's.
#[cfg(any(target_os = "none", test))]
mod panic;
mod fmt_buf;
pub mod heap;
pub mod log;
pub mod text_layer;
pub mod window;

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
/// Cleanup: when the event loop returns (user exits the app), the kept
/// state is dropped in reverse declaration order, running every wrapper's
/// `Drop` (windows destroy their SDK objects, services unsubscribe).
/// This is the finalized lifecycle — apps never manage teardown manually.
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

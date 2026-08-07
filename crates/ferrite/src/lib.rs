//! Safe Rust API and runtime for PebbleOS watchapps.

#![no_std]

pub use ferrite_sys as sys;

#[cfg(target_os = "none")]
mod panic;
pub mod text_layer;
pub mod window;

use core::ffi::CStr;

/// Capability token proving the app runtime is initialized.
///
/// A `&mut App` is handed to your setup code by [`app!`]; it cannot be
/// constructed by user code. Wrapper constructors take it so SDK calls are
/// structurally impossible before `main` runs.
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

/// Log a message at INFO level via the SDK's `app_log` (shows in `pebble logs`).
pub fn log_info(msg: &CStr) {
    unsafe {
        sys::app_log(
            sys::APP_LOG_LEVEL_INFO,
            c"rust".as_ptr(),
            0,
            c"%s".as_ptr(),
            msg.as_ptr(),
        );
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
/// lifetime — return every window/layer you create from the block:
///
/// ```ignore
/// ferrite::app! {
///     fn main(app: &mut App) {
///         let window = Window::new(app);
///         // ... configure, push ...
///         window // kept alive until the app exits
///     }
/// }
/// ```
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

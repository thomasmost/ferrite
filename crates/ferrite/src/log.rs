//! Logging via the SDK's `app_log` — lines appear in `pebble logs`.
//!
//! Two entry points per level: `&CStr` (zero formatting cost — prefer for
//! fixed messages) and the `error!`/`warn!`/`info!`/`debug!` macros
//! (`format_args!`-based, truncated at 127 bytes).

use core::ffi::CStr;
use core::fmt::Write;

use crate::fmt_buf::FixedBuf;
use crate::sys;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    fn raw(self) -> u8 {
        match self {
            Level::Error => sys::AppLogLevel::APP_LOG_LEVEL_ERROR.0,
            Level::Warn => sys::AppLogLevel::APP_LOG_LEVEL_WARNING.0,
            Level::Info => sys::AppLogLevel::APP_LOG_LEVEL_INFO.0,
            Level::Debug => sys::AppLogLevel::APP_LOG_LEVEL_DEBUG.0,
        }
    }
}

/// Log a fixed C-string message at the given level.
pub fn log(level: Level, msg: &CStr) {
    unsafe {
        sys::app_log(level.raw(), c"rust".as_ptr(), 0, c"%s".as_ptr(), msg.as_ptr());
    }
}

pub fn error(msg: &CStr) {
    log(Level::Error, msg);
}

pub fn warn(msg: &CStr) {
    log(Level::Warn, msg);
}

pub fn info(msg: &CStr) {
    log(Level::Info, msg);
}

pub fn debug(msg: &CStr) {
    log(Level::Debug, msg);
}

/// Format-and-log backend for the level macros. Truncates at 127 bytes.
pub fn log_fmt(level: Level, args: core::fmt::Arguments) {
    let mut buf = FixedBuf::new();
    let _ = buf.write_fmt(args);
    unsafe {
        sys::app_log(
            level.raw(),
            c"rust".as_ptr(),
            0,
            c"%s".as_ptr(),
            buf.as_cstr_ptr(),
        );
    }
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::Level::Error, ::core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::Level::Warn, ::core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::Level::Info, ::core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {
        $crate::log::log_fmt($crate::log::Level::Debug, ::core::format_args!($($arg)*))
    };
}

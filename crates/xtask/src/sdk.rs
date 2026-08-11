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
        .unwrap_or_default()
        .join("Library/Application Support/Pebble SDK/SDKs")
        .join(SDK_VERSION)
}

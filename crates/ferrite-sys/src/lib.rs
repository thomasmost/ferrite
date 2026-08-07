//! Raw FFI bindings to the rePebble SDK (Emery, SDK core 4.17).
//!
//! Phase 1: hand-written declarations for the hello-world surface only.
//! Signatures transcribed from
//! `$SDK/sdk-core/pebble/emery/include/pebble.h` (SDK 4.17).
//! Replaced by generated bindings in Phase 2.

#![no_std]
#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use core::ffi::{c_char, c_int};

// --- Value types (match C layout exactly; passed/returned by value) ---

/// 8-bit ARGB (2 bits per channel). C: `union GColor8 { uint8_t argb; ... }`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GColor8 {
    pub argb: u8,
}

pub type GColor = GColor8;

pub const GColorBlack: GColor8 = GColor8 { argb: 0b1100_0000 };
pub const GColorWhite: GColor8 = GColor8 { argb: 0b1111_1111 };
pub const GColorClear: GColor8 = GColor8 { argb: 0b0000_0000 };

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GPoint {
    pub x: i16,
    pub y: i16,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GSize {
    pub w: i16,
    pub h: i16,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GRect {
    pub origin: GPoint,
    pub size: GSize,
}

// Ports of the C constructor macros of the same names.
pub const fn GPoint(x: i16, y: i16) -> GPoint {
    GPoint { x, y }
}

pub const fn GSize(w: i16, h: i16) -> GSize {
    GSize { w, h }
}

pub const fn GRect(x: i16, y: i16, w: i16, h: i16) -> GRect {
    GRect {
        origin: GPoint(x, y),
        size: GSize(w, h),
    }
}

// --- Log levels (C enum AppLogLevel; app_log takes uint8_t) ---

pub const APP_LOG_LEVEL_ERROR: u8 = 1;
pub const APP_LOG_LEVEL_WARNING: u8 = 50;
pub const APP_LOG_LEVEL_INFO: u8 = 100;
pub const APP_LOG_LEVEL_DEBUG: u8 = 200;

// --- Opaque SDK object types ---

#[repr(C)]
pub struct Window {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Layer {
    _private: [u8; 0],
}

#[repr(C)]
pub struct TextLayer {
    _private: [u8; 0],
}

extern "C" {
    // Window
    pub fn window_create() -> *mut Window;
    pub fn window_destroy(window: *mut Window);
    pub fn window_get_root_layer(window: *const Window) -> *mut Layer;
    pub fn window_stack_push(window: *mut Window, animated: bool);

    // Layer
    pub fn layer_get_bounds(layer: *const Layer) -> GRect;
    pub fn layer_add_child(parent: *mut Layer, child: *mut Layer);

    // TextLayer
    pub fn text_layer_create(frame: GRect) -> *mut TextLayer;
    pub fn text_layer_destroy(text_layer: *mut TextLayer);
    pub fn text_layer_get_layer(text_layer: *mut TextLayer) -> *mut Layer;
    pub fn text_layer_set_text(text_layer: *mut TextLayer, text: *const c_char);

    // App
    pub fn app_event_loop();
    pub fn app_log(
        log_level: u8,
        src_filename: *const c_char,
        src_line_number: c_int,
        fmt: *const c_char,
        ...
    );
}

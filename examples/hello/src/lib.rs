//! Two-window demo: SELECT pushes the second window, BACK pops it
//! (automatic for watchapps). Heartbeat log retained for scripts/check.sh.

#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

use ferrite::click::Button;
use ferrite::text_layer::{system_font, TextLayer};
use ferrite::window::Window;
use ferrite::sys;

static TICKS: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_tick(_tick_time: *mut sys::tm, _units_changed: sys::TimeUnits) {
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    let boxed = alloc::boxed::Box::new(n);
    let boxed_8 = alloc::boxed::Box::new(n as u64);
    let free = ferrite::heap::heap_bytes_free();
    ferrite::info!("HEARTBEAT {} heap_free={} u64={}", *boxed, free, *boxed_8);
}

ferrite::app! {
    fn main(app: &mut App) {
        ferrite::log::info(c"hello starting");

        // Second screen, built up front, moved into the SELECT closure.
        let mut win2 = Window::new(app);
        let b2 = win2.bounds();
        let mut text2 = TextLayer::new(
            app,
            sys::GRect(0, b2.size.h / 2 - 20, b2.size.w, 40),
        );
        text2.set_text(c"Second window");
        text2.set_alignment(sys::GTextAlignment::GTextAlignmentCenter);
        text2.set_font(system_font(sys::FONT_KEY_GOTHIC_24));
        win2.add_child(&text2);
        win2.on_load(|| ferrite::log::info(c"window 2 loaded"));
        win2.on_unload(|| ferrite::log::info(c"window 2 unloaded"));

        // Main screen.
        let mut window = Window::new(app);
        let bounds = window.bounds();
        let mut text = TextLayer::new(
            app,
            sys::GRect(0, bounds.size.h / 2 - 30, bounds.size.w, 60),
        );
        text.set_text(c"Hello from Rust\nSELECT: next");
        text.set_alignment(sys::GTextAlignment::GTextAlignmentCenter);
        window.add_child(&text);
        window.on_load(|| ferrite::log::info(c"window 1 loaded"));

        // Capture win2 as a WHOLE variable. Do not bundle it into a tuple and
        // use one field in the closure: edition-2021 closures capture only the
        // paths they use (RFC 2229), so `move || tuple.1.push(..)` would move
        // just the window in and silently DROP the other field at the end of
        // this block -- destroying the text layer before the window is ever
        // shown. (That is exactly the bug the plan's original listing had.)
        window.on_click(Button::Select, move || {
            ferrite::log::info(c"SELECT pressed");
            win2.push(true);
        });

        window.push(true);

        unsafe {
            sys::tick_timer_service_subscribe(sys::TimeUnits::SECOND_UNIT, Some(on_tick));
        }

        // Children before parents: text2 (child of win2, which lives inside
        // window's click closure and drops with window) first, then text,
        // then window.
        (text2, text, window)
    }
}

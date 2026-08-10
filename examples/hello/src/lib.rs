//! Hello-world watchface with a heartbeat log: exercises the allocator
//! (Box) and the tick service, and is the target of scripts/check.sh.

#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

use ferrite::text_layer::TextLayer;
use ferrite::window::Window;
use ferrite::sys;

static TICKS: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_tick(_tick_time: *mut sys::tm, _units_changed: sys::TimeUnits) {
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    // Round-trip through the SDK heap so the smoke test exercises the
    // allocator on every heartbeat. Use both a 4-byte aligned box (fast path)
    // and an 8-byte aligned box to exercise the over-alignment shim on device.
    let boxed = alloc::boxed::Box::new(n);
    let boxed_8 = alloc::boxed::Box::new(n as u64);
    let free = ferrite::heap::heap_bytes_free();
    ferrite::info!("HEARTBEAT {} heap_free={} u64={}", *boxed, free, *boxed_8);
}

ferrite::app! {
    fn main(app: &mut App) {
        ferrite::log::info(c"hello starting");

        let mut window = Window::new(app);
        let bounds = window.bounds();
        let mut text = TextLayer::new(
            app,
            ferrite::sys::GRect(0, bounds.size.h / 2 - 20, bounds.size.w, 40),
        );
        text.set_text(c"Hello from Rust");
        window.add_child(&text);
        window.push();

        // Tick service is wrapped safely in Phase 6; raw sys is fine here.
        unsafe {
            sys::tick_timer_service_subscribe(sys::TimeUnits::SECOND_UNIT, Some(on_tick));
        }

        // CORRECTED during Phase 3: the original listing here said
        // `(window, text)`, which is the exact use-after-free drop order
        // Phase 1's review flagged as Critical (parent window destroyed
        // before its child text layer). Children before parents:
        (text, window)
    }
}

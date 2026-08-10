//! Menu-driven demo: home menu with two entries — a text screen and a
//! custom-drawn canvas screen. Heartbeat log retained for scripts/check.sh.

#![no_std]

extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

use ferrite::canvas::CanvasLayer;
use ferrite::menu_layer::MenuLayer;
use ferrite::text_layer::{system_font, TextLayer};
use ferrite::window::Window;
use ferrite::{sys, App};

static TICKS: AtomicU32 = AtomicU32::new(0);

extern "C" fn on_tick(_tick_time: *mut sys::tm, _units_changed: sys::TimeUnits) {
    let n = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    let boxed = alloc::boxed::Box::new(n);
    let boxed_8 = alloc::boxed::Box::new(n as u64);
    let free = ferrite::heap::heap_bytes_free();
    ferrite::info!("HEARTBEAT {} heap_free={} u64={}", *boxed, free, *boxed_8);
}

fn make_text_screen(app: &mut App) -> (Window, TextLayer) {
    let mut win = Window::new(app);
    let b = win.bounds();
    let mut text = TextLayer::new(app, sys::GRect(0, b.size.h / 2 - 20, b.size.w, 40));
    text.set_text(c"Hello from Rust");
    text.set_alignment(sys::GTextAlignment::GTextAlignmentCenter);
    text.set_font(system_font(sys::FONT_KEY_GOTHIC_24));
    win.add_child(&text);
    win.on_load(|| ferrite::log::info(c"text screen loaded"));
    (win, text)
}

fn make_canvas_screen(app: &mut App) -> (Window, CanvasLayer) {
    let mut win = Window::new(app);
    let b = win.bounds();
    let mut canvas = CanvasLayer::new(app, sys::GRect(0, 0, b.size.w, b.size.h));
    canvas.on_draw(|g| {
        let b = g.bounds();
        g.set_antialiased(true);
        // Diagonal cross
        g.set_stroke_color(sys::GColorBlack);
        g.set_stroke_width(3);
        g.draw_line(sys::GPoint(0, 0), sys::GPoint(b.size.w - 1, b.size.h - 1));
        g.draw_line(sys::GPoint(b.size.w - 1, 0), sys::GPoint(0, b.size.h - 1));
        // Filled circle in the center
        g.set_fill_color(sys::GColorFromRGB(255, 0, 0));
        g.fill_circle(sys::GPoint(b.size.w / 2, b.size.h / 2), 30);
    });
    win.add_child(&canvas);
    win.on_load(|| ferrite::log::info(c"canvas screen loaded"));
    (win, canvas)
}

ferrite::app! {
    fn main(app: &mut App) {
        ferrite::log::info(c"hello starting");

        // RFC-2229 fix: destructure to move whole windows into the closure,
        // not tuple fields. This ensures the layers (text, canvas) don't get
        // dropped early by disjoint capture.
        let (mut text_win, text) = make_text_screen(app);
        let (mut canvas_win, canvas) = make_canvas_screen(app);

        // Home: a menu filling the root window.
        let mut home = Window::new(app);
        let hb = home.bounds();
        let mut menu = MenuLayer::new(app, sys::GRect(0, 0, hb.size.w, hb.size.h));
        menu.rows(|| 2);
        menu.on_draw_row(|cell, row| match row {
            0 => cell.basic_draw_with_subtitle(c"Text", c"TextLayer demo"),
            _ => cell.basic_draw_with_subtitle(c"Canvas", c"custom drawing"),
        });

        // Move the windows into the closure. They will be dropped when the
        // closure (stored in menu's state) drops, which is after the menu
        // layer but before the home window, maintaining the correct order.
        menu.on_select(move |row| {
            ferrite::info!("menu select row={}", row);
            match row {
                0 => text_win.push(true),
                _ => canvas_win.push(true),
            }
        });
        menu.attach_clicks(&mut home);
        home.add_child(&menu);
        home.on_load(|| ferrite::log::info(c"home menu loaded"));
        home.push(true);

        unsafe {
            sys::tick_timer_service_subscribe(sys::TimeUnits::SECOND_UNIT, Some(on_tick));
        }

        // Children before parents: text (child of text window), canvas (child of
        // canvas window), menu (child of home window), then home.
        // The windows (text_win, canvas_win) live in menu's closure and drop with it.
        (text, canvas, menu, home)
    }
}

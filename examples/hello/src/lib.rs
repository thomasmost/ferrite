//! Hello-world watchface: proves the Ferrite build integration.

#![no_std]

use ferrite::text_layer::TextLayer;
use ferrite::window::Window;

ferrite::app! {
    fn main(app: &mut App) {
        ferrite::log_info(c"Hello from Rust");

        let mut window = Window::new(app);
        let bounds = window.bounds();
        let mut text = TextLayer::new(
            app,
            ferrite::sys::GRect(0, bounds.size.h / 2 - 20, bounds.size.w, 40),
        );
        text.set_text(c"Hello from Rust");
        text.set_text_color(ferrite::sys::GColorBlack);
        window.add_child(&text);
        window.push();

        // Returned state stays alive while the event loop runs.
        (window, text)
    }
}

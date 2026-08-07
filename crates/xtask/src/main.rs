//! Project automation: `cargo xtask <command>`.

use std::process::ExitCode;

mod gen_bindings;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bindgen") => gen_bindings::run(),
        _ => {
            eprintln!("usage: cargo xtask bindgen");
            ExitCode::FAILURE
        }
    }
}

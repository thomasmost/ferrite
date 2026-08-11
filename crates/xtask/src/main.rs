//! Project automation: `cargo xtask <command>`.

use std::process::ExitCode;

mod gen_bindings;
mod messagekeys;
mod sdk;
mod size;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("bindgen") => gen_bindings::run(),
        Some("messagekeys") => messagekeys::run(&args[1..]),
        Some("size") => size::run(&args[1..]),
        _ => {
            eprintln!("usage: cargo xtask <bindgen|messagekeys|size> [args]");
            ExitCode::FAILURE
        }
    }
}

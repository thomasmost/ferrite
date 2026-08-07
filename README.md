# Ferrite

A Rust toolchain for writing PebbleOS watchapps — targeting the Pebble Time 2
("Emery") with the community-maintained rePebble SDK (pebble-tool 5.0.x /
SDK 4.17).

Rust code compiles to a static library with Cargo; the SDK's own `pebble build`
links it into the app binary. See `examples/hello` for the project template.

**Status:** Phase 1 — build-integration proof.

## Prerequisites

- rePebble SDK: pebble-tool 5.0.x with SDK 4.17 installed
- Rust (stable) with the ARM target: `rustup target add thumbv7m-none-eabi`

## License

Licensed under either of Apache License, Version 2.0 (`LICENSE-APACHE`) or
MIT license (`LICENSE-MIT`) at your option.

//! `cargo xtask size [path/to/app.elf]`: static-size report against the
//! Emery app-memory budget. Defaults to the hello example's built ELF.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use crate::sdk::sdk_root;

/// Emery MAX_APP_MEMORY_SIZE (the linker's whole APP region; code, data,
/// bss, heap and stack all share it).
const APP_MEMORY_CAP: u64 = 0x20000;

fn workspace_root() -> PathBuf {
    // crates/xtask -> crates -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Parse Berkeley format size output: returns (text, data, bss) in bytes.
///
/// Berkeley format has a header line followed by a data line:
/// ```
///    text    data     bss     dec     hex filename
///   23456     712     104   24272    5ed0 pebble-app.elf
/// ```
/// This function extracts the first three fields (text, data, bss) from the
/// second line.
fn parse_berkeley(stdout: &str) -> Option<(u64, u64, u64)> {
    let fields: Vec<u64> = stdout
        .lines()
        .nth(1)?
        .split_whitespace()
        .take(3)
        .filter_map(|f| f.parse().ok())
        .collect();

    if fields.len() == 3 {
        Some((fields[0], fields[1], fields[2]))
    } else {
        None
    }
}

/// Pure budget arithmetic: (footprint, percent-of-cap, bytes remaining for
/// heap+stack, over-cap flag). The cap is inclusive: exactly at the cap is
/// allowed with zero remaining.
fn budget(text: u64, data: u64, bss: u64) -> (u64, u64, u64, bool) {
    let footprint = text + data + bss;
    let percent = footprint * 100 / APP_MEMORY_CAP;
    let remaining = APP_MEMORY_CAP - footprint.min(APP_MEMORY_CAP);
    (footprint, percent, remaining, footprint > APP_MEMORY_CAP)
}

pub fn run(args: &[String]) -> ExitCode {
    if matches!(args.first().map(String::as_str), Some("--help" | "-h")) {
        eprintln!("usage: cargo xtask size [path/to/app.elf]");
        eprintln!("  Reports .text/.data/.bss against Emery's 128 KB app-memory cap.");
        eprintln!("  Defaults to examples/hello/build/emery/pebble-app.elf.");
        return ExitCode::SUCCESS;
    }
    let elf = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("examples/hello/build/emery/pebble-app.elf"));
    if !elf.exists() {
        eprintln!(
            "error: {} not found — run `pebble build` in the app directory first",
            elf.display()
        );
        return ExitCode::FAILURE;
    }

    let size_tool = sdk_root().join("toolchain/arm-none-eabi/bin/arm-none-eabi-size");
    // Graceful on the foreseeable user condition (missing/mislocated SDK),
    // matching gen_bindings' house pattern rather than panicking.
    if !size_tool.exists() {
        eprintln!(
            "error: {} not found.\nInstall Pebble SDK {} or set PEBBLE_SDK_ROOT.",
            size_tool.display(),
            crate::sdk::SDK_VERSION
        );
        return ExitCode::FAILURE;
    }
    let output = match Command::new(&size_tool).arg(&elf).output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: failed to run {}: {e}", size_tool.display());
            return ExitCode::FAILURE;
        }
    };
    if !output.status.success() {
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        return ExitCode::FAILURE;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let (text, data, bss) = match parse_berkeley(&stdout) {
        Some(fields) => fields,
        None => {
            eprintln!("error: unexpected `size` output:\n{stdout}");
            return ExitCode::FAILURE;
        }
    };

    let (footprint, percent, remaining, over) = budget(text, data, bss);
    println!("{}", elf.display());
    println!("  .text (code+rodata): {text:>7} bytes");
    println!("  .data (init data):   {data:>7} bytes");
    println!("  .bss  (zeroed data): {bss:>7} bytes");
    println!(
        "  static footprint:    {footprint:>7} / {APP_MEMORY_CAP} bytes ({percent}% of app memory)"
    );
    println!("  left for heap+stack: {remaining:>7} bytes");

    if over {
        eprintln!("ERROR: exceeds the {APP_MEMORY_CAP}-byte app memory cap");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_berkeley_well_formed() {
        let output = "   text\tdata\tbss\tdec\thex\tfilename\n   23456\t712\t104\t24272\t5ed0\tpebble-app.elf\n";
        let result = parse_berkeley(output);
        assert_eq!(result, Some((23456, 712, 104)));
    }

    #[test]
    fn test_parse_berkeley_with_spaces() {
        let output = "   text    data     bss     dec     hex filename\n   23456    712     104    24272    5ed0 pebble-app.elf\n";
        let result = parse_berkeley(output);
        assert_eq!(result, Some((23456, 712, 104)));
    }

    #[test]
    fn test_parse_berkeley_missing_second_line() {
        let output = "   text    data     bss     dec     hex filename\n";
        let result = parse_berkeley(output);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_berkeley_non_numeric_fields() {
        let output = "   text    data     bss     dec     hex filename\n   text    data     bss    24272    5ed0 pebble-app.elf\n";
        let result = parse_berkeley(output);
        assert_eq!(result, None);
    }

    #[test]
    fn test_parse_berkeley_extra_whitespace() {
        let output = "   text    data     bss     dec     hex filename\n    23456     712      104    24272    5ed0 pebble-app.elf\n";
        let result = parse_berkeley(output);
        assert_eq!(result, Some((23456, 712, 104)));
    }

    #[test]
    fn test_parse_berkeley_large_numbers() {
        let output = "   text    data     bss     dec     hex filename\n   131072    0        0      131072   20000 pebble-app.elf\n";
        let result = parse_berkeley(output);
        assert_eq!(result, Some((131072, 0, 0)));
    }

    #[test]
    fn test_parse_berkeley_insufficient_fields() {
        let output = "   text    data     bss     dec     hex filename\n   23456    712\n";
        let result = parse_berkeley(output);
        assert_eq!(result, None);
    }

    /// The cap is inclusive: exactly at 0x20000 passes with zero remaining.
    #[test]
    fn budget_exactly_at_cap_is_allowed() {
        let (footprint, percent, remaining, over) = budget(APP_MEMORY_CAP, 0, 0);
        assert_eq!(footprint, APP_MEMORY_CAP);
        assert_eq!(percent, 100);
        assert_eq!(remaining, 0);
        assert!(!over);
    }

    /// Over the cap: flagged, zero remaining, and — the reason the remaining
    /// arithmetic uses `.min(cap)` — no underflow panic.
    #[test]
    fn budget_over_cap_is_flagged_without_underflow() {
        let (footprint, percent, remaining, over) = budget(APP_MEMORY_CAP, 1, 0);
        assert_eq!(footprint, APP_MEMORY_CAP + 1);
        assert_eq!(percent, 100);
        assert_eq!(remaining, 0);
        assert!(over);
    }

    #[test]
    fn budget_typical_footprint() {
        // The measured hello baseline at the time of writing.
        let (footprint, percent, remaining, over) = budget(12441, 444, 8);
        assert_eq!(footprint, 12893);
        assert_eq!(percent, 9);
        assert_eq!(remaining, 131072 - 12893);
        assert!(!over);
    }
}

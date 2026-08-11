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

pub fn run(args: &[String]) -> ExitCode {
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
    let output = Command::new(&size_tool)
        .arg(&elf)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", size_tool.display()));
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

    let footprint = text + data + bss;
    let percent = footprint * 100 / APP_MEMORY_CAP;
    println!("{}", elf.display());
    println!("  .text (code+rodata): {text:>7} bytes");
    println!("  .data (init data):   {data:>7} bytes");
    println!("  .bss  (zeroed data): {bss:>7} bytes");
    println!(
        "  static footprint:    {footprint:>7} / {APP_MEMORY_CAP} bytes ({percent}% of app memory)"
    );
    println!(
        "  left for heap+stack: {:>7} bytes",
        APP_MEMORY_CAP - footprint.min(APP_MEMORY_CAP)
    );

    if footprint > APP_MEMORY_CAP {
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
}

//! End-to-end smoke test for the `uuid` built-in through the `neojs` command.

use std::fs;
use std::process::Command;

/// Returns true when `value` has canonical lowercase text and the expected
/// version nibble. The variant nibble must be RFC 9562 (`10xx`).
fn is_uuid(value: &str, version: u8) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (index, byte) in bytes.iter().enumerate() {
        match index {
            8 | 13 | 18 | 23 => {
                if *byte != b'-' {
                    return false;
                }
            }
            _ => {
                if !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte) {
                    return false;
                }
            }
        }
    }
    bytes[14] == b'0' + version && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}

#[test]
fn cli_prints_uuid_v4_and_v7() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"import { v4 as aggregateV4 } from "neo:uuid";
import v4 from "neo:uuid/v4";
import v7 from "neo:uuid/v7";
if (v4 !== aggregateV4) throw new Error("v4 identity changed");
console.log(v4());
console.log(v7());
"#,
    )
    .expect("write source");

    let output = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .arg(&file)
        .output()
        .expect("spawn neojs");
    assert!(
        output.status.success(),
        "neojs failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "unexpected stdout: {stdout:?}");
    assert!(is_uuid(lines[0], 4), "not a v4 UUID: {}", lines[0]);
    assert!(is_uuid(lines[1], 7), "not a v7 UUID: {}", lines[1]);
}

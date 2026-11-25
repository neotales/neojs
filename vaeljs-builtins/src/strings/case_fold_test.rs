use super::case_fold::simple_fold;
use sha2::{Digest, Sha256};

/// The exact official Unicode 16.0 artifact the table is generated from.
const ARTIFACT: &[u8] = include_bytes!("../../data/unicode/CaseFolding-16.0.0.txt");
const ARTIFACT_SHA256: &str = "6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb";

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(text, "{byte:02x}").expect("writing a byte to a String cannot fail");
    }
    text
}

#[test]
fn the_artifact_is_the_pinned_official_unicode_16_file() {
    assert_eq!(
        hex(&Sha256::digest(ARTIFACT)),
        ARTIFACT_SHA256,
        "the CaseFolding.txt artifact hash changed"
    );
    let text = std::str::from_utf8(ARTIFACT).expect("Unicode data is UTF-8");
    assert!(
        text.starts_with("# CaseFolding-16.0.0.txt\n"),
        "the artifact is not the Unicode 16.0 CaseFolding file"
    );
}

#[test]
fn every_code_point_matches_the_pinned_artifact() {
    let text = std::str::from_utf8(ARTIFACT).expect("Unicode data is UTF-8");
    let mut expected = vec![None; 0x11_0000];
    let mut mapped = 0usize;
    for (line_number, line) in text.lines().enumerate() {
        let payload = line.split('#').next().unwrap_or("").trim();
        if payload.is_empty() {
            continue;
        }
        let fields: Vec<&str> = payload.split(';').map(str::trim).collect();
        assert!(
            fields.len() >= 3 && fields[3..].iter().all(|field| field.is_empty()),
            "line {} is not three fields",
            line_number + 1
        );
        let code = u32::from_str_radix(fields[0], 16).expect("a hexadecimal code point");
        let status = fields[1];
        if !matches!(status, "C" | "S") {
            continue;
        }
        let mapping: Vec<&str> = fields[2].split_whitespace().collect();
        assert_eq!(
            mapping.len(),
            1,
            "line {} is not a simple mapping",
            line_number + 1
        );
        let target = u32::from_str_radix(mapping[0], 16).expect("a hexadecimal target");
        let slot = expected
            .get_mut(code as usize)
            .expect("a code point below 0x110000");
        assert!(
            slot.replace(target).is_none(),
            "line {} duplicates U+{code:04X}",
            line_number + 1
        );
        mapped += 1;
    }
    assert_eq!(mapped, 1484, "unexpected number of simple mappings");

    for code in 0..0x11_0000u32 {
        let folded = simple_fold(code);
        let wanted = expected[code as usize].unwrap_or(code);
        assert_eq!(
            folded, wanted,
            "U+{code:04X} folded to U+{folded:04X}, expected U+{wanted:04X}"
        );
    }
}

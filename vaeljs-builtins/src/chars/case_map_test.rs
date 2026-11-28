use super::case_map::{simple_lower, simple_upper};
use sha2::{Digest, Sha256};

/// The exact official Unicode 16.0 artifacts the case-mapping tables are
/// generated from.
const UNICODEDATA_ARTIFACT: &[u8] = include_bytes!("../../data/unicode/UnicodeData-16.0.0.txt");
const UNICODEDATA_SHA256: &str = "ff58e5823bd095166564a006e47d111130813dcf8bf234ef79fa51a870edb48f";

const SPECIALCASING_ARTIFACT: &[u8] = include_bytes!("../../data/unicode/SpecialCasing-16.0.0.txt");
const SPECIALCASING_SHA256: &str =
    "8d5de354eef79f2395a54c9c7dcebbaf3d30fc962d0f85611ea97aa973a0c451";

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(text, "{byte:02x}").expect("writing a byte to a String cannot fail");
    }
    text
}

#[test]
fn the_artifacts_are_the_pinned_official_unicode_16_files() {
    assert_eq!(
        hex(&Sha256::digest(UNICODEDATA_ARTIFACT)),
        UNICODEDATA_SHA256,
        "the UnicodeData-16.0.0.txt artifact hash changed"
    );
    let ud_text = std::str::from_utf8(UNICODEDATA_ARTIFACT).expect("UnicodeData is UTF-8");
    assert!(
        ud_text.starts_with("0000;<control>;Cc;0;BN;;;;;N;NULL;;;;\n"),
        "the artifact is not the expected Unicode 16.0 UnicodeData file"
    );

    assert_eq!(
        hex(&Sha256::digest(SPECIALCASING_ARTIFACT)),
        SPECIALCASING_SHA256,
        "the SpecialCasing-16.0.0.txt artifact hash changed"
    );
    let sc_text = std::str::from_utf8(SPECIALCASING_ARTIFACT).expect("SpecialCasing is UTF-8");
    assert!(
        sc_text.starts_with("# SpecialCasing-16.0.0.txt\n"),
        "the artifact is not the expected Unicode 16.0 SpecialCasing file"
    );
}

#[test]
fn every_scalar_matches_the_pinned_unicodedata_and_specialcasing_artifacts() {
    let ud_text = std::str::from_utf8(UNICODEDATA_ARTIFACT).expect("UnicodeData is UTF-8");
    let mut expected_upper = vec![None; 0x11_0000];
    let mut expected_lower = vec![None; 0x11_0000];
    let mut upper_count = 0usize;
    let mut lower_count = 0usize;

    for (line_number, line) in ud_text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(';').collect();
        assert_eq!(
            fields.len(),
            15,
            "UnicodeData line {} does not have 15 fields",
            line_number + 1
        );
        let code = u32::from_str_radix(fields[0], 16).expect("a hexadecimal code point");
        if !fields[12].is_empty() {
            let target = u32::from_str_radix(fields[12], 16).expect("a hexadecimal upper target");
            assert!(
                expected_upper[code as usize].replace(target).is_none(),
                "UnicodeData line {} duplicates uppercase mapping for U+{code:04X}",
                line_number + 1
            );
            upper_count += 1;
        }
        if !fields[13].is_empty() {
            let target = u32::from_str_radix(fields[13], 16).expect("a hexadecimal lower target");
            assert!(
                expected_lower[code as usize].replace(target).is_none(),
                "UnicodeData line {} duplicates lowercase mapping for U+{code:04X}",
                line_number + 1
            );
            lower_count += 1;
        }
    }

    assert_eq!(
        upper_count, 1477,
        "unexpected number of UnicodeData simple uppercase mappings"
    );
    assert_eq!(
        lower_count, 1460,
        "unexpected number of UnicodeData simple lowercase mappings"
    );

    let sc_text = std::str::from_utf8(SPECIALCASING_ARTIFACT).expect("SpecialCasing is UTF-8");
    for (line_number, line) in sc_text.lines().enumerate() {
        let payload = line.split('#').next().unwrap_or("").trim();
        if payload.is_empty() {
            continue;
        }
        let fields: Vec<&str> = payload.split(';').map(str::trim).collect();
        assert!(
            fields.len() >= 4,
            "SpecialCasing line {} has fewer than 4 fields",
            line_number + 1
        );
        let condition = fields.get(4).copied().unwrap_or("");
        if !condition.is_empty() {
            continue;
        }
        let code = u32::from_str_radix(fields[0], 16).expect("a hexadecimal code point");
        if fields[1].split_whitespace().count() != 1 {
            expected_lower[code as usize] = None;
        }
        if fields[3].split_whitespace().count() != 1 {
            expected_upper[code as usize] = None;
        }
    }

    for code in 0..=0x10_FFFFu32 {
        if (0xD800..=0xDFFF).contains(&code) {
            continue;
        }
        let wanted_upper = expected_upper[code as usize].unwrap_or(code);
        let got_upper = simple_upper(code);
        assert_eq!(
            got_upper, wanted_upper,
            "U+{code:04X} simple_upper returned U+{got_upper:04X}, expected U+{wanted_upper:04X}"
        );

        let wanted_lower = expected_lower[code as usize].unwrap_or(code);
        let got_lower = simple_lower(code);
        assert_eq!(
            got_lower, wanted_lower,
            "U+{code:04X} simple_lower returned U+{got_lower:04X}, expected U+{wanted_lower:04X}"
        );
    }
}

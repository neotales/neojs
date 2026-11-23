//! Artifact-level checks for the shared `chars` implementation in QuickJS
//! builds.
//!
//! The needles are distinctive sequences from the actual implementation, not
//! markers. A disabled build must not contain them. They are decoded from hex at
//! runtime, so the test source does not satisfy its own scan.

use std::fs;

use neojs_lite_core::Runtime;

/// `const DECIMAL_DIGIT = /\p{Nd}/u;` from the shared `chars` implementation.
const CHARS_SEQUENCE_ONE_HEX: &str =
    "636f6e737420444543494d414c5f4449474954203d202f5c707b4e647d2f753b";
/// `throw new makeRangeError(\`${name} must be a Unicode scalar value\`);` from
/// the shared `chars` implementation.
const CHARS_SEQUENCE_TWO_HEX: &str = "7468726f77206e6577206d616b6552616e67654572726f722860247b6e616d657d206d757374206265206120556e69636f6465207363616c61722076616c756560293b";

/// A string that is always in the shim, proving the scan reads real source bytes.
const CONTROL_HEX: &str = "6765744275696c74496e4d6f64756c65734c697374";

fn decode_hex(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2));
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn assert_sequences(artifact: &[u8], sequences: &[&str], expected: bool, description: &str) {
    for hex in sequences {
        let needle = decode_hex(hex);
        assert_eq!(
            contains(artifact, &needle),
            expected,
            "{description}: implementation sequence {hex} presence is {expected}"
        );
    }
}

#[test]
fn shared_chars_source_bytes_follow_the_feature() {
    drop(Runtime::new().expect("runtime"));

    let artifact = fs::read(std::env::current_exe().expect("current exe")).expect("read artifact");
    let control = decode_hex(CONTROL_HEX);
    assert!(
        contains(&artifact, &control),
        "artifact scan control is missing; the scan is not reading shim source bytes"
    );

    let chars = [CHARS_SEQUENCE_ONE_HEX, CHARS_SEQUENCE_TWO_HEX];
    #[cfg(feature = "neo-chars")]
    assert_sequences(&artifact, &chars, true, "neo-chars is enabled");
    #[cfg(not(feature = "neo-chars"))]
    assert_sequences(&artifact, &chars, false, "neo-chars is disabled");
}

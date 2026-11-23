//! Artifact-level checks for the native `chars` and `strings` bindings in
//! QuickJS builds.
//!
//! The needles are Rust error texts used by the native bindings, not markers. A
//! disabled build does not link the binding, so the text is absent. They are
//! decoded from hex at runtime, so the test source does not satisfy its own scan.

use std::fs;

use neojs_lite_core::Runtime;

/// `NON_SCALAR_MESSAGE` from the Rust `chars` implementation and binding.
const CHARS_SEQUENCE_ONE_HEX: &str =
    "76616c7565206d757374206265206120556e69636f6465207363616c61722076616c7565";

/// `FROM_INDEX_NOT_INTEGER` from the Rust `strings` implementation and binding.
const STRINGS_SEQUENCE_ONE_HEX: &str = "66726f6d496e646578206d75737420626520616e20696e7465676572";

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
fn shared_source_bytes_follow_the_features() {
    drop(Runtime::new().expect("runtime"));

    let artifact = fs::read(std::env::current_exe().expect("current exe")).expect("read artifact");
    let control = decode_hex(CONTROL_HEX);
    assert!(
        contains(&artifact, &control),
        "artifact scan control is missing; the scan is not reading shim source bytes"
    );

    let chars = [CHARS_SEQUENCE_ONE_HEX];
    #[cfg(feature = "neo-chars")]
    assert_sequences(&artifact, &chars, true, "neo-chars is enabled");
    #[cfg(not(feature = "neo-chars"))]
    assert_sequences(&artifact, &chars, false, "neo-chars is disabled");

    let strings = [STRINGS_SEQUENCE_ONE_HEX];
    #[cfg(feature = "neo-strings")]
    assert_sequences(&artifact, &strings, true, "neo-strings is enabled");
    #[cfg(not(feature = "neo-strings"))]
    assert_sequences(&artifact, &strings, false, "neo-strings is disabled");
}

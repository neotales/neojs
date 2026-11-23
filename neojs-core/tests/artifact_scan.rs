//! Artifact-level checks for feature-selected built-in implementation bytes.
//!
//! Each needle is a distinctive sequence from the actual implementation, not a
//! marker added only for testing. A disabled build must not contain it; an
//! enabled build must. Reintroducing the original inline implementation into
//! the monolith would therefore fail the disabled check even without any marker.
//!
//! The needles are decoded from hex at runtime. They never appear contiguously
//! in this test's own data, so the test source cannot satisfy its own scan.

use std::fs;

use neojs_core::Runtime;

/// `const DECIMAL_DIGIT = /\p{Nd}/u;` from the shared `chars` implementation.
const CHARS_SEQUENCE_ONE_HEX: &str =
    "636f6e737420444543494d414c5f4449474954203d202f5c707b4e647d2f753b";
/// `throw new makeRangeError(\`${name} must be a Unicode scalar value\`);` from
/// the shared `chars` implementation.
const CHARS_SEQUENCE_TWO_HEX: &str = "7468726f77206e6577206d616b6552616e67654572726f722860247b6e616d657d206d757374206265206120556e69636f6465207363616c61722076616c756560293b";

/// `const CHAR_STATE = Symbol("charSliceState");` from `slices.impl.js`.
const SLICES_SEQUENCE_ONE_HEX: &str =
    "636f6e737420434841525f5354415445203d2053796d626f6c282263686172536c696365537461746522293b";
/// `throw makeTypeError("slices target must be an Array or a typed array");`
/// from `slices.impl.js`.
const SLICES_SEQUENCE_TWO_HEX: &str = "7468726f77206d616b65547970654572726f722822736c6963657320746172676574206d75737420626520616e204172726179206f72206120747970656420617272617922293b";

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
fn embedded_implementation_bytes_follow_the_features() {
    // Reference the shim through the real constructor so the enabled source is
    // linked into this test binary rather than left in an unused rlib object.
    drop(Runtime::new().expect("runtime"));

    let artifact = fs::read(std::env::current_exe().expect("current exe")).expect("read artifact");
    let control = decode_hex(CONTROL_HEX);
    assert!(
        contains(&artifact, &control),
        "artifact scan control is missing; the scan is not reading shim source bytes"
    );

    let chars = [CHARS_SEQUENCE_ONE_HEX, CHARS_SEQUENCE_TWO_HEX];
    #[cfg(feature = "neo-chars")]
    assert_sequences(&artifact, &chars, true, "chars is enabled");
    #[cfg(not(feature = "neo-chars"))]
    assert_sequences(&artifact, &chars, false, "chars is disabled");

    let slices = [SLICES_SEQUENCE_ONE_HEX, SLICES_SEQUENCE_TWO_HEX];
    #[cfg(feature = "neo-slices")]
    assert_sequences(&artifact, &slices, true, "slices is enabled");
    #[cfg(not(feature = "neo-slices"))]
    assert_sequences(&artifact, &slices, false, "slices is disabled");
}

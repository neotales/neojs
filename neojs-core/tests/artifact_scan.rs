//! Artifact-level checks for feature-selected built-in source.
//!
//! The shim is embedded in the library as string data, so scanning the test
//! executable proves whether a disabled implementation's source bytes were
//! compiled in. The sentinels are decoded from hex at runtime. They never appear
//! contiguously in this test's own data, so a disabled build cannot produce a
//! false positive from the test source itself.

use std::fs;

use neojs_core::Runtime;

const CHARS_SENTINEL_HEX: &str = "4e454f5f43484152535f41525449464143545f3346384332443141";
const SLICES_SENTINEL_HEX: &str = "4e454f5f534c494345535f41525449464143545f3742344539433244";
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

#[test]
fn embedded_implementation_bytes_follow_the_features() {
    // Reference the shim through the real constructor so the enabled source is
    // linked into this test binary rather than left in an unused rlib object.
    drop(Runtime::new().expect("runtime"));

    let artifact = fs::read(std::env::current_exe().expect("current exe")).expect("read artifact");
    let chars = decode_hex(CHARS_SENTINEL_HEX);
    let slices = decode_hex(SLICES_SENTINEL_HEX);
    let control = decode_hex(CONTROL_HEX);

    assert!(
        contains(&artifact, &control),
        "artifact scan control is missing; the scan is not reading shim source bytes"
    );

    #[cfg(feature = "neo-chars")]
    assert!(
        contains(&artifact, &chars),
        "chars is enabled but its source sentinel is absent from the binary"
    );
    #[cfg(not(feature = "neo-chars"))]
    assert!(
        !contains(&artifact, &chars),
        "chars is disabled but its source sentinel is in the binary"
    );

    #[cfg(feature = "neo-slices")]
    assert!(
        contains(&artifact, &slices),
        "slices is enabled but its source sentinel is absent from the binary"
    );
    #[cfg(not(feature = "neo-slices"))]
    assert!(
        !contains(&artifact, &slices),
        "slices is disabled but its source sentinel is in the binary"
    );
}

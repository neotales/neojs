//! Artifact-level checks for the shared `chars` source in QuickJS builds.
//!
//! The sentinel is decoded from hex at runtime, so the test source does not
//! contain the contiguous token and cannot create a false positive in a disabled
//! build. Creating a runtime references the shim, which links the enabled
//! implementation into this test binary.

use std::fs;

use neojs_lite_core::Runtime;

const CHARS_SENTINEL_HEX: &str = "4e454f5f43484152535f41525449464143545f3346384332443141";
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
fn shared_chars_source_bytes_follow_the_feature() {
    drop(Runtime::new().expect("runtime"));

    let artifact = fs::read(std::env::current_exe().expect("current exe")).expect("read artifact");
    let chars = decode_hex(CHARS_SENTINEL_HEX);
    let control = decode_hex(CONTROL_HEX);

    assert!(
        contains(&artifact, &control),
        "artifact scan control is missing; the scan is not reading shim source bytes"
    );

    #[cfg(feature = "neo-chars")]
    assert!(
        contains(&artifact, &chars),
        "neo-chars is enabled but the shared source sentinel is absent"
    );
    #[cfg(not(feature = "neo-chars"))]
    assert!(
        !contains(&artifact, &chars),
        "neo-chars is disabled but the shared source sentinel is present"
    );
}

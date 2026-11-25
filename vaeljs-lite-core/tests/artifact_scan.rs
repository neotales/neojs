//! Artifact-level checks for the native `chars` and `strings` bindings in
//! QuickJS builds.
//!
//! The needles are Rust error texts used by the native bindings, not markers. A
//! disabled build does not link the binding, so the text is absent. They are
//! decoded from hex at runtime, so the test source does not satisfy its own scan.

use std::fs;

use vaeljs_lite_core::Runtime;

/// `NON_SCALAR_MESSAGE` from the Rust `chars` implementation and binding.
const CHARS_SEQUENCE_ONE_HEX: &str =
    "76616c7565206d757374206265206120556e69636f6465207363616c61722076616c7565";

/// `FROM_INDEX_NOT_INTEGER` from the Rust `strings` implementation and binding.
const STRINGS_SEQUENCE_ONE_HEX: &str = "66726f6d496e646578206d75737420626520616e20696e7465676572";

/// `assert.not-thenable` from `vaeljs-builtins/src/assert.js`.
const ASSERT_SEQUENCE_ONE_HEX: &str = "6173736572742e6e6f742d7468656e61626c65";

/// `VAEL_TEST_DURING_RUN` from `vaeljs-builtins/src/test.js`.
const TEST_SEQUENCE_ONE_HEX: &str = "5641454c5f544553545f445552494e475f52554e";

/// `system clock is outside the UUID version 7 timestamp range` from `vaeljs-builtins/src/uuid.rs`.
const UUID_SEQUENCE_ONE_HEX: &str = "73797374656d20636c6f636b206973206f7574736964652074686520555549442076657273696f6e20372074696d657374616d702072616e6765";

/// `malformed or unsupported crypto envelope` from `vaeljs-crypto-core/src/envelope.rs`.
const CRYPTO_SEQUENCE_ONE_HEX: &str =
    "6d616c666f726d6564206f7220756e737570706f727465642063727970746f20656e76656c6f7065";

/// `vael:console host requires a print function and target object` from `vaeljs-builtins/src/console.js`.
const CONSOLE_SEQUENCE_ONE_HEX: &str = "7661656c3a636f6e736f6c6520686f73742072657175697265732061207072696e742066756e6374696f6e20616e6420746172676574206f626a656374";

/// `provide an absolute Windows base on the requested drive or share` from `vaeljs-builtins/src/path.js`.
const PATH_SEQUENCE_ONE_HEX: &str = "70726f7669646520616e206162736f6c7574652057696e646f77732062617365206f6e2074686520726571756573746564206472697665206f72207368617265";

/// `const CHAR_STATE = Symbol("charSliceState");` from `vaeljs-builtins/src/slices.js`.
const SLICES_SEQUENCE_ONE_HEX: &str =
    "636f6e737420434841525f5354415445203d2053796d626f6c282263686172536c696365537461746522293b";

/// `create_new and append cannot both be enabled` from `vaeljs-fs-core/src/ops.rs`.
const FS_SEQUENCE_ONE_HEX: &str =
    "6372656174655f6e657720616e6420617070656e642063616e6e6f7420626f746820626520656e61626c6564";

/// `node:child_process` from `vaeljs-node-core`.
const NODE_COMPAT_SEQUENCE_ONE_HEX: &str = "6e6f64653a6368696c645f70726f63657373";

/// `connect requires exactly one of hostKey or knownHosts; no insecure default is available`
/// from `vaeljs-ssh-core/src/error.rs`.
const SSH_SEQUENCE_ONE_HEX: &str = "636f6e6e6563742072657175697265732065786163746c79206f6e65206f6620686f73744b6579206f72206b6e6f776e486f7374733b206e6f20696e7365637572652064656661756c7420697320617661696c61626c65";

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
    #[cfg(feature = "vael-chars")]
    assert_sequences(&artifact, &chars, true, "vael-chars is enabled");
    #[cfg(not(feature = "vael-chars"))]
    assert_sequences(&artifact, &chars, false, "vael-chars is disabled");

    let strings = [STRINGS_SEQUENCE_ONE_HEX];
    #[cfg(feature = "vael-strings")]
    assert_sequences(&artifact, &strings, true, "vael-strings is enabled");
    #[cfg(not(feature = "vael-strings"))]
    assert_sequences(&artifact, &strings, false, "vael-strings is disabled");

    let assert_seqs = [ASSERT_SEQUENCE_ONE_HEX];
    #[cfg(feature = "vael-assert")]
    assert_sequences(&artifact, &assert_seqs, true, "vael-assert is enabled");
    #[cfg(not(feature = "vael-assert"))]
    assert_sequences(&artifact, &assert_seqs, false, "vael-assert is disabled");

    let test_seqs = [TEST_SEQUENCE_ONE_HEX];
    #[cfg(feature = "vael-test")]
    assert_sequences(&artifact, &test_seqs, true, "vael-test is enabled");
    #[cfg(not(feature = "vael-test"))]
    assert_sequences(&artifact, &test_seqs, false, "vael-test is disabled");

    let uuid_seqs = [UUID_SEQUENCE_ONE_HEX];
    #[cfg(feature = "vael-uuid")]
    assert_sequences(&artifact, &uuid_seqs, true, "vael-uuid is enabled");
    #[cfg(not(feature = "vael-uuid"))]
    assert_sequences(&artifact, &uuid_seqs, false, "vael-uuid is disabled");

    let crypto_seqs = [CRYPTO_SEQUENCE_ONE_HEX];
    #[cfg(any(feature = "vael-crypto", feature = "node-compat"))]
    assert_sequences(
        &artifact,
        &crypto_seqs,
        true,
        "vael-crypto or node-compat is enabled",
    );
    #[cfg(not(any(feature = "vael-crypto", feature = "node-compat")))]
    assert_sequences(
        &artifact,
        &crypto_seqs,
        false,
        "vael-crypto and node-compat are disabled",
    );

    let fs_seqs = [FS_SEQUENCE_ONE_HEX];
    #[cfg(any(feature = "vael-fs", feature = "node-compat"))]
    assert_sequences(
        &artifact,
        &fs_seqs,
        true,
        "vael-fs or node-compat is enabled",
    );
    #[cfg(not(any(feature = "vael-fs", feature = "node-compat")))]
    assert_sequences(
        &artifact,
        &fs_seqs,
        false,
        "vael-fs and node-compat are disabled",
    );

    let node_compat_seqs = [NODE_COMPAT_SEQUENCE_ONE_HEX];
    #[cfg(feature = "node-compat")]
    assert_sequences(&artifact, &node_compat_seqs, true, "node-compat is enabled");
    #[cfg(not(feature = "node-compat"))]
    assert_sequences(
        &artifact,
        &node_compat_seqs,
        false,
        "node-compat is disabled",
    );

    let ssh_seqs = [SSH_SEQUENCE_ONE_HEX];
    #[cfg(feature = "vael-ssh")]
    assert_sequences(&artifact, &ssh_seqs, true, "vael-ssh is enabled");
    #[cfg(not(feature = "vael-ssh"))]
    assert_sequences(&artifact, &ssh_seqs, false, "vael-ssh is disabled");

    let console_seqs = [CONSOLE_SEQUENCE_ONE_HEX];
    #[cfg(feature = "vael-console")]
    assert_sequences(&artifact, &console_seqs, true, "vael-console is enabled");
    #[cfg(not(feature = "vael-console"))]
    assert_sequences(&artifact, &console_seqs, false, "vael-console is disabled");

    let path_seqs = [PATH_SEQUENCE_ONE_HEX];
    #[cfg(feature = "vael-path")]
    assert_sequences(&artifact, &path_seqs, true, "vael-path is enabled");
    #[cfg(not(feature = "vael-path"))]
    assert_sequences(&artifact, &path_seqs, false, "vael-path is disabled");

    let slices_seqs = [SLICES_SEQUENCE_ONE_HEX];
    #[cfg(feature = "vael-slices")]
    assert_sequences(&artifact, &slices_seqs, true, "vael-slices is enabled");
    #[cfg(not(feature = "vael-slices"))]
    assert_sequences(&artifact, &slices_seqs, false, "vael-slices is disabled");
}

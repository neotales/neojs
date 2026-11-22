//! Deterministic test-support injection for the Shamir noise draw.
//!
//! The hook is compiled only with `neojs-core`'s off-by-default
//! `test-support` feature, which `neojs` enables for its dev tests. A normal
//! build has no injection point. The mutex serialises the two tests so the
//! single-shot flag cannot be consumed by the other test in this binary.

use std::fs;
use std::sync::Mutex;

use neojs_core::Runtime;

static LOCK: Mutex<()> = Mutex::new(());

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn synchronous_shamir_split_preserves_an_injected_random_failure() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    neojs_core::test_support::fail_next_shamir_random();
    run_js(
        r#"
import { shamirCombine, shamirSplit } from "neo:crypto";

let code = "none";
try {
    shamirSplit(Uint8Array.of(1, 2, 3), 2, 3);
} catch (error) {
    code = error.code;
}
if (code !== "NEO_CRYPTO_RANDOM") throw new Error("unexpected code: " + code);

// The single-shot injection is consumed, so a real draw now succeeds.
const shares = shamirSplit(Uint8Array.of(1, 2, 3), 2, 3);
const recovered = shamirCombine([shares[0], shares[2]]);
if (recovered.length !== 3 || recovered[0] !== 1 || recovered[1] !== 2 || recovered[2] !== 3) {
    throw new Error("split after injected failure");
}
"#,
    );
}

#[test]
fn asynchronous_shamir_split_preserves_an_injected_random_failure() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    neojs_core::test_support::fail_next_shamir_random();
    run_js(
        r#"
import { shamirCombineAsync, shamirSplitAsync } from "neo:crypto";

let code = "none";
try {
    await shamirSplitAsync(Uint8Array.of(1, 2, 3), 2, 3);
} catch (error) {
    code = error.code;
}
if (code !== "NEO_CRYPTO_RANDOM") throw new Error("unexpected code: " + code);

// The single-shot injection is consumed, so a real draw now succeeds on the
// worker.
const shares = await shamirSplitAsync(Uint8Array.of(1, 2, 3), 2, 3);
const recovered = await shamirCombineAsync([shares[0], shares[2]]);
if (recovered.length !== 3 || recovered[0] !== 1 || recovered[1] !== 2 || recovered[2] !== 3) {
    throw new Error("split after injected failure");
}
"#,
    );
}

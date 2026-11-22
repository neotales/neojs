//! Deterministic test-support injection for the UUID entropy draw.
//!
//! The hook is compiled only with `neojs-core`'s off-by-default `test-support`
//! feature, which `neojs` enables for its dev tests. A normal build has no
//! injection point. The mutex serialises the tests so the single-shot flag
//! cannot be consumed by the other test in this binary.

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
fn v4_preserves_an_injected_entropy_failure() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    neojs_core::test_support::fail_next_uuid_random();
    run_js(
        r#"
import { v4 } from "neo:uuid";

let code = "none";
let cause = "none";
let causeEnumerable = null;
try {
    v4();
} catch (error) {
    code = error.code;
    cause = error.cause;
    const descriptor = Object.getOwnPropertyDescriptor(error, "cause");
    causeEnumerable = descriptor === undefined ? null : descriptor.enumerable;
}
if (code !== "NEO_UUID_RANDOM") throw new Error("unexpected code: " + code);
// The retained getrandom error is converted to a fixed diagnostic, never its
// Display text, entropy bytes, or a UUID.
if (cause !== "getrandom: unexpected error") throw new Error("unexpected cause: " + cause);
if (causeEnumerable !== false) throw new Error("cause must be non-enumerable");

// The single-shot injection is consumed, so a real draw now succeeds.
const value = v4();
if (!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)) {
    throw new Error("v4 after injected failure: " + value);
}
"#,
    );
}

#[test]
fn v7_preserves_an_injected_entropy_failure() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    neojs_core::test_support::fail_next_uuid_random();
    run_js(
        r#"
import { v7 } from "neo:uuid";

let code = "none";
let cause = "none";
let causeEnumerable = null;
try {
    v7();
} catch (error) {
    code = error.code;
    cause = error.cause;
    const descriptor = Object.getOwnPropertyDescriptor(error, "cause");
    causeEnumerable = descriptor === undefined ? null : descriptor.enumerable;
}
if (code !== "NEO_UUID_RANDOM") throw new Error("unexpected code: " + code);
// The retained getrandom error is converted to a fixed diagnostic, never its
// Display text, entropy bytes, or a UUID.
if (cause !== "getrandom: unexpected error") throw new Error("unexpected cause: " + cause);
if (causeEnumerable !== false) throw new Error("cause must be non-enumerable");

// The single-shot injection is consumed, so a real draw now succeeds.
const value = v7();
if (!/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value)) {
    throw new Error("v7 after injected failure: " + value);
}
"#,
    );
}

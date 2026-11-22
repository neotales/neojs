//! Deterministic test-support injection for the bcrypt salt draw.
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
fn synchronous_bcrypt_hash_preserves_an_injected_random_failure() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    neojs_core::test_support::fail_next_bcrypt_random();
    run_js(
        r#"
import { bcryptHash } from "neo:crypto";

let code = "none";
try {
    bcryptHash("password", 4);
} catch (error) {
    code = error.code;
}
if (code !== "NEO_CRYPTO_RANDOM") throw new Error("unexpected code: " + code);

// The single-shot injection is consumed, so a real draw now succeeds.
const hash = bcryptHash("password", 4);
if (!hash.startsWith("$2b$04$") || hash.length !== 60) {
    throw new Error("hash after injected failure: " + hash);
}
"#,
    );
}

#[test]
fn asynchronous_bcrypt_hash_preserves_an_injected_random_failure() {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    neojs_core::test_support::fail_next_bcrypt_random();
    run_js(
        r#"
import { bcryptHashAsync } from "neo:crypto";

let code = "none";
try {
    await bcryptHashAsync("password", 4);
} catch (error) {
    code = error.code;
}
if (code !== "NEO_CRYPTO_RANDOM") throw new Error("unexpected code: " + code);

// The single-shot injection is consumed, so a real draw now succeeds on the
// worker.
const hash = await bcryptHashAsync("password", 4);
if (!hash.startsWith("$2b$04$") || hash.length !== 60) {
    throw new Error("hash after injected failure: " + hash);
}
"#,
    );
}

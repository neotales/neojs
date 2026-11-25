//! Deterministic worker-placement tests for asynchronous PBKDF2 and bcrypt.
//!
//! `vaeljs`'s dev-dependencies enable `vaeljs-core`'s off-by-default
//! `test-support` feature, which registers a probe reporting three placements:
//! the worker closure, PBKDF2 derivation, and bcrypt hashing/verification. Each
//! value is 1 on the isolate thread and 2 on another thread. A normal build
//! does not compile the probe.

use std::fs;

use vaeljs_core::Runtime;

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn async_pbkdf2_and_bcrypt_hash_run_on_a_blocking_worker() {
    run_js(
        r#"
import { bcryptHashAsync, pbkdf2Async } from "vael:crypto";

await pbkdf2Async("sha256", "passwd", "salt", 1, 16);
let placement = Deno.core.ops.op_vael_crypto_test_password_placement();
if (placement !== "2,2,0") throw new Error("pbkdf2 placement " + placement);

await bcryptHashAsync("password", 4);
placement = Deno.core.ops.op_vael_crypto_test_password_placement();
if (placement !== "2,2,2") throw new Error("bcrypt hash placement " + placement);
"#,
    );
}

#[test]
fn async_bcrypt_verify_runs_on_a_blocking_worker() {
    run_js(
        r#"
import { bcryptVerifyAsync } from "vael:crypto";

const hash = "$2b$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga";
if (!(await bcryptVerifyAsync("allmine", hash))) throw new Error("verify false");
const placement = Deno.core.ops.op_vael_crypto_test_password_placement();
if (placement !== "2,0,2") throw new Error("bcrypt verify placement " + placement);
"#,
    );
}

#[test]
fn malformed_bcrypt_hash_resolves_false_without_a_worker() {
    run_js(
        r#"
import { bcryptVerifyAsync } from "vael:crypto";

if (await bcryptVerifyAsync("allmine", "not a bcrypt hash")) {
    throw new Error("malformed hash verified");
}
// A syntactically valid 60-character over-cost hash is also rejected before a
// worker starts; the cost cap is checked without computing bcrypt.
if (await bcryptVerifyAsync("allmine",
    "$2b$31$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga")) {
    throw new Error("over-cost hash verified");
}
const placement = Deno.core.ops.op_vael_crypto_test_password_placement();
if (placement !== "0,0,0") throw new Error("unexpected worker placement " + placement);
"#,
    );
}

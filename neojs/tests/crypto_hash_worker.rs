//! Deterministic worker-placement test for asynchronous `crypto` hashing.
//!
//! `neojs`'s dev-dependencies enable `neojs-core`'s off-by-default
//! `test-support` feature, which registers a probe reporting three placements:
//! the worker closure, `DigestState::update`, and `HmacState::new` (key
//! construction). Each value is 1 on the isolate thread and 2 on another
//! thread. A normal build does not compile the probe.

use std::fs;

use neojs_core::Runtime;

#[test]
fn async_hash_runs_on_a_blocking_worker_not_the_isolate_thread() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { hmacAsync, hashAsync } from "neo:crypto";

await hashAsync("sha256", "abc", "hex");
let placement = Deno.core.ops.op_neo_crypto_test_worker_placement();
if (placement !== "2,2,0") throw new Error("hash placement " + placement);

await hmacAsync("sha512", "Jefe", "what do ya want for nothing?", "hex");
placement = Deno.core.ops.op_neo_crypto_test_worker_placement();
if (placement !== "2,2,2") throw new Error("hmac placement " + placement);
"#,
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

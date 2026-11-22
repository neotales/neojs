//! Deterministic worker-placement test for asynchronous Shamir secret sharing.
//!
//! `neojs`'s dev-dependencies enable `neojs-core`'s off-by-default
//! `test-support` feature, which reports where the worker closure and the
//! Shamir split/combine core ran. Each value is 1 on the isolate thread and 2
//! on another thread. A normal build does not compile the probe.

use std::fs;

use neojs_core::Runtime;

#[test]
fn async_shamir_runs_on_a_blocking_worker_not_the_isolate_thread() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { shamirCombineAsync, shamirSplitAsync } from "neo:crypto";

const secret = Uint8Array.of(1, 2, 3, 4);
const shares = await shamirSplitAsync(secret, 3, 5);
let placement = Deno.core.ops.op_neo_crypto_test_shamir_placement();
if (placement !== "2,2") throw new Error("split placement " + placement);

const recovered = await shamirCombineAsync([shares[0], shares[2], shares[4]], 3);
placement = Deno.core.ops.op_neo_crypto_test_shamir_placement();
if (placement !== "2,2") throw new Error("combine placement " + placement);
if (recovered.length !== 4) throw new Error("combine length");
for (let index = 0; index < recovered.length; index += 1) {
    if (recovered[index] !== secret[index]) throw new Error("combine byte " + index);
}
"#,
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

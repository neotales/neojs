//! Test-support check that V8-owned Shamir share and secret buffers are
//! released through the zeroizing backing-store deleter.
//!
//! The counter is compiled only with `neojs-core`'s off-by-default
//! `test-support` feature, which `neojs` enables for its dev tests. The test
//! runs in its own test binary, so the process-global counter has no competing
//! writer here.

use std::fs;

use neojs_core::Runtime;

#[test]
fn dropped_v8_shamir_buffers_are_zeroized_on_runtime_teardown() {
    let baseline = neojs_core::test_support::zeroized_shamir_buffers();
    let results_baseline = neojs_core::test_support::dropped_shamir_worker_results();
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { shamirCombine, shamirCombineAsync, shamirSplit, shamirSplitAsync } from "neo:crypto";

const secret = Uint8Array.of(1, 2, 3, 4);
const shares = shamirSplit(secret, 2, 3);
shamirCombine([shares[0], shares[2]]);

const asyncShares = await shamirSplitAsync(secret, 2, 3);
await shamirCombineAsync([asyncShares[0], asyncShares[2]]);

// A finished async result that JavaScript does not consume is still retained by
// its promise until the runtime is dropped.
const abandoned = shamirSplitAsync(Uint8Array.of(9, 9), 2, 3);
abandoned.catch(() => {});
"#,
    )
    .expect("write source");

    {
        let mut runtime = Runtime::new().expect("runtime");
        runtime.run_file(&file).expect("module assertions");
    }

    let after = neojs_core::test_support::zeroized_shamir_buffers();
    let results_after = neojs_core::test_support::dropped_shamir_worker_results();
    assert!(
        after > baseline,
        "expected dropped Shamir V8 buffers to be cleared: {baseline} -> {after}"
    );
    // Sync split (3), sync combine (1), async split (3), async combine (1), and
    // the abandoned async split (3) all use zeroizing backing stores.
    assert!(
        after >= baseline + 8,
        "too few Shamir V8 buffers were cleared: {baseline} -> {after}"
    );
    assert!(
        results_after >= results_baseline + 2,
        "too few Shamir worker results were dropped: {results_baseline} -> {results_after}"
    );
}

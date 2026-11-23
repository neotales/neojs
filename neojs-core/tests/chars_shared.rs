//! The cross-engine `neo:chars` contract on the V8 runtime.
//!
//! The fixture is shared verbatim with `neojs-lite-core/tests/chars.rs`. The
//! adapter tokens differ because this branch base still uses the bare registry
//! key and bare import spelling for every V8 built-in. The canonical-only
//! registry change that is in flight will change these two tokens to
//! `neo:chars`; the shared implementation and contract stay the same.
#![cfg(feature = "neo-chars")]

use std::fs;

use neojs_core::Runtime;

/// The same fixture the QuickJS runtime runs; see the file header for the tokens.
const CONTRACT: &str = include_str!("../../neojs-builtins/tests/fixtures/chars_contract.js");

#[test]
fn shared_chars_contract_holds_on_v8() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        CONTRACT
            .replace("__CHARS_SPECIFIER__", "chars")
            .replace("__CHARS_REGISTRY_KEY__", "chars"),
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("shared chars contract");
}

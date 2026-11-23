//! The cross-engine `neo:chars` contract on the V8 runtime.
//!
//! The fixture is shared verbatim with `neojs-lite-core/tests/chars.rs`. Both
//! engines use the canonical `neo:chars` specifier as the import spelling and
//! the registry key, so the two adapter tokens are the same value.
#![cfg(feature = "neo-chars")]

use std::fs;

use neojs_core::Runtime;

/// The same fixture the QuickJS runtime runs; see the file header for the tokens.
const CONTRACT: &str = include_str!("../../neojs-builtins/tests/fixtures/chars_contract.js");

#[test]
fn native_chars_exports_are_native_v8_functions() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import * as chars from "neo:chars";
for (const name of Object.keys(chars)) {
    const text = Function.prototype.toString.call(chars[name]);
    if (!text.includes("[native code]")) {
        throw new Error(`${name} is not a native function: ${text}`);
    }
}
"#,
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("native chars exports");
}

#[test]
fn shared_chars_contract_holds_on_v8() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        CONTRACT
            .replace("__CHARS_SPECIFIER__", "neo:chars")
            .replace("__CHARS_REGISTRY_KEY__", "neo:chars"),
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("shared chars contract");
}

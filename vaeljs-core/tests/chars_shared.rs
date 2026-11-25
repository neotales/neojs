//! The cross-engine `vael:chars` contract on the V8 runtime.
//!
//! The fixture is shared verbatim with `vaeljs-lite-core/tests/chars.rs`. Both
//! engines use the canonical `vael:chars` specifier as the import spelling and
//! the registry key, so the two adapter tokens are the same value.
#![cfg(feature = "vael-chars")]

use std::fs;

use vaeljs_core::Runtime;

/// The same fixture the QuickJS runtime runs; see the file header for the tokens.
const CONTRACT: &str = include_str!("../../vaeljs-builtins/tests/fixtures/chars_contract.js");

#[test]
fn native_chars_exports_are_native_v8_functions() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import * as chars from "vael:chars";
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
            .replace("__CHARS_SPECIFIER__", "vael:chars")
            .replace("__CHARS_REGISTRY_KEY__", "vael:chars"),
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("shared chars contract");
}

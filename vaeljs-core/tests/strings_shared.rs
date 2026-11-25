//! The cross-engine `vael:strings` contract on the V8 runtime.
//!
//! The fixture is shared verbatim with `vaeljs-lite-core/tests/strings.rs`. Both
//! engines use the canonical `vael:strings` specifier as the import spelling and
//! the registry key, so the two adapter tokens are the same value.
#![cfg(feature = "vael-strings")]

use std::fs;

use vaeljs_core::Runtime;

/// The same fixture the QuickJS runtime runs; see the file header for the tokens.
const CONTRACT: &str = include_str!("../../vaeljs-builtins/tests/fixtures/strings_contract.js");

#[test]
fn native_strings_exports_are_native_v8_functions() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import * as strings from "vael:strings";
for (const name of Object.keys(strings)) {
    const text = Function.prototype.toString.call(strings[name]);
    if (!text.includes("[native code]")) {
        throw new Error(`${name} is not a native function: ${text}`);
    }
}
"#,
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("native strings exports");
}

#[test]
fn shared_strings_contract_holds_on_v8() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        CONTRACT
            .replace("__STRINGS_SPECIFIER__", "vael:strings")
            .replace("__STRINGS_REGISTRY_KEY__", "vael:strings"),
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("shared strings contract");
}

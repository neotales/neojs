//! Cross-engine `vael:strings` contract and QuickJS-specific adapter rules.
//!
//! The contract fixture is shared verbatim with `vaeljs-core/tests/strings_shared.rs`.
//! Only the two adapter tokens differ, so a behavior change in the shared Rust
//! implementation has to satisfy both engines. An additional test proves the
//! exports are native QuickJS functions rather than JavaScript wrappers.
#![cfg(feature = "vael-strings")]

use std::fs;

use vaeljs_lite_core::{Error, Runtime};

/// The same fixture the V8 runtime runs; see the file header for the tokens.
const CONTRACT: &str = include_str!("../../vaeljs-builtins/tests/fixtures/strings_contract.js");

fn write(dir: &tempfile::TempDir, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create module directory");
    }
    fs::write(&path, source).expect("write source");
    path
}

fn contract_source() -> String {
    CONTRACT
        .replace("__STRINGS_SPECIFIER__", "vael:strings")
        .replace("__STRINGS_REGISTRY_KEY__", "vael:strings")
}

#[test]
fn native_strings_exports_are_native_quickjs_functions() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(
        &dir,
        "main.js",
        r#"
import * as strings from "vael:strings";
for (const name of Object.keys(strings)) {
    const text = Function.prototype.toString.call(strings[name]);
    if (!text.includes("[native code]")) {
        throw new Error(`${name} is not a native function: ${text}`);
    }
}
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("native strings exports");
}

#[test]
fn shared_strings_contract_holds_on_quickjs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "main.js", &contract_source());

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("shared strings contract");
}

#[test]
fn only_the_canonical_specifier_is_a_builtin() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "main.js", "import * as strings from \"strings\";\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("bare strings must not load");
    let Error::Module { module, message } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(module.ends_with("main.js"), "{module}");
    assert!(message.contains("bare module specifiers"), "{message}");
}

#[test]
fn canonical_builtin_rejects_import_attributes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(
        &dir,
        "main.js",
        "import * as strings from \"vael:strings\" with { type: \"json\" };\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("builtin attributes must not be ignored");
    let Error::Module { message, .. } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(message.contains("import attributes"), "{message}");
}

#[test]
fn a_query_or_fragment_does_not_alias_the_builtin() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(
        &dir,
        "main.js",
        "import * as strings from \"vael:strings?x\";\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("a query must not resolve the builtin");
    let Error::Module { message, .. } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(
        message.contains("vael:")
            && (message.contains("not implemented")
                || message.contains("cannot resolve")
                || message.contains("only file:")),
        "{message}"
    );
}

#[test]
fn a_replaced_slice_intrinsic_does_not_change_the_search() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
import { lastIndexOfFold } from "vael:strings";
let calls = 0;
String.prototype.slice = () => {
    calls += 1;
    throw new Error("the replaced slice was called");
};
const index = lastIndexOfFold("abcabc", "BC", 6);
if (index !== 4) throw new Error(`unexpected index: ${index}`);
if (calls !== 0) throw new Error(`the replaced slice ran ${calls} times`);
"#,
        )
        .expect("captured slice");
}

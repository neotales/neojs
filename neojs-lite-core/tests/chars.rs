//! Cross-engine `neo:chars` contract and QuickJS-specific adapter rules.
//!
//! The contract fixture is shared verbatim with `neojs-core/tests/chars_shared.rs`.
//! Only the two adapter tokens differ, so a behavior change in the shared
//! implementation has to satisfy both engines.

use std::fs;

use neojs_lite_core::{Error, Runtime};

/// The same fixture the V8 runtime runs; see the file header for the tokens.
const CONTRACT: &str = include_str!("../../neojs-builtins/tests/fixtures/chars_contract.js");

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
        .replace("__CHARS_SPECIFIER__", "neo:chars")
        .replace("__CHARS_REGISTRY_KEY__", "neo:chars")
}

#[test]
fn shared_chars_contract_holds_on_quickjs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "main.js", &contract_source());

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("shared chars contract");
}

#[test]
fn only_the_canonical_specifier_is_a_builtin() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "main.js", "import * as chars from \"chars\";\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("bare chars must not load");
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
        "import * as chars from \"neo:chars\" with { type: \"json\" };\n",
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
    let file = write(&dir, "main.js", "import * as chars from \"neo:chars?x\";\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("a query must not resolve the builtin");
    let Error::Module { message, .. } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(
        message.contains("neo:")
            && (message.contains("not implemented")
                || message.contains("cannot resolve")
                || message.contains("only file:")),
        "{message}"
    );
}

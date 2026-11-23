//! Tests for a build where the optional `neo:crypto` module is not enabled.
//!
//! These compile only when the `crypto` feature is off, so a run with the
//! feature enabled cannot pass this file vacuously.

#![cfg(not(feature = "crypto"))]

use std::fs;

use neojs_lite_core::Runtime;

fn run_js_error(source: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&file)
        .expect_err("module must fail")
        .to_string()
}

#[test]
fn disabled_build_lists_no_builtin_modules() {
    assert!(neojs_lite_core::enabled_builtin_modules().is_empty());
}

#[test]
fn disabled_crypto_explains_the_feature_instead_of_resolving() {
    for source in [
        "import \"neo:crypto\";\n",
        "import \"crypto\";\n",
        "import \"NEO:crypto\";\n",
    ] {
        let message = run_js_error(source);
        assert!(
            message.contains("not enabled") && message.contains("crypto"),
            "{message}"
        );
    }
}

#[test]
fn an_unknown_specifier_is_still_a_normal_load_failure() {
    let message = run_js_error("import \"lodash\";\n");
    assert!(message.contains("bare module specifiers"), "{message}");
}

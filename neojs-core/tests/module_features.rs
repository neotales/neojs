//! Compile-time module selection, driven through the real runtime.
//!
//! The `neo-strings` feature is the first per-module switch. These tests run
//! under whichever feature set Cargo selected, so the same file proves both
//! sides of the contract in separate invocations:
//!
//! ```text
//! cargo test -p neojs-core                                  # default: enabled
//! cargo test -p neojs-core --no-default-features            # disabled
//! cargo test -p neojs-core --no-default-features \
//!     --features neo-strings                                # explicitly enabled
//! ```
//!
//! A disabled module must be absent from the registry, the loader, and the
//! error list. It is surface selection, not a security boundary; the tests do
//! not claim otherwise.
//!
//! Imports here use the canonical `neo:strings` specifier only. The branch base
//! still accepts bare Neo names as a legacy loader alias; these tests do not
//! assert, bless, or extend that alias as future contract. `Neo.getBuiltInModule`
//! keeps its bare registry key because it is a registry lookup, not an import.

use std::fs;

use neojs_core::Runtime;

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[cfg(not(feature = "neo-strings"))]
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

/// The built-in list embedded in an unsupported-specifier message.
#[cfg(not(feature = "neo-strings"))]
fn advertised_builtins(message: &str) -> &str {
    message
        .split_once("built-in modules: ")
        .or_else(|| message.split_once("neojs implements "))
        .expect("the error names the implemented built-ins")
        .1
}

#[cfg(not(feature = "neo-strings"))]
fn advertises_strings(list: &str) -> bool {
    list.split(", ").any(|specifier| specifier == "neo:strings")
}

#[cfg(feature = "neo-strings")]
#[test]
fn the_default_build_advertises_and_loads_strings_by_canonical_specifier() {
    run_js(
        r#"
import * as strings from "neo:strings";

// `getBuiltInModule` takes the registry's bare name; it is not an import path.
const registry = Neo.getBuiltInModule("strings");
if (registry === undefined) throw new Error("strings is missing from the registry");
if (strings.equalFold !== registry.equalFold) {
    throw new Error("the module and registry do not share identity");
}
if (!strings.equalFold("K", "\u212A")) throw new Error("strings did not run");
if (import.meta.resolve("neo:strings") !== "neo:strings") {
    throw new Error("the canonical strings specifier did not resolve to itself");
}
"#,
    );
}

#[cfg(not(feature = "neo-strings"))]
#[test]
fn a_disabled_strings_module_is_absent_from_the_registry_and_loader() {
    // The public `Neo` object still exists; the disabled name is simply not
    // registered. A module is not present as a stub that throws later.
    run_js(
        r#"
if (typeof Neo !== "object" || Neo === null) throw new Error("Neo is missing");
if (Neo.getBuiltInModule("strings") !== undefined) {
    throw new Error("the disabled strings module is in the registry");
}
"#,
    );

    let canonical = run_js_error("import * as strings from \"neo:strings\";\n");
    assert!(canonical.contains("not implemented"), "{canonical}");
    assert!(
        !advertises_strings(advertised_builtins(&canonical)),
        "the disabled canonical specifier is still advertised: {canonical}"
    );
}

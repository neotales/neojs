//! Feature-surface tests for the shared `chars` and dependent `slices` modules.
//!
//! The full V8 suite runs with the default features. These tests are intended
//! for the feature matrix: `--no-default-features`, `--no-default-features
//! --features neo-chars`, and the default set. A disabled module must disappear
//! from the loader, the registry, and the introspection list, and its source
//! must not remain in the evaluated shim.

use std::fs;

use neojs_core::Runtime;

fn run(source: &str) -> Result<(), neojs_core::Error> {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file)
}

#[test]
#[cfg(feature = "neo-chars")]
fn chars_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("chars") || !list.includes("neo:chars")) {
    throw new Error(`chars is missing from the list: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("chars");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the chars registry object is missing");
}
const namespace = await import("chars");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.isLetter !== registry.isLetter) {
    throw new Error("the namespace export is not the registry function");
}
if (namespace.isLetter(0x41) !== true) throw new Error("chars did not classify A");
"#)
    .expect("chars feature");
}

#[test]
#[cfg(not(feature = "neo-chars"))]
fn chars_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("chars") || list.includes("neo:chars")) {
    throw new Error(`chars is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("chars") !== undefined) {
    throw new Error("the chars registry object is still present");
}
if (Neo.getBuiltInModule("neo:chars") !== undefined) {
    throw new Error("the canonical chars registry object is still present");
}
let imported = false;
try {
    await import("chars");
    imported = true;
} catch {
    // Expected: the loader has no chars entry.
}
if (imported) throw new Error("chars was importable while disabled");
"#)
    .expect("chars absent");
}

#[test]
#[cfg(feature = "neo-slices")]
fn slices_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("slices") || !list.includes("neo:slices")) {
    throw new Error(`slices is missing from the list: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("slices");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the slices registry object is missing");
}
const namespace = await import("slices");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.Slice !== registry.Slice) {
    throw new Error("the namespace class is not the registry class");
}
const view = new namespace.Slice([1, 2, 3], 1, 3);
if (view.length !== 2 || view.at(0) !== 2) throw new Error("Slice did not alias the target");
if (namespace.CharSlice !== registry.CharSlice) {
    throw new Error("the chars-dependent CharSlice is not the registry class");
}
"#)
    .expect("slices feature");
}

#[test]
#[cfg(not(feature = "neo-slices"))]
fn slices_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("slices") || list.includes("neo:slices")) {
    throw new Error(`slices is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("slices") !== undefined) {
    throw new Error("the slices registry object is still present");
}
if (Neo.getBuiltInModule("neo:slices") !== undefined) {
    throw new Error("the canonical slices registry object is still present");
}
let imported = false;
try {
    await import("slices");
    imported = true;
} catch {
    // Expected: the loader has no slices entry.
}
if (imported) throw new Error("slices was importable while disabled");
"#)
    .expect("slices absent");
}

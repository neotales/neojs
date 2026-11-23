//! Feature-surface tests for the QuickJS `neo:chars` module.
//!
//! The default feature set includes `neo-chars`. The `--no-default-features`
//! matrix run must show an empty registry/list and an unimportable specifier,
//! and must not compile the shared dependency.

use neojs_lite_core::Runtime;

fn run(source: &str) -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_source("memory.js", source)
}

#[test]
#[cfg(feature = "neo-chars")]
fn chars_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (JSON.stringify(list) !== JSON.stringify(["neo:chars"])) {
    throw new Error(`unexpected list: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:chars");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the chars registry object is missing");
}
const namespace = await import("neo:chars");
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
if (!Array.isArray(list) || list.length !== 0) {
    throw new Error(`the module list is not empty: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:chars") !== undefined) {
    throw new Error("the chars registry object is still present");
}
if (Neo.getBuiltInModule("chars") !== undefined) {
    throw new Error("a bare chars name returned a module");
}
let imported = false;
try {
    await import("neo:chars");
    imported = true;
} catch {
    // Expected: the loader has no chars entry.
}
if (imported) throw new Error("neo:chars was importable while disabled");
"#)
    .expect("chars absent");
}

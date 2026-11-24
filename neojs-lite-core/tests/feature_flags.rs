//! Feature-surface tests for the QuickJS shared modules.
//!
//! The default feature set includes `neo-chars` and `neo-strings`. The
//! `--no-default-features` matrix run must show an empty registry/list and
//! unimportable specifiers, and must not compile the shared dependency.

use neojs_lite_core::Runtime;

fn run(source: &str) -> Result<(), neojs_lite_core::Error> {
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_source("memory.js", source)
}

/// The canonical list this build selected, in registration order.
fn expected_list() -> String {
    let mut entries = Vec::new();
    if cfg!(feature = "neo-assert") {
        entries.push(r#""neo:assert""#);
    }
    if cfg!(feature = "neo-chars") {
        entries.push(r#""neo:chars""#);
    }
    if cfg!(feature = "neo-strings") {
        entries.push(r#""neo:strings""#);
    }
    if cfg!(feature = "neo-test") {
        entries.push(r#""neo:test""#);
    }
    format!("[{}]", entries.join(", "))
}

#[test]
fn the_module_list_matches_the_selected_features() {
    let source = format!(
        r#"
const list = Neo.getBuiltInModulesList();
if (JSON.stringify(list) !== JSON.stringify({})) {{
    throw new Error(`unexpected list: ${{JSON.stringify(list)}}`);
}}
if (!Object.isFrozen(list)) throw new Error("the list is not frozen");
"#,
        expected_list()
    );
    run(&source).expect("feature list");
}

#[test]
#[cfg(feature = "neo-chars")]
fn chars_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:chars")) {
    throw new Error(`chars is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("chars")) {
    throw new Error(`a bare chars name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:chars");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the chars registry object is missing");
}
if (Neo.getBuiltInModule("chars") !== undefined) {
    throw new Error("the registry accepted a bare chars name");
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
if (list.includes("neo:chars") || list.includes("chars")) {
    throw new Error(`chars is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:chars") !== undefined) {
    throw new Error("the canonical chars registry object is still present");
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

#[test]
#[cfg(feature = "neo-strings")]
fn strings_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:strings")) {
    throw new Error(`strings is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("strings")) {
    throw new Error(`a bare strings name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:strings");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the strings registry object is missing");
}
if (Neo.getBuiltInModule("strings") !== undefined) {
    throw new Error("the registry accepted a bare strings name");
}
const namespace = await import("neo:strings");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.equalFold !== registry.equalFold) {
    throw new Error("the namespace export is not the registry function");
}
if (!namespace.equalFold("K", "\u212A")) throw new Error("strings did not fold");
"#)
    .expect("strings feature");
}

#[test]
#[cfg(not(feature = "neo-strings"))]
fn strings_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:strings") || list.includes("strings")) {
    throw new Error(`strings is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:strings") !== undefined) {
    throw new Error("the canonical strings registry object is still present");
}
if (Neo.getBuiltInModule("strings") !== undefined) {
    throw new Error("a bare strings name returned a module");
}
let imported = false;
try {
    await import("neo:strings");
    imported = true;
} catch {
    // Expected: the loader has no strings entry.
}
if (imported) throw new Error("neo:strings was importable while disabled");
"#)
    .expect("strings absent");
}

#[test]
#[cfg(feature = "neo-assert")]
fn assert_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:assert")) {
    throw new Error(`assert is missing from the list: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:assert");
if (typeof registry !== "function" || !Object.isFrozen(registry)) {
    throw new Error("the assert registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("assert") !== undefined) {
    throw new Error("the registry accepted a bare assert name");
}
const namespace = await import("neo:assert");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.default !== registry || namespace.equals !== registry.equals) {
    throw new Error("the namespace exports do not match the registry");
}
namespace.equals({ a: [1, 2] }, { a: [1, 2] });
"#)
    .expect("assert feature");
}

#[test]
#[cfg(not(feature = "neo-assert"))]
fn assert_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:assert") || list.includes("assert")) {
    throw new Error(`assert is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:assert") !== undefined) {
    throw new Error("the canonical assert registry object is still present");
}
let imported = false;
try {
    await import("neo:assert");
    imported = true;
} catch {
    // Expected: the loader has no assert entry.
}
if (imported) throw new Error("neo:assert was importable while disabled");
"#)
    .expect("assert absent");
}

#[test]
#[cfg(feature = "neo-test")]
fn test_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:test")) {
    throw new Error(`test is missing from the list: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:test");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the test registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("test") !== undefined) {
    throw new Error("the registry accepted a bare test name");
}
const namespace = await import("neo:test");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.test !== registry.test || namespace.default !== registry.test) {
    throw new Error("the namespace exports do not match the registry");
}
"#)
    .expect("test feature");
}

#[test]
#[cfg(not(feature = "neo-test"))]
fn test_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:test") || list.includes("test")) {
    throw new Error(`test is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:test") !== undefined) {
    throw new Error("the canonical test registry object is still present");
}
let imported = false;
try {
    await import("neo:test");
    imported = true;
} catch {
    // Expected: the loader has no test entry.
}
if (imported) throw new Error("neo:test was importable while disabled");
"#)
    .expect("test absent");
}

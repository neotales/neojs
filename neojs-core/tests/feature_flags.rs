//! Feature-surface tests for the shared `chars` and `strings` modules and the
//! dependent `slices` module.
//!
//! The full V8 suite runs with the default features. These tests are intended
//! for the feature matrix: `--no-default-features`, one feature at a time, and
//! the default set. A disabled module must disappear
//! from the loader, the registry, and the introspection list, and its source
//! must not remain in the evaluated shim. Only canonical `neo:` specifiers are
//! importable and registry keys.

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
#[cfg(feature = "neo-slices")]
fn slices_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:slices")) {
    throw new Error(`slices is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("slices")) {
    throw new Error(`a bare slices name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:slices");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the slices registry object is missing");
}
if (Neo.getBuiltInModule("slices") !== undefined) {
    throw new Error("the registry accepted a bare slices name");
}
const namespace = await import("neo:slices");
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
if (list.includes("neo:slices") || list.includes("slices")) {
    throw new Error(`slices is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:slices") !== undefined) {
    throw new Error("the canonical slices registry object is still present");
}
if (Neo.getBuiltInModule("slices") !== undefined) {
    throw new Error("a bare slices name returned a module");
}
let imported = false;
try {
    await import("neo:slices");
    imported = true;
} catch {
    // Expected: the loader has no slices entry.
}
if (imported) throw new Error("neo:slices was importable while disabled");
"#)
    .expect("slices absent");
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
#[cfg(feature = "neo-http")]
fn http_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:http")) {
    throw new Error(`http is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("http")) {
    throw new Error(`a bare http name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:http");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the http registry object is missing");
}
if (Neo.getBuiltInModule("http") !== undefined) {
    throw new Error("the registry accepted a bare http name");
}
const namespace = await import("neo:http");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.createRouter !== registry.createRouter || namespace.serve !== registry.serve) {
    throw new Error("the namespace export is not the registry function");
}
const mux = namespace.createRouter().get("/ping", () => namespace.json({ pong: true }));
const res = await mux.fetch("/ping");
if (res.status !== 200 || res.json().pong !== true) {
    throw new Error("neo:http mux.fetch failed");
}
"#)
    .expect("http feature");
}

#[test]
#[cfg(not(feature = "neo-http"))]
fn http_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:http") || list.includes("http")) {
    throw new Error(`http is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:http") !== undefined) {
    throw new Error("the canonical http registry object is still present");
}
if (Neo.getBuiltInModule("http") !== undefined) {
    throw new Error("a bare http name returned a module");
}
let imported = false;
try {
    await import("neo:http");
    imported = true;
} catch {
    // Expected: the loader has no http entry.
}
if (imported) throw new Error("neo:http was importable while disabled");
"#)
    .expect("http absent");
}

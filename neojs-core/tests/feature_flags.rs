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
#[cfg(feature = "neo-template")]
fn template_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:template")) {
    throw new Error(`template is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("template")) {
    throw new Error(`a bare template name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:template");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the template registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("template") !== undefined) {
    throw new Error("the registry accepted a bare template name");
}
const namespace = await import("neo:template");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.default !== registry) {
    throw new Error("the default export must mirror the template registry object");
}
if (namespace.render !== registry.render) {
    throw new Error("the namespace export is not the registry function");
}
if (namespace.render("Hello {{.name}}!", { name: "Neo" }) !== "Hello Neo!") {
    throw new Error("template did not render");
}
"#)
    .expect("template feature");
}

#[test]
#[cfg(not(feature = "neo-template"))]
fn template_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:template") || list.includes("template")) {
    throw new Error(`template is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:template") !== undefined) {
    throw new Error("the canonical template registry object is still present");
}
if (Neo.getBuiltInModule("template") !== undefined) {
    throw new Error("a bare template name returned a module");
}
let imported = false;
try {
    await import("neo:template");
    imported = true;
} catch {
    // Expected: the loader has no template entry.
}
if (imported) throw new Error("neo:template was importable while disabled");
"#)
    .expect("template absent");
}

#[test]
#[cfg(feature = "neo-db")]
fn db_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:db")) {
    throw new Error(`db is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("db")) {
    throw new Error(`a bare db name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:db");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the db registry object is missing");
}
if (Neo.getBuiltInModule("db") !== undefined) {
    throw new Error("the registry accepted a bare db name");
}
const namespace = await import("neo:db");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.openSqlite !== registry.openSqlite) {
    throw new Error("the namespace export is not the registry function");
}
"#)
    .expect("db feature");
}

#[test]
#[cfg(not(feature = "neo-db"))]
fn db_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:db") || list.includes("db")) {
    throw new Error(`db is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:db") !== undefined) {
    throw new Error("the canonical db registry object is still present");
}
if (Neo.getBuiltInModule("db") !== undefined) {
    throw new Error("a bare db name returned a module");
}
let imported = false;
try {
    await import("neo:db");
    imported = true;
} catch {
    // Expected: the loader has no db entry.
}
if (imported) throw new Error("neo:db was importable while disabled");
"#)
    .expect("db absent");
}

#[test]
#[cfg(feature = "node-compat")]
fn node_compat_is_present_when_enabled_and_stays_isolated_from_neo_registry() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.some((spec) => spec.startsWith("node:"))) {
    throw new Error(`node:* leaked into Neo.getBuiltInModulesList(): ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("node:path") !== undefined) {
    throw new Error("Neo.getBuiltInModule(\"node:path\") must return undefined");
}
const nodePath = await import("node:path");
if (nodePath.posix.join("a", "b") !== "a/b") {
    throw new Error("node:path failed");
}
let bareImported = false;
try {
    await import("path");
    bareImported = true;
} catch {
    // Expected: bare name never aliases node: or neo:.
}
if (bareImported) throw new Error("bare \"path\" was importable");
"#)
    .expect("node-compat feature enabled");
}

#[test]
#[cfg(not(feature = "node-compat"))]
fn node_compat_is_absent_when_its_feature_is_disabled() {
    run(r#"
if (Neo.getBuiltInModule("node:path") !== undefined) {
    throw new Error("Neo.getBuiltInModule(\"node:path\") returned a module");
}
for (const spec of ["node:path", "node:fs", "node:fs/promises", "node:process"]) {
    let imported = false;
    try {
        await import(spec);
        imported = true;
    } catch {
        // Expected: node-compat feature is compiled out.
    }
    if (imported) throw new Error(`${spec} was importable while disabled`);
}
"#)
    .expect("node-compat absent");
}

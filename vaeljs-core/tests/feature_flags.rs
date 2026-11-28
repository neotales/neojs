//! Feature-surface tests for the shared `chars` and `strings` modules and the
//! dependent `slices` module.
//!
//! The full V8 suite runs with the default features. These tests are intended
//! for the feature matrix: `--no-default-features`, one feature at a time, and
//! the default set. A disabled module must disappear
//! from the loader, the registry, and the introspection list, and its source
//! must not remain in the evaluated shim. Only canonical `vael:` specifiers are
//! importable and registry keys.

use std::fs;

use vaeljs_core::Runtime;

fn run(source: &str) -> Result<(), vaeljs_core::Error> {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file)
}

#[test]
#[cfg(feature = "vael-chars")]
fn chars_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:chars")) {
    throw new Error(`chars is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("chars")) {
    throw new Error(`a bare chars name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:chars");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the chars registry object is missing");
}
if (Vael.getBuiltInModule("chars") !== undefined) {
    throw new Error("the registry accepted a bare chars name");
}
const namespace = await import("vael:chars");
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
#[cfg(not(feature = "vael-chars"))]
fn chars_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:chars") || list.includes("chars")) {
    throw new Error(`chars is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:chars") !== undefined) {
    throw new Error("the canonical chars registry object is still present");
}
if (Vael.getBuiltInModule("chars") !== undefined) {
    throw new Error("a bare chars name returned a module");
}
let imported = false;
try {
    await import("vael:chars");
    imported = true;
} catch {
    // Expected: the loader has no chars entry.
}
if (imported) throw new Error("vael:chars was importable while disabled");
"#)
    .expect("chars absent");
}

#[test]
#[cfg(feature = "vael-slices")]
fn slices_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:slices")) {
    throw new Error(`slices is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("slices")) {
    throw new Error(`a bare slices name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:slices");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the slices registry object is missing");
}
if (Vael.getBuiltInModule("slices") !== undefined) {
    throw new Error("the registry accepted a bare slices name");
}
const namespace = await import("vael:slices");
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
#[cfg(not(feature = "vael-slices"))]
fn slices_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:slices") || list.includes("slices")) {
    throw new Error(`slices is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:slices") !== undefined) {
    throw new Error("the canonical slices registry object is still present");
}
if (Vael.getBuiltInModule("slices") !== undefined) {
    throw new Error("a bare slices name returned a module");
}
let imported = false;
try {
    await import("vael:slices");
    imported = true;
} catch {
    // Expected: the loader has no slices entry.
}
if (imported) throw new Error("vael:slices was importable while disabled");
"#)
    .expect("slices absent");
}

#[test]
#[cfg(feature = "vael-strings")]
fn strings_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:strings")) {
    throw new Error(`strings is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("strings")) {
    throw new Error(`a bare strings name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:strings");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the strings registry object is missing");
}
if (Vael.getBuiltInModule("strings") !== undefined) {
    throw new Error("the registry accepted a bare strings name");
}
const namespace = await import("vael:strings");
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
#[cfg(not(feature = "vael-strings"))]
fn strings_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:strings") || list.includes("strings")) {
    throw new Error(`strings is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:strings") !== undefined) {
    throw new Error("the canonical strings registry object is still present");
}
if (Vael.getBuiltInModule("strings") !== undefined) {
    throw new Error("a bare strings name returned a module");
}
let imported = false;
try {
    await import("vael:strings");
    imported = true;
} catch {
    // Expected: the loader has no strings entry.
}
if (imported) throw new Error("vael:strings was importable while disabled");
"#)
    .expect("strings absent");
}

#[test]
#[cfg(feature = "vael-template")]
fn template_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:template")) {
    throw new Error(`template is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("template")) {
    throw new Error(`a bare template name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:template");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the template registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("template") !== undefined) {
    throw new Error("the registry accepted a bare template name");
}
const namespace = await import("vael:template");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.default !== registry) {
    throw new Error("the default export must mirror the template registry object");
}
if (namespace.render !== registry.render) {
    throw new Error("the namespace export is not the registry function");
}
if (namespace.render("Hello {{.name}}!", { name: "Vael" }) !== "Hello Vael!") {
    throw new Error("template did not render");
}
"#)
    .expect("template feature");
}

#[test]
#[cfg(not(feature = "vael-template"))]
fn template_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:template") || list.includes("template")) {
    throw new Error(`template is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:template") !== undefined) {
    throw new Error("the canonical template registry object is still present");
}
if (Vael.getBuiltInModule("template") !== undefined) {
    throw new Error("a bare template name returned a module");
}
let imported = false;
try {
    await import("vael:template");
    imported = true;
} catch {
    // Expected: the loader has no template entry.
}
if (imported) throw new Error("vael:template was importable while disabled");
"#)
    .expect("template absent");
}

#[test]
#[cfg(feature = "vael-db")]
fn db_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:db")) {
    throw new Error(`db is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("db")) {
    throw new Error(`a bare db name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:db");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the db registry object is missing");
}
if (Vael.getBuiltInModule("db") !== undefined) {
    throw new Error("the registry accepted a bare db name");
}
const namespace = await import("vael:db");
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
#[cfg(not(feature = "vael-db"))]
fn db_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:db") || list.includes("db")) {
    throw new Error(`db is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:db") !== undefined) {
    throw new Error("the canonical db registry object is still present");
}
if (Vael.getBuiltInModule("db") !== undefined) {
    throw new Error("a bare db name returned a module");
}
let imported = false;
try {
    await import("vael:db");
    imported = true;
} catch {
    // Expected: the loader has no db entry.
}
if (imported) throw new Error("vael:db was importable while disabled");
"#)
    .expect("db absent");
}

#[test]
#[cfg(feature = "node-compat")]
fn node_compat_is_present_when_enabled_and_stays_isolated_from_vael_registry() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.some((spec) => spec.startsWith("node:"))) {
    throw new Error(`node:* leaked into Vael.getBuiltInModulesList(): ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("node:path") !== undefined) {
    throw new Error("Vael.getBuiltInModule(\"node:path\") must return undefined");
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
    // Expected: bare name never aliases node: or vael:.
}
if (bareImported) throw new Error("bare \"path\" was importable");
"#)
    .expect("node-compat feature enabled");
}

#[test]
#[cfg(not(feature = "node-compat"))]
fn node_compat_is_absent_when_its_feature_is_disabled() {
    run(r#"
if (Vael.getBuiltInModule("node:path") !== undefined) {
    throw new Error("Vael.getBuiltInModule(\"node:path\") returned a module");
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

#[test]
#[cfg(feature = "vael-http")]
fn http_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:http")) {
    throw new Error(`http is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("http")) {
    throw new Error(`a bare http name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:http");
if (registry === undefined || typeof registry !== "object") {
    throw new Error("the http registry object is missing");
}
if (Vael.getBuiltInModule("http") !== undefined) {
    throw new Error("the registry accepted a bare http name");
}
const namespace = await import("vael:http");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.createRouter !== registry.createRouter || namespace.serve !== registry.serve) {
    throw new Error("the namespace export is not the registry function");
}
const mux = namespace.createRouter().get("/ping", () => namespace.json({ pong: true }));
const res = await mux.fetch("/ping");
if (res.status !== 200 || res.json().pong !== true) {
    throw new Error("vael:http mux.fetch failed");
}
"#)
    .expect("http feature");
}

#[test]
#[cfg(not(feature = "vael-http"))]
fn http_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:http") || list.includes("http")) {
    throw new Error(`http is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:http") !== undefined) {
    throw new Error("the canonical http registry object is still present");
}
if (Vael.getBuiltInModule("http") !== undefined) {
    throw new Error("a bare http name returned a module");
}
let imported = false;
try {
    await import("vael:http");
    imported = true;
} catch {
    // Expected: the loader has no http entry.
}
if (imported) throw new Error("vael:http was importable while disabled");
"#)
    .expect("http absent");
}

#[test]
#[cfg(feature = "vael-fetch")]
fn fetch_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:fetch")) {
    throw new Error(`fetch is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("fetch")) {
    throw new Error(`a bare fetch name is listed: ${JSON.stringify(list)}`);
}
if (globalThis.fetch !== undefined) {
    throw new Error("globalThis.fetch must never be installed");
}
const registry = Vael.getBuiltInModule("vael:fetch");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the fetch registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("fetch") !== undefined) {
    throw new Error("the registry accepted a bare fetch name");
}
const namespace = await import("vael:fetch");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.fetch !== registry.fetch || namespace.fetchSync !== registry.fetchSync) {
    throw new Error("the namespace export is not the registry function");
}
if (namespace.download !== registry.download || namespace.downloadSync !== registry.downloadSync) {
    throw new Error("the namespace download export is not the registry function");
}
if (namespace.FetchError !== registry.FetchError) {
    throw new Error("the namespace FetchError export is not the registry class");
}
"#)
    .expect("fetch feature");
}

#[test]
#[cfg(not(feature = "vael-fetch"))]
fn fetch_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:fetch") || list.includes("fetch")) {
    throw new Error(`fetch is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:fetch") !== undefined) {
    throw new Error("the canonical fetch registry object is still present");
}
if (Vael.getBuiltInModule("fetch") !== undefined) {
    throw new Error("a bare fetch name returned a module");
}
let imported = false;
try {
    await import("vael:fetch");
    imported = true;
} catch {
    // Expected: the loader has no fetch entry.
}
if (imported) throw new Error("vael:fetch was importable while disabled");
"#)
    .expect("fetch absent");
}

#[test]
#[cfg(feature = "vael-exec")]
fn exec_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:exec")) {
    throw new Error(`exec is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("exec")) {
    throw new Error(`a bare exec name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:exec");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the exec registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("exec") !== undefined) {
    throw new Error("the registry accepted a bare exec name");
}
const namespace = await import("vael:exec");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.Command !== registry.Command || namespace.exec !== registry.exec) {
    throw new Error("the namespace export is not the registry export");
}
if (namespace.lockFile !== registry.lockFile || namespace.which !== registry.which) {
    throw new Error("the namespace lockFile/which export is not the registry export");
}
"#)
    .expect("exec feature");
}

#[test]
#[cfg(not(feature = "vael-exec"))]
fn exec_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:exec") || list.includes("exec")) {
    throw new Error(`exec is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:exec") !== undefined) {
    throw new Error("the canonical exec registry object is still present");
}
if (Vael.getBuiltInModule("exec") !== undefined) {
    throw new Error("a bare exec name returned a module");
}
let imported = false;
try {
    await import("vael:exec");
    imported = true;
} catch {
    // Expected: the loader has no exec entry.
}
if (imported) throw new Error("vael:exec was importable while disabled");
"#)
    .expect("exec absent");
}

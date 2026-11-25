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
    if cfg!(feature = "neo-console") {
        entries.push(r#""neo:console""#);
    }
    if cfg!(feature = "neo-crypto") {
        entries.push(r#""neo:crypto""#);
    }
    if cfg!(feature = "neo-fs") {
        entries.push(r#""neo:fs""#);
    }
    if cfg!(feature = "neo-os") {
        entries.push(r#""neo:os""#);
    }
    if cfg!(feature = "neo-path") {
        entries.push(r#""neo:path""#);
    }
    if cfg!(feature = "neo-slices") {
        entries.push(r#""neo:slices""#);
    }
    if cfg!(feature = "neo-ssh") {
        entries.push(r#""neo:ssh""#);
    }
    if cfg!(feature = "neo-strings") {
        entries.push(r#""neo:strings""#);
    }
    if cfg!(feature = "neo-template") {
        entries.push(r#""neo:template""#);
    }
    if cfg!(feature = "neo-test") {
        entries.push(r#""neo:test""#);
    }
    if cfg!(feature = "neo-uuid") {
        entries.push(r#""neo:uuid""#);
        entries.push(r#""neo:uuid/v4""#);
        entries.push(r#""neo:uuid/v7""#);
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

#[test]
#[cfg(feature = "neo-os")]
fn os_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:os")) {
    throw new Error(`os is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("os")) {
    throw new Error(`a bare os name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:os");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the os registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("os") !== undefined) {
    throw new Error("the registry accepted a bare os name");
}
const namespace = await import("neo:os");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.platform !== registry.platform || namespace.arch !== registry.arch) {
    throw new Error("the namespace exports do not match the registry");
}
"#)
    .expect("os feature");
}

#[test]
#[cfg(not(feature = "neo-os"))]
fn os_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:os") || list.includes("os")) {
    throw new Error(`os is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:os") !== undefined) {
    throw new Error("the canonical os registry object is still present");
}
if (Neo.getBuiltInModule("os") !== undefined) {
    throw new Error("a bare os name returned a module");
}
let imported = false;
try {
    await import("neo:os");
    imported = true;
} catch {
    // Expected: the loader has no os entry.
}
if (imported) throw new Error("neo:os was importable while disabled");
"#)
    .expect("os absent");
}

#[test]
#[cfg(feature = "neo-fs")]
fn fs_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:fs")) {
    throw new Error(`fs is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("fs")) {
    throw new Error(`a bare fs name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:fs");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the fs registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("fs") !== undefined) {
    throw new Error("the registry accepted a bare fs name");
}
const namespace = await import("neo:fs");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (namespace.readFile !== registry.readFile || namespace.exists !== registry.exists) {
    throw new Error("the namespace exports do not match the registry");
}
"#)
    .expect("fs feature");
}

#[test]
#[cfg(not(feature = "neo-fs"))]
fn fs_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:fs") || list.includes("fs")) {
    throw new Error(`fs is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:fs") !== undefined) {
    throw new Error("the canonical fs registry object is still present");
}
if (Neo.getBuiltInModule("fs") !== undefined) {
    throw new Error("a bare fs name returned a module");
}
let imported = false;
try {
    await import("neo:fs");
    imported = true;
} catch {
    // Expected: the loader has no fs entry.
}
if (imported) throw new Error("neo:fs was importable while disabled");
"#)
    .expect("fs absent");
}

#[test]
#[cfg(feature = "neo-uuid")]
fn uuid_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
for (const specifier of ["neo:uuid", "neo:uuid/v4", "neo:uuid/v7"]) {
    if (!list.includes(specifier)) {
        throw new Error(`${specifier} is missing from the list: ${JSON.stringify(list)}`);
    }
    const registry = Neo.getBuiltInModule(specifier);
    if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
        throw new Error(`the ${specifier} registry object is missing or unfrozen`);
    }
    const bare = specifier.slice("neo:".length);
    if (list.includes(bare) || Neo.getBuiltInModule(bare) !== undefined) {
        throw new Error(`bare ${bare} leaked into registry or list`);
    }
}
const uuidNs = await import("neo:uuid");
const v4Ns = await import("neo:uuid/v4");
const v7Ns = await import("neo:uuid/v7");
const uuidReg = Neo.getBuiltInModule("neo:uuid");
if (uuidNs === uuidReg) throw new Error("namespace must be distinct from registry");
if (uuidNs.v4 !== uuidReg.v4 || v4Ns.v4 !== uuidReg.v4 || v4Ns.default !== uuidReg.v4) {
    throw new Error("v4 identity mismatch");
}
if (uuidNs.v7 !== uuidReg.v7 || v7Ns.v7 !== uuidReg.v7 || v7Ns.default !== uuidReg.v7) {
    throw new Error("v7 identity mismatch");
}
"#)
    .expect("uuid feature");
}

#[test]
#[cfg(not(feature = "neo-uuid"))]
fn uuid_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
for (const specifier of ["neo:uuid", "neo:uuid/v4", "neo:uuid/v7"]) {
    const bare = specifier.slice("neo:".length);
    if (list.includes(specifier) || list.includes(bare)) {
        throw new Error(`${specifier} is still listed: ${JSON.stringify(list)}`);
    }
    if (Neo.getBuiltInModule(specifier) !== undefined || Neo.getBuiltInModule(bare) !== undefined) {
        throw new Error(`${specifier} registry object is still present`);
    }
    let imported = false;
    try {
        await import(specifier);
        imported = true;
    } catch {
        // Expected: the loader has no uuid entry.
    }
    if (imported) throw new Error(`${specifier} was importable while disabled`);
}
"#)
    .expect("uuid absent");
}

#[test]
#[cfg(feature = "neo-ssh")]
fn ssh_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:ssh")) {
    throw new Error(`ssh is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("ssh")) {
    throw new Error(`a bare ssh name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:ssh");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the ssh registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("ssh") !== undefined) {
    throw new Error("the registry accepted a bare ssh name");
}
const namespace = await import("neo:ssh");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
if (
    namespace.connect !== registry.connect ||
    namespace.MAX_COMMAND_BYTES !== registry.MAX_COMMAND_BYTES ||
    namespace.MAX_OUTPUT_BYTES !== registry.MAX_OUTPUT_BYTES ||
    namespace.DEFAULT_CONNECT_TIMEOUT_MS !== registry.DEFAULT_CONNECT_TIMEOUT_MS ||
    namespace.DEFAULT_EXEC_TIMEOUT_MS !== registry.DEFAULT_EXEC_TIMEOUT_MS
) {
    throw new Error("the namespace exports do not match the registry");
}
"#)
    .expect("ssh feature");
}

#[test]
#[cfg(not(feature = "neo-ssh"))]
fn ssh_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:ssh") || list.includes("ssh")) {
    throw new Error(`ssh is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:ssh") !== undefined) {
    throw new Error("the canonical ssh registry object is still present");
}
if (Neo.getBuiltInModule("ssh") !== undefined) {
    throw new Error("a bare ssh name returned a module");
}
let imported = false;
try {
    await import("neo:ssh");
    imported = true;
} catch {
    // Expected: the loader has no ssh entry.
}
if (imported) throw new Error("neo:ssh was importable while disabled");
"#)
    .expect("ssh absent");
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
if (
    namespace.render !== registry.render ||
    namespace.compile !== registry.compile ||
    namespace.createTemplateSet !== registry.createTemplateSet ||
    namespace.escapeHtml !== registry.escapeHtml
) {
    throw new Error("the namespace exports do not match the registry");
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
#[cfg(feature = "neo-crypto")]
fn crypto_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:crypto")) {
    throw new Error(`crypto is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("crypto")) {
    throw new Error(`a bare crypto name is listed: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:crypto");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the crypto registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("crypto") !== undefined) {
    throw new Error("the registry accepted a bare crypto name");
}
const namespace = await import("neo:crypto");
if (namespace === registry) {
    throw new Error("the ES module namespace must be distinct from the registry object");
}
const expected = [
    "bcryptHash",
    "bcryptVerify",
    "createDecryptor",
    "createEncryptor",
    "createHash",
    "createHmac",
    "decrypt",
    "encrypt",
    "generateEnvelopeKey",
    "pbkdf2",
    "randomBytes",
    "shamirCombine",
    "shamirSplit",
];
if (JSON.stringify(Object.keys(registry).sort()) !== JSON.stringify(expected)) {
    throw new Error(`unexpected registry keys: ${JSON.stringify(Object.keys(registry).sort())}`);
}
if (JSON.stringify(Object.keys(namespace).sort()) !== JSON.stringify(expected)) {
    throw new Error(`unexpected namespace keys: ${JSON.stringify(Object.keys(namespace).sort())}`);
}
for (const name of expected) {
    if (namespace[name] !== registry[name]) {
        throw new Error(`the namespace export ${name} is not the registry function`);
    }
}
for (const asyncName of [
    "bcryptHashAsync",
    "bcryptVerifyAsync",
    "hashAsync",
    "hmacAsync",
    "pbkdf2Async",
    "shamirCombineAsync",
    "shamirSplitAsync",
]) {
    if (asyncName in registry || asyncName in namespace) {
        throw new Error(`unexpected async export ${asyncName} in neojs-lite`);
    }
}
"#)
    .expect("crypto feature");
}

#[test]
#[cfg(not(feature = "neo-crypto"))]
fn crypto_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:crypto") || list.includes("crypto")) {
    throw new Error(`crypto is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:crypto") !== undefined) {
    throw new Error("the canonical crypto registry object is still present");
}
if (Neo.getBuiltInModule("crypto") !== undefined) {
    throw new Error("a bare crypto name returned a module");
}
let imported = false;
try {
    await import("neo:crypto");
    imported = true;
} catch {
    // Expected: the loader has no crypto entry.
}
if (imported) throw new Error("neo:crypto was importable while disabled");
"#)
    .expect("crypto absent");
}

#[test]
#[cfg(not(feature = "neo-test"))]
fn run_tests_returns_error_when_test_feature_is_disabled() {
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_tests()
        .expect_err("run_tests must fail when neo-test is disabled");
    assert!(
        matches!(error, neojs_lite_core::Error::TestRunner(ref message) if message.contains("neo-test")),
        "{error:?}"
    );
}

#[test]
#[cfg(feature = "neo-console")]
fn console_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:console") || list.includes("console")) {
    throw new Error(`unexpected console list entries: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:console");
if (registry !== globalThis.console || Object.isFrozen(registry)) {
    throw new Error("neo:console registry must be the live mutable global console");
}
if (Neo.getBuiltInModule("console") !== undefined) {
    throw new Error("the registry accepted a bare console name");
}
const namespace = await import("neo:console");
if (namespace === registry || namespace.default !== registry) {
    throw new Error("neo:console namespace/default contract mismatch");
}
for (const method of ["log", "debug", "info", "warn", "error"]) {
    if (namespace[method] !== registry[method]) {
        throw new Error(`neo:console.${method} mismatch`);
    }
}
"#)
    .expect("console feature");
}

#[test]
#[cfg(not(feature = "neo-console"))]
fn console_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:console") || list.includes("console")) {
    throw new Error(`console is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:console") !== undefined || Neo.getBuiltInModule("console") !== undefined) {
    throw new Error("console registry entry is still present");
}
let imported = false;
try {
    await import("neo:console");
    imported = true;
} catch {
    // Expected: the loader has no console entry.
}
if (imported) throw new Error("neo:console was importable while disabled");
"#)
    .expect("console absent");
}

#[test]
#[cfg(feature = "neo-path")]
fn path_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:path") || list.includes("path")) {
    throw new Error(`unexpected path list entries: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:path");
if (!registry || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the path registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("path") !== undefined) {
    throw new Error("the registry accepted a bare path name");
}
const namespace = await import("neo:path");
if (namespace === registry || namespace.join !== registry.join) {
    throw new Error("neo:path namespace/registry mismatch");
}
if (namespace.posix.join("a", "b") !== "a/b") {
    throw new Error("neo:path.posix.join failed");
}
"#)
    .expect("path feature");
}

#[test]
#[cfg(not(feature = "neo-path"))]
fn path_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (list.includes("neo:path") || list.includes("path")) {
    throw new Error(`path is still listed: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModule("neo:path") !== undefined || Neo.getBuiltInModule("path") !== undefined) {
    throw new Error("path registry entry is still present");
}
let imported = false;
try {
    await import("neo:path");
    imported = true;
} catch {
    // Expected: the loader has no path entry.
}
if (imported) throw new Error("neo:path was importable while disabled");
"#)
    .expect("path absent");
}

#[test]
#[cfg(feature = "neo-slices")]
fn slices_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Neo.getBuiltInModulesList();
if (!list.includes("neo:slices") || list.includes("slices")) {
    throw new Error(`unexpected slices list entries: ${JSON.stringify(list)}`);
}
const registry = Neo.getBuiltInModule("neo:slices");
if (!registry || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the slices registry object is missing or unfrozen");
}
if (Neo.getBuiltInModule("slices") !== undefined) {
    throw new Error("the registry accepted a bare slices name");
}
const namespace = await import("neo:slices");
if (namespace === registry || namespace.slice !== registry.slice) {
    throw new Error("neo:slices namespace/registry mismatch");
}
if (namespace.CharSlice.fromString("ab").toUpper().toString() !== "AB") {
    throw new Error("neo:slices.CharSlice failed");
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
if (Neo.getBuiltInModule("neo:slices") !== undefined || Neo.getBuiltInModule("slices") !== undefined) {
    throw new Error("slices registry entry is still present");
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
#[cfg(feature = "node-compat")]
fn node_compat_is_present_on_lite_respects_runtime_opt_out_and_excludes_fs_promises() {
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
let promisesImported = false;
try {
    await import("node:fs/promises");
    promisesImported = true;
} catch {
    // Expected: node:fs/promises is V8-only.
}
if (promisesImported) throw new Error("node:fs/promises must not be available in neojs-lite");
let bareImported = false;
try {
    await import("fs");
    bareImported = true;
} catch {
    // Expected: bare name never aliases node: or neo:.
}
if (bareImported) throw new Error("bare \"fs\" was importable");
"#)
    .expect("lite node-compat feature enabled");

    let mut disabled =
        Runtime::with_options(neojs_lite_core::RuntimeOptions::new().with_node_compat(false))
            .expect("runtime with node_compat disabled");
    let err = disabled
        .run_source("disabled.js", "import 'node:path';\n")
        .expect_err("node:path must fail when node_compat is disabled");
    assert!(err.to_string().contains("node:"), "{err}");

    let mut exit_rt = Runtime::new().expect("runtime");
    exit_rt
        .run_source(
            "exit.js",
            "import process from 'node:process';\nprocess.exit(19);\nthrow new Error('unreachable');\n",
        )
        .expect("process.exit terminates cleanly");
    assert_eq!(exit_rt.exit_code(), Some(19));

    let mut code_rt = Runtime::new().expect("runtime");
    code_rt
        .run_source(
            "exit_code.js",
            "import process from 'node:process';\nprocess.exitCode = 11;\n",
        )
        .expect("process.exitCode completes cleanly");
    assert_eq!(code_rt.exit_code(), Some(11));
}

#[test]
#[cfg(not(feature = "node-compat"))]
fn node_compat_is_absent_on_lite_when_feature_is_disabled() {
    run(r#"
if (Neo.getBuiltInModule("node:path") !== undefined) {
    throw new Error("Neo.getBuiltInModule(\"node:path\") returned a module");
}
for (const spec of ["node:path", "node:fs", "node:process", "node:crypto"]) {
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
    .expect("lite node-compat absent");
}

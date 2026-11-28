//! Feature-surface tests for the QuickJS shared modules.
//!
//! The default feature set includes `vael-chars` and `vael-strings`. The
//! `--no-default-features` matrix run must show an empty registry/list and
//! unimportable specifiers, and must not compile the shared dependency.

use vaeljs_lite_core::Runtime;

fn run(source: &str) -> Result<(), vaeljs_lite_core::Error> {
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_source("memory.js", source)
}

/// The canonical list this build selected, in registration order.
fn expected_list() -> String {
    let mut entries = Vec::new();
    if cfg!(feature = "vael-assert") {
        entries.push(r#""vael:assert""#);
    }
    if cfg!(feature = "vael-chars") {
        entries.push(r#""vael:chars""#);
    }
    if cfg!(feature = "vael-console") {
        entries.push(r#""vael:console""#);
    }
    if cfg!(feature = "vael-crypto") {
        entries.push(r#""vael:crypto""#);
    }
    if cfg!(feature = "vael-exec") {
        entries.push(r#""vael:exec""#);
    }
    if cfg!(feature = "vael-fetch") {
        entries.push(r#""vael:fetch""#);
    }
    if cfg!(feature = "vael-fs") {
        entries.push(r#""vael:fs""#);
    }
    if cfg!(feature = "vael-os") {
        entries.push(r#""vael:os""#);
    }
    if cfg!(feature = "vael-path") {
        entries.push(r#""vael:path""#);
    }
    if cfg!(feature = "vael-slices") {
        entries.push(r#""vael:slices""#);
    }
    if cfg!(feature = "vael-ssh") {
        entries.push(r#""vael:ssh""#);
    }
    if cfg!(feature = "vael-strings") {
        entries.push(r#""vael:strings""#);
    }
    if cfg!(feature = "vael-template") {
        entries.push(r#""vael:template""#);
    }
    if cfg!(feature = "vael-test") {
        entries.push(r#""vael:test""#);
    }
    if cfg!(feature = "vael-uuid") {
        entries.push(r#""vael:uuid""#);
        entries.push(r#""vael:uuid/v4""#);
        entries.push(r#""vael:uuid/v7""#);
    }
    format!("[{}]", entries.join(", "))
}

#[test]
fn the_module_list_matches_the_selected_features() {
    let source = format!(
        r#"
const list = Vael.getBuiltInModulesList();
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
#[cfg(feature = "vael-assert")]
fn assert_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:assert")) {
    throw new Error(`assert is missing from the list: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:assert");
if (typeof registry !== "function" || !Object.isFrozen(registry)) {
    throw new Error("the assert registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("assert") !== undefined) {
    throw new Error("the registry accepted a bare assert name");
}
const namespace = await import("vael:assert");
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
#[cfg(not(feature = "vael-assert"))]
fn assert_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:assert") || list.includes("assert")) {
    throw new Error(`assert is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:assert") !== undefined) {
    throw new Error("the canonical assert registry object is still present");
}
let imported = false;
try {
    await import("vael:assert");
    imported = true;
} catch {
    // Expected: the loader has no assert entry.
}
if (imported) throw new Error("vael:assert was importable while disabled");
"#)
    .expect("assert absent");
}

#[test]
#[cfg(feature = "vael-test")]
fn test_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:test")) {
    throw new Error(`test is missing from the list: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:test");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the test registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("test") !== undefined) {
    throw new Error("the registry accepted a bare test name");
}
const namespace = await import("vael:test");
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
#[cfg(not(feature = "vael-test"))]
fn test_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:test") || list.includes("test")) {
    throw new Error(`test is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:test") !== undefined) {
    throw new Error("the canonical test registry object is still present");
}
let imported = false;
try {
    await import("vael:test");
    imported = true;
} catch {
    // Expected: the loader has no test entry.
}
if (imported) throw new Error("vael:test was importable while disabled");
"#)
    .expect("test absent");
}

#[test]
#[cfg(feature = "vael-os")]
fn os_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:os")) {
    throw new Error(`os is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("os")) {
    throw new Error(`a bare os name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:os");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the os registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("os") !== undefined) {
    throw new Error("the registry accepted a bare os name");
}
const namespace = await import("vael:os");
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
#[cfg(not(feature = "vael-os"))]
fn os_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:os") || list.includes("os")) {
    throw new Error(`os is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:os") !== undefined) {
    throw new Error("the canonical os registry object is still present");
}
if (Vael.getBuiltInModule("os") !== undefined) {
    throw new Error("a bare os name returned a module");
}
let imported = false;
try {
    await import("vael:os");
    imported = true;
} catch {
    // Expected: the loader has no os entry.
}
if (imported) throw new Error("vael:os was importable while disabled");
"#)
    .expect("os absent");
}

#[test]
#[cfg(feature = "vael-fs")]
fn fs_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:fs")) {
    throw new Error(`fs is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("fs")) {
    throw new Error(`a bare fs name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:fs");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the fs registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("fs") !== undefined) {
    throw new Error("the registry accepted a bare fs name");
}
const namespace = await import("vael:fs");
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
#[cfg(not(feature = "vael-fs"))]
fn fs_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:fs") || list.includes("fs")) {
    throw new Error(`fs is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:fs") !== undefined) {
    throw new Error("the canonical fs registry object is still present");
}
if (Vael.getBuiltInModule("fs") !== undefined) {
    throw new Error("a bare fs name returned a module");
}
let imported = false;
try {
    await import("vael:fs");
    imported = true;
} catch {
    // Expected: the loader has no fs entry.
}
if (imported) throw new Error("vael:fs was importable while disabled");
"#)
    .expect("fs absent");
}

#[test]
#[cfg(feature = "vael-uuid")]
fn uuid_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
for (const specifier of ["vael:uuid", "vael:uuid/v4", "vael:uuid/v7"]) {
    if (!list.includes(specifier)) {
        throw new Error(`${specifier} is missing from the list: ${JSON.stringify(list)}`);
    }
    const registry = Vael.getBuiltInModule(specifier);
    if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
        throw new Error(`the ${specifier} registry object is missing or unfrozen`);
    }
    const bare = specifier.slice("vael:".length);
    if (list.includes(bare) || Vael.getBuiltInModule(bare) !== undefined) {
        throw new Error(`bare ${bare} leaked into registry or list`);
    }
}
const uuidNs = await import("vael:uuid");
const v4Ns = await import("vael:uuid/v4");
const v7Ns = await import("vael:uuid/v7");
const uuidReg = Vael.getBuiltInModule("vael:uuid");
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
#[cfg(not(feature = "vael-uuid"))]
fn uuid_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
for (const specifier of ["vael:uuid", "vael:uuid/v4", "vael:uuid/v7"]) {
    const bare = specifier.slice("vael:".length);
    if (list.includes(specifier) || list.includes(bare)) {
        throw new Error(`${specifier} is still listed: ${JSON.stringify(list)}`);
    }
    if (Vael.getBuiltInModule(specifier) !== undefined || Vael.getBuiltInModule(bare) !== undefined) {
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
#[cfg(feature = "vael-ssh")]
fn ssh_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:ssh")) {
    throw new Error(`ssh is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("ssh")) {
    throw new Error(`a bare ssh name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:ssh");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the ssh registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("ssh") !== undefined) {
    throw new Error("the registry accepted a bare ssh name");
}
const namespace = await import("vael:ssh");
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
#[cfg(not(feature = "vael-ssh"))]
fn ssh_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:ssh") || list.includes("ssh")) {
    throw new Error(`ssh is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:ssh") !== undefined) {
    throw new Error("the canonical ssh registry object is still present");
}
if (Vael.getBuiltInModule("ssh") !== undefined) {
    throw new Error("a bare ssh name returned a module");
}
let imported = false;
try {
    await import("vael:ssh");
    imported = true;
} catch {
    // Expected: the loader has no ssh entry.
}
if (imported) throw new Error("vael:ssh was importable while disabled");
"#)
    .expect("ssh absent");
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
if (
    namespace.render !== registry.render ||
    namespace.compile !== registry.compile ||
    namespace.createTemplateSet !== registry.createTemplateSet ||
    namespace.escapeHtml !== registry.escapeHtml
) {
    throw new Error("the namespace exports do not match the registry");
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
#[cfg(feature = "vael-crypto")]
fn crypto_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:crypto")) {
    throw new Error(`crypto is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("crypto")) {
    throw new Error(`a bare crypto name is listed: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:crypto");
if (registry === undefined || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the crypto registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("crypto") !== undefined) {
    throw new Error("the registry accepted a bare crypto name");
}
const namespace = await import("vael:crypto");
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
        throw new Error(`unexpected async export ${asyncName} in vaeljs-lite`);
    }
}
"#)
    .expect("crypto feature");
}

#[test]
#[cfg(not(feature = "vael-crypto"))]
fn crypto_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:crypto") || list.includes("crypto")) {
    throw new Error(`crypto is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:crypto") !== undefined) {
    throw new Error("the canonical crypto registry object is still present");
}
if (Vael.getBuiltInModule("crypto") !== undefined) {
    throw new Error("a bare crypto name returned a module");
}
let imported = false;
try {
    await import("vael:crypto");
    imported = true;
} catch {
    // Expected: the loader has no crypto entry.
}
if (imported) throw new Error("vael:crypto was importable while disabled");
"#)
    .expect("crypto absent");
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
const expected = [
    "Command",
    "CommandError",
    "FileLock",
    "NotFoundOnPathError",
    "Output",
    "PathFinder",
    "addLogger",
    "clearLoggers",
    "cmd",
    "execSync",
    "lockFileSync",
    "pathFinder",
    "runSync",
    "whichAllSync",
    "whichSync",
];
if (JSON.stringify(Object.keys(registry).sort()) !== JSON.stringify(expected)) {
    throw new Error(`unexpected exec registry keys: ${JSON.stringify(Object.keys(registry).sort())}`);
}
if (JSON.stringify(Object.keys(namespace).sort()) !== JSON.stringify(expected)) {
    throw new Error(`unexpected exec namespace keys: ${JSON.stringify(Object.keys(namespace).sort())}`);
}
if ("default" in namespace) {
    throw new Error("vael:exec must not have a default export");
}
for (const name of expected) {
    if (namespace[name] !== registry[name]) {
        throw new Error(`vael:exec.${name} namespace/registry identity mismatch`);
    }
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
if (Vael.getBuiltInModule("vael:exec") !== undefined || Vael.getBuiltInModule("exec") !== undefined) {
    throw new Error("exec registry entry is still present");
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

#[test]
#[cfg(feature = "vael-fetch")]
fn fetch_is_present_when_its_feature_is_enabled() {
    run(r#"
if ("fetch" in globalThis) {
    throw new Error("vael:fetch must never install globalThis.fetch");
}
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:fetch")) {
    throw new Error(`fetch is missing from the list: ${JSON.stringify(list)}`);
}
if (list.includes("fetch")) {
    throw new Error(`a bare fetch name is listed: ${JSON.stringify(list)}`);
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
const expected = ["FetchError", "downloadSync", "fetchSync"];
if (JSON.stringify(Object.keys(registry).sort()) !== JSON.stringify(expected)) {
    throw new Error(`unexpected fetch registry keys: ${JSON.stringify(Object.keys(registry).sort())}`);
}
if (JSON.stringify(Object.keys(namespace).sort()) !== JSON.stringify(expected)) {
    throw new Error(`unexpected fetch namespace keys: ${JSON.stringify(Object.keys(namespace).sort())}`);
}
if ("default" in namespace) {
    throw new Error("vael:fetch must not have a default export");
}
for (const name of expected) {
    if (namespace[name] !== registry[name]) {
        throw new Error(`vael:fetch.${name} namespace/registry identity mismatch`);
    }
}
"#)
    .expect("fetch feature");
}

#[test]
#[cfg(not(feature = "vael-fetch"))]
fn fetch_is_absent_when_its_feature_is_disabled() {
    run(r#"
if ("fetch" in globalThis) {
    throw new Error("globalThis.fetch must not exist");
}
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:fetch") || list.includes("fetch")) {
    throw new Error(`fetch is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:fetch") !== undefined || Vael.getBuiltInModule("fetch") !== undefined) {
    throw new Error("fetch registry entry is still present");
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
#[cfg(not(feature = "vael-test"))]
fn run_tests_returns_error_when_test_feature_is_disabled() {
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_tests()
        .expect_err("run_tests must fail when vael-test is disabled");
    assert!(
        matches!(error, vaeljs_lite_core::Error::TestRunner(ref message) if message.contains("vael-test")),
        "{error:?}"
    );
}

#[test]
#[cfg(feature = "vael-console")]
fn console_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:console") || list.includes("console")) {
    throw new Error(`unexpected console list entries: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:console");
if (registry !== globalThis.console || Object.isFrozen(registry)) {
    throw new Error("vael:console registry must be the live mutable global console");
}
if (Vael.getBuiltInModule("console") !== undefined) {
    throw new Error("the registry accepted a bare console name");
}
const namespace = await import("vael:console");
if (namespace === registry || namespace.default !== registry) {
    throw new Error("vael:console namespace/default contract mismatch");
}
for (const method of ["log", "debug", "info", "warn", "error"]) {
    if (namespace[method] !== registry[method]) {
        throw new Error(`vael:console.${method} mismatch`);
    }
}
"#)
    .expect("console feature");
}

#[test]
#[cfg(not(feature = "vael-console"))]
fn console_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:console") || list.includes("console")) {
    throw new Error(`console is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:console") !== undefined || Vael.getBuiltInModule("console") !== undefined) {
    throw new Error("console registry entry is still present");
}
let imported = false;
try {
    await import("vael:console");
    imported = true;
} catch {
    // Expected: the loader has no console entry.
}
if (imported) throw new Error("vael:console was importable while disabled");
"#)
    .expect("console absent");
}

#[test]
#[cfg(feature = "vael-path")]
fn path_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:path") || list.includes("path")) {
    throw new Error(`unexpected path list entries: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:path");
if (!registry || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the path registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("path") !== undefined) {
    throw new Error("the registry accepted a bare path name");
}
const namespace = await import("vael:path");
if (namespace === registry || namespace.join !== registry.join) {
    throw new Error("vael:path namespace/registry mismatch");
}
if (namespace.posix.join("a", "b") !== "a/b") {
    throw new Error("vael:path.posix.join failed");
}
"#)
    .expect("path feature");
}

#[test]
#[cfg(not(feature = "vael-path"))]
fn path_is_absent_when_its_feature_is_disabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:path") || list.includes("path")) {
    throw new Error(`path is still listed: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:path") !== undefined || Vael.getBuiltInModule("path") !== undefined) {
    throw new Error("path registry entry is still present");
}
let imported = false;
try {
    await import("vael:path");
    imported = true;
} catch {
    // Expected: the loader has no path entry.
}
if (imported) throw new Error("vael:path was importable while disabled");
"#)
    .expect("path absent");
}

#[test]
#[cfg(feature = "vael-slices")]
fn slices_is_present_when_its_feature_is_enabled() {
    run(r#"
const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:slices") || list.includes("slices")) {
    throw new Error(`unexpected slices list entries: ${JSON.stringify(list)}`);
}
const registry = Vael.getBuiltInModule("vael:slices");
if (!registry || typeof registry !== "object" || !Object.isFrozen(registry)) {
    throw new Error("the slices registry object is missing or unfrozen");
}
if (Vael.getBuiltInModule("slices") !== undefined) {
    throw new Error("the registry accepted a bare slices name");
}
const namespace = await import("vael:slices");
if (namespace === registry || namespace.slice !== registry.slice) {
    throw new Error("vael:slices namespace/registry mismatch");
}
if (namespace.CharSlice.fromString("ab").toUpper().toString() !== "AB") {
    throw new Error("vael:slices.CharSlice failed");
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
if (Vael.getBuiltInModule("vael:slices") !== undefined || Vael.getBuiltInModule("slices") !== undefined) {
    throw new Error("slices registry entry is still present");
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
#[cfg(feature = "node-compat")]
fn node_compat_is_present_on_lite_respects_runtime_opt_out_and_excludes_fs_promises() {
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
let promisesImported = false;
try {
    await import("node:fs/promises");
    promisesImported = true;
} catch {
    // Expected: node:fs/promises is V8-only.
}
if (promisesImported) throw new Error("node:fs/promises must not be available in vaeljs-lite");
let bareImported = false;
try {
    await import("fs");
    bareImported = true;
} catch {
    // Expected: bare name never aliases node: or vael:.
}
if (bareImported) throw new Error("bare \"fs\" was importable");
"#)
    .expect("lite node-compat feature enabled");

    let mut disabled =
        Runtime::with_options(vaeljs_lite_core::RuntimeOptions::new().with_node_compat(false))
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
if (Vael.getBuiltInModule("node:path") !== undefined) {
    throw new Error("Vael.getBuiltInModule(\"node:path\") returned a module");
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

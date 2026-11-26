//! Integration tests for the `os` built-in module, driven through `Runtime`.
//!
//! The JavaScript modules assert on their own and throw on failure, so a test
//! is one module plus the Rust call that runs it.
//!
//! These tests cover only `platform` and `arch`, the two values this slice
//! implements. `version`, `codename`, `variant`, user, keyring, and scheduler
//! surfaces are deliberately absent; the module reads no environment variable
//! or operating-system file, and vaeljs installs no `os` global.

use std::fs;

use vaeljs_core::Runtime;

/// Runs a JavaScript module whose own assertions throw on failure.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn imports_are_cached_frozen_and_identity_shared() {
    run_js(
        r#"
import * as os from "vael:os";
import * as alias from "VAEL:os";

const registry = Vael.getBuiltInModule("vael:os");

if (os !== alias) throw new Error("os and VAEL:os namespaces differ");
if (alias !== (await import("vael:os"))) throw new Error("dynamic os namespace differs");
if (registry !== Vael.getBuiltInModule("vael:os")) throw new Error("registry lookup is not cached");
if (Object.isFrozen(registry) !== true) throw new Error("registry is not frozen");
if (Object.isExtensible(os) !== false) throw new Error("namespace is extensible");
if ("default" in os) throw new Error("unexpected default export");

const exported = Object.keys(os).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(["arch", "platform"])) {
    throw new Error(`unexpected exports: ${JSON.stringify(exported)}`);
}
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error("module exports and registry properties differ");
}
for (const name of exported) {
    if (os[name] !== registry[name]) throw new Error(`${name} is not the registry value`);
    if (typeof os[name] !== "string") throw new Error(`${name} is not a string`);
}
if (os.platform.length === 0 || os.arch.length === 0) {
    throw new Error("a constant is empty");
}

if (import.meta.resolve("vael:os") !== "vael:os") throw new Error("canonical os specifier");
if (import.meta.resolve("VAEL:os") !== "vael:os") throw new Error("scheme is not case-insensitive");

// The registry and loader accept only the canonical specifier, and the
// module installs no global.
if (Vael.getBuiltInModule("os") !== undefined) {
    throw new Error("registry accepted a bare name");
}
let bareRejected = false;
try {
    await import("os");
} catch (error) {
    bareRejected = error instanceof TypeError;
}
if (!bareRejected) throw new Error("bare os import was accepted");
for (const name of ["os", "__vaelOs", "__VAEL_OS_PLATFORM__", "__VAEL_OS_ARCH__"]) {
    if (name in globalThis) throw new Error(`${name} leaked onto globalThis`);
}
"#,
    );
}

#[test]
fn exports_equal_the_normalized_build_target_constants() {
    let expected_platform = if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    };
    let expected_arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        "arm" => "arm",
        "riscv64" => "riscv64",
        "powerpc64" => {
            if cfg!(target_endian = "little") {
                "ppc64le"
            } else {
                "ppc64"
            }
        }
        "s390x" => "s390x",
        "loongarch64" => "loong64",
        other => other,
    };
    let source = format!(
        r#"
import {{ platform, arch }} from "vael:os";
const registry = Vael.getBuiltInModule("vael:os");
if (platform !== {platform:?}) throw new Error(`platform: expected {platform:?}, got ${{platform}}`);
if (arch !== {arch:?}) throw new Error(`arch: expected {arch:?}, got ${{arch}}`);
if (platform !== registry.platform) throw new Error("platform differs from the registry");
if (arch !== registry.arch) throw new Error("arch differs from the registry");
"#,
        platform = expected_platform,
        arch = expected_arch,
    );
    run_js(&source);
}

#[test]
fn frozen_values_resist_replacement() {
    run_js(
        r#"
import * as os from "vael:os";

for (const name of ["platform", "arch"]) {
    let assignmentFailed = false;
    try {
        os[name] = "replaced";
    } catch (error) {
        assignmentFailed = error instanceof TypeError;
    }
    if (!assignmentFailed) throw new Error(`${name} assignment did not throw TypeError`);

    let redefineFailed = false;
    try {
        Object.defineProperty(Vael.getBuiltInModule("vael:os"), name, { value: "replaced" });
    } catch (error) {
        redefineFailed = error instanceof TypeError;
    }
    if (!redefineFailed) throw new Error(`${name} redefinition did not throw TypeError`);
}

if (os.platform === "replaced" || os.arch === "replaced") {
    throw new Error("a value was replaced");
}
"#,
    );
}

#[test]
fn lookup_ignores_replaced_intrinsics() {
    run_js(
        r#"
const os = Vael.getBuiltInModule("vael:os");
const originalGet = Map.prototype.get;
const originalFreeze = Object.freeze;
let replacedGetCalls = 0;
Map.prototype.get = function () {
    replacedGetCalls += 1;
    return "hijacked";
};
Object.freeze = () => "not frozen";
try {
    if (Vael.getBuiltInModule("vael:os") !== os) {
        throw new Error("known lookup did not return the cached module");
    }
    if (Vael.getBuiltInModule("missing") !== undefined) {
        throw new Error("unknown lookup exposed the registry");
    }
    if (os.platform !== Vael.getBuiltInModule("vael:os").platform) {
        throw new Error("lookup returned replaced values");
    }
} finally {
    Map.prototype.get = originalGet;
    Object.freeze = originalFreeze;
}
if (replacedGetCalls !== 0) throw new Error("lookup called the replaced Map.prototype.get");
if (Object.isFrozen(os) !== true) throw new Error("module is no longer frozen");
"#,
    );
}

#[test]
fn module_does_not_add_an_os_global() {
    run_js(
        r#"
import "vael:os";
import "VAEL:os";
if (typeof globalThis.os !== "undefined") throw new Error("os global was installed");
"#,
    );
}

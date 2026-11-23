//! Integration tests for the `uuid` built-in module through the public `Runtime`.
//!
//! The JavaScript modules assert on their own and throw on failure, so a test
//! is one module plus the Rust call that runs it. The Rust unit tests in
//! `neojs-core/src/uuid.rs` cover the RFC 9562 vectors, the clock bounds, and
//! the injected entropy failure.
//!
//! These tests cover generation only. Parsing, validation, timestamp
//! extraction, and versions other than 4 and 7 are deliberately absent.

use std::fs;

use neojs_core::Runtime;

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
import * as uuid from "neo:uuid";
import * as alias from "NEO:uuid";

const registry = Neo.getBuiltInModule("neo:uuid");

if (uuid !== alias) throw new Error("uuid and NEO:uuid namespaces differ");
if (alias !== (await import("neo:uuid"))) throw new Error("dynamic uuid namespace differs");
if (registry !== Neo.getBuiltInModule("neo:uuid")) throw new Error("registry lookup is not cached");
if (Object.isFrozen(registry) !== true) throw new Error("registry is not frozen");
if (Object.isExtensible(uuid) !== false) throw new Error("namespace is extensible");
if ("default" in uuid) throw new Error("unexpected default export");

const exported = Object.keys(uuid).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(["v4", "v7"])) {
    throw new Error(`unexpected exports: ${JSON.stringify(exported)}`);
}
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error("module exports and registry properties differ");
}
for (const name of exported) {
    if (uuid[name] !== registry[name]) throw new Error(`${name} is not the registry value`);
    if (typeof uuid[name] !== "function") throw new Error(`${name} is not a function`);
}

if (import.meta.resolve("neo:uuid") !== "neo:uuid") throw new Error("canonical uuid specifier");
if (import.meta.resolve("NEO:uuid") !== "neo:uuid") throw new Error("scheme is not case-insensitive");

// The registry and loader accept only the canonical specifier, and the module
// installs no global.
for (const name of ["uuid", "uuid/v4", "uuid/v7"]) {
    if (Neo.getBuiltInModule(name) !== undefined) {
        throw new Error(`registry accepted bare ${name}`);
    }
    let bareRejected = false;
    try {
        await import(name);
    } catch (error) {
        bareRejected = error instanceof TypeError;
    }
    if (!bareRejected) throw new Error(`bare ${name} import was accepted`);
}
for (const name of ["uuid", "__neoUuidV4", "__neoUuidV7", "__neoUUID"]) {
    if (name in globalThis) throw new Error(`${name} leaked onto globalThis`);
}

// The version subpaths are thin modules over the same functions. Both the
// named and default exports are the aggregate module's function object.
const v4Module = await import("neo:uuid/v4");
const v4Alias = await import("NEO:uuid/v4");
const v7Module = await import("neo:uuid/v7");
const v7Alias = await import("NEO:uuid/v7");
if (v4Module !== v4Alias) throw new Error("uuid/v4 namespaces differ");
if (v7Module !== v7Alias) throw new Error("uuid/v7 namespaces differ");
if (v4Module.v4 !== uuid.v4) throw new Error("uuid/v4 named export differs");
if (v4Module.default !== uuid.v4) throw new Error("uuid/v4 default export differs");
if (v7Module.v7 !== uuid.v7) throw new Error("uuid/v7 named export differs");
if (v7Module.default !== uuid.v7) throw new Error("uuid/v7 default export differs");

const registryV4 = Neo.getBuiltInModule("neo:uuid/v4");
const registryV7 = Neo.getBuiltInModule("neo:uuid/v7");
if (registryV4 !== Neo.getBuiltInModule("neo:uuid/v4")) throw new Error("uuid/v4 lookup is not cached");
if (registryV7 !== Neo.getBuiltInModule("neo:uuid/v7")) throw new Error("uuid/v7 lookup is not cached");
if (!Object.isFrozen(registryV4) || !Object.isFrozen(registryV7)) {
    throw new Error("a uuid subpath registry object is not frozen");
}
if (JSON.stringify(Object.keys(v4Module).sort()) !== JSON.stringify(["default", "v4"])) {
    throw new Error("unexpected uuid/v4 exports");
}
if (JSON.stringify(Object.keys(v7Module).sort()) !== JSON.stringify(["default", "v7"])) {
    throw new Error("unexpected uuid/v7 exports");
}
if (JSON.stringify(Object.keys(registryV4).sort()) !== JSON.stringify(["default", "v4"])) {
    throw new Error("unexpected uuid/v4 registry properties");
}
if (JSON.stringify(Object.keys(registryV7).sort()) !== JSON.stringify(["default", "v7"])) {
    throw new Error("unexpected uuid/v7 registry properties");
}
if (registryV4.v4 !== uuid.v4 || registryV4.default !== uuid.v4) {
    throw new Error("uuid/v4 registry identity differs");
}
if (registryV7.v7 !== uuid.v7 || registryV7.default !== uuid.v7) {
    throw new Error("uuid/v7 registry identity differs");
}
if (import.meta.resolve("neo:uuid/v4") !== "neo:uuid/v4") throw new Error("canonical uuid/v4 specifier");
if (import.meta.resolve("neo:uuid/v4") !== "neo:uuid/v4") {
    throw new Error("canonical neo:uuid/v4 specifier");
}
if (import.meta.resolve("neo:uuid/v7") !== "neo:uuid/v7") throw new Error("canonical uuid/v7 specifier");
if (import.meta.resolve("neo:uuid/v7") !== "neo:uuid/v7") {
    throw new Error("canonical neo:uuid/v7 specifier");
}
"#,
    );
}

#[test]
fn values_are_canonical_rfc_text_with_the_requested_version() {
    run_js(
        r#"
import { v4, v7 } from "neo:uuid";

const canonical = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const version4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const version7 = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const fours = new Set();
const sevens = new Set();
for (let index = 0; index < 256; index += 1) {
    const four = v4();
    if (typeof four !== "string" || four.length !== 36) throw new Error("v4 is not 36 characters");
    if (!canonical.test(four)) throw new Error(`v4 is not canonical: ${four}`);
    if (!version4.test(four)) throw new Error(`v4 version or variant is wrong: ${four}`);
    if (four !== four.toLowerCase()) throw new Error("v4 is not lowercase");
    fours.add(four);

    const seven = v7();
    if (typeof seven !== "string" || seven.length !== 36) throw new Error("v7 is not 36 characters");
    if (!canonical.test(seven)) throw new Error(`v7 is not canonical: ${seven}`);
    if (!version7.test(seven)) throw new Error(`v7 version or variant is wrong: ${seven}`);
    if (seven !== seven.toLowerCase()) throw new Error("v7 is not lowercase");
    sevens.add(seven);
}
if (fours.size !== 256) throw new Error("v4 repeated within 256 draws");
if (sevens.size !== 256) throw new Error("v7 repeated within 256 draws");

// The first 48 bits are big-endian Unix milliseconds. Read the timestamp back
// from the text rather than from a host-side helper this module does not expose.
function timestampOf(value) {
    const hex = value.slice(0, 8) + value.slice(9, 13);
    return Number.parseInt(hex, 16);
}
const before = Date.now();
const current = v7();
const after = Date.now();
const timestamp = timestampOf(current);
if (timestamp < before - 1000 || timestamp > after + 1000) {
    throw new Error(`v7 timestamp ${timestamp} is not near ${before}..${after}`);
}
"#,
    );
}

#[test]
fn argument_counts_are_strict_and_synchronous() {
    run_js(
        r#"
import * as uuid from "neo:uuid";

function expectTypeError(fn, label) {
    let error;
    try {
        fn();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof TypeError)) throw new Error(label + ": expected TypeError");
    if ("code" in error) throw new Error(label + ": unexpected code");
}

let called = false;
for (const value of [undefined, null, 0, "", "v4", {}, [], () => { called = true; }]) {
    expectTypeError(() => uuid.v4(value), "v4 argument");
    expectTypeError(() => uuid.v7(value), "v7 argument");
}
if (called) throw new Error("a callback argument was invoked");

const registry = Neo.getBuiltInModule("neo:uuid");
expectTypeError(() => registry.v4(undefined), "registry v4 argument");
expectTypeError(() => registry.v7(undefined, undefined), "registry v7 arguments");

if (typeof uuid.v4().then === "function") throw new Error("v4 returned a thenable");
if (typeof uuid.v7().then === "function") throw new Error("v7 returned a thenable");
"#,
    );
}

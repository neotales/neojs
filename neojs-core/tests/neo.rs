//! Integration tests for the `Neo` introspection surface.
//!
//! These run real ECMAScript modules through [`Runtime`], so they cover the
//! installed global, the registry, and the loader together. The JavaScript
//! modules assert on their own and throw on failure.

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
fn engine_and_module_list_are_frozen_exact_and_stable() {
    run_js(
        r#"
const list = Neo.getBuiltInModulesList();
const expected = [
    "age", "neo:age",
    "assert", "neo:assert",
    "chars", "neo:chars",
    "console", "neo:console",
    "cron", "neo:cron",
    "crypto", "neo:crypto",
    "os", "neo:os",
    "path", "neo:path",
    "slices", "neo:slices",
    "ssh", "neo:ssh",
    "stdio", "neo:stdio",
    "strings", "neo:strings",
    "test", "neo:test",
    "uuid", "neo:uuid",
    "uuid/v4", "neo:uuid/v4",
    "uuid/v7", "neo:uuid/v7",
];

if (Neo.engine !== "neo") throw new Error(`engine: ${Neo.engine}`);
if (JSON.stringify(list) !== JSON.stringify(expected)) {
    throw new Error(`unexpected list: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModulesList() !== list) {
    throw new Error("repeated calls returned different arrays");
}
if (Object.isFrozen(list) !== true) throw new Error("list is not frozen");
if (Object.isExtensible(list) !== false) throw new Error("list is extensible");

const descriptor = Object.getOwnPropertyDescriptor(globalThis, "Neo");
if (!descriptor) throw new Error("Neo is not an own global property");
if (descriptor.writable !== false) throw new Error("Neo is writable");
if (descriptor.enumerable !== false) throw new Error("Neo is enumerable");
if (descriptor.configurable !== false) throw new Error("Neo is configurable");
if (Object.isFrozen(Neo) !== true) throw new Error("Neo is not frozen");
if (typeof Neo.getBuiltInModule !== "function") throw new Error("missing getBuiltInModule");
if (Object.keys(Neo).sort().join(",") !== "engine,getBuiltInModule,getBuiltInModulesList") {
    throw new Error(`unexpected Neo keys: ${Object.keys(Neo).sort().join(",")}`);
}

const attempts = [
    () => { list[0] = "changed"; },
    () => { list.length = 0; },
    () => { list.push("changed"); },
    () => { delete list[0]; },
    () => { Object.defineProperty(list, 0, { value: "changed" }); },
];
for (const attempt of attempts) {
    let threw = false;
    try {
        attempt();
    } catch (error) {
        threw = error instanceof TypeError;
    }
    if (!threw) throw new Error("a mutation attempt did not throw TypeError");
}
if (JSON.stringify(list) !== JSON.stringify(expected)) throw new Error("list changed");
"#,
    );
}

#[test]
fn every_listed_specifier_resolves_to_one_importable_module() {
    run_js(
        r#"
const list = Neo.getBuiltInModulesList();
const names = [];
const specifiers = [];

for (const entry of list) {
    if (typeof entry !== "string" || entry.length === 0) {
        throw new Error(`invalid entry: ${String(entry)}`);
    }
    if (entry.startsWith("node:")) throw new Error(`Node module claimed: ${entry}`);
    if (entry.startsWith("neo:")) {
        specifiers.push(entry);
    } else {
        names.push(entry);
    }
}
if (names.length !== 16) throw new Error(`expected 16 names, got ${names.length}`);
if (specifiers.length !== 16) throw new Error(`expected 16 specifiers, got ${specifiers.length}`);

for (const name of names) {
    const canonical = `neo:${name}`;
    if (Neo.getBuiltInModule(name) === undefined) {
        throw new Error(`registry is missing ${name}`);
    }
    if (Neo.getBuiltInModule(canonical) !== undefined) {
        throw new Error(`registry accepted canonical specifier ${canonical}`);
    }
    if (import.meta.resolve(name) !== canonical) {
        throw new Error(`${name} did not resolve to ${canonical}`);
    }
    if (import.meta.resolve(canonical) !== canonical) {
        throw new Error(`${canonical} did not resolve to itself`);
    }
    const namespace = await import(name);
    const alias = await import(canonical);
    if (namespace !== alias) throw new Error(`${name} and ${canonical} are not one module`);
}

for (const specifier of specifiers) {
    if (!names.includes(specifier.slice("neo:".length))) {
        throw new Error(`canonical specifier has no bare name: ${specifier}`);
    }
}

for (const name of ["node:test", "node:fs", "node:path", "node:os", "fs", "process", "version"]) {
    if (list.includes(name)) throw new Error(`list claims ${name}`);
    if (list.includes(`neo:${name}`)) throw new Error(`list claims neo:${name}`);
    if (Neo.getBuiltInModule(name) !== undefined) {
        throw new Error(`registry returns a module for ${name}`);
    }
}
"#,
    );
}

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
    "neo:age",
    "neo:assert",
    "neo:chars",
    "neo:console",
    "neo:cron",
    "neo:crypto",
    "neo:os",
    "neo:path",
    "neo:slices",
    "neo:ssh",
    "neo:stdio",
    "neo:strings",
    "neo:test",
    "neo:uuid",
    "neo:uuid/v4",
    "neo:uuid/v7",
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

for (const specifier of list) {
    if (typeof specifier !== "string" || !specifier.startsWith("neo:")) {
        throw new Error(`not a canonical specifier: ${String(specifier)}`);
    }
    if (Neo.getBuiltInModule(specifier) === undefined) {
        throw new Error(`registry is missing ${specifier}`);
    }
    if (import.meta.resolve(specifier) !== specifier) {
        throw new Error(`${specifier} did not resolve to itself`);
    }
    const namespace = await import(specifier);
    if (namespace === null || typeof namespace !== "object") {
        throw new Error(`${specifier} did not produce a module namespace`);
    }
}

for (const name of ["node:test", "node:fs", "node:path", "node:os", "fs", "process", "version"]) {
    if (list.includes(name)) throw new Error(`list claims ${name}`);
    if (Neo.getBuiltInModule(name) !== undefined) {
        throw new Error(`registry returns a module for ${name}`);
    }
}
"#,
    );
}

#[test]
fn bare_registry_names_are_not_importable() {
    run_js(
        r#"
// Every name behind a canonical specifier is also a bare string. None of the
// bare forms is a built-in module: the loader must reject them and the registry
// must not expose them. This is the negative half of the canonical-only rule.
const names = Neo.getBuiltInModulesList().map((specifier) => specifier.slice("neo:".length));

for (const name of names) {
    if (Neo.getBuiltInModule(name) !== undefined) {
        throw new Error(`registry accepted bare name ${name}`);
    }
    let accepted = false;
    try {
        await import(name);
        accepted = true;
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
    }
    if (accepted) throw new Error(`loader imported bare name ${name}`);
}
"#,
    );
}

#[test]
fn a_bare_built_in_name_is_rejected_with_the_canonical_suggestion() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, "import { isLetter } from \"chars\";\n").expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("a bare name must fail");
    let message = error.to_string();

    assert!(message.contains("chars"), "{message}");
    assert!(
        message.contains("canonical `neo:` specifier") && message.contains("neo:chars"),
        "{message}"
    );
    assert!(message.contains("Bare module specifiers"), "{message}");
}

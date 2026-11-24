//! Tests for the embedding API, driven through `Runtime` rather than the binary.

use std::fs;

use neojs_lite_core::{Error, Runtime};

fn write(dir: &tempfile::TempDir, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create module directory");
    }
    fs::write(&path, source).expect("write source");
    path
}

#[test]
fn embedded_runtime_runs_a_module_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(
        &dir,
        "main.js",
        r#"
const value = "embedded";
if (value !== "embedded") throw new Error("value changed");
console.log(value);
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("run file");
}

#[cfg(feature = "neo-chars")]
fn expected_list() -> (String, usize) {
    let mut entries = Vec::new();
    if cfg!(feature = "neo-assert") {
        entries.push(r#""neo:assert""#);
    }
    if cfg!(feature = "neo-chars") {
        entries.push(r#""neo:chars""#);
    }
    if cfg!(feature = "neo-fs") {
        entries.push(r#""neo:fs""#);
    }
    if cfg!(feature = "neo-os") {
        entries.push(r#""neo:os""#);
    }
    if cfg!(feature = "neo-ssh") {
        entries.push(r#""neo:ssh""#);
    }
    if cfg!(feature = "neo-strings") {
        entries.push(r#""neo:strings""#);
    }
    if cfg!(feature = "neo-test") {
        entries.push(r#""neo:test""#);
    }
    if cfg!(feature = "neo-uuid") {
        entries.push(r#""neo:uuid""#);
        entries.push(r#""neo:uuid/v4""#);
        entries.push(r#""neo:uuid/v7""#);
    }
    (format!("[{}]", entries.join(", ")), entries.len())
}

#[cfg(feature = "neo-chars")]
#[test]
fn neo_reports_the_lite_engine_and_the_canonical_shared_module() {
    let mut runtime = Runtime::new().expect("runtime");
    let source = r#"
const list = Neo.getBuiltInModulesList();
if (Neo.engine !== "neo-lite") throw new Error(`engine: ${Neo.engine}`);
if (!Array.isArray(list)) throw new Error("module list is not an array");
if (JSON.stringify(list) !== JSON.stringify(__EXPECTED_LIST__)) {
    throw new Error(`unexpected module list: ${JSON.stringify(list)}`);
}
if (Neo.getBuiltInModulesList() !== list) {
    throw new Error("repeated calls returned different arrays");
}
if (Object.isFrozen(list) !== true) throw new Error("module list is not frozen");
if (Object.isExtensible(list) !== false) throw new Error("module list is extensible");
if (Object.isFrozen(Neo) !== true) throw new Error("Neo is not frozen");

const chars = Neo.getBuiltInModule("neo:chars");
if (chars === undefined || typeof chars !== "object") {
    throw new Error("the canonical chars module is missing");
}
if (Object.isFrozen(chars) !== true) throw new Error("chars is not frozen");
if (Neo.getBuiltInModule("neo:chars") !== chars) {
    throw new Error("repeated chars lookups returned different objects");
}

for (const name of [
    "age", "assert", "chars", "console", "cron", "crypto", "os", "path", "slices",
    "ssh", "stdio", "strings", "test", "uuid", "uuid/v4", "uuid/v7",
    "neo:age", "neo:cron", "node:test", "node:fs", "fs",
    "", "__proto__", "constructor", "neo:chars?x", "neo:chars#x",
]) {
    if (Neo.getBuiltInModule(name) !== undefined) {
        throw new Error(`Neo claimed ${name}`);
    }
}
let badNameThrew = false;
try {
    Neo.getBuiltInModule(1);
} catch (error) {
    badNameThrew = error instanceof TypeError;
}
if (!badNameThrew) throw new Error("a non-string name did not throw TypeError");

const descriptor = Object.getOwnPropertyDescriptor(globalThis, "Neo");
if (!descriptor) throw new Error("Neo is not an own global property");
if (descriptor.writable !== false) throw new Error("Neo is writable");
if (descriptor.enumerable !== false) throw new Error("Neo is enumerable");
if (descriptor.configurable !== false) throw new Error("Neo is configurable");
const engine = Object.getOwnPropertyDescriptor(Neo, "engine");
if (!engine) throw new Error("engine is not an own property");
if (engine.writable !== false) throw new Error("engine is writable");
if (engine.configurable !== false) throw new Error("engine is configurable");

let mutationThrew = false;
try {
    list.push("age");
} catch (error) {
    mutationThrew = error instanceof TypeError;
}
if (!mutationThrew) throw new Error("a list mutation did not throw TypeError");
if (list.length !== __EXPECTED_LENGTH__) throw new Error("the module list changed");
"#;
    let (list_json, list_len) = expected_list();
    let source = source
        .replace("__EXPECTED_LIST__", &list_json)
        .replace("__EXPECTED_LENGTH__", &list_len.to_string());
    runtime
        .run_source("memory.js", &source)
        .expect("Neo introspection");
}

#[test]
fn neo_lookup_uses_the_intrinsic_type_error_after_a_global_replacement() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
const IntrinsicTypeError = TypeError;
let substitutedCalls = 0;
globalThis.TypeError = function (...args) {
    substitutedCalls += 1;
    return new IntrinsicTypeError(`substituted:${args.length}`);
};

let caught;
try {
    Neo.getBuiltInModule(1);
} catch (error) {
    caught = error;
}

if (substitutedCalls !== 0) throw new Error("the substituted TypeError ran");
if (!(caught instanceof IntrinsicTypeError)) throw new Error("not the intrinsic TypeError");
if (caught.message !== "module name must be a string") {
    throw new Error(`message: ${caught.message}`);
}
"#,
        )
        .expect("intrinsic TypeError");
}

#[test]
fn relative_static_imports_resolve_from_the_entry_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "greet.js",
        "export function greet(name) { return `hello, ${name}`; }\n",
    );
    let main = write(
        &dir,
        "nested/main.js",
        r#"
import { greet } from "../greet.js";

if (greet("world") !== "hello, world") throw new Error(greet("world"));
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&main).expect("relative import");
}

#[test]
fn dynamic_imports_use_the_same_loader() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "value.js", "export default 41;\n");
    let main = write(
        &dir,
        "main.js",
        r#"
const value = (await import("./value.js")).default;
if (value !== 41) throw new Error(`dynamic import returned ${value}`);
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&main).expect("dynamic import");
}

#[test]
fn top_level_await_settles_from_the_job_queue() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(
        &dir,
        "main.js",
        r#"
const value = await Promise.resolve(7);
if (value !== 7) throw new Error(`await returned ${value}`);
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("top-level await");
}

#[test]
fn queued_jobs_run_after_the_entry_settles() {
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_source(
            "memory.js",
            r#"
Promise.resolve().then(() => {
    Promise.reject(new Error("queued job ran"));
});
"#,
        )
        .expect_err("queued rejection");
    let Error::UnhandledRejection { details, .. } = error else {
        panic!("expected an unhandled rejection, got {error:?}");
    };
    assert!(details.contains("queued job ran"), "{details}");
}

#[test]
fn unawaited_dynamic_import_is_drained() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "child.js", "throw new Error('dynamic child ran');\n");
    let main = write(&dir, "main.js", "import('./child.js');\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&main).expect_err("import rejection");
    let Error::UnhandledRejection { details, .. } = error else {
        panic!("expected an unhandled rejection, got {error:?}");
    };
    assert!(details.contains("dynamic child ran"), "{details}");
}

#[test]
fn unhandled_rejections_are_reported_with_their_cause() {
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_source("memory.js", "Promise.reject(new Error('not handled'));\n")
        .expect_err("unhandled rejection");
    let Error::UnhandledRejection { count, details, .. } = error else {
        panic!("expected an unhandled rejection, got {error:?}");
    };
    assert_eq!(count, 1);
    assert!(details.contains("not handled"), "{details}");
}

#[test]
fn a_rejection_handled_later_is_not_reported() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
const rejected = Promise.reject(new Error("handled later"));
Promise.resolve().then(() => rejected.catch(() => {}));
"#,
        )
        .expect("late handler");
}

#[test]
fn a_nonconverging_job_queue_fails_closed() {
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_source(
            "memory.js",
            r#"
function again() {
    Promise.resolve().then(again);
}
again();
"#,
        )
        .expect_err("nonconverging queue");
    assert!(
        matches!(error, Error::NonConvergingJobs { jobs, .. } if jobs == 100_000),
        "{error:?}"
    );
}

#[test]
fn a_throwing_queued_job_fails() {
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_source(
            "memory.js",
            "queueMicrotask(() => { throw new Error('microtask boom'); });\n",
        )
        .expect_err("throwing job");
    let Error::Module { message, .. } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(
        message.contains("uncaught exception in queued job"),
        "{message}"
    );
    assert!(message.contains("microtask boom"), "{message}");
}

#[test]
fn run_source_resolves_relative_imports_from_an_absolute_identity() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "child.js", "console.log('child loaded');\n");
    let identity = dir.path().join("memory.js");
    let identity = identity.to_str().expect("UTF-8 path");

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(identity, "import './child.js';\n")
        .expect("relative import from in-memory entry");
}

#[test]
fn javascript_errors_are_reported() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "main.js", "throw new Error('boom');\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("throwing module");
    let Error::Module { message, .. } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(message.contains("boom"), "{message}");
}

#[test]
fn missing_files_are_reported_before_evaluation() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("missing.js");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&missing).expect_err("missing file");
    assert!(matches!(error, Error::Entry { .. }), "{error:?}");

    // A preflight failure does not consume the runtime.
    let file = write(&dir, "main.js", "export default 1;\n");
    runtime.run_file(&file).expect("runtime still usable");
}

#[test]
fn commonjs_entries_are_rejected_with_the_shared_reason() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "old.cjs", "module.exports = 1;\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("commonjs");
    let Error::UnsupportedEntry { reason, .. } = error else {
        panic!("expected an unsupported entry, got {error:?}");
    };
    assert!(
        reason.contains(".cjs extension is not supported"),
        "{reason}"
    );
}

#[cfg(not(feature = "typescript"))]
#[test]
fn typescript_entries_are_rejected_when_feature_is_disabled() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "main.ts", "const value: number = 1;\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("typescript");
    let Error::UnsupportedEntry { reason, .. } = error else {
        panic!("expected an unsupported entry, got {error:?}");
    };
    assert!(
        reason.contains("TypeScript is not enabled in this build"),
        "{reason}"
    );
}

#[cfg(not(feature = "typescript"))]
#[test]
fn imported_typescript_is_rejected_when_feature_is_disabled() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "value.ts", "export default 1;\n");
    let main = write(&dir, "main.js", "import value from './value.ts';\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&main).expect_err("typescript import");
    let Error::Module { message, .. } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(
        message.contains("TypeScript is not enabled in this build"),
        "{message}"
    );
}

#[cfg(feature = "typescript")]
#[test]
fn runs_erasable_typescript() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(
        &dir,
        "main.ts",
        r#"
interface Point {
    x: number;
    y: number;
}

type Pair<T> = [T, T];

class Counter {
    #count: number = 0;
    step(by: number): number {
        this.#count += by;
        return this.#count;
    }
}

function sum(values: number[]): number {
    return values.reduce((total, value) => total + value, 0);
}

function pick(value: number): number;
function pick(value: string): string;
function pick(value: unknown): unknown {
    return value;
}

const identity = <T,>(value: T): T => value;
const point: Point = { x: 1, y: 2 };
const pair: Pair<number> = [point.x, point.y];
const upper = ("value" as string satisfies string).toUpperCase();
const maybe: number | undefined = pair[0];
const definite: number = maybe! + sum(pair);
const optional = (value?: string): string => value ?? "none";

if (upper !== "VALUE") throw new Error("as/satisfies failed");
if (definite !== 4) throw new Error(`non-null or sum failed: ${definite}`);
if (optional() !== "none") throw new Error("optional parameter failed");
if (identity(7) !== 7) throw new Error("generic arrow failed");
if (pick("x") !== "x") throw new Error("overload failed");
if (new Counter().step(2) !== 2) throw new Error("class field types failed");
const loaded = await Promise.resolve(identity("done"));
if (loaded !== "done") throw new Error("top-level await failed");
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("erasable TypeScript");
}

#[cfg(feature = "typescript")]
#[test]
fn typescript_is_not_type_checked() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(
        &dir,
        "unchecked.ts",
        r#"
const value: number = "text";
const missing = undefined as unknown as string;
if (typeof value !== "string") throw new Error("the value changed");
if (missing !== undefined) throw new Error("the cast added a value");
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&file)
        .expect("a type error must not stop the program");
}

#[cfg(feature = "typescript")]
#[test]
fn rejects_typescript_that_needs_a_transform() {
    let dir = tempfile::tempdir().expect("temp dir");
    let cases = [
        (
            "enum.ts",
            "enum Color { Red }\n",
            "enum is not supported in strip-only mode",
        ),
        (
            "namespace.ts",
            "namespace N { export const x = 1; }\n",
            "namespace declaration is not supported",
        ),
        (
            "parameter.ts",
            "class A { constructor(private x: number) {} }\n",
            "parameter property is not supported",
        ),
        (
            "equals.ts",
            "import value = require(\"./value.js\");\n",
            "import equals declaration is not supported",
        ),
        (
            "assignment.ts",
            "export = 1;\n",
            "export assignment is not supported",
        ),
        (
            "angle.ts",
            "const value = <number>1;\n",
            "angle-bracket syntax",
        ),
        (
            "decorator.ts",
            "function dec() {}\n@dec class A {}\n",
            "valid TypeScript",
        ),
    ];

    for (name, source, expected) in cases {
        let file = write(&dir, name, source);
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_file(&file)
            .expect_err("construct must be refused");
        match error {
            Error::TypeScript {
                path: error_path,
                reason,
            } => {
                assert_eq!(error_path, file);
                assert!(reason.contains(expected), "{name}: {reason}");
            }
            other => panic!("{name}: unexpected error: {other:?}"),
        }
    }
}

#[cfg(feature = "typescript")]
#[test]
fn reports_invalid_typescript_with_a_position() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "invalid.ts", "const value: = 1;\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("invalid syntax must fail");

    match error {
        Error::TypeScript { path, reason } => {
            assert_eq!(path, file);
            assert!(reason.contains("SyntaxError"), "{reason}");
            assert!(reason.contains("invalid.ts:1:"), "{reason}");
            assert!(reason.contains("const value: = 1;"), "{reason}");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[cfg(feature = "typescript")]
#[test]
fn a_typescript_strip_error_leaves_the_runtime_unused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let broken = write(&dir, "broken.ts", "enum E { A }\n");
    let valid = write(&dir, "valid.js", "console.log(\"valid\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&broken)
        .expect_err("unsupported TypeScript must fail");
    assert!(matches!(error, Error::TypeScript { .. }), "{error:?}");

    runtime
        .run_file(&valid)
        .expect("the runtime is usable after a strip failure");
}

#[test]
fn rejects_jsx_commonjs_typescript_and_declarations() {
    let dir = tempfile::tempdir().expect("temp dir");
    let cases = [
        ("view.tsx", "export const x = <div />;\n", "JSX and TSX"),
        ("view.jsx", "export const x = <div />;\n", "JSX and TSX"),
        (
            "old.cts",
            "export = 1;\n",
            "CommonJS TypeScript (.cts) is not supported",
        ),
        (
            "types.d.ts",
            "export declare const x: number;\n",
            "declaration files",
        ),
        (
            "types.d.mts",
            "export declare const x: number;\n",
            "declaration files",
        ),
        (
            "types.d.cts",
            "export declare const x: number;\n",
            "declaration files",
        ),
    ];

    for (name, source, expected) in cases {
        let path = write(&dir, name, source);
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime.run_file(&path).expect_err("file must be refused");
        match error {
            Error::UnsupportedEntry { reason, .. } => {
                assert!(reason.contains(expected), "{name}: {reason}");
            }
            other => panic!("{name}: unexpected error: {other:?}"),
        }
    }
}

#[cfg(feature = "typescript")]
#[test]
fn imports_mix_typescript_and_javascript() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "lib/offset.mjs", "export const OFFSET = 3;\n");
    write(
        &dir,
        "lib/shapes.ts",
        r#"
import { OFFSET } from "./offset.mjs";
export interface Circle {
    radius: number;
}
export function area(radius: number): number {
    return radius * radius + OFFSET;
}
"#,
    );
    write(
        &dir,
        "lib/dynamic.mts",
        r#"
export const twice = (n: number): number => n * 2;
export const label: string = "mts";
"#,
    );
    let ts_main = write(
        &dir,
        "main.ts",
        r#"
import type { Circle } from "./lib/shapes.ts";
import type { NeverLoaded } from "./missing.d.ts";
import { area } from "./lib/shapes.ts";
const dynamic = await import("./lib/dynamic.mts");
const c: Circle = { radius: 3 };
const check = area(c.radius) + dynamic.twice(21);
if (check !== 54 || dynamic.label !== "mts") {
    throw new Error(`unexpected result: ${check} ${dynamic.label}`);
}
"#,
    );
    let js_main = write(
        &dir,
        "entry.js",
        r#"
import { area } from "./lib/shapes.ts";
const dynamic = await import("./lib/dynamic.mts");
if (area(4) !== 19 || dynamic.twice(5) !== 10) {
    throw new Error("JS -> TS/MTS import failed");
}
"#,
    );

    let mut ts_runtime = Runtime::new().expect("runtime");
    ts_runtime
        .run_file(&ts_main)
        .expect("TS -> TS/MJS/MTS imports");

    let mut js_runtime = Runtime::new().expect("runtime");
    js_runtime.run_file(&js_main).expect("JS -> TS/MTS imports");
}

#[cfg(feature = "typescript")]
#[test]
fn an_imported_typescript_error_names_the_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "bad.ts", "export enum Color { Red }\n");
    let main = write(&dir, "main.js", "import { Color } from './bad.ts';\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&main)
        .expect_err("an import that cannot be stripped must fail");
    let Error::Module { message, .. } = error else {
        panic!("expected Error::Module, got {error:?}");
    };
    assert!(
        message.contains("enum is not supported in strip-only mode"),
        "{message}"
    );
    assert!(message.contains("bad.ts"), "{message}");
}

#[cfg(feature = "typescript")]
#[test]
fn typescript_stack_traces_preserve_line_and_column_numbers() {
    let dir = tempfile::tempdir().expect("temp dir");
    let ts_file = write(
        &dir,
        "trace.ts",
        concat!(
            "interface Ignored {\n",
            "    id: string;\n",
            "}\n",
            "function fail(input: Ignored): never {\n",
            "    const tag: string = input.id; throw new Error(`boom:${tag}`);\n",
            "}\n",
            "const item: Ignored = { id: 'x' };\n",
            "try {\n",
            "    fail(item);\n",
            "} catch (error: unknown) {\n",
            "    const stack = (error as Error).stack ?? '';\n",
            "    if (!stack.includes('trace.ts:5:') || !stack.includes('trace.ts:9:')) {\n",
            "        throw new Error(`unexpected stack: ${stack}`);\n",
            "    }\n",
            "    // `: string` before `throw new Error(...)` must remain spaces so the column stays >= 40.\n",
            "    const match = /trace\\.ts:5:(\\d+)/.exec(stack);\n",
            "    if (!match || Number(match[1]) < 40) {\n",
            "        throw new Error(`column shifted in stack: ${stack}`);\n",
            "    }\n",
            "}\n",
        ),
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&ts_file)
        .expect("stack trace line and column numbers must be preserved");
}

#[test]
fn bare_specifiers_are_rejected() {
    let dir = tempfile::tempdir().expect("temp dir");
    let main = write(&dir, "main.js", "import value from 'lodash';\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&main).expect_err("bare specifier");
    let Error::Module { message, .. } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(message.contains("bare module specifiers"), "{message}");
}

#[test]
fn a_runtime_runs_one_entry() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = write(&dir, "first.js", "export default 1;\n");
    let second = write(&dir, "second.js", "export default 2;\n");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&first).expect("first entry");
    let error = runtime.run_file(&second).expect_err("second entry");
    assert!(matches!(error, Error::RuntimeConsumed), "{error:?}");
}

#[test]
fn run_source_accepts_in_memory_modules() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source("memory.js", "export default 1;\n")
        .expect("in-memory module");
}

#[cfg(all(feature = "neo-assert", feature = "neo-test"))]
#[test]
fn run_tests_reports_sync_async_failure_pending_and_unhandled() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
import test from "neo:test";
import assert from "neo:assert";

test("sync pass", () => assert.equals(1 + 1, 2));
test("async pass", async () => {
    await Promise.resolve();
    assert.equals(2 + 2, 4);
});
test("sync fail", () => assert.equals(1, 2));
test("pending case");
test("unhandled", () => {
    Promise.reject(new Error("unhandled in lite test"));
});
"#,
        )
        .expect("run source");
    let report = runtime.run_tests().expect("run tests");
    assert_eq!(report.total, 5);
    assert_eq!(report.passed, 3);
    assert_eq!(report.failed, 1);
    assert_eq!(report.pending, 1);
    assert_eq!(report.unhandled, 1);
    assert_eq!(report.unsettled, 0);
    assert!(!report.all_passed());
    assert!(
        report
            .failures
            .iter()
            .any(|line| line.contains("unhandled in lite test")),
        "{:?}",
        report.failures
    );
}

#[cfg(feature = "neo-test")]
#[test]
fn run_tests_rejects_timeout_option_explicitly_without_timers() {
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_source(
            "memory.js",
            r#"
import test from "neo:test";
test("with timeout", () => {}, { timeout: 50 });
"#,
        )
        .expect_err("timeout option must fail");
    let neojs_lite_core::Error::Module { message, .. } = error else {
        panic!("expected module error, got {error:?}");
    };
    assert!(message.contains("timeout"), "{message}");
    assert!(
        message.contains("timer support is unavailable"),
        "{message}"
    );
}

#[cfg(feature = "neo-test")]
#[test]
fn run_tests_reports_empty_when_test_module_is_unused() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source("memory.js", "const x = 1 + 1;\n")
        .expect("run source");
    let report = runtime.run_tests().expect("run tests");
    assert_eq!(report, neojs_lite_core::TestReport::default());
    assert!(report.all_passed());
}

#[cfg(feature = "neo-assert")]
#[test]
fn assert_observes_native_promises_without_consulting_then_or_species() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
import assert from "neo:assert";

let touched = 0;
class EvilPromise extends Promise {
    static get [Symbol.species]() {
        touched += 1;
        throw new Error("species touched");
    }
    then() {
        touched += 1;
        throw new Error("then touched");
    }
}

const fulfilled = new EvilPromise((resolve) => resolve("ok"));
if (await assert.doesNotReject(() => fulfilled) !== "ok") {
    throw new Error("lost fulfilled subclass value");
}

const rejected = new EvilPromise((_, reject) => reject(new RangeError("bad")));
const reason = await assert.rejects(() => rejected, RangeError);
if (!(reason instanceof RangeError) || reason.message !== "bad") {
    throw new Error("lost rejected subclass reason");
}

if (touched !== 0) {
    throw new Error(`overridden then or species ran ${touched} times`);
}
"#,
        )
        .expect("native promise observation");
}

#[cfg(feature = "neo-test")]
#[test]
fn run_tests_reports_unsettled_for_long_microtask_chain() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
import test from "neo:test";

test("long microtask chain", () => {
    let remaining = 150;
    function step() {
        if (remaining > 0) {
            remaining -= 1;
            Promise.resolve().then(step);
        }
    }
    Promise.resolve().then(step);
});
"#,
        )
        .expect("run source");
    let report = runtime.run_tests().expect("run tests");
    assert_eq!(report.total, 1);
    assert_eq!(report.passed, 1);
    assert_eq!(report.unsettled, 1);
    assert!(!report.all_passed());
    assert!(
        report
            .failures
            .iter()
            .any(|line| line.contains("unsettled work")),
        "{:?}",
        report.failures
    );
}

#[cfg(feature = "neo-test")]
#[test]
fn run_tests_reports_unsettled_for_non_converging_microtask_loop() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
import test from "neo:test";

test("infinite microtask loop", () => {
    function spin() {
        Promise.resolve().then(spin);
    }
    Promise.resolve().then(spin);
});
"#,
        )
        .expect("run source");
    let report = runtime.run_tests().expect("run tests");
    assert_eq!(report.total, 1);
    assert_eq!(report.passed, 1);
    assert_eq!(report.unsettled, 1);
    assert!(!report.all_passed());
    assert!(
        report
            .failures
            .iter()
            .any(|line| line.contains("unsettled work")),
        "{:?}",
        report.failures
    );
}

#[cfg(feature = "neo-test")]
#[test]
fn run_tests_handles_lone_surrogates_in_failure_lines() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
import test from "neo:test";

test("lone \ud800 name", () => {
    throw new Error("lone \ud800 message");
});
"#,
        )
        .expect("run source");
    let report = runtime.run_tests().expect("run tests");
    assert_eq!(report.total, 1);
    assert_eq!(report.failed, 1);
    assert_eq!(report.failures.len(), 1);
    assert!(
        report.failures[0].contains("lone "),
        "{:?}",
        report.failures
    );
    assert!(
        report.failures[0].contains("message"),
        "{:?}",
        report.failures
    );
}

#[cfg(feature = "neo-test")]
#[test]
fn run_tests_fails_clearly_for_unresolved_promise() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
import test from "neo:test";

test("unresolved", () => new Promise(() => {}));
"#,
        )
        .expect("run source");
    let error = runtime
        .run_tests()
        .expect_err("unresolved promise must fail run_tests");
    let Error::TestRunner(message) = error else {
        panic!("expected Error::TestRunner, got {error:?}");
    };
    assert!(message.contains("test runner did not settle"), "{message}");
    assert!(!message.contains("top-level await"), "{message}");
}

#[cfg(feature = "neo-test")]
#[test]
fn run_tests_handles_hostile_promises_thenables_cycles_and_chains() {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source(
            "memory.js",
            r#"
import test from "neo:test";

class EvilPromise extends Promise {}
Object.defineProperty(EvilPromise, Symbol.species, {
    get() { throw new Error("species touched"); },
});
Object.defineProperty(EvilPromise.prototype, "constructor", {
    get() { throw new Error("constructor touched"); },
});
Object.defineProperty(EvilPromise.prototype, "then", {
    get() { throw new Error("then touched"); },
});

Promise.prototype.then = function (fulfilled) {
    if (typeof fulfilled === "function") fulfilled("fake");
    return this;
};

test("fulfilled subclass", () => new EvilPromise((resolve) => resolve("ok")));
test("rejected subclass", () => new EvilPromise((_, reject) => {
    reject(new Error("real subclass rejection"));
}));

const cycle = { then(resolve) { resolve(cycle); } };
test("cyclic thenable", () => cycle);

test("deep thenable chain", () => {
    let value = 1;
    for (let i = 0; i < 1001; i += 1) {
        const next = value;
        value = { then(resolve) { resolve(next); } };
    }
    return value;
});
"#,
        )
        .expect("run source");
    let report = runtime.run_tests().expect("run tests");
    assert_eq!(report.total, 4);
    assert_eq!(report.passed, 1);
    assert_eq!(report.failed, 3);
    assert!(!report.all_passed());
    assert!(
        report
            .failures
            .iter()
            .any(|line| line.contains("real subclass rejection")),
        "{:?}",
        report.failures
    );
    assert!(
        report
            .failures
            .iter()
            .any(|line| line.contains("NEO_TEST_CYCLE")),
        "{:?}",
        report.failures
    );
    assert!(
        report
            .failures
            .iter()
            .any(|line| line.contains("NEO_TEST_CHAIN")),
        "{:?}",
        report.failures
    );
    assert!(
        !report.failures.iter().any(|line| line.contains("touched")),
        "{:?}",
        report.failures
    );
}

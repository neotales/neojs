//! Tests for the embedding API, driven through `Runtime` rather than the binary.

use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

use neojs_core::Error;
use neojs_core::Runtime;

#[test]
fn embedded_runtime_runs_a_module_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, "console.log(\"embedded\");\n").expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("run file");
}

#[test]
fn runs_erasable_typescript() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
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

#[test]
fn typescript_is_not_type_checked() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
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
        let file = write_module(&dir, name, source);
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

#[test]
fn reports_invalid_typescript_with_a_position() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "invalid.ts", "const value: = 1;\n");

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

#[test]
fn a_typescript_strip_error_leaves_the_runtime_unused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let broken = write_module(&dir, "broken.ts", "enum E { A }\n");
    let valid = write_module(&dir, "valid.js", "console.log(\"valid\");\n");

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
        let path = write_module(&dir, name, source);
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

#[test]
fn imports_mix_typescript_and_javascript() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_module(
        &dir,
        "main.js",
        r#"
import { area } from "./lib/shapes.ts";
const dynamic = await import("./lib/dynamic.mts");
console.log(area(3), dynamic.twice(21), dynamic.label);
"#,
    );
    write_module(
        &dir,
        "lib/shapes.ts",
        r#"
import type { Shape } from "./types.ts";
import { scale } from "./scale.js";

export interface NamedShape extends Shape {
    name: string;
}

export function area(shape: Shape): number {
    return scale(shape.width ** 2);
}
"#,
    );
    write_module(
        &dir,
        "lib/types.ts",
        "export interface Shape {\n    width: number;\n}\n",
    );
    write_module(&dir, "lib/scale.js", "export const scale = (n) => n;\n");
    write_module(
        &dir,
        "lib/dynamic.mts",
        r#"
import { label } from "./label.ts";
export const twice = (value: number): number => value * 2;
export { label };
"#,
    );
    write_module(
        &dir,
        "lib/label.ts",
        "export const label: string = \"ok\";\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(dir.path().join("main.js"))
        .expect("mixed module graph");
}

#[test]
fn typescript_imports_javascript_and_other_typescript() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_module(
        &dir,
        "main.mts",
        r#"
import { greet } from "./greet.js";
import type { Options } from "./options.ts";

const options: Options = { name: "world" };
const dynamic = await import("./extra.mts");
if (greet(options.name) !== "hello, world") throw new Error("static import failed");
if (dynamic.double(2) !== 4) throw new Error("dynamic import failed");
"#,
    );
    write_module(
        &dir,
        "greet.js",
        "export function greet(name) { return `hello, ${name}`; }\n",
    );
    write_module(
        &dir,
        "options.ts",
        "export interface Options {\n    name: string;\n}\n",
    );
    write_module(
        &dir,
        "extra.mts",
        "export const double = (value: number): number => value * 2;\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(dir.path().join("main.mts"))
        .expect("TypeScript entry with mixed imports");
}

#[test]
fn type_only_imports_are_erased_before_resolution() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.ts",
        r#"
import type { Missing } from "./missing.ts";
import { type AlsoMissing, value } from "./values.ts";

const used: Missing | null = null;
const alsoUsed: AlsoMissing | null = null;
if (used !== null || alsoUsed !== null) throw new Error("unexpected value");
if (value !== 7) throw new Error("value was not imported");
"#,
    );
    write_module(&dir, "values.ts", "export const value: number = 7;\n");

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&file)
        .expect("type-only imports must not be loaded");
}

#[test]
fn rejects_import_attributes_for_files_and_builtins() {
    let cases = [
        (
            "static file",
            "import value from \"./data.js\" with { type: \"json\" };\n",
            "json",
        ),
        (
            "dynamic file",
            "await import(\"./data.js\", { with: { type: \"json\" } });\n",
            "json",
        ),
        (
            "static builtin",
            "import * as chars from \"chars\" with { type: \"text\" };\n",
            "text",
        ),
        (
            "dynamic builtin",
            "await import(\"chars\", { with: { type: \"bytes\" } });\n",
            "bytes",
        ),
        (
            "custom",
            "await import(\"./data.js\", { with: { type: \"css\" } });\n",
            "css",
        ),
    ];

    for (name, source, kind) in cases {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = write_module(&dir, "main.js", source);
        write_module(&dir, "data.js", "export const value = 1;\n");
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_file(&file)
            .expect_err("import attributes must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("module type attributes are not implemented"),
            "{name}: {message}"
        );
        assert!(
            message.contains(&format!("type: \"{kind}\"")),
            "{name}: {message}"
        );
    }
}

#[test]
fn dynamic_imports_of_files_and_builtins_still_work() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
const data = await import("./data.js");
const chars = await import("chars");
const alias = await import("neo:chars");
if (data.value !== 1) throw new Error("file import failed");
if (alias !== chars) throw new Error("builtin specifiers differ");
if (!chars.isLetter(65)) throw new Error("builtin function failed");
"#,
    );
    write_module(&dir, "data.js", "export const value = 1;\n");

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&file)
        .expect("untyped dynamic imports must keep working");
}

#[test]
fn a_missing_typescript_import_reports_the_specifier() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.ts", "import \"./missing.ts\";\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("missing import must fail");
    let message = error.to_string();

    assert!(message.contains("cannot load"), "{message}");
    assert!(message.contains("missing.ts"), "{message}");
}

#[test]
fn an_imported_typescript_error_names_the_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.ts",
        "import { value } from \"./bad.ts\";\nconsole.log(value);\n",
    );
    write_module(&dir, "bad.ts", "export enum Color { Red }\n");
    let valid = write_module(&dir, "valid.js", "console.log(\"valid\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("an import that cannot be stripped must fail");
    let message = error.to_string();

    assert!(message.contains("bad.ts"), "{message}");
    assert!(
        message.contains("enum is not supported in strip-only mode"),
        "{message}"
    );
    let error = runtime
        .run_file(&valid)
        .expect_err("a failed import must consume the runtime");
    assert!(matches!(error, Error::RuntimeConsumed), "{error:?}");
}

#[test]
fn reports_a_missing_entry_with_its_path() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("missing.js");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("missing file must fail");
    let message = error.to_string();

    assert!(message.contains("cannot read"), "{message}");
    assert!(message.contains("missing.js"), "{message}");
}

#[test]
fn run_file_works_inside_tokio_through_the_async_form() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, "console.log(\"async\");\n").expect("write source");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        let mut runtime = Runtime::new().expect("neojs runtime");
        runtime.run_file_async(&file).await.expect("run file");
    });
}

#[test]
fn sync_entry_refuses_to_block_inside_tokio() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        let mut runtime = Runtime::new().expect("neojs runtime");
        let error = runtime
            .run_file("unused.js")
            .expect_err("blocking must be refused");
        assert!(matches!(error, Error::BlockingInAsyncContext), "{error:?}");
    });
}

#[test]
fn run_file_async_works_without_an_ambient_tokio_runtime() {
    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "this test must run outside Tokio"
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.ts",
        r#"
const chars = await import("chars");
const value: number = await Promise.resolve(41);
if (value + 1 !== 42) throw new Error("top-level await failed");
if (!chars.isLetter("A".codePointAt(0))) throw new Error("dynamic builtin failed");
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    block_on_std(runtime.run_file_async(&file)).expect("async entry outside Tokio");
}

#[test]
fn a_runtime_created_inside_tokio_runs_outside_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", "console.log(\"outside\");\n");

    let caller = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    let mut runtime = caller.block_on(async { Runtime::new().expect("neojs runtime") });
    drop(caller);

    runtime
        .run_file(&file)
        .expect("the owned driver must not depend on the creating runtime");
}

#[test]
fn a_runtime_created_outside_tokio_runs_async_inside_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.ts",
        "const value: number = await Promise.resolve(7);\nif (value !== 7) throw new Error(\"bad\");\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    let caller = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    caller.block_on(async {
        runtime
            .run_file_async(&file)
            .await
            .expect("async entry on the caller's runtime");
    });
}

#[test]
fn a_runtime_can_be_created_and_dropped_inside_tokio() {
    let caller = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    caller.block_on(async {
        for _ in 0..3 {
            let runtime = Runtime::new().expect("neojs runtime");
            drop(runtime);
        }
    });
}

#[test]
fn runtime_lifecycle_repeats() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.ts",
        r#"
const values: number[] = Array.from({ length: 1000 }, (_, index) => index);
if (values.length !== 1000) throw new Error("values were not built");
"#,
    );

    for round in 0..3 {
        let mut runtime = Runtime::new().expect("runtime");
        if round % 2 == 0 {
            runtime.run_file(&file).expect("sync entry");
        } else {
            block_on_std(runtime.run_file_async(&file)).expect("async entry");
        }
    }
}

#[test]
fn a_never_settling_entry_is_reported_and_consumes_the_runtime() {
    let dir = tempfile::tempdir().expect("temp dir");
    let pending = write_module(&dir, "pending.js", "await new Promise(() => {});\n");
    let valid = write_module(&dir, "valid.js", "console.log(\"valid\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = block_on_std(runtime.run_file_async(&pending))
        .expect_err("a module that never settles must fail");
    assert!(error.to_string().contains("never resolved"), "{error:?}");

    let error = runtime
        .run_file(&valid)
        .expect_err("the failed entry must consume the runtime");
    assert!(matches!(error, Error::RuntimeConsumed), "{error:?}");
}

/// A waker that unparks the thread waiting in [`block_on_std`].
struct WakeSignal(mpsc::Sender<()>);

impl Wake for WakeSignal {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
}

/// Polls a future to completion on the current thread, without a Tokio runtime.
fn block_on_std<F: Future>(future: F) -> F::Output {
    let (sender, receiver) = mpsc::channel();
    let waker = Waker::from(Arc::new(WakeSignal(sender)));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => receiver.recv().expect("the future must wake the thread"),
        }
    }
}

fn write_module(dir: &tempfile::TempDir, name: &str, source: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(&path, source).expect("write source");
    path
}

/// Runs a JavaScript module whose own assertions throw on failure.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", source);
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn built_in_module_lookup_is_cached_and_frozen() {
    run_js(
        r#"
const first = Neo.getBuiltInModule("chars");
const second = Neo.getBuiltInModule("chars");
if (first !== second) throw new Error("chars lookup is not cached");
if (typeof first !== "object" || first === null) throw new Error("chars is not an object");
if (!Object.isFrozen(first)) throw new Error("chars module is not frozen");
for (const name of ["isChar", "isAscii", "isLatin1", "toLower", "toUpper"]) {
    if (typeof first[name] !== "function") throw new Error(`${name} is missing`);
}
"#,
    );
}

#[test]
fn built_in_lookup_ignores_a_replaced_map_get() {
    run_js(
        r#"
const chars = Neo.getBuiltInModule("chars");
const originalGet = Map.prototype.get;
let replacedCalls = 0;
Map.prototype.get = function (key) {
    replacedCalls += 1;
    return this;
};
try {
    if (Neo.getBuiltInModule("chars") !== chars) {
        throw new Error("known lookup did not return the cached module");
    }
    if (Neo.getBuiltInModule("missing") !== undefined) {
        throw new Error("unknown lookup exposed the registry");
    }
    if (Neo.getBuiltInModule("__proto__") !== undefined) {
        throw new Error("inherited name exposed the registry");
    }
    if (replacedCalls !== 0) {
        throw new Error("lookup called the replaced Map.prototype.get");
    }
} finally {
    Map.prototype.get = originalGet;
}
"#,
    );
}

#[test]
fn unknown_built_in_modules_return_undefined() {
    run_js(
        r#"
const names = ["", "fs", "neo:chars", "constructor", "__proto__", "toString"];
for (const name of names) {
    const module = Neo.getBuiltInModule(name);
    if (module !== undefined) throw new Error(`unknown module ${name} was returned`);
}
"#,
    );
}

#[test]
fn built_in_module_name_must_be_a_string() {
    run_js(
        r#"
const values = [undefined, null, 1, {}, [], Symbol("chars")];
for (const value of values) {
    let error;
    try {
        Neo.getBuiltInModule(value);
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof TypeError)) throw new Error("expected TypeError");
}
"#,
    );
}

/// Runs a JavaScript module that is expected to fail, returning the error text.
fn run_js_error(source: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", source);
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("module must fail");
    error.to_string()
}

#[test]
fn built_in_module_named_exports_are_the_registry_functions() {
    run_js(
        r#"
import * as chars from "chars";
const registry = Neo.getBuiltInModule("chars");
const exported = Object.keys(chars).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error(`export list ${exported} does not match ${registered}`);
}
for (const name of registered) {
    if (typeof chars[name] !== "function") throw new Error(`${name} is not a function`);
    if (chars[name] !== registry[name]) throw new Error(`${name} is not the registry function`);
}
if ("default" in chars) throw new Error("chars must not have a default export");
"#,
    );
}

#[test]
fn built_in_module_specifiers_share_one_module_instance() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_module(
        &dir,
        "main.js",
        r#"
import * as staticChars from "chars";
import { isLetter as throughModule } from "./consumer.js";

const first = await import("chars");
const alias = await import("neo:chars");
const uppercase = await import("NEO:chars");
const again = await import("chars");

if (staticChars !== first) throw new Error("static and dynamic import differ");
if (first !== alias) throw new Error("chars and neo:chars differ");
if (alias !== uppercase) throw new Error("scheme matching is not case-insensitive");
if (alias !== again) throw new Error("repeated dynamic import was not cached");
if (first.isLetter !== staticChars.isLetter) throw new Error("namespace export differs");
if (throughModule !== staticChars.isLetter) {
    throw new Error("an importer of neo:chars saw a different function");
}

if (import.meta.resolve("chars") !== "neo:chars") throw new Error("chars did not resolve canonically");
if (import.meta.resolve("neo:chars") !== "neo:chars") throw new Error("neo:chars did not resolve canonically");
"#,
    );
    write_module(
        &dir,
        "consumer.js",
        "export { isLetter } from \"neo:chars\";\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(dir.path().join("main.js"))
        .expect("module assertions");
}

#[test]
fn built_in_module_namespace_and_registry_are_immutable() {
    run_js(
        r#"
import * as chars from "chars";
const registry = Neo.getBuiltInModule("chars");

let rejections = 0;
try {
    chars.isLetter = () => "namespace";
} catch (error) {
    if (!(error instanceof TypeError)) throw error;
    rejections += 1;
}
try {
    registry.isLetter = () => "registry";
} catch (error) {
    if (!(error instanceof TypeError)) throw error;
    rejections += 1;
}
if (rejections !== 2) throw new Error("assignment did not fail in both paths");

if (chars.isLetter !== registry.isLetter) throw new Error("function identity changed");
if (chars.isLetter("A".codePointAt(0)) !== true) throw new Error("function was replaced");

// The function objects are shared, so properties added to one are visible
// through the other. The module object and the namespace cannot be replaced.
chars.isLetter.marker = 42;
if (registry.isLetter.marker !== 42) throw new Error("function objects are not shared");
delete chars.isLetter.marker;
"#,
    );
}

#[test]
fn chars_predicates_ignore_a_replaced_regexp_exec() {
    run_js(
        r#"
import * as chars from "chars";
const registry = Neo.getBuiltInModule("chars");

const predicates = [
    "isControl", "isDigit", "isLetter", "isLetterOrDigit", "isLower",
    "isUpper", "isPunc", "isSpace", "isSymbol",
];
const values = [
    0x00, 0x09, 0x0A, 0x20, 0x21, 0x30, 0x41, 0x61, 0x85, 0xA0,
    0x3BB, 0x1F600, 0xD800, "A", NaN,
];
const cases = [];
for (const name of predicates) {
    for (const value of values) cases.push([name, value]);
}
const baseline = cases.map(([name, value]) => chars[name](value));

const originalExec = RegExp.prototype.exec;
const overrides = [
    ["return null", function () { return null; }],
    ["throw", function () { throw new Error("replaced exec was called"); }],
];
for (const [description, replacement] of overrides) {
    RegExp.prototype.exec = replacement;
    try {
        for (let index = 0; index < cases.length; index += 1) {
            const [name, value] = cases[index];
            for (const target of [chars, registry]) {
                let actual;
                try {
                    actual = target[name](value);
                } catch (error) {
                    throw new Error(
                        `${name}(${String(value)}) threw when exec would ${description}: ${error}`,
                    );
                }
                if (actual !== baseline[index]) {
                    throw new Error(
                        `${name}(${String(value)}) changed when exec would ${description}`,
                    );
                }
            }
        }
    } finally {
        RegExp.prototype.exec = originalExec;
    }
}

if (RegExp.prototype.exec !== originalExec) throw new Error("exec was not restored");
if (chars.isLetter("A".codePointAt(0)) !== true) throw new Error("classification changed after restore");
"#,
    );
}

#[test]
fn unknown_built_in_exports_fail_to_link() {
    let bare = run_js_error("import { nope } from \"chars\";\n");
    assert!(
        bare.contains("does not provide an export named 'nope'"),
        "{bare}"
    );
    assert!(bare.contains("chars"), "{bare}");

    let alias = run_js_error("import { nope } from \"neo:chars\";\n");
    assert!(
        alias.contains("does not provide an export named 'nope'"),
        "{alias}"
    );
    assert!(alias.contains("neo:chars"), "{alias}");
}

#[test]
fn local_modules_are_not_shadowed_by_built_in_names() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_module(
        &dir,
        "main.js",
        r#"
import { value } from "./chars.js";
import { value as extensionless } from "./chars";
if (value !== 7) throw new Error("relative chars.js was not loaded");
if (extensionless !== 8) throw new Error("extensionless ./chars was not loaded");
"#,
    );
    write_module(&dir, "chars.js", "export const value = 7;\n");
    write_module(&dir, "chars", "export const value = 8;\n");

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(dir.path().join("main.js"))
        .expect("local modules");
}

#[test]
fn chars_predicates_use_scalar_code_points() {
    run_js(
        r#"
const chars = Neo.getBuiltInModule("chars");
const predicates = [
    "isChar", "isAscii", "isLatin1", "isControl", "isDigit", "isLetter",
    "isLetterOrDigit", "isLower", "isUpper", "isPunc", "isSpace", "isSymbol",
];
const invalid = ["A", null, undefined, NaN, Infinity, -1, 1.5, 0x110000, 0xD800, 0xDFFF];
for (const predicate of predicates) {
    for (const value of invalid) {
        if (chars[predicate](value) !== false) {
            throw new Error(`${predicate}(${String(value)}) should be false`);
        }
    }
}

const cases = [
    ["isChar", 0, true],
    ["isChar", 0x10FFFF, true],
    ["isChar", 0xD800, false],
    ["isAscii", 0x7F, true],
    ["isAscii", 0x80, false],
    ["isLatin1", 0xFF, true],
    ["isLatin1", 0x100, false],
    ["isControl", 0x0A, true],
    ["isControl", 0x7F, true],
    ["isControl", 0x85, true],
    ["isControl", 0x9F, true],
    ["isControl", 0xA0, false],
    ["isDigit", 0x30, true],
    ["isDigit", 0x0663, true],
    ["isDigit", 0x2168, false],
    ["isLetter", 0x41, true],
    ["isLetter", 0x03BB, true],
    ["isLetter", 0x0416, true],
    ["isLetter", 0x30, false],
    ["isLetterOrDigit", 0x30, true],
    ["isLetterOrDigit", 0x41, true],
    ["isLetterOrDigit", 0x5F, false],
    ["isLower", 0x61, true],
    ["isLower", 0x03B1, true],
    ["isLower", 0x41, false],
    ["isUpper", 0x41, true],
    ["isUpper", 0x0391, true],
    ["isUpper", 0x61, false],
    ["isPunc", 0x21, true],
    ["isPunc", 0x2C, true],
    ["isPunc", 0x24, false],
    ["isSpace", 0x20, true],
    ["isSpace", 0x09, true],
    ["isSpace", 0x85, true],
    ["isSpace", 0xA0, true],
    ["isSpace", 0x1680, true],
    ["isSpace", 0x3000, true],
    ["isSpace", 0x200B, false],
    ["isSpace", 0xFEFF, false],
    ["isSymbol", 0x2B, true],
    ["isSymbol", 0x20AC, true],
    ["isSymbol", 0x1F600, true],
    ["isSymbol", 0x41, false],
];
for (const [predicate, value, expected] of cases) {
    const actual = chars[predicate](value);
    if (actual !== expected) {
        throw new Error(`${predicate}(${value}) was ${actual}, expected ${expected}`);
    }
}
"#,
    );
}

#[test]
fn chars_case_mapping_returns_one_code_point_or_the_input() {
    run_js(
        r#"
const chars = Neo.getBuiltInModule("chars");
const cases = [
    ["toLower", 0x00, 0x00],
    ["toUpper", 0x00, 0x00],
    ["toLower", 0x41, 0x61],
    ["toLower", 0x61, 0x61],
    ["toUpper", 0x61, 0x41],
    ["toUpper", 0x41, 0x41],
    ["toLower", 0x0391, 0x03B1],
    ["toUpper", 0x03B1, 0x0391],
    ["toLower", 0x1E9E, 0x00DF],
    // Full mappings that expand to two code points are left unchanged.
    ["toLower", 0x0130, 0x0130],
    ["toUpper", 0x00DF, 0x00DF],
];
for (const [method, value, expected] of cases) {
    const actual = chars[method](value);
    if (actual !== expected) {
        throw new Error(`${method}(${value}) was ${actual}, expected ${expected}`);
    }
}

for (const method of ["toLower", "toUpper"]) {
    for (const value of ["A", null, undefined, NaN, Infinity, -1, 1.5, 0x110000, 0xD800, 0xDFFF]) {
        let error;
        try {
            chars[method](value);
        } catch (thrown) {
            error = thrown;
        }
        const expected = typeof value === "number" ? RangeError : TypeError;
        if (!(error instanceof expected)) {
            throw new Error(`${method}(${String(value)}) did not throw ${expected.name}`);
        }
    }
}
"#,
    );
}

#[test]
fn console_module_exports_the_global_console_and_its_methods() {
    run_js(
        r#"
import * as consoleModule from "console";
import defaultConsole from "console";

const registry = Neo.getBuiltInModule("console");

if (registry !== globalThis.console) throw new Error("registry is not the global console");
if (defaultConsole !== globalThis.console) throw new Error("default export is not the global console");
if (consoleModule.default !== globalThis.console) {
    throw new Error("namespace default is not the global console");
}
if (Object.isFrozen(registry)) throw new Error("the live console must not be frozen");
if (Neo.getBuiltInModule("neo:console") !== undefined) {
    throw new Error("the registry takes bare names only");
}

const exports = Object.keys(consoleModule).sort();
const expected = ["debug", "default", "error", "info", "log", "warn"];
if (JSON.stringify(exports) !== JSON.stringify(expected)) {
    throw new Error(`console exports ${exports} do not match ${expected}`);
}
for (const name of ["log", "debug", "info", "warn", "error"]) {
    if (typeof consoleModule[name] !== "function") throw new Error(`${name} is not a function`);
    if (consoleModule[name] !== globalThis.console[name]) {
        throw new Error(`${name} is not the global console method`);
    }
    if (registry[name] !== consoleModule[name]) {
        throw new Error(`${name} differs between the registry and the module`);
    }
}
"#,
    );
}

#[test]
fn console_module_specifiers_share_one_module_instance() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_module(
        &dir,
        "main.js",
        r#"
import * as staticConsole from "console";
import { log as throughModule } from "./consumer.js";

const first = await import("console");
const alias = await import("neo:console");
const uppercase = await import("NEO:console");
const again = await import("console");

if (staticConsole !== first) throw new Error("static and dynamic import differ");
if (first !== alias) throw new Error("console and neo:console differ");
if (alias !== uppercase) throw new Error("scheme matching is not case-insensitive");
if (alias !== again) throw new Error("repeated dynamic import was not cached");
if (first.default !== staticConsole.default) throw new Error("namespace default differs");
if (throughModule !== staticConsole.log) {
    throw new Error("an importer of neo:console saw a different function");
}
if (import.meta.resolve("console") !== "neo:console") {
    throw new Error("console did not resolve canonically");
}
if (import.meta.resolve("neo:console") !== "neo:console") {
    throw new Error("neo:console did not resolve canonically");
}
"#,
    );
    write_module(
        &dir,
        "consumer.js",
        "export { log } from \"neo:console\";\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(dir.path().join("main.js"))
        .expect("module assertions");
}

#[test]
fn console_module_default_export_is_live_and_named_exports_are_captured() {
    run_js(
        r#"
import defaultConsole, { log } from "console";
import * as namespace from "console";

const registry = Neo.getBuiltInModule("console");
const original = defaultConsole.log;
let replacedCalls = 0;
const replacement = () => {
    replacedCalls += 1;
};

defaultConsole.log = replacement;
try {
    if (globalThis.console.log !== replacement) {
        throw new Error("replacing through the default export did not change the global");
    }
    if (registry.log !== replacement) {
        throw new Error("the registry did not see the replacement");
    }
    if (namespace.default.log !== replacement) {
        throw new Error("the namespace default did not see the replacement");
    }
    if (log !== original) throw new Error("the named export followed the replacement");
    if (namespace.log !== original) throw new Error("the namespace export followed the replacement");

    globalThis.console.log("not printed");
    if (replacedCalls !== 1) throw new Error("the replacement was not called");
} finally {
    defaultConsole.log = original;
}

if (globalThis.console.log !== original) throw new Error("the original method was not restored");
if (registry.log !== original) throw new Error("the registry did not see the restore");

let rejections = 0;
try {
    namespace.log = replacement;
} catch (error) {
    if (!(error instanceof TypeError)) throw error;
    rejections += 1;
}
if (rejections !== 1) throw new Error("assigning to the namespace did not throw TypeError");

// The function objects are shared, so properties added to one are visible
// through every reference.
log.marker = 42;
if (globalThis.console.log.marker !== 42) throw new Error("function objects are not shared");
if (registry.log.marker !== 42) throw new Error("the registry is not sharing the function");
delete log.marker;
if ("marker" in original) throw new Error("the marker was not removed");
"#,
    );
}

#[test]
fn console_registry_keeps_the_installed_console() {
    run_js(
        r#"
import defaultConsole from "console";

const installed = globalThis.console;
const registry = Neo.getBuiltInModule("console");
const replacement = { log() {} };

globalThis.console = replacement;
try {
    if (globalThis.console !== replacement) throw new Error("the global was not replaced");
    if (registry !== installed) throw new Error("the registry changed with the global property");
    if (defaultConsole !== installed) throw new Error("the default export changed with the global property");
    if (Neo.getBuiltInModule("console") !== installed) {
        throw new Error("a later lookup changed with the global property");
    }
} finally {
    globalThis.console = installed;
}
if (Neo.getBuiltInModule("console") !== globalThis.console) {
    throw new Error("the restore did not rejoin the registry and the global");
}
"#,
    );
}

#[test]
fn console_named_exports_capture_a_pre_evaluation_replacement() {
    run_js(
        r#"
const installed = Neo.getBuiltInModule("console");
const original = installed.log;
let calls = 0;
const replacement = () => {
    calls += 1;
};

// The console module has not been imported yet, so this replacement is in place
// before its first evaluation.
installed.log = replacement;
try {
    if (globalThis.console.log !== replacement) {
        throw new Error("the installed console was not changed");
    }
    const first = await import("console");
    const alias = await import("neo:console");

    if (first.default !== installed) throw new Error("the default export is not the installed object");
    if (first.log !== replacement) throw new Error("the named export did not capture the replacement");
    if (alias.log !== replacement) throw new Error("the alias captured a different function");
    if (first.log === original) throw new Error("the named export kept the installed method");

    first.log();
    if (calls !== 1) throw new Error("the captured replacement was not callable");
} finally {
    installed.log = original;
}

// Restoring the property does not rewrite the already-evaluated module binding.
const later = await import("console");
if (later.log !== replacement) throw new Error("the captured binding changed after restore");
if (later.default.log !== original) {
    throw new Error("the default export did not follow the restored property");
}
"#,
    );
}

#[test]
fn console_module_does_not_follow_a_global_replacement_before_import() {
    run_js(
        r#"
const installed = Neo.getBuiltInModule("console");
const installedLog = installed.log;
const replacementConsole = {
    log() {
        throw new Error("the replacement console was used");
    },
    error() {
        throw new Error("the replacement console was used");
    },
};

// Replace the whole global property before the console module is first
// evaluated. The registry and the module still have to use the installed
// console object, not this replacement.
globalThis.console = replacementConsole;
try {
    const first = await import("console");
    const alias = await import("neo:console");

    if (Neo.getBuiltInModule("console") !== installed) {
        throw new Error("the registry followed the global property");
    }
    if (first.default !== installed) throw new Error("the default export followed the global property");
    if (alias.default !== installed) throw new Error("the alias default followed the global property");
    if (first.log !== installedLog) {
        throw new Error("the named export did not come from the installed console");
    }
    if (first.log === globalThis.console.log) {
        throw new Error("the named export came from the replacement global");
    }
    if (first.default === globalThis.console) {
        throw new Error("the default export is the replacement global");
    }
} finally {
    globalThis.console = installed;
}

if (Neo.getBuiltInModule("console") !== globalThis.console) {
    throw new Error("the restore did not rejoin the registry and the global");
}
"#,
    );
}

#[test]
fn unknown_console_exports_fail_to_link() {
    let bare = run_js_error("import { nope } from \"console\";\n");
    assert!(
        bare.contains("does not provide an export named 'nope'"),
        "{bare}"
    );
    assert!(bare.contains("console"), "{bare}");

    let alias = run_js_error("import { nope } from \"neo:console\";\n");
    assert!(
        alias.contains("does not provide an export named 'nope'"),
        "{alias}"
    );
    assert!(alias.contains("neo:console"), "{alias}");
}

#[test]
fn console_module_is_importable_from_typescript() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.ts",
        r#"
import defaultConsole, { log, warn } from "neo:console";

interface Check {
    identity: boolean;
}

const check: Check = {
    identity:
        log === globalThis.console.log &&
        warn === globalThis.console.warn &&
        defaultConsole === globalThis.console,
};
if (check.identity !== true) throw new Error("console identity failed from TypeScript");
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&file)
        .expect("TypeScript imports the console module");
}

#[test]
fn a_local_module_named_console_is_not_shadowed() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_module(
        &dir,
        "main.js",
        r#"
import { value } from "./console.js";
import * as builtin from "console";
if (value !== 7) throw new Error("relative console.js was not loaded");
if (builtin.default !== globalThis.console) throw new Error("the built-in was not loaded");
"#,
    );
    write_module(&dir, "console.js", "export const value = 7;\n");

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(dir.path().join("main.js"))
        .expect("local module");
}

#[test]
fn runs_only_one_entry_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = write_module(&dir, "first.js", "console.log(\"first\");\n");
    let second = write_module(&dir, "second.js", "console.log(\"second\");\n");
    let typed = dir.path().join("typed.ts");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&first).expect("first entry");

    let error = runtime
        .run_file(&second)
        .expect_err("second entry must fail");
    assert!(matches!(error, Error::RuntimeConsumed), "{error:?}");

    // The consumed check comes before entry validation and file access.
    let error = runtime
        .run_file(&typed)
        .expect_err("a consumed runtime must fail regardless of the path");
    assert!(matches!(error, Error::RuntimeConsumed), "{error:?}");
}

#[test]
fn a_failed_load_consumes_the_runtime() {
    let dir = tempfile::tempdir().expect("temp dir");
    let broken = write_module(&dir, "broken.js", "import \"./missing.js\";\n");
    let valid = write_module(&dir, "valid.js", "console.log(\"valid\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&broken)
        .expect_err("broken module must fail");
    assert!(matches!(error, Error::Core(_)), "{error:?}");

    let error = runtime
        .run_file(&valid)
        .expect_err("a failed load must consume the runtime");
    assert!(matches!(error, Error::RuntimeConsumed), "{error:?}");

    let mut fresh = Runtime::new().expect("fresh runtime");
    fresh
        .run_file(&valid)
        .expect("fresh runtime runs the valid file");
}

#[test]
fn a_preflight_failure_leaves_the_runtime_unused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("missing.js");
    let valid = write_module(&dir, "valid.js", "console.log(\"valid\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&missing)
        .expect_err("missing entry must fail");
    assert!(matches!(error, Error::Entry { .. }), "{error:?}");

    runtime
        .run_file(&valid)
        .expect("the runtime is usable after a preflight failure");
}

#[test]
fn an_unsupported_entry_leaves_the_runtime_unused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let typed = dir.path().join("view.tsx");
    let valid = write_module(&dir, "valid.js", "console.log(\"valid\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime
        .run_file(&typed)
        .expect_err("unsupported entry must fail");
    assert!(matches!(error, Error::UnsupportedEntry { .. }), "{error:?}");

    runtime
        .run_file(&valid)
        .expect("the runtime is usable after an unsupported entry");
}

#[test]
fn the_async_entry_point_is_also_single_use() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = write_module(&dir, "first.js", "console.log(\"first\");\n");
    let second = write_module(&dir, "second.js", "console.log(\"second\");\n");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        let mut runtime = Runtime::new().expect("neojs runtime");
        runtime.run_file_async(&first).await.expect("first entry");
        let error = runtime
            .run_file_async(&second)
            .await
            .expect_err("second entry must fail");
        assert!(matches!(error, Error::RuntimeConsumed), "{error:?}");
    });
}

#[test]
fn consumed_check_precedes_the_tokio_guard() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = write_module(&dir, "first.js", "console.log(\"first\");\n");
    let second = write_module(&dir, "second.js", "console.log(\"second\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&first).expect("first entry");

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    tokio_runtime.block_on(async {
        let error = runtime
            .run_file(&second)
            .expect_err("a consumed runtime must fail even inside Tokio");
        assert!(matches!(error, Error::RuntimeConsumed), "{error:?}");
    });
}

#[test]
fn an_unpolled_async_entry_leaves_the_runtime_unused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let first = write_module(&dir, "first.js", "console.log(\"first\");\n");
    let second = write_module(&dir, "second.js", "console.log(\"second\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    {
        let _unpolled = runtime.run_file_async(&first);
    }

    runtime
        .run_file(&second)
        .expect("an unpolled future must not consume the runtime");
}

#[test]
fn an_async_preflight_failure_leaves_the_runtime_unused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("missing.js");
    let valid = write_module(&dir, "valid.js", "console.log(\"valid\");\n");

    let tokio_runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    tokio_runtime.block_on(async {
        let mut runtime = Runtime::new().expect("neojs runtime");
        let error = runtime
            .run_file_async(&missing)
            .await
            .expect_err("missing entry must fail");
        assert!(matches!(error, Error::Entry { .. }), "{error:?}");

        runtime
            .run_file_async(&valid)
            .await
            .expect("the runtime is usable after an async preflight failure");
    });
}

//! Tests for the embedding API, driven through `Runtime` rather than the binary.

use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

use vaeljs_core::Error;
use vaeljs_core::Runtime;

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

#[cfg(feature = "vael-chars")]
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
            "import * as chars from \"vael:chars\" with { type: \"text\" };\n",
            "text",
        ),
        (
            "dynamic builtin",
            "await import(\"vael:chars\", { with: { type: \"bytes\" } });\n",
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

#[cfg(feature = "vael-chars")]
#[test]
fn dynamic_imports_of_files_and_builtins_still_work() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
const data = await import("./data.js");
const chars = await import("vael:chars");
const alias = await import("vael:chars");
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
        let mut runtime = Runtime::new().expect("vaeljs runtime");
        runtime.run_file_async(&file).await.expect("run file");
    });
}

#[test]
fn sync_entry_refuses_to_block_inside_tokio() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        let mut runtime = Runtime::new().expect("vaeljs runtime");
        let error = runtime
            .run_file("unused.js")
            .expect_err("blocking must be refused");
        assert!(matches!(error, Error::BlockingInAsyncContext), "{error:?}");
    });
}

#[cfg(feature = "vael-chars")]
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
const chars = await import("vael:chars");
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
    let mut runtime = caller.block_on(async { Runtime::new().expect("vaeljs runtime") });
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
            let runtime = Runtime::new().expect("vaeljs runtime");
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

#[cfg(feature = "vael-chars")]
#[test]
fn built_in_module_lookup_is_cached_and_frozen() {
    run_js(
        r#"
const first = Vael.getBuiltInModule("vael:chars");
const second = Vael.getBuiltInModule("vael:chars");
if (first !== second) throw new Error("chars lookup is not cached");
if (typeof first !== "object" || first === null) throw new Error("chars is not an object");
if (!Object.isFrozen(first)) throw new Error("chars module is not frozen");
for (const name of ["isChar", "isAscii", "isLatin1", "toLower", "toUpper"]) {
    if (typeof first[name] !== "function") throw new Error(`${name} is missing`);
}
"#,
    );
}

#[cfg(feature = "vael-chars")]
#[test]
fn built_in_lookup_ignores_a_replaced_map_get() {
    run_js(
        r#"
const chars = Vael.getBuiltInModule("vael:chars");
const originalGet = Map.prototype.get;
let replacedCalls = 0;
Map.prototype.get = function (key) {
    replacedCalls += 1;
    return this;
};
try {
    if (Vael.getBuiltInModule("vael:chars") !== chars) {
        throw new Error("known lookup did not return the cached module");
    }
    if (Vael.getBuiltInModule("missing") !== undefined) {
        throw new Error("unknown lookup exposed the registry");
    }
    if (Vael.getBuiltInModule("__proto__") !== undefined) {
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
const names = ["", "fs", "chars", "constructor", "__proto__", "toString"];
for (const name of names) {
    const module = Vael.getBuiltInModule(name);
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
        Vael.getBuiltInModule(value);
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

#[cfg(feature = "vael-chars")]
#[test]
fn built_in_module_named_exports_are_the_registry_functions() {
    run_js(
        r#"
import * as chars from "vael:chars";
const registry = Vael.getBuiltInModule("vael:chars");
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

#[cfg(feature = "vael-chars")]
#[test]
fn built_in_module_specifiers_share_one_module_instance() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_module(
        &dir,
        "main.js",
        r#"
import * as staticChars from "vael:chars";
import { isLetter as throughModule } from "./consumer.js";

const first = await import("vael:chars");
const uppercase = await import("VAEL:chars");
const again = await import("vael:chars");

if (staticChars !== first) throw new Error("static and dynamic import differ");
if (first !== uppercase) throw new Error("scheme matching is not case-insensitive");
if (first !== again) throw new Error("repeated dynamic import was not cached");
if (first.isLetter !== staticChars.isLetter) throw new Error("namespace export differs");
if (throughModule !== staticChars.isLetter) {
    throw new Error("an importer of vael:chars saw a different function");
}

if (import.meta.resolve("vael:chars") !== "vael:chars") {
    throw new Error("vael:chars did not resolve canonically");
}
if (import.meta.resolve("VAEL:chars") !== "vael:chars") {
    throw new Error("VAEL:chars did not resolve canonically");
}

let bareRejected = false;
try {
    await import("chars");
} catch (error) {
    bareRejected = error instanceof TypeError;
}
if (!bareRejected) throw new Error("bare chars import was accepted");
"#,
    );
    write_module(
        &dir,
        "consumer.js",
        "export { isLetter } from \"vael:chars\";\n",
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(dir.path().join("main.js"))
        .expect("module assertions");
}

#[cfg(feature = "vael-chars")]
#[test]
fn malformed_builtin_specifiers_do_not_resolve_or_import() {
    run_js(
        r#"
// The URL parser trims leading/trailing C0 controls and space and removes
// embedded tab/newline characters. A module specifier must not inherit that:
// only the scheme is case-insensitive and the path is matched byte for byte.
const malformed = [
    " vael:chars",
    "vael:chars ",
    " vael:chars ",
    "\tvael:chars",
    "vael:chars\t",
    "\nvael:chars",
    "vael:chars\n",
    "\rvael:chars",
    "vael:chars\r",
    "vael:ch ars",
    "vael:ch\tars",
    "vael:ch\nars",
    "vael:ch\rars",
    "vael:\u0000chars",
    "vael:ch\u0001ars",
    "vael:chars?x",
    "vael:chars#x",
];

for (const specifier of malformed) {
    let resolved;
    try {
        resolved = import.meta.resolve(specifier);
    } catch {
        resolved = undefined;
    }
    if (resolved !== undefined) {
        throw new Error(`resolve accepted malformed ${JSON.stringify(specifier)} as ${resolved}`);
    }

    let imported = false;
    try {
        await import(specifier);
        imported = true;
    } catch {
        imported = false;
    }
    if (imported) {
        throw new Error(`dynamic import accepted malformed ${JSON.stringify(specifier)}`);
    }
}

// Scheme-only case folding is still supported, and it reaches the one module.
if (import.meta.resolve("VAEL:chars") !== "vael:chars") {
    throw new Error("VAEL:chars did not resolve canonically");
}
const upper = await import("VAEL:chars");
const lower = await import("vael:chars");
if (upper !== lower) throw new Error("VAEL:chars and vael:chars are not one module");
"#,
    );
}

#[cfg(feature = "vael-chars")]
#[test]
fn built_in_module_namespace_and_registry_are_immutable() {
    run_js(
        r#"
import * as chars from "vael:chars";
const registry = Vael.getBuiltInModule("vael:chars");

if (chars === registry) {
    throw new Error("the module namespace must be distinct from the registry object");
}

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

#[cfg(feature = "vael-chars")]
#[test]
fn chars_predicates_ignore_a_replaced_regexp_exec() {
    run_js(
        r#"
import * as chars from "vael:chars";
const registry = Vael.getBuiltInModule("vael:chars");

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

#[cfg(feature = "vael-chars")]
#[test]
fn unknown_built_in_exports_fail_to_link() {
    let bare = run_js_error("import { nope } from \"vael:chars\";\n");
    assert!(
        bare.contains("does not provide an export named 'nope'"),
        "{bare}"
    );
    assert!(bare.contains("chars"), "{bare}");

    let alias = run_js_error("import { nope } from \"vael:chars\";\n");
    assert!(
        alias.contains("does not provide an export named 'nope'"),
        "{alias}"
    );
    assert!(alias.contains("vael:chars"), "{alias}");
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

#[cfg(feature = "vael-chars")]
#[test]
fn chars_predicates_use_scalar_code_points() {
    run_js(
        r#"
const chars = Vael.getBuiltInModule("vael:chars");
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

#[cfg(feature = "vael-chars")]
#[test]
fn chars_case_mapping_returns_one_code_point_or_the_input() {
    run_js(
        r#"
const chars = Vael.getBuiltInModule("vael:chars");
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
import * as consoleModule from "vael:console";
import defaultConsole from "vael:console";

const registry = Vael.getBuiltInModule("vael:console");

if (registry !== globalThis.console) throw new Error("registry is not the global console");
if (defaultConsole !== globalThis.console) throw new Error("default export is not the global console");
if (consoleModule.default !== globalThis.console) {
    throw new Error("namespace default is not the global console");
}
if (Object.isFrozen(registry)) throw new Error("the live console must not be frozen");
if (Vael.getBuiltInModule("console") !== undefined) {
    throw new Error("the registry accepted a bare name");
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
import * as staticConsole from "vael:console";
import { log as throughModule } from "./consumer.js";

const first = await import("vael:console");
const alias = await import("vael:console");
const uppercase = await import("VAEL:console");
const again = await import("vael:console");

if (staticConsole !== first) throw new Error("static and dynamic import differ");
if (first !== alias) throw new Error("console and vael:console differ");
if (alias !== uppercase) throw new Error("scheme matching is not case-insensitive");
if (alias !== again) throw new Error("repeated dynamic import was not cached");
if (first.default !== staticConsole.default) throw new Error("namespace default differs");
if (throughModule !== staticConsole.log) {
    throw new Error("an importer of vael:console saw a different function");
}
if (import.meta.resolve("vael:console") !== "vael:console") {
    throw new Error("console did not resolve canonically");
}
if (import.meta.resolve("vael:console") !== "vael:console") {
    throw new Error("vael:console did not resolve canonically");
}
"#,
    );
    write_module(
        &dir,
        "consumer.js",
        "export { log } from \"vael:console\";\n",
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
import defaultConsole, { log } from "vael:console";
import * as namespace from "vael:console";

const registry = Vael.getBuiltInModule("vael:console");
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
import defaultConsole from "vael:console";

const installed = globalThis.console;
const registry = Vael.getBuiltInModule("vael:console");
const replacement = { log() {} };

globalThis.console = replacement;
try {
    if (globalThis.console !== replacement) throw new Error("the global was not replaced");
    if (registry !== installed) throw new Error("the registry changed with the global property");
    if (defaultConsole !== installed) throw new Error("the default export changed with the global property");
    if (Vael.getBuiltInModule("vael:console") !== installed) {
        throw new Error("a later lookup changed with the global property");
    }
} finally {
    globalThis.console = installed;
}
if (Vael.getBuiltInModule("vael:console") !== globalThis.console) {
    throw new Error("the restore did not rejoin the registry and the global");
}
"#,
    );
}

#[test]
fn console_named_exports_capture_a_pre_evaluation_replacement() {
    run_js(
        r#"
const installed = Vael.getBuiltInModule("vael:console");
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
    const first = await import("vael:console");
    const alias = await import("vael:console");

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
const later = await import("vael:console");
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
const installed = Vael.getBuiltInModule("vael:console");
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
    const first = await import("vael:console");
    const alias = await import("vael:console");

    if (Vael.getBuiltInModule("vael:console") !== installed) {
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

if (Vael.getBuiltInModule("vael:console") !== globalThis.console) {
    throw new Error("the restore did not rejoin the registry and the global");
}
"#,
    );
}

#[test]
fn unknown_console_exports_fail_to_link() {
    let bare = run_js_error("import { nope } from \"vael:console\";\n");
    assert!(
        bare.contains("does not provide an export named 'nope'"),
        "{bare}"
    );
    assert!(bare.contains("console"), "{bare}");

    let alias = run_js_error("import { nope } from \"vael:console\";\n");
    assert!(
        alias.contains("does not provide an export named 'nope'"),
        "{alias}"
    );
    assert!(alias.contains("vael:console"), "{alias}");
}

#[test]
fn console_module_is_importable_from_typescript() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.ts",
        r#"
import defaultConsole, { log, warn } from "vael:console";

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
import * as builtin from "vael:console";
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
        let mut runtime = Runtime::new().expect("vaeljs runtime");
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
        let mut runtime = Runtime::new().expect("vaeljs runtime");
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

#[cfg(feature = "vael-strings")]
#[test]
fn strings_folded_matching_uses_es_simple_folding() {
    run_js(
        r#"
import * as strings from "vael:strings";

const cases = [
    // [text, other, expected, description]
    ["Hello", "HELLO", true, "ASCII case"],
    ["café", "CAFÉ", true, "accented Latin"],
    ["K", "\u212A", true, "Kelvin sign"],
    ["\u212A", "k", true, "Kelvin sign reversed"],
    ["S", "\u017F", true, "long s"],
    ["\u03C3", "\u03C2", true, "sigma and final sigma"],
    ["I", "\u0131", false, "dotless i is not I"],
    ["i", "\u0130", false, "dotted I is not i"],
    ["ss", "\u00DF", false, "sharp s is full folding only"],
    ["a\n", "A\n", true, "newline inside the text"],
    ["a\uD800", "a\uD800", true, "the same lone surrogate"],
    ["a\uD800", "a", false, "a lone surrogate is not absent"],
    ["\uD83D", "\u{1F600}", false, "half a pair is not the pair"],
    ["\u{1F600}", "\u{1F600}", true, "the same astral character"],
    ["", "", true, "both empty"],
    ["", "\u0000", false, "empty is not NUL"],
    ["a.b", "a.b", true, "a literal dot"],
    ["a.b", "axb", false, "the dot is not a wildcard"],
    ["[a-z]", "[a-z]", true, "a literal character class"],
    ["\\d", "\\d", true, "a literal backslash"],
    ["a*", "aaa", false, "a literal star"],
];
for (const [text, other, expected, description] of cases) {
    const actual = strings.equalFold(text, other);
    if (actual !== expected) {
        throw new Error(`equalFold(${JSON.stringify(text)}, ${JSON.stringify(other)}) is ${actual}: ${description}`);
    }
    if (strings.equalFold(other, text) !== actual) {
        throw new Error(`equalFold is not symmetric for ${description}`);
    }
}

if (!strings.startsWithFold("Hello World", "HELLO")) throw new Error("startsWithFold missed");
if (strings.startsWithFold("Hello", "World")) throw new Error("startsWithFold overmatched");
if (!strings.startsWithFold("abc", "")) throw new Error("startsWithFold rejected an empty prefix");
if (!strings.startsWithFold("(x)", "(x)")) throw new Error("startsWithFold treated a paren as a group");
if (!strings.startsWithFold("|x", "|")) throw new Error("startsWithFold treated a pipe as alternation");

if (!strings.endsWithFold("Hello World", "WORLD")) throw new Error("endsWithFold missed");
if (strings.endsWithFold("Hello", "Hell")) throw new Error("endsWithFold overmatched");
if (!strings.endsWithFold("abc", "")) throw new Error("endsWithFold rejected an empty suffix");
// JS `$` without `m` matches only at the end of the input.
if (strings.endsWithFold("a\n", "a")) throw new Error("endsWithFold matched before a final newline");
if (!strings.endsWithFold("a\n", "\n")) throw new Error("endsWithFold missed a final newline");
if (!strings.endsWithFold("a\n", "a\n")) throw new Error("endsWithFold missed the newline in the suffix");
if (strings.endsWithFold("a\r\n", "a")) throw new Error("endsWithFold matched before CRLF");
"#,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_indices_are_utf16_and_last_index_finds_overlaps() {
    run_js(
        r#"
import * as strings from "vael:strings";

const cases = [
    // [text, search, fromIndex, expected, description]
    ["\u{1F600}a", "A", undefined, 2, "astral prefix counts two code units"],
    ["a\u{1F600}b", "B", undefined, 3, "astral in the middle"],
    ["abc", "z", undefined, -1, "absent"],
    ["abc", "", undefined, 0, "empty search at the start"],
    ["abc", "", 2, 2, "empty search at an index"],
    ["\u{1F600}", "", 1, 1, "empty search inside a surrogate pair"],
    ["\u{1F600}a", "a", 1, 2, "non-empty search starts at the next code point"],
    ["abc", "b", -5, 1, "a negative index clamps to zero"],
    ["abc", "c", Infinity, -1, "Infinity clamps to the length"],
    ["abc", "", Infinity, 3, "an empty search at Infinity is the length"],
    ["abc", "a", 99, -1, "an index above the length finds nothing"],
    ["x.a.y", "a.", undefined, 2, "regex metacharacters are literal"],
];
for (const [text, search, fromIndex, expected, description] of cases) {
    const actual = strings.indexOfFold(text, search, fromIndex);
    if (actual !== expected) {
        throw new Error(`indexOfFold(${JSON.stringify(text)}, ${JSON.stringify(search)}, ${fromIndex}) is ${actual}, expected ${expected}: ${description}`);
    }
}

const lastCases = [
    ["aaa", "aa", undefined, 1, "overlapping occurrences"],
    ["aaaa", "aa", undefined, 2, "three overlapping occurrences"],
    ["aAa", "a", undefined, 2, "case-insensitive last occurrence"],
    ["a\u{1F600}b\u{1F600}", "B", undefined, 3, "astral text"],
    ["abc", "z", undefined, -1, "absent"],
    ["abc", "", undefined, 3, "empty search is the length"],
    ["abc", "", 1, 1, "empty search at an index"],
    ["aaaa", "aa", 2, 2, "a match starting at the limit"],
    ["aaaa", "aa", 1, 1, "a match starting below the limit"],
    ["aaa", "a", 0, 0, "a zero limit"],
    ["aaa", "a", -4, 0, "a negative limit clamps to zero"],
    ["\u212A\u212A", "k", undefined, 1, "folded last occurrence"],
];
for (const [text, search, fromIndex, expected, description] of lastCases) {
    const actual = strings.lastIndexOfFold(text, search, fromIndex);
    if (actual !== expected) {
        throw new Error(`lastIndexOfFold(${JSON.stringify(text)}, ${JSON.stringify(search)}, ${fromIndex}) is ${actual}, expected ${expected}: ${description}`);
    }
}

// Overlap parity with String.prototype.lastIndexOf on an ASCII haystack.
const overlapText = "aaaa";
for (let limit = -1; limit <= overlapText.length + 1; limit += 1) {
    const actual = strings.lastIndexOfFold(overlapText, "aa", limit);
    const expected = overlapText.lastIndexOf("aa", limit);
    if (actual !== expected) {
        throw new Error(`overlap parity failed at limit ${limit}: ${actual} !== ${expected}`);
    }
}
"#,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_trim_removes_a_code_point_set_and_defaults_to_js_whitespace() {
    run_js(
        r###"
import * as strings from "vael:strings";

const trimCases = [
    ["...hi...", ".", "hi", "one character"],
    ["##a##", "#", "a", "hash"],
    ["xyhelloyx", "xy", "hello", "a set of characters"],
    ["\u{1F600}x\u{1F600}", "\u{1F600}", "x", "an astral code point"],
    ["  a  ", "", "  a  ", "an empty set trims nothing"],
    ["....", ".", "", "a fully trimmed string"],
    ["abc", ".", "abc", "no leading or trailing member"],
    ["", ".", "", "empty text"],
    ["a.b", ".", "a.b", "a member in the middle is kept"],
    ["\uD800x\uD800", "\uD800", "x", "a lone surrogate in the set"],
];
for (const [text, chars, expected, description] of trimCases) {
    if (strings.trim(text, chars) !== expected) {
        throw new Error(`trim failed for ${description}`);
    }
}

if (strings.trimStart("///a/b", "/") !== "a/b") throw new Error("trimStart failed");
if (strings.trimEnd("a/b///", "/") !== "a/b") throw new Error("trimEnd failed");
if (strings.trimStart("\u{1F600}a", "\u{1F600}") !== "a") throw new Error("trimStart astral failed");
if (strings.trimEnd("a\u{1F600}", "\u{1F600}") !== "a") throw new Error("trimEnd astral failed");

// No `chars` is exactly String.prototype.trim: JS whitespace contains U+FEFF
// and excludes U+0085. This is deliberately not vael:chars' Go isSpace.
const jsWhitespace = "\uFEFF\t a \n\u0085";
if (strings.trim(jsWhitespace) !== jsWhitespace.trim()) {
    throw new Error("trim() without chars does not match String.prototype.trim");
}
if (strings.trim("\uFEFF a \uFEFF") !== "a") throw new Error("trim() did not remove U+FEFF");
if (strings.trim("\u0085a\u0085") !== "\u0085a\u0085") {
    throw new Error("trim() removed U+0085, which is not JS whitespace");
}
if (strings.trimStart("  a  ") !== "a  ") throw new Error("trimStart default failed");
if (strings.trimEnd("  a  ") !== "  a") throw new Error("trimEnd default failed");
"###,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_reject_non_string_and_bad_index_arguments() {
    run_js(
        r#"
import * as strings from "vael:strings";

const typeErrors = [
    () => strings.equalFold(1, "a"),
    () => strings.equalFold("a", 1),
    () => strings.startsWithFold("a", null),
    () => strings.endsWithFold(Symbol("x"), "a"),
    () => strings.indexOfFold("a", 1),
    () => strings.lastIndexOfFold("a", "a", "1"),
    () => strings.trim("a", 1),
    () => strings.trimStart(undefined, "a"),
    () => strings.trimEnd("a", {}),
];
for (const call of typeErrors) {
    let threw = false;
    try {
        call();
    } catch (error) {
        if (!(error instanceof TypeError)) {
            throw new Error(`expected TypeError, got ${error}`);
        }
        threw = true;
    }
    if (!threw) throw new Error("a non-string argument was accepted");
}

for (const index of [1.5, NaN]) {
    let threw = false;
    try {
        strings.indexOfFold("a", "a", index);
    } catch (error) {
        if (!(error instanceof RangeError)) {
            throw new Error(`expected RangeError for ${index}, got ${error}`);
        }
        threw = true;
    }
    if (!threw) throw new Error(`a non-integer fromIndex was accepted: ${index}`);
}

// A number is rejected rather than coerced, even when its string form matches.
let coerced = false;
try {
    strings.indexOfFold("a1", 1);
} catch (error) {
    coerced = error instanceof TypeError;
}
if (!coerced) throw new Error("indexOfFold coerced a number to a string");
"#,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_survive_replaced_intrinsics() {
    run_js(
        r#"
import * as strings from "vael:strings";

const baseline = [
    strings.equalFold("K", "\u212A"),
    strings.startsWithFold("Hello", "HEL"),
    strings.endsWithFold("a\n", "\n"),
    strings.indexOfFold("\u{1F600}a", "A"),
    strings.lastIndexOfFold("aaa", "aa"),
    strings.trim("...hi...", "."),
    strings.trim("\uFEFF a \uFEFF"),
];

const originals = {
    exec: RegExp.prototype.exec,
    regexp: globalThis.RegExp,
    trim: String.prototype.trim,
    trimStart: String.prototype.trimStart,
    trimEnd: String.prototype.trimEnd,
    slice: String.prototype.slice,
    charCodeAt: String.prototype.charCodeAt,
    codePointAt: String.prototype.codePointAt,
    has: Set.prototype.has,
    add: Set.prototype.add,
    get: Map.prototype.get,
    set: Map.prototype.set,
};
// Keep the prototype itself: after the global `RegExp` identifier is
// replaced, `RegExp.prototype` no longer names this object.
const regexpPrototype = RegExp.prototype;
const boom = () => {
    throw new Error("a replaced intrinsic was called");
};
regexpPrototype.exec = boom;
globalThis.RegExp = boom;
String.prototype.trim = boom;
String.prototype.trimStart = boom;
String.prototype.trimEnd = boom;
String.prototype.slice = boom;
String.prototype.charCodeAt = boom;
String.prototype.codePointAt = boom;
Set.prototype.has = boom;
Set.prototype.add = boom;
Map.prototype.get = boom;
Map.prototype.set = boom;
try {
    const after = [
        strings.equalFold("K", "\u212A"),
        strings.startsWithFold("Hello", "HEL"),
        strings.endsWithFold("a\n", "\n"),
        strings.indexOfFold("\u{1F600}a", "A"),
        strings.lastIndexOfFold("aaa", "aa"),
        strings.trim("...hi...", "."),
        strings.trim("\uFEFF a \uFEFF"),
    ];
    if (JSON.stringify(after) !== JSON.stringify(baseline)) {
        throw new Error(`results changed: ${JSON.stringify(after)} !== ${JSON.stringify(baseline)}`);
    }
} finally {
    globalThis.RegExp = originals.regexp;
    regexpPrototype.exec = originals.exec;
    String.prototype.trim = originals.trim;
    String.prototype.trimStart = originals.trimStart;
    String.prototype.trimEnd = originals.trimEnd;
    String.prototype.slice = originals.slice;
    String.prototype.charCodeAt = originals.charCodeAt;
    String.prototype.codePointAt = originals.codePointAt;
    Set.prototype.has = originals.has;
    Set.prototype.add = originals.add;
    Map.prototype.get = originals.get;
    Map.prototype.set = originals.set;
}
if (globalThis.RegExp !== originals.regexp) throw new Error("RegExp was not restored");
"#,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_handle_long_input() {
    run_js(
        r#"
import * as strings from "vael:strings";

const long = "a".repeat(1000000);
const tail = long + "needle";

if (strings.indexOfFold(tail, "NEEDLE") !== 1000000) {
    throw new Error("indexOfFold failed on a long string");
}
if (strings.lastIndexOfFold(tail, "NEEDLE") !== 1000000) {
    throw new Error("lastIndexOfFold failed on a long string");
}
if (!strings.equalFold(long, long)) throw new Error("equalFold failed on identical long strings");
if (strings.equalFold(long, long + "x")) throw new Error("equalFold accepted a longer string");
if (!strings.equalFold(long + "a", long + "A")) {
    throw new Error("equalFold failed on a long ASCII string");
}
if (!strings.startsWithFold(long, long)) throw new Error("startsWithFold failed on a long prefix");
if (!strings.endsWithFold(tail, "NEEDLE")) throw new Error("endsWithFold failed on a long text");

const trimmed = strings.trim(".".repeat(500000) + "x" + ".".repeat(500000), ".");
if (trimmed !== "x") throw new Error("trim failed on a long string");

const accented = "\u00E9".repeat(200000);
if (!strings.equalFold(accented, "\u00C9".repeat(200000))) {
    throw new Error("equalFold failed on a long non-ASCII string");
}
"#,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_module_named_exports_are_the_registry_functions() {
    run_js(
        r#"
import * as strings from "vael:strings";
const registry = Vael.getBuiltInModule("vael:strings");
if (registry !== Vael.getBuiltInModule("vael:strings")) {
    throw new Error("strings lookup is not cached");
}
if (!Object.isFrozen(registry)) throw new Error("strings is not frozen");
const exported = Object.keys(strings).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error(`export list ${exported} does not match ${registered}`);
}
const expected = [
    "endsWithFold", "equalFold", "indexOfFold", "lastIndexOfFold",
    "startsWithFold", "trim", "trimEnd", "trimStart",
];
if (JSON.stringify(registered) !== JSON.stringify(expected)) {
    throw new Error(`unexpected exports: ${registered}`);
}
for (const name of registered) {
    if (typeof strings[name] !== "function") throw new Error(`${name} is not a function`);
    if (strings[name] !== registry[name]) throw new Error(`${name} is not the registry function`);
}
if ("default" in strings) throw new Error("strings must not have a default export");

const alias = await import("vael:strings");
const upper = await import("VAEL:strings");
if (alias !== strings) throw new Error("vael:strings is not the same module instance");
if (upper !== strings) throw new Error("the scheme is not matched case-insensitively");
if (import.meta.resolve("vael:strings") !== "vael:strings") {
    throw new Error("strings did not resolve canonically");
}

let rejected = false;
try {
    strings.trim = () => "replaced";
} catch (error) {
    rejected = error instanceof TypeError;
}
if (!rejected) throw new Error("a namespace export was assignable");
if (strings.trim("...x...", ".") !== "x") throw new Error("trim was replaced");
"#,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_folded_search_stays_linear_on_repeated_prefixes() {
    run_js(
        r#"
import * as strings from "vael:strings";

// A per-position retry of the whole needle performs about 10^10 comparisons
// for the ASCII case and about 2.5 * 10^9 for the astral case at these sizes,
// so restoring that implementation fails this test by not completing. KMP
// finishes in milliseconds.
const asciiPrefix = "a".repeat(100000);
if (strings.indexOfFold(asciiPrefix + "c", asciiPrefix + "b") !== -1) {
    throw new Error("ASCII repeated-prefix miss was found");
}
if (strings.lastIndexOfFold(asciiPrefix + "c", asciiPrefix + "b") !== -1) {
    throw new Error("lastIndexOfFold ASCII repeated-prefix miss was found");
}
const asciiHit = asciiPrefix + asciiPrefix + "b";
if (strings.indexOfFold(asciiHit, asciiPrefix + "b") !== 100000) {
    throw new Error("ASCII repeated-prefix hit was missed");
}
if (strings.lastIndexOfFold(asciiHit, asciiPrefix + "b") !== 100000) {
    throw new Error("lastIndexOfFold ASCII repeated-prefix hit was missed");
}

const astralBlock = "\u{1F600}".repeat(50000);
if (strings.indexOfFold(astralBlock + "x", astralBlock + "y") !== -1) {
    throw new Error("astral repeated-prefix miss was found");
}
if (strings.lastIndexOfFold(astralBlock + "x", astralBlock + "y") !== -1) {
    throw new Error("lastIndexOfFold astral repeated-prefix miss was found");
}

const foldedBlock = "\u03C3".repeat(50000);
const foldedNeedle = "\u03A3".repeat(50000) + "y";
const foldedHit = foldedBlock + "\u03A3".repeat(50000) + "y";
if (strings.indexOfFold(foldedHit, foldedNeedle) !== 50000) {
    throw new Error("folded repeated-prefix hit was missed");
}
if (strings.lastIndexOfFold(foldedHit, foldedNeedle) !== 50000) {
    throw new Error("lastIndexOfFold folded repeated-prefix hit was missed");
}
"#,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_folded_search_keeps_unicode_and_overlap_semantics() {
    run_js(
        r#"
import * as strings from "vael:strings";

const indexCases = [
    ["aaaa", "aa", 0, "overlapping first match"],
    ["aaab", "aab", 1, "KMP fallback after a failed prefix"],
    ["xabab", "abA", 1, "folded needle"],
    ["kK", "K\u212A", 0, "fold-equivalent code points in the prefix"],
    ["\u03A3\u03A3x\u03A3\u03A3y", "\u03A3\u03A3y", 3, "folded repeated prefix"],
    ["\u{1F600}a\u{1F600}a", "A\u{1F600}", 2, "astral match starts on a boundary"],
    ["\u{1F600}", "\uDE00", -1, "a lone surrogate never matches half a pair"],
];
for (const [text, search, expected, description] of indexCases) {
    const actual = strings.indexOfFold(text, search);
    if (actual !== expected) {
        throw new Error(`indexOfFold(${JSON.stringify(text)}, ${JSON.stringify(search)}) is ${actual}, expected ${expected}: ${description}`);
    }
}
if (strings.indexOfFold("\u{1F600}a", "a", 1) !== 2) {
    throw new Error("fromIndex inside a pair did not move to the next boundary");
}

const lastCases = [
    ["aaaa", "aa", 2, "overlapping last match"],
    ["aaa", "aa", 1, "single overlap"],
    ["xababA", "abA", 3, "folded needle at the end"],
    ["\u03C3".repeat(1000), "\u03C3\u03C3", 998, "long overlapping block"],
    ["\u{1F600}\u{1F600}", "\u{1F600}", 2, "astral last match"],
];
for (const [text, search, expected, description] of lastCases) {
    const actual = strings.lastIndexOfFold(text, search);
    if (actual !== expected) {
        throw new Error(`lastIndexOfFold(${JSON.stringify(text)}, ${JSON.stringify(search)}) is ${actual}, expected ${expected}: ${description}`);
    }
}

// KMP must report the same last index as String.prototype.lastIndexOf on an
// ASCII haystack for every limit, including limits inside a match.
const overlapText = "abababab";
for (let limit = -1; limit <= overlapText.length + 1; limit += 1) {
    const actual = strings.lastIndexOfFold(overlapText, "abab", limit);
    const expected = overlapText.lastIndexOf("abab", limit);
    if (actual !== expected) {
        throw new Error(`overlap parity failed at ${limit}: ${actual} !== ${expected}`);
    }
}
"#,
    );
}

#[cfg(feature = "vael-strings")]
#[test]
fn strings_last_index_stops_at_a_small_limit() {
    run_js(
        r#"
import * as strings from "vael:strings";

// One million code points with no match. With fromIndex 0 the scan must stop
// as soon as no pending or future match can start at or before 0; a scan that
// ignores the limit reads the whole text.
const astral = "\u{1F600}".repeat(1000000) + "needle";
if (strings.lastIndexOfFold(astral, "zzz", 0) !== -1) {
    throw new Error("small-limit no-match found something");
}
if (strings.lastIndexOfFold(astral, "NEEDLE", 0) !== -1) {
    throw new Error("a match after the limit was reported");
}

// A match at the limit is still found.
if (strings.lastIndexOfFold("needle" + "x".repeat(1000000), "NEEDLE", 0) !== 0) {
    throw new Error("a match at the limit was skipped");
}
if (strings.lastIndexOfFold("a".repeat(1000000) + "b", "B", 1000000) !== 1000000) {
    throw new Error("a match exactly at a large limit was skipped");
}
if (strings.lastIndexOfFold("a".repeat(1000000) + "b", "B", 999999) !== -1) {
    throw new Error("a match after the limit was reported");
}

// A pending partial match that starts at the limit keeps the scan alive until
// it fails; a limit inside a surrogate pair admits no boundary after it.
if (strings.lastIndexOfFold("ab" + "x".repeat(1000000), "abc", 0) !== -1) {
    throw new Error("a pending match at the limit was mishandled");
}
if (strings.lastIndexOfFold("\u{1F600}" + "a".repeat(1000000), "a", 1) !== -1) {
    throw new Error("a mid-pair limit matched a later boundary");
}
if (strings.lastIndexOfFold("ab" + "x".repeat(1000000), "ab", 0) !== 0) {
    throw new Error("an overlapping match at the limit was skipped");
}

// Folded characters behave the same way under a small limit.
if (strings.lastIndexOfFold("\u03C3".repeat(1000000) + "x", "\u03A3", 0) !== 0) {
    throw new Error("a folded match at the limit was skipped");
}
if (strings.lastIndexOfFold("\u03C3" + "\u03A3".repeat(1000000), "z", 0) !== -1) {
    throw new Error("a folded small-limit no-match found something");
}
"#,
    );
}

/// The `assert` module object is the callable function itself, and its named
/// exports are that function's properties. `assertEquals` is deliberately
/// absent.
#[test]
fn assert_module_exports_the_callable_without_assert_equals() {
    run_js(
        r#"
import assertDefault, * as assertModule from "vael:assert";
import { AssertionError, equals, ok, strictEquals } from "vael:assert";

const registry = Vael.getBuiltInModule("vael:assert");
if (registry !== assertDefault) throw new Error("the default export is not the registry");
if (registry !== assertModule.assert) throw new Error("the named assert is not the registry");
if (registry !== ok) throw new Error("ok is not the registry callable");
if (registry.ok !== registry) throw new Error("assert.ok is not the callable");
if (registry.equals !== equals) throw new Error("equals is not the registry function");
if (registry.strictEquals !== strictEquals) {
    throw new Error("strictEquals is not the registry function");
}
if (registry.AssertionError !== AssertionError) {
    throw new Error("AssertionError is not the registry class");
}
if (!Object.isFrozen(registry)) throw new Error("the assert module is not frozen");
if (typeof registry !== "function") throw new Error("the assert module is not callable");

const exported = Object.keys(assertModule).sort();
const expected = [
    "AssertionError",
    "assert",
    "default",
    "doesNotReject",
    "doesNotThrow",
    "equals",
    "fail",
    "notEquals",
    "notStrictEquals",
    "ok",
    "rejects",
    "strictEquals",
    "throws",
].sort();
if (JSON.stringify(exported) !== JSON.stringify(expected)) {
    throw new Error(`assert exports ${exported} do not match ${expected}`);
}
for (const name of Object.keys(registry)) {
    const value = registry[name];
    if (typeof value !== "function") throw new Error(`${name} is not a function`);
    if (assertModule[name] !== value) throw new Error(`${name} differs from the registry`);
}
if ("assertEquals" in registry) throw new Error("assertEquals is a property of assert");
if ("assertEquals" in assertModule) throw new Error("assertEquals is a module export");
if (Object.keys(registry).includes("assertEquals")) {
    throw new Error("assertEquals is an enumerable registry property");
}
if ("default" in registry) throw new Error("the registry must not have a default property");
"#,
    );

    let error = run_js_error("import { assertEquals } from \"vael:assert\";\n");
    assert!(
        error.contains("does not provide an export named 'assertEquals'"),
        "{error}"
    );
    assert!(error.contains("assert"), "{error}");
}

/// `strictEquals` is `Object.is`; `equals` is structural equality. The test
/// names each semantic so a regression reports which rule changed.
#[test]
fn assert_equality_distinguishes_identity_from_structure() {
    run_js(
        r#"
import { AssertionError, equals, notEquals, notStrictEquals, strictEquals } from "vael:assert";

function passes(label, fn) {
    try {
        fn();
    } catch (error) {
        throw new Error(`${label} failed unexpectedly: ${error}`);
    }
}
function fails(label, fn) {
    let error;
    try {
        fn();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof AssertionError)) {
        throw new Error(`${label} threw ${error} instead of AssertionError`);
    }
    return error;
}

const same = { value: 1 };
passes("strict same object", () => strictEquals(same, same));
passes("strict NaN", () => strictEquals(NaN, NaN));
passes("strict global symbol", () => strictEquals(Symbol.for("k"), Symbol.for("k")));
fails("strict distinct objects", () => strictEquals({ value: 1 }, { value: 1 }));
fails("strict -0 and 0", () => strictEquals(0, -0));
fails("strict local symbols", () => strictEquals(Symbol("k"), Symbol("k")));
passes("notStrict -0", () => notStrictEquals(0, -0));

passes("structural object", () => equals({ value: 1 }, { value: 1 }));
passes("structural nested", () => equals({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] }));
passes("structural NaN", () => equals(NaN, NaN));
passes("structural shared graph", () => {
    const shared = { value: 1 };
    return equals({ a: shared, b: shared }, { a: { value: 1 }, b: { value: 1 } });
});
passes("structural cycle", () => {
    const actual = { value: 1 };
    actual.self = actual;
    const expected = { value: 1 };
    expected.self = expected;
    return equals(actual, expected);
});
fails("structural prototype", () => equals(Object.create(null), {}));
fails("structural -0 and 0", () => equals(0, -0));
fails("structural array length", () => equals([1], [1, 2]));
fails("structural array hole", () => equals([1, , 3], [1, undefined, 3]));
fails("structural missing property", () => equals({ a: 1 }, {}));
fails("structural function identity", () => equals(() => 1, () => 1));
fails("structural Promise identity", () => equals(Promise.resolve(1), Promise.resolve(1)));
fails("structural cycle difference", () => {
    const actual = { value: 1 };
    actual.self = actual;
    const expected = { value: 1, extra: true };
    expected.self = expected;
    return equals(actual, expected);
});
passes("notEquals", () => notEquals({ a: 1 }, { a: 2 }));
fails("notEquals equal", () => notEquals({ a: 1 }, { a: 1 }));

fails("unreadable prototype", () =>
    equals({}, new Proxy({}, {
        getPrototypeOf() {
            throw new Error("trap");
        },
    })));
let descriptorCalls = 0;
const unreadableProperties = new Proxy({ a: 1 }, {
    ownKeys() {
        return ["a"];
    },
    getOwnPropertyDescriptor(target, key) {
        descriptorCalls += 1;
        if (descriptorCalls > 1) throw new Error("trap");
        return Object.getOwnPropertyDescriptor(target, key);
    },
});
fails("unreadable own property", () => equals({ a: 1 }, unreadableProperties));

// `expected`'s prototype trap revokes `actual` after actual's prototype was
// already read. Kind detection then meets a revoked proxy; it must report an
// AssertionError rather than let Array.isArray throw a TypeError.
const crossRevocable = Proxy.revocable({}, {});
const crossActual = crossRevocable.proxy;
const crossExpected = new Proxy({}, {
    getPrototypeOf() {
        crossRevocable.revoke();
        return Object.prototype;
    },
});
const crossError = fails("kind detection after cross-revocation", () =>
    equals(crossActual, crossExpected));
if (!crossError.message.includes("could not be inspected")) {
    throw new Error(`unexpected cross-revocation message: ${crossError.message}`);
}

// The same guard covers a proxy that revokes itself in its prototype trap.
const selfRevocable = Proxy.revocable({}, {});
const selfExpected = new Proxy({}, {
    getPrototypeOf() {
        selfRevocable.revoke();
        return Object.prototype;
    },
});
fails("kind detection after self-revocation", () =>
    equals(selfRevocable.proxy, selfExpected));
"#,
    );
}

/// Map and Set matching is order-insensitive and uses deep keys; typed arrays
/// compare type, length, and elements with `Object.is`; DataViews compare only
/// the bytes in their view.
#[test]
fn assert_equality_handles_maps_sets_and_binary_views() {
    run_js(
        r#"
import { AssertionError, equals } from "vael:assert";

function passes(label, fn) {
    try {
        fn();
    } catch (error) {
        throw new Error(`${label} failed unexpectedly: ${error}`);
    }
}
function fails(label, fn) {
    let error;
    try {
        fn();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof AssertionError)) {
        throw new Error(`${label} threw ${error} instead of AssertionError`);
    }
    return error;
}

passes("Map insertion order", () =>
    equals(
        new Map([["a", 1], [{ key: 2 }, 3]]),
        new Map([[{ key: 2 }, 3], ["a", 1]]),
    ));
passes("Map deep key", () => equals(new Map([[{ a: [1] }, 2]]), new Map([[{ a: [1] }, 2]])));
passes("Map cycle key", () => {
    const actual = { a: 1 };
    actual.self = actual;
    const expected = { a: 1 };
    expected.self = expected;
    return equals(new Map([[actual, 1]]), new Map([[expected, 1]]));
});
fails("Map value", () => equals(new Map([["a", 1]]), new Map([["a", 2]])));
fails("Map size", () => equals(new Map([["a", 1]]), new Map([["a", 1], ["b", 2]])));
fails("Map rollback", () => {
    const one = { a: 1 };
    one.self = one;
    const two = { a: 1 };
    two.self = two;
    const three = { b: 2 };
    three.self = three;
    const four = { b: 2 };
    four.self = four;
    return equals(
        new Map([[one, "wrong"], [three, "right"]]),
        new Map([[two, "other"], [four, "right"]]),
    );
});

passes("Set insertion order", () => equals(new Set([1, { a: 2 }]), new Set([{ a: 2 }, 1])));
passes("Set cycle value", () => {
    const actual = { a: 1 };
    actual.self = actual;
    const expected = { a: 1 };
    expected.self = expected;
    return equals(new Set([actual]), new Set([expected]));
});
fails("Set value", () => equals(new Set([1]), new Set([2])));
fails("Set size", () => equals(new Set([1]), new Set([1, 2])));

passes("typed array elements", () =>
    equals(new Uint8Array([1, 2, 3]), new Uint8Array([1, 2, 3])));
passes("typed NaN and -0", () =>
    equals(new Float64Array([NaN, -0]), new Float64Array([NaN, -0])));
passes("bigint typed array", () =>
    equals(new BigInt64Array([1n, -2n]), new BigInt64Array([1n, -2n])));
fails("typed element", () => equals(new Uint8Array([1, 2]), new Uint8Array([1, 3])));
fails("typed type", () => equals(new Uint8Array([1, 2]), new Uint8ClampedArray([1, 2])));
fails("typed length", () => equals(new Uint8Array([1]), new Uint8Array([1, 2])));
fails("float -0", () => equals(new Float32Array([0]), new Float32Array([-0])));

const first = new Uint8Array([0, 1, 2, 3]);
const second = new Uint8Array([9, 1, 2, 9]);
passes("DataView view", () =>
    equals(new DataView(first.buffer, 1, 2), new DataView(second.buffer, 1, 2)));
fails("DataView bytes", () =>
    equals(new DataView(first.buffer, 1, 2), new DataView(first.buffer, 0, 2)));

passes("ArrayBuffer bytes", () => equals(first.buffer, new Uint8Array([0, 1, 2, 3]).buffer));
fails("ArrayBuffer bytes differ", () => equals(first.buffer, second.buffer));

passes("Date time", () => equals(new Date(123), new Date(123)));
passes("invalid Date", () => equals(new Date(NaN), new Date(NaN)));
fails("Date time differs", () => equals(new Date(123), new Date(124)));
passes("RegExp source and flags", () => equals(/a/gi, /a/gi));
fails("RegExp flags", () => equals(/a/g, /a/i));
fails("RegExp lastIndex", () => {
    const left = /a/g;
    const right = /a/g;
    left.lastIndex = 1;
    right.lastIndex = 2;
    return equals(left, right);
});

passes("boxed number", () => equals(Object(1), new Number(1)));
passes("boxed NaN", () => equals(Object(NaN), Object(NaN)));
passes("boxed string", () => equals(Object("a"), Object("a")));
fails("boxed number differs", () => equals(Object(1), Object(2)));
passes("Error name and message", () => equals(new TypeError("x"), new TypeError("x")));
fails("Error name", () => equals(new TypeError("x"), new RangeError("x")));
fails("Error message", () => equals(new TypeError("x"), new TypeError("y")));
"#,
    );
}

/// `AssertionError` exposes a stable vaeljs code and the failed values, and the
/// message names the first differing path.
#[test]
fn assert_assertion_error_is_informative() {
    run_js(
        r#"
import { AssertionError, assert, equals, fail, strictEquals } from "vael:assert";

let error = null;
try {
    equals({ a: { b: [0, 1] } }, { a: { b: [0, 2] } });
} catch (thrown) {
    error = thrown;
}
if (!(error instanceof AssertionError)) throw new Error("equals did not throw AssertionError");
if (!(error instanceof Error)) throw new Error("AssertionError is not an Error");
if (error.name !== "AssertionError") throw new Error(`name was ${error.name}`);
if (error.code !== "VAEL_ASSERT") throw new Error(`code was ${error.code}`);
if (error.operator !== "equals") throw new Error(`operator was ${error.operator}`);
if (error.actual.a.b[1] !== 1) throw new Error("actual was not preserved");
if (error.expected.a.b[1] !== 2) throw new Error("expected was not preserved");
if (!error.message.includes("$.a.b[1]")) throw new Error(`path missing from ${error.message}`);
if (!error.message.includes("actual:")) throw new Error(`actual detail missing from ${error.message}`);
if (!error.message.includes("expected:")) throw new Error(`expected detail missing from ${error.message}`);

error = null;
try {
    strictEquals({}, {});
} catch (thrown) {
    error = thrown;
}
if (!(error instanceof AssertionError)) throw new Error("strictEquals did not throw AssertionError");
if (error.operator !== "strictEquals") throw new Error(`operator was ${error.operator}`);
if (!error.message.includes("Object.is")) throw new Error("strictEquals message does not say Object.is");

let custom = null;
try {
    equals(1, 2, "the parser");
} catch (thrown) {
    custom = thrown;
}
if (!custom.message.startsWith("the parser: ")) throw new Error("custom message was not a prefix");

error = null;
try {
    assert(0);
} catch (thrown) {
    error = thrown;
}
if (!(error instanceof AssertionError)) throw new Error("assert did not throw AssertionError");
if (error.actual !== 0 || error.expected !== true) throw new Error("assert values were wrong");
if (error.operator !== "assert") throw new Error(`assert operator was ${error.operator}`);

error = null;
try {
    fail("stop");
} catch (thrown) {
    error = thrown;
}
if (!(error instanceof AssertionError) || error.message !== "stop") {
    throw new Error("fail message was wrong");
}
if (error.operator !== "fail") throw new Error(`fail operator was ${error.operator}`);

const typeErrors = [
    () => assert(),
    () => assert(true, 1),
    () => equals(1, 1, null),
    () => equals(1),
    () => equals(1, 1, "message", "extra"),
    () => strictEquals(1, 1, {}),
    () => fail(1),
];
for (const call of typeErrors) {
    let thrown = null;
    try {
        call();
    } catch (candidate) {
        thrown = candidate;
    }
    if (!(thrown instanceof TypeError)) {
        throw new Error(`expected TypeError, got ${thrown}`);
    }
}
"#,
    );
}

/// `throws` is synchronous and fails closed on a Promise; `rejects` is
/// Promise-aware and also treats a synchronous throw as a rejection.
#[test]
fn assert_throws_and_rejects_handle_sync_and_promise_callbacks() {
    run_js(
        r#"
import {
    AssertionError,
    doesNotReject,
    doesNotThrow,
    rejects,
    throws,
} from "vael:assert";

function fails(label, fn) {
    let error;
    try {
        fn();
    } catch (thrown) {
        error = thrown;
    }
    if (error === undefined) throw new Error(`${label} did not throw`);
    return error;
}
async function failsAsync(label, fn) {
    let error;
    try {
        await fn();
    } catch (thrown) {
        error = thrown;
    }
    if (error === undefined) throw new Error(`${label} did not reject`);
    return error;
}

const thrown = throws(() => {
    throw new TypeError("boom");
}, TypeError);
if (!(thrown instanceof TypeError) || thrown.message !== "boom") {
    throw new Error("throws did not return the thrown value");
}
throws(() => {
    throw new Error("boom");
}, /bo+m/);
throws(() => {
    throw "plain";
}, /plai/);

let error = fails("throws return", () => throws(() => 7));
if (!(error instanceof AssertionError) || error.operator !== "throws") {
    throw new Error("throws accepted a return");
}
error = fails("throws class", () => throws(() => {
    throw new TypeError("x");
}, RangeError));
if (!(error instanceof AssertionError)) throw new Error("throws wrong class was not AssertionError");
error = fails("throws regex", () => throws(() => {
    throw new Error("x");
}, /y/));
if (!(error instanceof AssertionError)) throw new Error("throws wrong regex was not AssertionError");
error = fails("throws async", () => throws(async () => {
    throw new Error("x");
}));
if (!(error instanceof TypeError)) throw new Error("throws async was not a TypeError");
if (!error.message.includes("assert.rejects")) throw new Error("throws async did not point at rejects");
error = fails("throws promise", () => throws(() => Promise.reject(new Error("x"))));
if (!(error instanceof TypeError)) throw new Error("throws promise was not a TypeError");
error = fails("throws real promise", () => throws(() => Promise.resolve(1)));
if (!(error instanceof TypeError)) throw new Error("throws real promise was not a TypeError");

// An object that merely inherits from Promise.prototype has no Promise
// internal slot, so the synchronous guard must treat it as an ordinary return.
const spoofedPromise = Object.create(Promise.prototype);
error = fails("throws spoofed promise", () => throws(() => spoofedPromise));
if (!(error instanceof AssertionError)) {
    throw new Error("throws spoofed promise was not an AssertionError");
}
if (doesNotThrow(() => spoofedPromise) !== spoofedPromise) {
    throw new Error("doesNotThrow rejected a spoofed Promise");
}

// A Proxy around a Promise is not the Promise itself.
const proxiedPromise = new Proxy(Promise.resolve(1), {});
error = fails("throws proxied promise", () => throws(() => proxiedPromise));
if (!(error instanceof AssertionError)) {
    throw new Error("throws proxied promise was not an AssertionError");
}

// A revoked Proxy has no Promise slot and must not make the guard throw.
const revoked = Proxy.revocable({}, {});
revoked.revoke();
error = fails("throws revoked proxy", () => throws(() => revoked.proxy));
if (!(error instanceof AssertionError)) {
    throw new Error("throws revoked proxy was not an AssertionError");
}
if (doesNotThrow(() => revoked.proxy) !== revoked.proxy) {
    throw new Error("doesNotThrow rejected a revoked proxy");
}

error = fails("throws string expected", () => throws(() => {}, "message"));
if (!(error instanceof TypeError)) throw new Error("throws string expected was not a TypeError");

if (doesNotThrow(() => 42) !== 42) throw new Error("doesNotThrow did not return the result");
error = fails("doesNotThrow throw", () => doesNotThrow(() => {
    throw new Error("x");
}));
if (!(error instanceof AssertionError) || error.operator !== "doesNotThrow") {
    throw new Error("doesNotThrow did not fail with AssertionError");
}
error = fails("doesNotThrow async", () => doesNotThrow(async () => 1));
if (!(error instanceof TypeError)) throw new Error("doesNotThrow async was not a TypeError");
if (!error.message.includes("assert.doesNotReject")) {
    throw new Error("doesNotThrow async did not point at doesNotReject");
}

const syncRejection = await rejects(() => {
    throw new RangeError("sync");
}, RangeError);
if (!(syncRejection instanceof RangeError)) throw new Error("rejects lost a sync throw");
const asyncRejection = await rejects(async () => {
    throw new SyntaxError("async");
}, /async/);
if (!(asyncRejection instanceof SyntaxError)) throw new Error("rejects lost an async rejection");
await rejects(() => ({
    then(resolve, reject) {
        reject(new TypeError("thenable"));
    },
}), TypeError);
await rejects(() => new Proxy(Promise.resolve(1), {}), TypeError);
const realReason = await rejects(() => Promise.reject(new RangeError("real")), RangeError);
if (!(realReason instanceof RangeError)) throw new Error("rejects lost a native rejection");

// A stateful then getter must be read exactly once, then called once. Reading
// it again to test for thenability would consume the second value or throw.
let thenReads = 0;
const statefulThenable = {
    get then() {
        thenReads += 1;
        return function (resolve, reject) {
            reject(new RangeError("stateful"));
        };
    },
};
const statefulReason = await rejects(() => statefulThenable, RangeError);
if (thenReads !== 1) throw new Error(`then was read ${thenReads} times`);
if (!(statefulReason instanceof RangeError)) {
    throw new Error("rejects lost a stateful thenable rejection");
}

let flakyReads = 0;
const flakyThenable = {
    get then() {
        flakyReads += 1;
        if (flakyReads > 1) throw new SyntaxError("second read");
        return function (resolve, reject) {
            reject(new TypeError("first read"));
        };
    },
};
const flakyReason = await rejects(() => flakyThenable, TypeError);
if (flakyReads !== 1) throw new Error(`flaky then was read ${flakyReads} times`);
if (!(flakyReason instanceof TypeError)) {
    throw new Error("rejects lost a flaky thenable rejection");
}

await rejects(() => ({
    get then() {
        throw new RangeError("getter");
    },
}), RangeError);
await rejects(() => ({
    then() {
        throw new SyntaxError("call");
    },
}), SyntaxError);
error = await failsAsync("rejects non-thenable then", () => rejects(() => ({ then: 1 })));
if (!(error instanceof AssertionError)) {
    throw new Error("rejects accepted a non-thenable then property");
}
error = await failsAsync("rejects null", () => rejects(() => null));
if (!(error instanceof AssertionError)) {
    throw new Error("rejects accepted null as a Promise");
}

// The non-thenable sentinel is a symbol, so a polluted inherited `then` cannot
// turn a primitive return into a false rejection.
const hadOwnThen = Object.prototype.hasOwnProperty.call(Object.prototype, "then");
const originalThen = Object.prototype.then;
Object.prototype.then = function () {
    throw new Error("polluted then");
};
try {
    let pollutedError;
    try {
        await rejects(() => 42);
    } catch (caught) {
        pollutedError = caught;
    }
    if (!(pollutedError instanceof AssertionError)) {
        throw new Error("rejects primitive false-passed under polluted Object.prototype.then");
    }
    if (!pollutedError.message.includes("expected the function to return a Promise")) {
        throw new Error(`unexpected polluted-then rejection: ${pollutedError.message}`);
    }
    pollutedError = undefined;
    try {
        await doesNotReject(() => 42);
    } catch (caught) {
        pollutedError = caught;
    }
    if (!(pollutedError instanceof AssertionError)) {
        throw new Error("doesNotReject primitive did not fail under polluted then");
    }
    if (!pollutedError.message.includes("expected the function to return a Promise")) {
        throw new Error(`unexpected polluted-then assertion: ${pollutedError.message}`);
    }
    if (await doesNotReject(() => Promise.resolve(7)) !== 7) {
        throw new Error("a native Promise was affected by polluted Object.prototype.then");
    }
    if (await rejects(() => Promise.reject("polluted primitive")) !== "polluted primitive") {
        throw new Error("a native primitive rejection was affected by polluted then");
    }
} finally {
    if (hadOwnThen) Object.prototype.then = originalThen;
    else delete Object.prototype.then;
}

// A genuine Promise subclass is observed through the V8 host op, not through
// its overridden `then`. Settled and pending promises use the same observation.
let overriddenThenCalls = 0;
class OverriddenThenPromise extends Promise {
    then() {
        overriddenThenCalls += 1;
        throw new Error("overridden then was called");
    }
}
const fulfilledSubclass = new OverriddenThenPromise((resolve) => resolve("subclass value"));
if (await doesNotReject(() => fulfilledSubclass) !== "subclass value") {
    throw new Error("doesNotReject lost a fulfilled subclass value");
}
const rejectedSubclass = new OverriddenThenPromise((resolve, reject) =>
    reject(new RangeError("subclass reason")));
const subclassReason = await rejects(() => rejectedSubclass, RangeError);
if (!(subclassReason instanceof RangeError)) {
    throw new Error("rejects lost a rejected subclass reason");
}
let settleSubclass;
const pendingSubclass = new OverriddenThenPromise((resolve) => {
    settleSubclass = resolve;
});
const pendingSubclassResult = doesNotReject(() => pendingSubclass);
settleSubclass("pending value");
if (await pendingSubclassResult !== "pending value") {
    throw new Error("doesNotReject lost a pending subclass value");
}
if (overriddenThenCalls !== 0) {
    throw new Error(`overridden then was called ${overriddenThenCalls} times`);
}

// The V8 observation does not consult Symbol.species, so settled and pending
// subclasses whose species throws are still readable.
let speciesReads = 0;
class ThrowingSpeciesPromise extends Promise {
    static get [Symbol.species]() {
        speciesReads += 1;
        throw new Error("species trap");
    }
}
const fulfilledBadSpecies = new ThrowingSpeciesPromise((resolve) => resolve("settled"));
if (await doesNotReject(() => fulfilledBadSpecies) !== "settled") {
    throw new Error("doesNotReject could not read a settled throwing-species promise");
}
const rejectedBadSpecies = new ThrowingSpeciesPromise((resolve, reject) =>
    reject(new RangeError("settled reason")));
const badSpeciesReason = await rejects(() => rejectedBadSpecies, RangeError);
if (!(badSpeciesReason instanceof RangeError)) {
    throw new Error("rejects could not read a settled rejecting throwing-species promise");
}
let settleBadSpecies;
const pendingBadSpecies = new ThrowingSpeciesPromise((resolve) => {
    settleBadSpecies = resolve;
});
const pendingBadSpeciesResult = doesNotReject(() => pendingBadSpecies);
settleBadSpecies("pending species value");
if (await pendingBadSpeciesResult !== "pending species value") {
    throw new Error("doesNotReject could not read a pending throwing-species promise");
}
let rejectBadSpecies;
const pendingBadRejection = new ThrowingSpeciesPromise((resolve, reject) => {
    rejectBadSpecies = reject;
});
const pendingBadRejectionResult = rejects(() => pendingBadRejection, RangeError);
rejectBadSpecies(new RangeError("pending species reason"));
const pendingBadRejectionReason = await pendingBadRejectionResult;
if (!(pendingBadRejectionReason instanceof RangeError)) {
    throw new Error("rejects could not read a pending rejecting throwing-species promise");
}
if (speciesReads !== 0) {
    throw new Error(`Symbol.species was read ${speciesReads} times`);
}

// A rejection reason that is itself a thenable is returned from an async
// function, so native Promise resolution assimilates it. The target did reject
// (the assertion passes), but the returned Promise follows that reason's
// settlement. These cases document both outcomes and keep the distinction from
// a false pass: a fulfilling target still fails the assertion.
const fulfillingReason = {
    then(resolve) {
        resolve("assimilated reason");
    },
};
if (await rejects(() => Promise.reject(fulfillingReason)) !== "assimilated reason") {
    throw new Error("a fulfilling thenable reason was not assimilated");
}
const rejectingReason = {
    then(resolve, reject) {
        reject(new RangeError("assimilated rejection"));
    },
};
let assimilatedRejection;
try {
    await rejects(() => Promise.reject(rejectingReason));
} catch (caught) {
    assimilatedRejection = caught;
}
if (
    !(assimilatedRejection instanceof RangeError) ||
    assimilatedRejection.message !== "assimilated rejection"
) {
    throw new Error("a rejecting thenable reason did not reject the returned Promise");
}
const promiseReason = Promise.resolve("promise assimilation");
if (await rejects(() => Promise.reject(promiseReason)) !== "promise assimilation") {
    throw new Error("a Promise reason was not assimilated");
}
const rejectedPromiseReason = Promise.reject(new SyntaxError("promise rejection"));
let assimilatedPromiseRejection;
try {
    await rejects(() => Promise.reject(rejectedPromiseReason));
} catch (caught) {
    assimilatedPromiseRejection = caught;
}
if (
    !(assimilatedPromiseRejection instanceof SyntaxError) ||
    assimilatedPromiseRejection.message !== "promise rejection"
) {
    throw new Error("a rejected Promise reason did not reject the returned Promise");
}
const fulfillingTarget = {
    then(resolve) {
        resolve("not a rejection");
    },
};
let fulfillingTargetError;
try {
    await rejects(() => fulfillingTarget);
} catch (caught) {
    fulfillingTargetError = caught;
}
if (!(fulfillingTargetError instanceof AssertionError)) {
    throw new Error("a fulfilling thenable was accepted as a rejection");
}

error = await failsAsync("rejects arity", () => rejects());
if (!(error instanceof TypeError)) throw new Error("rejects arity was not a TypeError");
error = await failsAsync("doesNotReject arity", () => doesNotReject());
if (!(error instanceof TypeError)) throw new Error("doesNotReject arity was not a TypeError");

error = await failsAsync("rejects fulfilled", () => rejects(async () => 1));
if (!(error instanceof AssertionError) || error.operator !== "rejects") {
    throw new Error("rejects accepted a fulfilment");
}
error = await failsAsync("rejects non-promise", () => rejects(() => 1));
if (!(error instanceof AssertionError)) throw new Error("rejects accepted a plain return");
error = await failsAsync("rejects wrong class", () => rejects(async () => {
    throw new TypeError("x");
}, RangeError));
if (!(error instanceof AssertionError)) throw new Error("rejects accepted the wrong class");
error = await failsAsync("rejects wrong regex", () => rejects(async () => {
    throw new Error("x");
}, /y/));
if (!(error instanceof AssertionError)) throw new Error("rejects accepted the wrong regex");

if (await doesNotReject(async () => "fulfilled") !== "fulfilled") {
    throw new Error("doesNotReject did not return the fulfilment value");
}
if (await doesNotReject(() => Promise.resolve("real")) !== "real") {
    throw new Error("doesNotReject lost a native fulfilment");
}
let doesNotThenReads = 0;
const fulfillingThenable = {
    get then() {
        doesNotThenReads += 1;
        return function (resolve) {
            resolve(42);
        };
    },
};
if (await doesNotReject(() => fulfillingThenable) !== 42) {
    throw new Error("doesNotReject lost a thenable fulfilment");
}
if (doesNotThenReads !== 1) {
    throw new Error(`doesNotReject read then ${doesNotThenReads} times`);
}
const firstWins = {
    then(resolve, reject) {
        resolve("first");
        reject(new Error("second"));
    },
};
if (await doesNotReject(() => firstWins) !== "first") {
    throw new Error("doesNotReject did not use the first resolution");
}
error = await failsAsync("doesNotReject throwing then getter", () =>
    doesNotReject(() => ({
        get then() {
            throw new RangeError("getter");
        },
    })));
if (!(error instanceof AssertionError)) {
    throw new Error("doesNotReject accepted a throwing then getter");
}
error = await failsAsync("doesNotReject rejection", () =>
    doesNotReject(async () => {
        throw new Error("x");
    }));
if (!(error instanceof AssertionError) || error.operator !== "doesNotReject") {
    throw new Error("doesNotReject accepted a rejection");
}
error = await failsAsync("doesNotReject non-promise", () => doesNotReject(() => 1));
if (!(error instanceof AssertionError)) throw new Error("doesNotReject accepted a plain return");
error = await failsAsync("doesNotReject sync throw", () => doesNotReject(() => {
    throw new Error("x");
}));
if (!(error instanceof AssertionError)) throw new Error("doesNotReject accepted a sync throw");
"#,
    );
}

/// The `test` built-in must count sync and async passes, a sync failure, and a
/// pending registration. The counts are the embedding API's contract, not the
/// printed text.
#[test]
fn test_built_in_reports_sync_async_failure_and_pending() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
import test from "vael:test";
import assert from "vael:assert";

test("sync pass", () => assert.equals(1 + 1, 2));
test("async pass", async () => {
    await Promise.resolve();
    assert.equals(2 + 2, 4);
});
test("sync fail", () => assert.equals(1, 2));
test("pending case");
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("run file");
    let report = runtime.run_tests().expect("run tests");

    assert_eq!(report.total, 4);
    assert_eq!(report.passed, 2);
    assert_eq!(report.failed, 1);
    assert_eq!(report.pending, 1);
    assert_eq!(report.unhandled, 0);
    assert_eq!(report.failures.len(), 2);
    assert!(!report.all_passed());
}

/// A returned Promise that never settles is failed by the per-test timeout.
#[test]
fn test_built_in_timeout_is_a_failure() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
import test from "vael:test";
test("never", () => new Promise(() => {}), { timeout: 10 });
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("run file");
    let report = runtime.run_tests().expect("run tests");

    assert_eq!(report.total, 1);
    assert_eq!(report.failed, 1);
    assert!(report.failures[0].contains("did not settle within 10 ms"));
}

/// An unhandled rejection is observed while the test module is active and is
/// reported separately from executed tests.
#[test]
fn test_built_in_records_unhandled_rejections() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
import test from "vael:test";
test("schedules a rejection", () => {
    Promise.reject(new Error("unhandled from a test"));
});
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("run file");
    let report = runtime.run_tests().expect("run tests");

    assert_eq!(report.total, 1);
    assert_eq!(report.passed, 1);
    assert_eq!(report.unhandled, 1);
    assert!(!report.all_passed());
    assert!(
        report
            .failures
            .iter()
            .any(|line| line.contains("unhandled from a test")),
        "{:?}",
        report.failures
    );
}

/// A runtime that never imports `test` has no queue, not a pass.
#[test]
fn test_built_in_reports_nothing_when_unused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", "console.log(\"plain\");\n");

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("run file");
    let report = runtime.run_tests().expect("run tests");

    assert_eq!(report, vaeljs_core::TestReport::default());
    assert!(report.all_passed());
}

/// The async runner form must work on a caller's current-thread executor,
/// including an async test that awaits a resolved Promise.
#[test]
fn test_runner_async_form_works_inside_tokio() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
import test from "vael:test";
test("async pass", async () => { await Promise.resolve(); });
"#,
    );
    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("caller runtime");
    caller.block_on(async {
        let mut runtime = Runtime::new().expect("runtime");
        runtime.run_file_async(&file).await.expect("run file");
        let report = runtime.run_tests_async().await.expect("run tests");
        assert_eq!(report.total, 1);
        assert_eq!(report.passed, 1);
        assert!(report.all_passed());
    });
}

/// The default `run()` throws whenever the cumulative retained report has a
/// failure, including on a second call with no new tests.
#[test]
fn test_built_in_repeated_run_throws_on_retained_failure() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
import test, { run, TestFailureError } from "vael:test";
test("fails once", () => { throw new Error("retained"); });
let first = false;
let second = false;
try { await run(); } catch (error) { first = error instanceof TestFailureError; }
try { await run(); } catch (error) { second = error instanceof TestFailureError; }
if (!first || !second) throw new Error(`repeated run: ${first}/${second}`);
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&file)
        .expect("both default runs must throw and be caught");
    let report = runtime.run_tests().expect("run tests");

    assert_eq!(report.total, 1);
    assert_eq!(report.failed, 1);
    assert!(!report.all_passed());
}

/// A refed timer left by a test is reported as unsettled after the bounded
/// settle window, and `run_tests` returns instead of waiting for it.
#[test]
fn test_built_in_reports_unsettled_detached_timer() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
import test from "vael:test";
test("leaves a long timer", () => {
    Deno.core.createTimer(() => {}, 60_000, undefined, false, true, false);
});
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("run file");
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

/// A short detached timer that rejects is drained inside the bounded settle
/// window and reported as an unhandled rejection, not silently lost and not
/// mislabeled as unsettled.
#[test]
fn test_built_in_drains_short_detached_rejection() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
import test from "vael:test";
test("late rejection", () => {
    Deno.core.createTimer(
        () => { Promise.reject(new Error("late rejection")); },
        10,
        undefined,
        false,
        true,
        false,
    );
});
"#,
    );

    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("run file");
    let report = runtime.run_tests().expect("run tests");

    assert_eq!(report.total, 1);
    assert_eq!(report.passed, 1);
    assert_eq!(report.unhandled, 1);
    assert_eq!(report.unsettled, 0);
    assert!(!report.all_passed());
    assert!(
        report
            .failures
            .iter()
            .any(|line| line.contains("late rejection")),
        "{:?}",
        report.failures
    );
}

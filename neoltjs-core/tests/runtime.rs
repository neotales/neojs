//! Tests for the embedding API, driven through `Runtime` rather than the binary.

use std::fs;

use neoltjs_core::{Error, Runtime};

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

#[test]
fn typescript_entries_are_rejected_as_not_implemented() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write(&dir, "main.ts", "const value: number = 1;\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("typescript");
    let Error::UnsupportedEntry { reason, .. } = error else {
        panic!("expected an unsupported entry, got {error:?}");
    };
    assert!(reason.contains("TypeScript is not implemented"), "{reason}");
}

#[test]
fn imported_typescript_is_rejected_as_not_implemented() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "value.ts", "export default 1;\n");
    let main = write(&dir, "main.js", "import value from './value.ts';\n");

    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&main).expect_err("typescript import");
    let Error::Module { message, .. } = error else {
        panic!("expected a module error, got {error:?}");
    };
    assert!(
        message.contains("TypeScript is not implemented"),
        "{message}"
    );
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

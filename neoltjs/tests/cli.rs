//! End-to-end tests for the `neoltjs` executable.

use std::fs;
use std::process::Command;

fn write(dir: &tempfile::TempDir, name: &str, source: &str) {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create module directory");
    }
    fs::write(path, source).expect("write source");
}

fn run(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_neoltjs"))
        .args(args)
        .output()
        .expect("run neoltjs")
}

fn stdout(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8")
}

fn stderr(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8")
}

#[test]
fn runs_a_module_and_prints_console_output() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "hello.js", "console.log(\"hello from neojs\");\n");
    let file = dir.path().join("hello.js");

    let output = run(&[file.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "hello from neojs\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn runs_relative_imports_from_the_entry_directory() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "greet.js",
        "export function greet(name) {\n    return `hello, ${name}`;\n}\n",
    );
    write(
        &dir,
        "main.js",
        "import { greet } from \"./greet.js\";\n\nconsole.log(greet(\"world\"));\n",
    );
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "hello, world\n");
}

#[test]
fn drains_a_promise_chain_queued_by_the_entry() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "main.js",
        "Promise.resolve().then(() => console.log(\"then ran\"));\n",
    );
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "then ran\n");
}

#[test]
fn drains_an_unawaited_dynamic_import() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "child.js", "console.log(\"dynamic ran\");\n");
    write(&dir, "main.js", "import(\"./child.js\");\n");
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "dynamic ran\n");
}

#[test]
fn an_unhandled_rejection_exits_one_with_its_cause() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "main.js",
        "Promise.reject(new Error('rejected without a handler'));\n",
    );
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.contains("unhandled promise rejection"), "{error}");
    assert!(error.contains("rejected without a handler"), "{error}");
}

#[test]
fn a_rejection_handled_later_exits_zero() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "main.js",
        "const rejected = Promise.reject(new Error('handled'));\nPromise.resolve().then(() => rejected.catch(() => {}));\n",
    );
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "");
}

#[test]
fn a_failing_module_exits_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "main.js", "throw new Error('boom');\n");
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    let error = stderr(&output);
    assert!(error.starts_with("neoltjs: cannot evaluate"), "{error}");
    assert!(error.contains("boom"), "{error}");
}

#[test]
fn a_missing_entry_is_a_startup_failure() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("missing.js");

    let output = run(&[file.as_os_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("cannot read"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_unsupported_extension_is_a_startup_failure_with_the_shared_reason() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "old.cjs", "module.exports = 1;\n");
    let file = dir.path().join("old.cjs");

    let output = run(&[file.as_os_str()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains(".cjs extension is not supported"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn no_arguments_is_a_startup_failure() {
    let output = run(&[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("no input file"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn help_and_version_succeed() {
    let help = run(&[std::ffi::OsStr::new("--help")]);
    assert!(help.status.success());
    assert!(stdout(&help).contains("Usage:"), "{}", stdout(&help));

    let version = run(&[std::ffi::OsStr::new("--version")]);
    assert!(version.status.success());
    assert!(
        stdout(&version).starts_with("neoltjs "),
        "{}",
        stdout(&version)
    );
}

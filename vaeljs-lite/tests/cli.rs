//! End-to-end tests for the `vaeljs-lite` executable.

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
    Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
        .args(args)
        .output()
        .expect("run vaeljs-lite")
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
    write(&dir, "hello.js", "console.log(\"hello from vaeljs\");\n");
    let file = dir.path().join("hello.js");

    let output = run(&[file.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "hello from vaeljs\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn exposes_the_lite_engine_and_the_shared_builtin() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "main.js",
        r#"
console.log(Vael.engine);
console.log(Vael.getBuiltInModulesList().length);
console.log(Object.isFrozen(Vael.getBuiltInModulesList()));
console.log(typeof Vael.getBuiltInModule("vael:age"));
console.log(typeof Vael.getBuiltInModule("age"));
console.log(typeof Vael.getBuiltInModule("vael:chars"));
console.log(typeof Vael.getBuiltInModule("vael:console"));
console.log(typeof Vael.getBuiltInModule("vael:crypto"));
console.log(typeof Vael.getBuiltInModule("vael:exec"));
console.log(typeof Vael.getBuiltInModule("vael:fetch"));
console.log(typeof Vael.getBuiltInModule("vael:fs"));
console.log(typeof Vael.getBuiltInModule("vael:os"));
console.log(typeof Vael.getBuiltInModule("vael:path"));
console.log(typeof Vael.getBuiltInModule("vael:slices"));
console.log(typeof Vael.getBuiltInModule("vael:ssh"));
console.log(typeof Vael.getBuiltInModule("vael:strings"));
console.log(typeof Vael.getBuiltInModule("vael:template"));
console.log(typeof Vael.getBuiltInModule("vael:uuid"));
"#,
    );
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    let expected_count = 14
        + usize::from(cfg!(feature = "vael-exec"))
        + usize::from(cfg!(feature = "vael-fetch"))
        + usize::from(cfg!(feature = "vael-template"));
    let expected_exec = if cfg!(feature = "vael-exec") {
        "object"
    } else {
        "undefined"
    };
    let expected_fetch = if cfg!(feature = "vael-fetch") {
        "object"
    } else {
        "undefined"
    };
    let expected_template = if cfg!(feature = "vael-template") {
        "object"
    } else {
        "undefined"
    };
    let expected = format!(
        "vael-lite\n{expected_count}\ntrue\nundefined\nundefined\nobject\nobject\nobject\n{expected_exec}\n{expected_fetch}\nobject\nobject\nobject\nobject\nobject\nobject\n{expected_template}\nobject\n"
    );
    assert_eq!(stdout(&output), expected);
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
fn entry_error_diagnostics_do_not_run_getters() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "main.js",
        r#"
const error = new Error("original");
Object.defineProperty(error, "message", {
    get() {
        console.log("GETTER RAN");
        throw new Error("getter threw");
    },
});
throw error;
"#,
    );
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let error = stderr(&output);
    assert!(
        error.contains("<rejected Error with no readable message>"),
        "{error}"
    );
    assert!(!error.contains("GETTER RAN"), "{error}");
}

#[test]
fn unhandled_proxy_rejection_exits_one_without_traps() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "main.js",
        r#"
const reason = new Proxy({}, {
    get() {
        console.log("TRAP RAN");
        throw new Error("get trap");
    },
    getOwnPropertyDescriptor() {
        console.log("TRAP RAN");
        throw new Error("descriptor trap");
    },
});
Promise.reject(reason);
"#,
    );
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let error = stderr(&output);
    assert!(error.contains("<rejected Proxy>"), "{error}");
    assert!(!error.contains("TRAP RAN"), "{error}");
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
    assert!(error.starts_with("vjs-lite: cannot evaluate"), "{error}");
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
        stdout(&version).starts_with("vjs-lite "),
        "{}",
        stdout(&version)
    );
}

#[test]
fn test_subcommand_runs_globs_deduplicates_and_reports_summary() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "a.test.js",
        r#"
import test from "vael:test";
import assert from "vael:assert";
test("sync pass", () => assert.equals({ ok: true }, { ok: true }));
"#,
    );
    write(
        &dir,
        "b.test.js",
        r#"
import test from "vael:test";
import assert from "vael:assert";
test("async pass", async () => {
    const val = await Promise.resolve(42);
    assert.strictEquals(val, 42);
});
"#,
    );
    let glob_arg = format!("{}/*.test.js", dir.path().display());
    let a_file = dir.path().join("a.test.js");

    let output = run(&[
        std::ffi::OsStr::new("test"),
        std::ffi::OsStr::new(&glob_arg),
        a_file.as_os_str(),
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("ok 1 - sync pass"), "{out}");
    assert!(out.contains("ok 1 - async pass"), "{out}");
    assert!(
        out.contains("# 2 selected file(s): 2 test(s): 2 passed, 0 failed, 0 pending, 0 unhandled, 0 unsettled; 0 file(s) failed"),
        "{out}"
    );
}

#[test]
fn test_subcommand_continues_after_failure_and_isolates_files() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "a-fail.test.js",
        r#"
import test from "vael:test";
globalThis.leaked = true;
test("fails", () => { throw new Error("boom"); });
"#,
    );
    write(
        &dir,
        "b-pass.test.js",
        r#"
import test from "vael:test";
test("isolated", () => {
    if (globalThis.leaked !== undefined) throw new Error("leaked");
});
"#,
    );
    let glob_arg = format!("{}/*.test.js", dir.path().display());

    let output = run(&[
        std::ffi::OsStr::new("test"),
        std::ffi::OsStr::new(&glob_arg),
    ]);
    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("not ok 1 - fails"), "{out}");
    assert!(out.contains("ok 1 - isolated"), "{out}");
    assert!(
        out.contains("# 2 selected file(s): 2 test(s): 1 passed, 1 failed"),
        "{out}"
    );
}

#[test]
fn test_subcommand_fails_empty_files_and_unmatched_globs() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "empty.test.js", "console.log('no tests');\n");
    let empty_file = dir.path().join("empty.test.js");

    let empty = run(&[std::ffi::OsStr::new("test"), empty_file.as_os_str()]);
    assert_eq!(empty.status.code(), Some(1), "stderr: {}", stderr(&empty));
    assert!(
        stderr(&empty).contains("no tests registered"),
        "{}",
        stderr(&empty)
    );

    let missing_glob = format!("{}/missing*.test.js", dir.path().display());
    let unmatched = run(&[
        std::ffi::OsStr::new("test"),
        std::ffi::OsStr::new(&missing_glob),
    ]);
    assert_eq!(unmatched.status.code(), Some(2));
    assert!(
        stderr(&unmatched).contains("matched no files"),
        "{}",
        stderr(&unmatched)
    );

    let no_args = run(&[std::ffi::OsStr::new("test")]);
    assert_eq!(no_args.status.code(), Some(2));
    assert!(
        stderr(&no_args).contains("no test files or globs given"),
        "{}",
        stderr(&no_args)
    );
}

#[test]
fn resolves_local_imports_prefix_imports_and_workspace_packages() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "vael.jsonc",
        r#"{
            "name": "@lite/ws",
            "workspaces": ["packages/*"],
            "imports": {
                "root-greet": "./shared/greet.js",
                "utils/": "./shared/utils/"
            }
        }"#,
    );
    write(
        &dir,
        "shared/greet.js",
        "export const greet = (name) => `hi ${name}`;\n",
    );
    write(
        &dir,
        "shared/utils/num.mjs",
        "export const double = (n) => n * 2;\n",
    );
    write(
        &dir,
        "packages/a/vael.jsonc",
        r#"{
            "name": "@lite/a",
            "exports": "./mod.js",
            "imports": { "local-a": "./local.js" }
        }"#,
    );
    write(&dir, "packages/a/local.js", "export const tag = 'A';\n");
    write(
        &dir,
        "packages/b/vael.jsonc",
        r#"{
            "name": "@lite/b",
            "exports": {
                ".": "./index.js",
                "./sub": "./sub.js"
            }
        }"#,
    );
    write(&dir, "packages/b/index.js", "export const b = 'B';\n");
    write(&dir, "packages/b/sub.js", "export const sub = 'SUB';\n");
    write(&dir, "packages/b/secret.js", "export const secret = 1;\n");

    let entry = dir.path().join("packages/a/mod.js");
    write(
        &dir,
        "packages/a/mod.js",
        r#"
import { greet } from "root-greet";
import { double } from "utils/num.mjs";
import { tag } from "local-a";
import { b } from "@lite/b";
import { sub } from "@lite/b/sub";
console.log(`${greet("lite")}:${double(5)}:${tag}:${b}:${sub}`);
"#,
    );

    let output = run(&[entry.as_os_str()]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "hi lite:10:A:B:SUB\n");

    // Unexported subpath must fail with exit code 1.
    let bad = dir.path().join("packages/a/bad.js");
    write(
        &dir,
        "packages/a/bad.js",
        "import { secret } from '@lite/b/secret'; console.log(secret);\n",
    );
    let bad_out = run(&[bad.as_os_str()]);
    assert_eq!(bad_out.status.code(), Some(1));
    assert!(
        stderr(&bad_out).contains("not exported"),
        "{}",
        stderr(&bad_out)
    );
}

#[test]
fn rejects_remote_import_map_targets_clearly() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "vael.jsonc",
        r#"{
            "name": "@lite/remote",
            "imports": {
                "remote": "https://example.test/mod.js",
                "jsr-lib": "jsr:@scope/pkg@1.0.0"
            }
        }"#,
    );
    let file1 = dir.path().join("r1.js");
    write(&dir, "r1.js", "import 'remote';\n");
    let out1 = run(&[file1.as_os_str()]);
    assert_eq!(out1.status.code(), Some(1));
    let err1 = stderr(&out1);
    assert!(
        err1.contains("remote module loading is disabled")
            || err1.contains("remote module loading is not enabled"),
        "{err1}"
    );

    let file2 = dir.path().join("r2.js");
    write(&dir, "r2.js", "import 'jsr-lib';\n");
    let out2 = run(&[file2.as_os_str()]);
    assert_eq!(out2.status.code(), Some(1));
    let err2 = stderr(&out2);
    assert!(
        err2.contains("jsr: modules require remote module loading")
            || err2.contains("remote module loading is not enabled"),
        "{err2}"
    );
}

#[test]
fn config_and_no_config_flags_and_malformed_manifest_exit_codes() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "vael.jsonc", "{ malformed");
    write(
        &dir,
        "alt/custom.jsonc",
        r#"{ "name": "@lite/alt", "imports": { "dep": "./dep.js" } }"#,
    );
    write(&dir, "alt/dep.js", "export const msg = 'explicit';\n");
    let main = dir.path().join("main.js");
    write(
        &dir,
        "main.js",
        "import { msg } from 'dep'; console.log(msg);\n",
    );

    // Auto discovery fails closed with exit code 2 on malformed vael.jsonc.
    let malformed = run(&[main.as_os_str()]);
    assert_eq!(malformed.status.code(), Some(2));
    assert!(
        stderr(&malformed).contains("cannot load manifest"),
        "{}",
        stderr(&malformed)
    );

    // `--config <path>` uses explicit manifest and exits 0.
    let custom = dir.path().join("alt/custom.jsonc");
    let explicit = run(&[
        std::ffi::OsStr::new("--config"),
        custom.as_os_str(),
        main.as_os_str(),
    ]);
    assert_eq!(
        explicit.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&explicit)
    );
    assert_eq!(stdout(&explicit), "explicit\n");

    // `--no-config` skips discovery -> bare specifier fails with exit code 1.
    let no_cfg = run(&[std::ffi::OsStr::new("--no-config"), main.as_os_str()]);
    assert_eq!(no_cfg.status.code(), Some(1));
    assert!(
        stderr(&no_cfg).contains("bare module specifiers"),
        "{}",
        stderr(&no_cfg)
    );

    // Missing `--config` -> exit code 2.
    let missing = dir.path().join("missing.jsonc");
    let missing_out = run(&[
        std::ffi::OsStr::new("--config"),
        missing.as_os_str(),
        main.as_os_str(),
    ]);
    assert_eq!(missing_out.status.code(), Some(2));

    // Conflicting `--config` + `--no-config` -> exit code 2.
    let conflict = run(&[
        std::ffi::OsStr::new("--config"),
        custom.as_os_str(),
        std::ffi::OsStr::new("--no-config"),
        main.as_os_str(),
    ]);
    assert_eq!(conflict.status.code(), Some(2));
    assert!(
        stderr(&conflict).contains("cannot be used together"),
        "{}",
        stderr(&conflict)
    );
}

#[test]
fn test_subcommand_runs_workspace_tests() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "vael.jsonc",
        r#"{ "name": "@lite/ws-test", "workspaces": ["packages/*"] }"#,
    );
    write(
        &dir,
        "packages/math/vael.jsonc",
        r#"{ "name": "@lite/math", "exports": "./mod.js" }"#,
    );
    write(
        &dir,
        "packages/math/mod.js",
        "export const add = (a, b) => a + b;\n",
    );
    write(
        &dir,
        "packages/app/vael.jsonc",
        r#"{
            "name": "@lite/app",
            "exports": "./mod.js",
            "imports": { "base": "./base.js" }
        }"#,
    );
    write(&dir, "packages/app/base.js", "export const BASE = 10;\n");
    write(&dir, "packages/app/mod.js", "export const ok = true;\n");
    let test_file = dir.path().join("packages/app/tests/ws.test.js");
    write(
        &dir,
        "packages/app/tests/ws.test.js",
        r#"
import test from "vael:test";
import assert from "vael:assert";
import { add } from "@lite/math";
import { BASE } from "base";

test("workspace test in vaeljs-lite", () => {
    assert.strictEquals(add(BASE, 5), 15);
});
"#,
    );

    let pass = run(&[std::ffi::OsStr::new("test"), test_file.as_os_str()]);
    assert_eq!(pass.status.code(), Some(0), "stderr: {}", stderr(&pass));
    assert!(stdout(&pass).contains("ok 1 - workspace test in vaeljs-lite"));

    let disabled = run(&[
        std::ffi::OsStr::new("test"),
        std::ffi::OsStr::new("--no-config"),
        test_file.as_os_str(),
    ]);
    assert_eq!(disabled.status.code(), Some(1));
    assert!(
        stderr(&disabled).contains("bare module specifiers"),
        "{}",
        stderr(&disabled)
    );
}

#[test]
fn runs_erasable_typescript_and_mts_with_imports() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "lib/greeter.mts",
        r#"
export interface Greeting {
    text: string;
}
export function greet(name: string): Greeting {
    return { text: `hello, ${name}` };
}
"#,
    );
    write(
        &dir,
        "main.ts",
        r#"
import type { Greeting } from "./lib/greeter.mts";
import { greet } from "./lib/greeter.mts";
const g: Greeting = greet("ts-lite" as string satisfies string);
console.log(g.text);
"#,
    );
    let entry = dir.path().join("main.ts");

    let output = run(&[entry.as_os_str()]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "hello, ts-lite\n");
}

#[test]
fn rejects_non_erasable_typescript_with_exit_one_and_tsx_with_exit_two() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "bad.ts", "enum Color { Red }\n");
    let bad_ts = dir.path().join("bad.ts");
    let ts_out = run(&[bad_ts.as_os_str()]);
    assert_eq!(ts_out.status.code(), Some(1), "stderr: {}", stderr(&ts_out));
    assert!(
        stderr(&ts_out).contains("enum is not supported in strip-only mode"),
        "{}",
        stderr(&ts_out)
    );

    write(&dir, "view.tsx", "export const x = <div />;\n");
    let tsx = dir.path().join("view.tsx");
    let tsx_out = run(&[tsx.as_os_str()]);
    assert_eq!(
        tsx_out.status.code(),
        Some(2),
        "stderr: {}",
        stderr(&tsx_out)
    );
    assert!(
        stderr(&tsx_out).contains("JSX and TSX"),
        "{}",
        stderr(&tsx_out)
    );
}

#[test]
fn test_subcommand_runs_typescript_test_files() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "math.test.ts",
        r#"
import test from "vael:test";
import assert from "vael:assert";

type Pair = [number, number];
const add = (pair: Pair): number => pair[0] + pair[1];

test("typed test passes", () => {
    assert.strictEquals(add([20, 22]), 42);
});
"#,
    );
    let glob_arg = format!("{}/*.test.ts", dir.path().display());

    let output = run(&[
        std::ffi::OsStr::new("test"),
        std::ffi::OsStr::new(&glob_arg),
    ]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let out = stdout(&output);
    assert!(out.contains("ok 1 - typed test passes"), "{out}");
    assert!(
        out.contains("# 1 selected file(s): 1 test(s): 1 passed, 0 failed"),
        "{out}"
    );
}

#[test]
fn types_subcommand_defaults_to_lite_and_supports_v8_out_dir_and_help() {
    let default_out = run(&[std::ffi::OsStr::new("types")]);
    assert_eq!(
        default_out.status.code(),
        Some(0),
        "{}",
        stderr(&default_out)
    );
    assert_eq!(stderr(&default_out), "");
    assert_eq!(
        stdout(&default_out),
        vaeljs_dts::render_bundle(vaeljs_dts::Profile::Lite)
    );

    for args in [
        ["types", "--profile", "lite"].as_slice(),
        ["types", "--profile=lite"].as_slice(),
    ] {
        let out = run(&args.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>());
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        assert_eq!(stderr(&out), "");
        assert_eq!(
            stdout(&out),
            vaeljs_dts::render_bundle(vaeljs_dts::Profile::Lite)
        );
    }

    for args in [
        ["types", "--profile", "v8"].as_slice(),
        ["types", "--profile=v8"].as_slice(),
    ] {
        let out = run(&args.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>());
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
        assert_eq!(stderr(&out), "");
        assert_eq!(
            stdout(&out),
            vaeljs_dts::render_bundle(vaeljs_dts::Profile::V8)
        );
    }

    let dir = tempfile::tempdir().expect("temp dir");
    let lite_dir = dir.path().join("types-lite");
    let out_lite = run(&[
        std::ffi::OsStr::new("types"),
        std::ffi::OsStr::new("--out-dir"),
        lite_dir.as_os_str(),
    ]);
    assert_eq!(out_lite.status.code(), Some(0), "{}", stderr(&out_lite));
    assert_eq!(stdout(&out_lite), "");
    assert_eq!(stderr(&out_lite), "");
    for file in vaeljs_dts::files(vaeljs_dts::Profile::Lite) {
        let written = fs::read_to_string(lite_dir.join(file.path))
            .unwrap_or_else(|err| panic!("missing {}: {err}", file.path));
        assert_eq!(written, file.content, "mismatch for {}", file.path);
    }
    assert!(lite_dir.join("vael/ssh.d.ts").is_file());
    assert!(!lite_dir.join("vael/age.d.ts").exists());

    for flag in ["--help", "-h"] {
        let help = run(&[std::ffi::OsStr::new("types"), std::ffi::OsStr::new(flag)]);
        assert_eq!(help.status.code(), Some(0), "{flag}: {}", stderr(&help));
        assert_eq!(stderr(&help), "");
        let text = stdout(&help);
        assert!(text.contains("vjs-lite types"), "{text}");
        assert!(text.contains("--profile <lite|v8>"), "{text}");
        assert!(text.contains("--out-dir <dir>"), "{text}");
    }

    let bad_profile = run(&[
        std::ffi::OsStr::new("types"),
        std::ffi::OsStr::new("--profile"),
        std::ffi::OsStr::new("deno"),
    ]);
    assert_eq!(bad_profile.status.code(), Some(2));
    assert!(stderr(&bad_profile).contains("invalid profile `deno`"));

    let unknown = run(&[
        std::ffi::OsStr::new("types"),
        std::ffi::OsStr::new("--unknown"),
    ]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stderr(&unknown).contains("unknown option `--unknown`"));

    let extra = run(&[std::ffi::OsStr::new("types"), std::ffi::OsStr::new("extra")]);
    assert_eq!(extra.status.code(), Some(2));
    assert!(stderr(&extra).contains("unexpected argument `extra`"));
}

#[test]
#[cfg(feature = "node-compat")]
fn no_node_compat_flag_disables_node_builtins_in_run_and_test() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "main.js",
        "import path from \"node:path\";\nconsole.log(path.join(\"a\", \"b\"));\n",
    );
    write(
        &dir,
        "node.test.js",
        "import test from \"vael:test\";\nimport path from \"node:path\";\ntest(\"ok\", () => { path.join(\"a\", \"b\"); });\n",
    );
    let main_path = dir.path().join("main.js");
    let test_path = dir.path().join("node.test.js");

    let ok = run(&[main_path.as_os_str()]);
    assert_eq!(ok.status.code(), Some(0), "{}", stderr(&ok));

    let disabled_run = run(&[
        std::ffi::OsStr::new("--no-node-compat"),
        main_path.as_os_str(),
    ]);
    assert_eq!(disabled_run.status.code(), Some(1));
    assert!(
        stderr(&disabled_run).contains("node:"),
        "{}",
        stderr(&disabled_run)
    );

    for test_args in [
        [
            std::ffi::OsStr::new("--no-node-compat"),
            std::ffi::OsStr::new("test"),
            test_path.as_os_str(),
        ]
        .as_slice(),
        [
            std::ffi::OsStr::new("test"),
            std::ffi::OsStr::new("--no-node-compat"),
            test_path.as_os_str(),
        ]
        .as_slice(),
    ] {
        let disabled_test = run(test_args);
        assert_eq!(disabled_test.status.code(), Some(1));
        assert!(
            stderr(&disabled_test).contains("node:"),
            "{test_args:?}: {}",
            stderr(&disabled_test)
        );
    }
}

#[test]
#[cfg(feature = "node-compat")]
fn node_process_exit_and_exit_code_propagate_through_cli() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "exit_42.js",
        "import process from \"node:process\";\nconsole.log(\"before\");\nprocess.exit(42);\nconsole.log(\"after\");\n",
    );
    write(
        &dir,
        "exit_code_7.js",
        "import process from \"node:process\";\nprocess.exitCode = 7;\nconsole.log(\"done\");\n",
    );

    let exit_out = run(&[dir.path().join("exit_42.js").as_os_str()]);
    assert_eq!(exit_out.status.code(), Some(42), "{}", stderr(&exit_out));
    assert_eq!(stdout(&exit_out), "before\n");

    let code_out = run(&[dir.path().join("exit_code_7.js").as_os_str()]);
    assert_eq!(code_out.status.code(), Some(7), "{}", stderr(&code_out));
    assert_eq!(stdout(&code_out), "done\n");
}

#[test]
#[cfg(not(feature = "node-compat"))]
fn node_compat_disabled_at_compile_time_refuses_node_specifiers() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "main.js", "import \"node:path\";\n");
    let main_path = dir.path().join("main.js");
    let output = run(&[main_path.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("node:"), "{}", stderr(&output));
}

#[test]
#[cfg(unix)]
fn help_and_usage_use_symlink_program_name() {
    let dir = tempfile::tempdir().expect("temp dir");
    let symlink_path = dir.path().join("my-vjs");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_vjs-lite"), &symlink_path)
        .expect("create symlink");

    for args in [
        &["--help"][..],
        &["-h"][..],
        &["test", "--help"][..],
        &["test", "-h"][..],
    ] {
        let output = Command::new(&symlink_path)
            .args(args)
            .output()
            .expect("run symlinked vjs-lite");
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            stderr(&output)
        );
        let text = stdout(&output);
        assert!(text.contains("  my-vjs <file>"), "{args:?}: {text}");
        assert!(
            text.contains("  my-vjs test <file-or-glob>..."),
            "{args:?}: {text}"
        );
        assert!(text.contains("`my-vjs test`"), "{args:?}: {text}");
        assert!(
            text.contains("  2  my-vjs could not start:"),
            "{args:?}: {text}"
        );
        assert!(!text.contains("  vjs-lite "), "{args:?}: {text}");
        assert!(!text.contains("`vjs-lite test`"), "{args:?}: {text}");
    }

    for flag in ["--help", "-h"] {
        let output = Command::new(&symlink_path)
            .args(["types", flag])
            .output()
            .expect("run symlinked vjs-lite types --help");
        assert_eq!(output.status.code(), Some(0), "{flag}: {}", stderr(&output));
        let text = stdout(&output);
        assert!(text.contains("  my-vjs types [options]"), "{text}");
        assert!(text.contains("  my-vjs types > vjs-lite.d.ts"), "{text}");
        assert!(!text.contains("  vjs-lite types"), "{text}");
    }

    let bad = Command::new(&symlink_path)
        .output()
        .expect("run symlinked vjs-lite with no args");
    assert_eq!(bad.status.code(), Some(2));
    assert_eq!(
        stderr(&bad),
        "my-vjs: no input file\nTry 'my-vjs --help'.\n"
    );

    let version = Command::new(&symlink_path)
        .arg("--version")
        .output()
        .expect("run symlinked vjs-lite --version");
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        stdout(&version),
        format!("vjs-lite {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[cfg(feature = "remote")]
#[test]
fn remote_url_entrypoints_require_allow_remote_and_reject_http() {
    for entry in [
        "https://example.test/mod.js",
        "gh:owner/repo@v1/mod.ts",
        "jsr:@scope/pkg@1.0.0",
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
            .env_remove("VAELJS_ALLOW_REMOTE")
            .arg(entry)
            .output()
            .expect("run vjs-lite");
        assert_eq!(out.status.code(), Some(2), "{entry}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("remote module loading is disabled"),
            "{entry}: {}",
            stderr(&out)
        );
    }

    let http_out = Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
        .args(["--allow-remote", "http://example.test/mod.js"])
        .output()
        .expect("run vjs-lite with http");
    assert_eq!(http_out.status.code(), Some(2));
    assert!(
        stderr(&http_out).contains("https:"),
        "{}",
        stderr(&http_out)
    );

    let bad_env = Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
        .env("VAELJS_ALLOW_REMOTE", "maybe")
        .arg("https://example.test/mod.js")
        .output()
        .expect("run vjs-lite with bad env");
    assert_eq!(bad_env.status.code(), Some(2));
    assert!(
        stderr(&bad_env).contains("invalid VAELJS_ALLOW_REMOTE"),
        "{}",
        stderr(&bad_env)
    );
}

#[cfg(all(feature = "remote", feature = "npm"))]
#[test]
fn ancestor_vael_jsonc_import_maps_for_npm_jsr_gh_and_https_in_run_and_test() {
    use std::collections::{BTreeMap, HashMap};
    use std::io::Write as _;
    use std::sync::Arc;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use sha2::{Digest, Sha256, Sha512};
    use vaeljs_lite_core::test_support::TestRemoteResponse;
    use vaeljs_lite_core::{RemoteOptions, Runtime, RuntimeOptions};
    use vaeljs_manifest::discover_manifest;
    use vaeljs_npm::{
        CasStore, DEFAULT_REGISTRY_ORIGIN, LockedPackage, Lockfile, NpmLimits, TarballIngestor,
    };

    fn sha256_hex(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        let mut out = String::with_capacity(digest.len() * 2);
        for b in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{b:02x}");
        }
        out
    }

    fn write_tar_header(name: &str, size: usize) -> [u8; 512] {
        let mut header = [0u8; 512];
        let name_bytes = name.as_bytes();
        let copy_len = name_bytes.len().min(100);
        header[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
        header[100..108].copy_from_slice(b"0000644\0");
        header[108..116].copy_from_slice(b"0000000\0");
        header[116..124].copy_from_slice(b"0000000\0");
        let size_str = format!("{size:011o}\0");
        header[124..136].copy_from_slice(size_str.as_bytes());
        header[136..148].copy_from_slice(b"00000000000\0");
        header[148..156].copy_from_slice(b"        ");
        header[156] = b'0';
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        let csum = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(csum.as_bytes());
        header
    }

    fn build_tgz(files: &[(&str, &str)]) -> (Vec<u8>, String) {
        let mut tar_buf = Vec::new();
        for (rel_path, content) in files {
            let full = format!("package/{rel_path}");
            let bytes = content.as_bytes();
            tar_buf.extend_from_slice(&write_tar_header(&full, bytes.len()));
            if !bytes.is_empty() {
                tar_buf.extend_from_slice(bytes);
                let pad = (512 - (bytes.len() % 512)) % 512;
                tar_buf.resize(tar_buf.len() + pad, 0);
            }
        }
        tar_buf.resize(tar_buf.len() + 1024, 0);
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(&tar_buf).expect("write tar");
        let tgz = gz.finish().expect("finish gzip");
        let sri = format!("sha512-{}", BASE64_STANDARD.encode(Sha512::digest(&tgz)));
        (tgz, sri)
    }

    let xdg_cache = tempfile::tempdir().expect("xdg cache");
    let packages_cache = xdg_cache.path().join("vaeljs/packages");
    let cas = CasStore::new(&packages_cache);
    let (npm_tgz, npm_sri) = build_tgz(&[
        (
            "package.json",
            r#"{
                "name": "demo-pkg",
                "version": "1.0.0",
                "type": "module",
                "exports": { ".": "./index.ts", "./sub": "./sub.ts" }
            }"#,
        ),
        (
            "index.ts",
            "export const npmRoot: string = 'cli-npm-root';\n",
        ),
        ("sub.ts", "export const npmSub: string = 'cli-npm-sub';\n"),
    ]);
    let npm_tree = TarballIngestor::new(cas.clone(), NpmLimits::default())
        .ingest(DEFAULT_REGISTRY_ORIGIN, &npm_sri, &npm_tgz)
        .expect("ingest npm tarball")
        .tree_sha256;

    let project = tempfile::tempdir().expect("project");
    let mut lock = Lockfile::new();
    lock.specifiers
        .insert("demo-pkg@^1.0.0".to_owned(), "demo-pkg@1.0.0".to_owned());
    lock.packages.insert(
        "demo-pkg@1.0.0".to_owned(),
        LockedPackage {
            version: "1.0.0".to_owned(),
            integrity: npm_sri,
            tree_sha256: Some(npm_tree),
            tarball_url: "https://registry.npmjs.org/demo-pkg/-/1.0.0.tgz".to_owned(),
            dependencies: BTreeMap::new(),
        },
    );
    lock.write_atomic(&project.path().join("vael.lock"))
        .unwrap();

    let jsr_mod_bytes = b"export const jsrVal: string = 'cli-jsr-root';\n";
    let jsr_sub_bytes = b"export const jsrSubVal: string = 'cli-jsr-sub';\n";
    let mut remote_replies: HashMap<String, Vec<u8>> = HashMap::new();
    remote_replies.insert(
        "https://cdn.example.test/mod.ts".to_owned(),
        b"export const httpsVal: string = 'cli-https-root';\n".to_vec(),
    );
    remote_replies.insert(
        "https://raw.githubusercontent.com/acme/repo/v1.0.0/mod.ts".to_owned(),
        b"export const ghVal: string = 'cli-gh-root';\n".to_vec(),
    );
    remote_replies.insert(
        "https://raw.githubusercontent.com/acme/repo/v1.0.0/alt.ts".to_owned(),
        b"export const githubVal: string = 'cli-github-alt';\n".to_vec(),
    );
    remote_replies.insert(
        "https://jsr.io/@demo/pkg/meta.json".to_owned(),
        br#"{"scope":"demo","name":"pkg","latest":"1.0.0","versions":{"1.0.0":{}}}"#.to_vec(),
    );
    remote_replies.insert(
        "https://jsr.io/@demo/pkg/1.0.0_meta.json".to_owned(),
        format!(
            r#"{{
                "exports": {{ ".": "./mod.ts", "./sub": "./sub.ts" }},
                "manifest": {{
                    "/mod.ts": {{ "size": {}, "checksum": "sha256-{}" }},
                    "/sub.ts": {{ "size": {}, "checksum": "sha256-{}" }}
                }}
            }}"#,
            jsr_mod_bytes.len(),
            sha256_hex(jsr_mod_bytes),
            jsr_sub_bytes.len(),
            sha256_hex(jsr_sub_bytes),
        )
        .into_bytes(),
    );
    remote_replies.insert(
        "https://jsr.io/@demo/pkg/1.0.0/mod.ts".to_owned(),
        jsr_mod_bytes.to_vec(),
    );
    remote_replies.insert(
        "https://jsr.io/@demo/pkg/1.0.0/sub.ts".to_owned(),
        jsr_sub_bytes.to_vec(),
    );
    let remote_replies = Arc::new(remote_replies);

    write(
        &project,
        "vael.jsonc",
        r#"{
            "name": "@demo/cli-lite-ancestor",
            "imports": {
                "my-npm": "npm:demo-pkg@^1.0.0",
                "my-npm/": "npm:demo-pkg@^1.0.0/",
                "my-jsr": "jsr:@demo/pkg@^1.0.0",
                "my-jsr/": "jsr:@demo/pkg@^1.0.0/",
                "my-gh": "gh:acme/repo@v1.0.0/mod.ts",
                "my-github": "github:acme/repo@v1.0.0/alt.ts",
                "my-https": "https://cdn.example.test/mod.ts"
            }
        }"#,
    );

    let main_src = r#"
import { npmRoot } from "my-npm";
import { npmSub } from "my-npm/sub";
const dynNpm = await import("my-npm");
console.log(`${npmRoot}|${npmSub}|${dynNpm.npmRoot}`);
"#;
    let test_src = r#"
import test from "vael:test";
import assert from "vael:assert";
import { npmRoot } from "my-npm";
import { npmSub } from "my-npm/sub";

test("ancestor vael.jsonc npm exact and prefix subpath", async () => {
    const dynNpm = await import("my-npm");
    assert.strictEquals(npmRoot, "cli-npm-root");
    assert.strictEquals(npmSub, "cli-npm-sub");
    assert.strictEquals(dynNpm.npmRoot, "cli-npm-root");
});
"#;
    write(&project, "apps/nested/sub/main.ts", main_src);
    write(&project, "apps/nested/sub/app.test.ts", test_src);

    let run_in_project = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
            .current_dir(project.path())
            .env_remove("VAELJS_ALLOW_REMOTE")
            .env("XDG_CACHE_HOME", xdg_cache.path())
            .args(args)
            .output()
            .expect("run vjs-lite")
    };

    // 1. Offline CAS + ancestor `vael.lock` succeeds in `vjs-lite` and `vjs-lite test`.
    let npm_run = run_in_project(&["--frozen-lockfile", "--offline", "apps/nested/sub/main.ts"]);
    assert_eq!(
        npm_run.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&npm_run)
    );
    assert_eq!(stdout(&npm_run), "cli-npm-root|cli-npm-sub|cli-npm-root\n");

    let npm_test = run_in_project(&[
        "test",
        "--frozen-lockfile",
        "--offline",
        "apps/nested/sub/app.test.ts",
    ]);
    assert_eq!(
        npm_test.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&npm_test),
        stderr(&npm_test)
    );
    assert!(
        stdout(&npm_test).contains("ok 1 - ancestor vael.jsonc npm exact and prefix subpath"),
        "{}",
        stdout(&npm_test)
    );
    assert!(!project.path().join("node_modules").exists());

    // 2. `--no-npm` and `--no-config` fail cleanly in both `vjs-lite` and `vjs-lite test`.
    for args in [
        ["--no-npm", "apps/nested/sub/main.ts"].as_slice(),
        ["test", "--no-npm", "apps/nested/sub/app.test.ts"].as_slice(),
    ] {
        let out = run_in_project(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("--no-npm"),
            "{args:?}: {}",
            stderr(&out)
        );
    }
    for args in [
        ["--no-config", "apps/nested/sub/main.ts"].as_slice(),
        ["test", "--no-config", "apps/nested/sub/app.test.ts"].as_slice(),
    ] {
        let out = run_in_project(args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
        assert!(
            stderr(&out).contains("bare module specifiers"),
            "{args:?}: {}",
            stderr(&out)
        );
    }

    // 3. Seed remote cache in `XDG_CACHE_HOME` and verify `--allow-remote` executes
    // all 4 schemes (`npm:`, `jsr:`, `gh:`/`github:`, `https:`) in `vjs-lite` and `vjs-lite test`.
    let seed_script = project.path().join("apps/nested/sub/seed_all.ts");
    write(
        &project,
        "apps/nested/sub/seed_all.ts",
        r#"
import { npmRoot } from "my-npm";
import { npmSub } from "my-npm/sub";
import { jsrVal } from "my-jsr";
import { jsrSubVal } from "my-jsr/sub";
import { ghVal } from "my-gh";
import { githubVal } from "my-github";
import { httpsVal } from "my-https";
if (
    npmRoot !== "cli-npm-root" ||
    npmSub !== "cli-npm-sub" ||
    jsrVal !== "cli-jsr-root" ||
    jsrSubVal !== "cli-jsr-sub" ||
    ghVal !== "cli-gh-root" ||
    githubVal !== "cli-github-alt" ||
    httpsVal !== "cli-https-root"
) {
    throw new Error("seed mismatch");
}
"#,
    );
    let all_schemes_src = r#"
import { npmRoot } from "my-npm";
import { npmSub } from "my-npm/sub";
import { jsrVal } from "my-jsr";
import { jsrSubVal } from "my-jsr/sub";
import { ghVal } from "my-gh";
import { githubVal } from "my-github";
import { httpsVal } from "my-https";
console.log([npmRoot, npmSub, jsrVal, jsrSubVal, ghVal, githubVal, httpsVal].join("|"));
"#;
    let all_schemes_test_src = r#"
import test from "vael:test";
import assert from "vael:assert";
import { npmRoot } from "my-npm";
import { npmSub } from "my-npm/sub";
import { jsrVal } from "my-jsr";
import { jsrSubVal } from "my-jsr/sub";
import { ghVal } from "my-gh";
import { githubVal } from "my-github";
import { httpsVal } from "my-https";

test("ancestor vael.jsonc all four external schemes on lite", () => {
    assert.strictEquals(npmRoot, "cli-npm-root");
    assert.strictEquals(npmSub, "cli-npm-sub");
    assert.strictEquals(jsrVal, "cli-jsr-root");
    assert.strictEquals(jsrSubVal, "cli-jsr-sub");
    assert.strictEquals(ghVal, "cli-gh-root");
    assert.strictEquals(githubVal, "cli-github-alt");
    assert.strictEquals(httpsVal, "cli-https-root");
});
"#;
    write(&project, "apps/nested/sub/all_schemes.ts", all_schemes_src);
    write(
        &project,
        "apps/nested/sub/all_schemes.test.ts",
        all_schemes_test_src,
    );

    let discovered = discover_manifest(seed_script.parent().expect("nested"), None)
        .expect("discover")
        .expect("ancestor vael.jsonc");
    let resolver = vaeljs_lite_core::ImportResolver::from_manifest(&discovered);
    let replies_ref = Arc::clone(&remote_replies);
    let seed_opts = RuntimeOptions::new()
        .with_remote(RemoteOptions::new().with_cache_dir(&packages_cache))
        .with_npm_cache_dir(&packages_cache)
        .with_offline(true)
        .with_import_resolver(resolver);
    Runtime::with_options_and_transport(seed_opts, move |url, _, _, _| {
        let body = replies_ref
            .get(url)
            .cloned()
            .ok_or_else(|| format!("unexpected seed URL {url}"))?;
        Ok(TestRemoteResponse {
            status: 200,
            declared_length: Some(body.len() as u64),
            cache_control: Some("max-age=3600".to_owned()),
            body,
            ..TestRemoteResponse::default()
        })
    })
    .expect("seed runtime")
    .run_file(&seed_script)
    .expect("seed all schemes into XDG_CACHE_HOME");

    let cli_all_run = run_in_project(&[
        "--allow-remote",
        "--offline",
        "apps/nested/sub/all_schemes.ts",
    ]);
    assert_eq!(
        cli_all_run.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&cli_all_run)
    );
    assert_eq!(
        stdout(&cli_all_run),
        "cli-npm-root|cli-npm-sub|cli-jsr-root|cli-jsr-sub|cli-gh-root|cli-github-alt|cli-https-root\n"
    );

    let cli_all_test = run_in_project(&[
        "test",
        "--allow-remote",
        "--offline",
        "apps/nested/sub/all_schemes.test.ts",
    ]);
    assert_eq!(
        cli_all_test.status.code(),
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&cli_all_test),
        stderr(&cli_all_test)
    );
    assert!(
        stdout(&cli_all_test)
            .contains("ok 1 - ancestor vael.jsonc all four external schemes on lite"),
        "{}",
        stdout(&cli_all_test)
    );

    // 4. Direct URL entrypoints (`https:`, `gh:`, `jsr:`) from seeded cache, plus env var + override.
    for entry in [
        "https://cdn.example.test/mod.ts",
        "gh:acme/repo@v1.0.0/mod.ts",
        "jsr:@demo/pkg@^1.0.0",
    ] {
        let direct = run_in_project(&["--allow-remote", entry]);
        assert_eq!(
            direct.status.code(),
            Some(0),
            "{entry}: {}",
            stderr(&direct)
        );
    }

    let env_enabled = Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
        .current_dir(project.path())
        .env("VAELJS_ALLOW_REMOTE", "1")
        .env("XDG_CACHE_HOME", xdg_cache.path())
        .args(["--offline", "apps/nested/sub/all_schemes.ts"])
        .output()
        .expect("run with VAELJS_ALLOW_REMOTE");
    assert_eq!(
        env_enabled.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&env_enabled)
    );

    let env_overridden = Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
        .current_dir(project.path())
        .env("VAELJS_ALLOW_REMOTE", "1")
        .env("XDG_CACHE_HOME", xdg_cache.path())
        .args([
            "--no-allow-remote",
            "--offline",
            "apps/nested/sub/all_schemes.ts",
        ])
        .output()
        .expect("run with --no-allow-remote override");
    assert_eq!(env_overridden.status.code(), Some(1));
    assert!(
        stderr(&env_overridden).contains("remote module loading"),
        "{}",
        stderr(&env_overridden)
    );
}

#[cfg(feature = "npm")]
#[test]
fn no_config_skips_project_npmrc_and_default_rejects_project_npmrc_auth() {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use sha2::{Digest, Sha512};
    use std::collections::BTreeMap;
    use std::io::Write as _;
    use vaeljs_npm::{
        CasStore, DEFAULT_REGISTRY_ORIGIN, LockedPackage, Lockfile, NpmLimits, TarballIngestor,
    };

    fn tar_header(name: &str, size: usize) -> [u8; 512] {
        let mut h = [0u8; 512];
        let nb = name.as_bytes();
        h[..nb.len()].copy_from_slice(nb);
        h[100..108].copy_from_slice(b"0000644\0");
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[148..156].fill(b' ');
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        h
    }

    let mut tar_buf = Vec::new();
    for (name, body) in [
        (
            "package/package.json",
            r#"{"name":"cached-pkg","version":"1.0.0","type":"module","exports":"./index.mjs"}"#,
        ),
        ("package/index.mjs", "export const ok = 'from-cas';\n"),
    ] {
        let bytes = body.as_bytes();
        tar_buf.extend_from_slice(&tar_header(name, bytes.len()));
        tar_buf.extend_from_slice(bytes);
        let pad = (512 - (bytes.len() % 512)) % 512;
        tar_buf.resize(tar_buf.len() + pad, 0);
    }
    tar_buf.resize(tar_buf.len() + 1024, 0);
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_buf).expect("write tar");
    let tgz = gz.finish().expect("finish gzip");
    let sri = format!("sha512-{}", BASE64_STANDARD.encode(Sha512::digest(&tgz)));

    let xdg_cache = tempfile::tempdir().expect("xdg cache");
    let home = tempfile::tempdir().expect("home");
    let cas = CasStore::new(xdg_cache.path().join("vaeljs").join("packages"));
    let tree = TarballIngestor::new(cas, NpmLimits::default())
        .ingest(DEFAULT_REGISTRY_ORIGIN, &sri, &tgz)
        .expect("ingest")
        .tree_sha256;

    let project = tempfile::tempdir().expect("project");
    let mut lock = Lockfile::new();
    lock.specifiers.insert(
        "cached-pkg@^1.0.0".to_owned(),
        "cached-pkg@1.0.0".to_owned(),
    );
    lock.packages.insert(
        "cached-pkg@1.0.0".to_owned(),
        LockedPackage {
            version: "1.0.0".to_owned(),
            integrity: sri,
            tree_sha256: Some(tree),
            tarball_url: "https://registry.npmjs.org/cached-pkg/-/1.0.0.tgz".to_owned(),
            dependencies: BTreeMap::new(),
        },
    );
    lock.write_atomic(&project.path().join("vael.lock"))
        .expect("write lock");
    write(
        &project,
        ".npmrc",
        "registry=https://evil.test/\n//evil.test/:_authToken=${VAEL_SECRET_SENTINEL}\n",
    );
    write(
        &project,
        "main.mjs",
        "import { ok } from 'npm:cached-pkg@^1.0.0'; console.log(ok);\n",
    );

    let blocked = Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
        .current_dir(project.path())
        .env("HOME", home.path())
        .env("XDG_CACHE_HOME", xdg_cache.path())
        .env("VAEL_SECRET_SENTINEL", "top_secret_token")
        .args(["--offline", "--frozen-lockfile", "main.mjs"])
        .output()
        .expect("spawn vjs-lite");
    assert_eq!(
        blocked.status.code(),
        Some(1),
        "stderr: {}",
        stderr(&blocked)
    );
    assert!(
        stderr(&blocked).contains("project .npmrc"),
        "stderr: {}",
        stderr(&blocked)
    );
    assert!(!stderr(&blocked).contains("top_secret_token"));

    let skipped = Command::new(env!("CARGO_BIN_EXE_vjs-lite"))
        .current_dir(project.path())
        .env("HOME", home.path())
        .env("XDG_CACHE_HOME", xdg_cache.path())
        .env("VAEL_SECRET_SENTINEL", "top_secret_token")
        .args(["--no-config", "--offline", "--frozen-lockfile", "main.mjs"])
        .output()
        .expect("spawn vjs-lite --no-config");
    assert_eq!(
        skipped.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&skipped)
    );
    assert_eq!(stdout(&skipped), "from-cas\n");
}

#[cfg(not(feature = "remote"))]
#[test]
fn no_default_features_cli_rejects_remote_and_npm_flags() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "main.js", "console.log('hi');\n");
    let main_path = dir.path().join("main.js");

    for (flag, expected) in [
        (
            "--allow-remote",
            "requires `vaeljs-lite` built with the `remote` feature",
        ),
        (
            "--no-npm",
            "requires `vaeljs-lite` built with the `npm` feature",
        ),
        (
            "--frozen-lockfile",
            "requires `vaeljs-lite` built with the `npm` feature",
        ),
        (
            "--offline",
            "requires `vaeljs-lite` built with the `npm` feature",
        ),
    ] {
        let out = run(&[std::ffi::OsStr::new(flag), main_path.as_os_str()]);
        assert_eq!(out.status.code(), Some(2), "{flag}: {}", stderr(&out));
        assert!(stderr(&out).contains(expected), "{flag}: {}", stderr(&out));
    }

    let url_out = run(&[std::ffi::OsStr::new("https://example.test/mod.js")]);
    assert_eq!(url_out.status.code(), Some(2));
    assert!(
        stderr(&url_out).contains("requires `vaeljs-lite` built with the `remote` feature"),
        "{}",
        stderr(&url_out)
    );
}

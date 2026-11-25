//! End-to-end tests for the `neojs-lite` executable.

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
    Command::new(env!("CARGO_BIN_EXE_neojs-lite"))
        .args(args)
        .output()
        .expect("run neojs-lite")
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
fn exposes_the_lite_engine_and_the_shared_builtin() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(
        &dir,
        "main.js",
        r#"
console.log(Neo.engine);
console.log(Neo.getBuiltInModulesList().length);
console.log(Object.isFrozen(Neo.getBuiltInModulesList()));
console.log(typeof Neo.getBuiltInModule("neo:age"));
console.log(typeof Neo.getBuiltInModule("age"));
console.log(typeof Neo.getBuiltInModule("neo:chars"));
console.log(typeof Neo.getBuiltInModule("neo:console"));
console.log(typeof Neo.getBuiltInModule("neo:crypto"));
console.log(typeof Neo.getBuiltInModule("neo:fs"));
console.log(typeof Neo.getBuiltInModule("neo:os"));
console.log(typeof Neo.getBuiltInModule("neo:path"));
console.log(typeof Neo.getBuiltInModule("neo:slices"));
console.log(typeof Neo.getBuiltInModule("neo:ssh"));
console.log(typeof Neo.getBuiltInModule("neo:strings"));
console.log(typeof Neo.getBuiltInModule("neo:uuid"));
"#,
    );
    let file = dir.path().join("main.js");

    let output = run(&[file.as_os_str()]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "neo-lite\n14\ntrue\nundefined\nundefined\nobject\nobject\nobject\nobject\nobject\nobject\nobject\nobject\nobject\nobject\n"
    );
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
    assert!(error.starts_with("neojs-lite: cannot evaluate"), "{error}");
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
        stdout(&version).starts_with("neojs-lite "),
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
import test from "neo:test";
import assert from "neo:assert";
test("sync pass", () => assert.equals({ ok: true }, { ok: true }));
"#,
    );
    write(
        &dir,
        "b.test.js",
        r#"
import test from "neo:test";
import assert from "neo:assert";
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
import test from "neo:test";
globalThis.leaked = true;
test("fails", () => { throw new Error("boom"); });
"#,
    );
    write(
        &dir,
        "b-pass.test.js",
        r#"
import test from "neo:test";
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
        "neo.jsonc",
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
        "packages/a/neo.jsonc",
        r#"{
            "name": "@lite/a",
            "exports": "./mod.js",
            "imports": { "local-a": "./local.js" }
        }"#,
    );
    write(&dir, "packages/a/local.js", "export const tag = 'A';\n");
    write(
        &dir,
        "packages/b/neo.jsonc",
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
        "neo.jsonc",
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
    assert!(
        stderr(&out1).contains("remote module loading is not supported in neojs-lite"),
        "{}",
        stderr(&out1)
    );

    let file2 = dir.path().join("r2.js");
    write(&dir, "r2.js", "import 'jsr-lib';\n");
    let out2 = run(&[file2.as_os_str()]);
    assert_eq!(out2.status.code(), Some(1));
    assert!(
        stderr(&out2).contains("remote module loading is not supported in neojs-lite"),
        "{}",
        stderr(&out2)
    );
}

#[test]
fn config_and_no_config_flags_and_malformed_manifest_exit_codes() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir, "neo.jsonc", "{ malformed");
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

    // Auto discovery fails closed with exit code 2 on malformed neo.jsonc.
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
        "neo.jsonc",
        r#"{ "name": "@lite/ws-test", "workspaces": ["packages/*"] }"#,
    );
    write(
        &dir,
        "packages/math/neo.jsonc",
        r#"{ "name": "@lite/math", "exports": "./mod.js" }"#,
    );
    write(
        &dir,
        "packages/math/mod.js",
        "export const add = (a, b) => a + b;\n",
    );
    write(
        &dir,
        "packages/app/neo.jsonc",
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
import test from "neo:test";
import assert from "neo:assert";
import { add } from "@lite/math";
import { BASE } from "base";

test("workspace test in neojs-lite", () => {
    assert.strictEquals(add(BASE, 5), 15);
});
"#,
    );

    let pass = run(&[std::ffi::OsStr::new("test"), test_file.as_os_str()]);
    assert_eq!(pass.status.code(), Some(0), "stderr: {}", stderr(&pass));
    assert!(stdout(&pass).contains("ok 1 - workspace test in neojs-lite"));

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
import test from "neo:test";
import assert from "neo:assert";

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
        neojs_dts::render_bundle(neojs_dts::Profile::Lite)
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
            neojs_dts::render_bundle(neojs_dts::Profile::Lite)
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
            neojs_dts::render_bundle(neojs_dts::Profile::V8)
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
    for file in neojs_dts::files(neojs_dts::Profile::Lite) {
        let written = fs::read_to_string(lite_dir.join(file.path))
            .unwrap_or_else(|err| panic!("missing {}: {err}", file.path));
        assert_eq!(written, file.content, "mismatch for {}", file.path);
    }
    assert!(lite_dir.join("neo/ssh.d.ts").is_file());
    assert!(!lite_dir.join("neo/age.d.ts").exists());

    for flag in ["--help", "-h"] {
        let help = run(&[std::ffi::OsStr::new("types"), std::ffi::OsStr::new(flag)]);
        assert_eq!(help.status.code(), Some(0), "{flag}: {}", stderr(&help));
        assert_eq!(stderr(&help), "");
        let text = stdout(&help);
        assert!(text.contains("neojs-lite types"), "{text}");
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
        "import test from \"neo:test\";\nimport path from \"node:path\";\ntest(\"ok\", () => { path.join(\"a\", \"b\"); });\n",
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

//! Opt-in Node compatibility through the QuickJS runtime and its real loader.
//!
//! QuickJS has no host-asynchrony loop in this runtime, so these tests cover
//! only the synchronous surface. The same module source and policy core are used
//! by the V8 adapter; this file proves the QuickJS seam serves and executes it.

#![cfg(all(feature = "node-compat", unix))]

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use neojs_lite_core::ChildProcessPolicy;
use neojs_lite_core::NodeOptions;
use neojs_lite_core::Runtime;

fn require(path: &str) {
    assert!(Path::new(path).exists(), "test requires {path}");
}

fn policy(programs: &[&str]) -> NodeOptions {
    let mut policy = ChildProcessPolicy::new();
    for program in programs {
        policy = policy.allow_program(program).expect("allow program");
    }
    NodeOptions::new(policy)
}

fn write_module(dir: &tempfile::TempDir, name: &str, source: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, source).expect("write module");
    path
}

fn run(options: NodeOptions, source: &str) -> Result<(), neojs_lite_core::Error> {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", source);
    let mut runtime = Runtime::with_node_compat(options).expect("runtime");
    runtime.run_file(&file)
}

#[test]
fn the_default_runtime_rejects_every_node_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        "import { spawnSync } from \"node:child_process\";\n",
    );
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("node is disabled");
    let text = format!("{error}");
    assert!(text.contains("Node compatibility is disabled"), "{text}");
}

#[test]
fn a_bare_node_name_is_not_an_alias() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", "import \"child_process\";\n");
    let mut runtime = Runtime::with_node_compat(policy(&[])).expect("runtime");
    let error = runtime.run_file(&file).expect_err("bare name must fail");
    let text = format!("{error}");
    assert!(text.contains("node:` prefix"), "{text}");
    assert!(!text.contains("neo:child_process"), "{text}");
}

#[test]
fn node_compat_requires_the_node_prefix_for_overlapping_names() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", "import \"crypto\";\n");
    let mut runtime = Runtime::with_node_compat(policy(&[])).expect("runtime");
    let error = runtime.run_file(&file).expect_err("bare crypto must fail");
    let text = format!("{error}");
    assert!(text.contains("node:` prefix"), "{text}");
    assert!(!text.contains("neo:crypto"), "{text}");
}

#[test]
fn an_unimplemented_node_module_is_not_replaced_with_a_neo_module() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", "import \"node:fs\";\n");
    let mut runtime = Runtime::with_node_compat(policy(&[])).expect("runtime");
    let error = runtime
        .run_file(&file)
        .expect_err("node:fs is not implemented");
    let text = format!("{error}");
    assert!(text.contains("node:fs"), "{text}");
    assert!(text.contains("not implemented"), "{text}");
    assert!(!text.contains("neo:fs"), "{text}");
}

#[test]
fn node_modules_are_not_in_the_neo_registry() {
    run(
        policy(&[]),
        r#"
import * as childProcess from "node:child_process";
const list = Neo.getBuiltInModulesList();
if (list.includes("node:child_process")) throw new Error("node module leaked into Neo");
if (Neo.getBuiltInModule("node:child_process") !== undefined) {
    throw new Error("node module leaked into Neo.getBuiltInModule");
}
if (typeof childProcess.spawnSync !== "function") {
    throw new Error("spawnSync is missing");
}
"#,
    )
    .expect("registry stays separate");
}

#[test]
fn an_empty_policy_imports_the_module_but_denies_execution() {
    run(
        policy(&[]),
        r#"
import { execFileSync } from "node:child_process";
let denied = false;
try {
    execFileSync("/bin/echo", ["not-run"], { encoding: "utf8" });
} catch (error) {
    denied = error && error.code === "ERR_NODE_CHILD_PROCESS_DENIED";
}
if (!denied) throw new Error("the empty policy did not deny execution");
"#,
    )
    .expect("denied execution");
}

#[test]
fn spawn_sync_and_exec_file_sync_really_run_allowed_programs() {
    require("/bin/echo");
    require("/bin/cat");
    require("/bin/sh");
    run(
        policy(&["/bin/echo", "/bin/cat", "/bin/sh"]),
        r#"
import { execFileSync, spawnSync } from "node:child_process";

const echoed = execFileSync("/bin/echo", ["hello"], { encoding: "utf8" });
if (echoed !== "hello\n") throw new Error(`echo: ${JSON.stringify(echoed)}`);

const cat = spawnSync("/bin/cat", [], { encoding: "utf8", input: "abc" });
if (cat.status !== 0) throw new Error(`cat status: ${cat.status}`);
if (cat.stdout !== "abc") throw new Error(`cat stdout: ${JSON.stringify(cat.stdout)}`);
if (cat.output[1] !== "abc") throw new Error("cat output array is wrong");

const exit = spawnSync("/bin/sh", ["-c", "exit 7"], { encoding: "utf8" });
if (exit.status !== 7) throw new Error(`exit status: ${exit.status}`);
if (exit.error !== undefined) throw new Error(`nonzero exit has an error: ${exit.error.code}`);

let failed = null;
try {
    execFileSync("/bin/sh", ["-c", "printf out; printf err >&2; exit 3"], {
        encoding: "utf8",
    });
} catch (error) {
    failed = error;
}
if (!failed || failed.status !== 3) throw new Error(`exec failure status: ${failed && failed.status}`);
if (failed.stdout !== "out" || failed.stderr !== "err") {
    throw new Error("exec failure did not keep stdout and stderr");
}

const ambient = execFileSync("/bin/sh", ["-c", "printf %s \"${HOME-unset}\""], {
    encoding: "utf8",
});
if (ambient !== "unset") throw new Error(`ambient environment leaked: ${ambient}`);

const explicit = execFileSync("/bin/sh", ["-c", "printf %s \"$NEOJS_NODE\""], {
    encoding: "utf8",
    env: { NEOJS_NODE: "explicit" },
});
if (explicit !== "explicit") throw new Error(`explicit environment is wrong: ${explicit}`);

const literal = execFileSync("/bin/echo", ["a;b"], { encoding: "utf8" });
if (literal !== "a;b\n") throw new Error(`shell interpretation: ${JSON.stringify(literal)}`);

const undefinedArgs = execFileSync("/bin/echo", undefined, { encoding: "utf8" });
if (undefinedArgs !== "\n") throw new Error(`undefined args: ${String(undefinedArgs)}`);
const undefinedSpawn = spawnSync("/bin/echo", undefined, { encoding: "utf8" });
if (undefinedSpawn.status !== 0 || undefinedSpawn.stdout !== "\n") {
    throw new Error(`undefined spawn args: ${undefinedSpawn.status} ${String(undefinedSpawn.stdout)}`);
}
"#,
    )
    .expect("real child processes");
}

#[test]
fn timeout_and_max_buffer_are_reported_through_the_node_wrapper() {
    require("/bin/sh");
    require("/bin/sleep");
    run(
        policy(&["/bin/sh", "/bin/sleep"]),
        r#"
import { execFileSync, spawnSync } from "node:child_process";

const timed = spawnSync("/bin/sleep", ["5"], {
    encoding: "utf8",
    timeout: 50,
});
if (!timed.error || timed.error.code !== "ETIMEDOUT") {
    throw new Error(`timeout error: ${timed.error && timed.error.code}`);
}
if (timed.status !== null) throw new Error(`timeout status: ${timed.status}`);

let overflow = null;
try {
    execFileSync("/bin/sh", ["-c", "printf 0123456789"], {
        encoding: "utf8",
        maxBuffer: 4,
    });
} catch (error) {
    overflow = error;
}
if (!overflow || overflow.code !== "ERR_CHILD_PROCESS_STDIO_MAXBUFFER") {
    throw new Error(`maxBuffer error: ${overflow && overflow.code}`);
}

let signalled = null;
try {
    execFileSync("/bin/sh", ["-c", "kill -TERM $$"], { encoding: "utf8" });
} catch (error) {
    signalled = error;
}
if (!signalled || signalled.code !== "ERR_NODE_CHILD_PROCESS_EXIT") {
    throw new Error(`signal failure: ${signalled && signalled.code}`);
}
if (signalled.status !== null) throw new Error(`signal status: ${signalled.status}`);
"#,
    )
    .expect("bounded failures");
}

#[test]
fn an_inherited_pipe_cannot_extend_completion() {
    require("/bin/sh");
    require("/bin/sleep");
    let started = Instant::now();
    run(
        policy(&["/bin/sh", "/bin/sleep"]),
        r#"
import { execFileSync, spawnSync } from "node:child_process";
const result = spawnSync("/bin/sh", ["-c", "(/bin/sleep 5) & exit 0"], {
    encoding: "utf8",
});
if (result.status !== null) throw new Error(`status: ${result.status}`);
if (!result.error || result.error.code !== "ERR_NODE_CHILD_PROCESS_INCOMPLETE") {
    throw new Error(`incomplete error: ${result.error && result.error.code}`);
}

let execError = null;
try {
    execFileSync("/bin/sh", ["-c", "(/bin/sleep 5) & exit 0"], { encoding: "utf8" });
} catch (error) {
    execError = error;
}
if (!execError || execError.code !== "ERR_NODE_CHILD_PROCESS_INCOMPLETE") {
    throw new Error(`exec incomplete error: ${execError && execError.code}`);
}
"#,
    )
    .expect("bounded inherited-pipe completion");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "an inherited pipe extended completion for {:?}",
        started.elapsed()
    );
}

#[test]
fn the_native_module_has_no_bridge_and_huge_native_inputs_are_capped() {
    run(
        policy(&[]),
        r#"
if (globalThis.__neojsNodeChildProcess !== undefined) {
    throw new Error("a JavaScript bridge global is present");
}
if (globalThis.__neojsNodeRawRun !== undefined) {
    throw new Error("a raw host function global is present");
}
const childProcess = await import("node:child_process");
const again = await import("node:child_process");
if (childProcess !== again) {
    throw new Error("the native module namespace changed");
}

let denied = null;
try {
    childProcess.execFileSync("/bin/echo", ["x"], { encoding: "utf8" });
} catch (error) {
    denied = error && error.code;
}
if (denied !== "ERR_NODE_CHILD_PROCESS_DENIED") {
    throw new Error(`actual denied call: ${denied}`);
}

const oversized = [
    () => childProcess.execFileSync("x".repeat(8192), [], { encoding: "utf8" }),
    () => childProcess.execFileSync("/bin/echo", ["x".repeat(100000)], { encoding: "utf8" }),
    () => childProcess.execFileSync("/bin/echo", [], {
        encoding: "utf8",
        env: { NEOJS_OVERSIZED: "x".repeat(100000) },
    }),
    () => childProcess.execFileSync("/bin/echo", [], {
        encoding: "utf8",
        input: "x".repeat(8 * 1024 * 1024),
    }),
];
for (const call of oversized) {
    let code = null;
    try {
        call();
    } catch (error) {
        code = error && error.code;
    }
    if (code !== "ERR_NODE_REQUEST_TOO_LARGE") {
        throw new Error(`normalization cap: ${code}`);
    }
}
"#,
    )
    .expect("native module boundary");
}

#[test]
fn native_property_keys_are_bounded_after_enumeration() {
    run(
        policy(&[]),
        r#"
const childProcess = await import("node:child_process");
const hugeKey = "x".repeat(100000);

let code = null;
try {
    childProcess.execFileSync("/bin/echo", [], {
        encoding: "utf8",
        [hugeKey]: true,
    });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") {
    throw new Error(`huge option key: ${code}`);
}

code = null;
try {
    childProcess.execFileSync("/bin/echo", [], {
        encoding: "utf8",
        env: { [hugeKey]: "value" },
    });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") {
    throw new Error(`huge env key: ${code}`);
}

const tooMany = {};
for (let index = 0; index < 129; index += 1) {
    tooMany[`KEY_${index}`] = "value";
}
code = null;
try {
    childProcess.execFileSync("/bin/echo", [], { encoding: "utf8", env: tooMany });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") {
    throw new Error(`too many env keys: ${code}`);
}

code = null;
try {
    childProcess.execFileSync("/bin/echo", [], {
        encoding: "utf8",
        env: { NEOJS_EXPANSION: "é".repeat(40000) },
    });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") {
    throw new Error(`UTF-8 expansion: ${code}`);
}

let ownKeysCalls = 0;
const proxyOptions = new Proxy({}, {
    ownKeys() {
        ownKeysCalls += 1;
        return [];
    },
});
code = null;
try {
    childProcess.execFileSync("/bin/echo", [], proxyOptions);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") {
    throw new Error(`Proxy options: ${code}`);
}
if (ownKeysCalls !== 0) {
    throw new Error(`Proxy options invoked ownKeys ${ownKeysCalls} time(s)`);
}

const proxyEnv = new Proxy({}, {
    ownKeys() {
        ownKeysCalls += 1;
        return [];
    },
});
code = null;
try {
    childProcess.execFileSync("/bin/echo", [], { encoding: "utf8", env: proxyEnv });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") {
    throw new Error(`Proxy env: ${code}`);
}
if (ownKeysCalls !== 0) {
    throw new Error(`Proxy env invoked ownKeys ${ownKeysCalls} time(s)`);
}
"#,
    )
    .expect("native property-key bounds");
}

#[test]
fn the_native_module_identity_survives_import_and_allows_execution() {
    require("/bin/echo");
    run(
        policy(&["/bin/echo"]),
        r#"
const childProcess = await import("node:child_process");
const again = await import("node:child_process");
if (childProcess !== again) {
    throw new Error("the native module namespace changed during import");
}
if (childProcess.default.spawnSync !== childProcess.spawnSync) {
    throw new Error("default.spawnSync is not the named export");
}
if (childProcess.default.execFileSync !== childProcess.execFileSync) {
    throw new Error("default.execFileSync is not the named export");
}
const output = childProcess.execFileSync("/bin/echo", ["ok"], { encoding: "utf8" });
if (output !== "ok\n") throw new Error(`allowed output: ${String(output)}`);
"#,
    )
    .expect("native module identity and allowed call");
}

#[test]
fn hostile_serialization_hooks_do_not_run_or_allocate() {
    require("/bin/echo");
    run(
        policy(&["/bin/echo"]),
        r#"
Object.prototype.toJSON = () => ({ evil: "x".repeat(1000000) });
Array.prototype.toJSON = () => "evil";
const originalStringify = JSON.stringify;
JSON.stringify = () => "x".repeat(10000000);
try {
    const childProcess = await import("node:child_process");
    const output = childProcess.execFileSync("/bin/echo", ["ok"], { encoding: "utf8" });
    if (output !== "ok\n") throw new Error(`output: ${String(output)}`);
} finally {
    JSON.stringify = originalStringify;
    delete Object.prototype.toJSON;
    delete Array.prototype.toJSON;
}
"#,
    )
    .expect("safe serialization");
}

#[test]
fn option_getters_are_read_once() {
    require("/bin/sh");
    require("/bin/cat");
    require("/bin/pwd");
    run(
        policy(&["/bin/sh", "/bin/cat", "/bin/pwd"]),
        r#"
import { execFileSync } from "node:child_process";
const reads = { cwd: 0, env: 0, input: 0, timeout: 0, maxBuffer: 0 };
const options = {
    get encoding() { return "utf8"; },
    get cwd() {
        reads.cwd += 1;
        return reads.cwd === 1 ? "/tmp" : "/definitely-not-a-directory";
    },
    get env() {
        reads.env += 1;
        return reads.env === 1
            ? { NEOJS_GETTER: "yes" }
            : { NEOJS_GETTER: "x".repeat(1000000) };
    },
    get input() {
        reads.input += 1;
        return reads.input === 1 ? "ok" : "x".repeat(1000000);
    },
    get timeout() {
        reads.timeout += 1;
        return reads.timeout === 1 ? 1000 : 1000000000;
    },
    get maxBuffer() {
        reads.maxBuffer += 1;
        return reads.maxBuffer === 1 ? 1024 : 1;
    },
};
const output = execFileSync(
    "/bin/sh",
    ["-c", "/bin/pwd; printf %s \"$NEOJS_GETTER\"; /bin/cat"],
    options,
);
if (output !== "/tmp\nyesok") throw new Error(`output: ${String(output)}`);
for (const name of ["cwd", "env", "input", "timeout", "maxBuffer"]) {
    if (reads[name] !== 1) throw new Error(`${name} reads: ${reads[name]}`);
}
"#,
    )
    .expect("single option reads");
}

#[test]
fn proxy_argument_length_is_read_once() {
    require("/bin/echo");
    run(
        policy(&["/bin/echo"]),
        r#"
import { execFileSync } from "node:child_process";
let lengthReads = 0;
const target = new Array(300).fill("x");
const args = new Proxy(target, {
    get(target, key, receiver) {
        if (key === "length") {
            lengthReads += 1;
            return lengthReads === 1 ? 1 : 257;
        }
        return Reflect.get(target, key, receiver);
    },
});
const output = execFileSync("/bin/echo", args, { encoding: "utf8" });
if (output !== "x\n") throw new Error(`output: ${String(output)}`);
if (lengthReads !== 1) throw new Error(`length reads: ${lengthReads}`);
"#,
    )
    .expect("single argument-length read");
}

#[test]
fn numeric_options_reject_coercing_objects_and_replaced_integer_checks() {
    run(
        policy(&["/bin/echo"]),
        r#"
import { spawnSync } from "node:child_process";
Number.isInteger = () => true;
let coerced = 0;
let serialized = 0;
const options = {
    encoding: "utf8",
    timeout: {
        valueOf() { coerced += 1; return 1; },
        toJSON() { serialized += 1; return "x".repeat(1000000); },
    },
    maxBuffer: {
        valueOf() { coerced += 1; return 1024; },
        toJSON() { serialized += 1; return "x".repeat(1000000); },
    },
};
let code = null;
try {
    spawnSync("/bin/echo", ["x"], options);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_INVALID_ARG_VALUE") throw new Error(`code: ${code}`);
if (coerced !== 0) throw new Error(`coercion calls: ${coerced}`);
if (serialized !== 0) throw new Error(`serialization calls: ${serialized}`);
"#,
    )
    .expect("primitive numeric validation");
}

#[test]
fn unsupported_asynchronous_forms_throw_instead_of_faking_a_promise() {
    run(
        policy(&[]),
        r#"
import { spawn, spawnSync, exec, execFile, execSync, fork, ChildProcess } from "node:child_process";

for (const [name, call] of [
    ["spawn", () => spawn("/bin/echo")],
    ["exec", () => exec("/bin/echo")],
    ["execFile", () => execFile("/bin/echo")],
    ["execSync", () => execSync("/bin/echo")],
    ["fork", () => fork("/bin/echo")],
    ["ChildProcess", () => ChildProcess()],
]) {
    let threw = false;
    try {
        call();
    } catch (error) {
        threw = error && error.code === "ERR_NODE_UNSUPPORTED";
    }
    if (!threw) throw new Error(`${name} did not throw ERR_NODE_UNSUPPORTED`);
}

for (const [name, options] of [
    ["shell", { encoding: "utf8", shell: true }],
    ["stdio", { encoding: "utf8", stdio: "inherit" }],
]) {
    let code = null;
    try {
        spawnSync("/bin/echo", ["x"], options);
    } catch (error) {
        code = error && error.code;
    }
    if (code !== "ERR_NODE_UNSUPPORTED") {
        throw new Error(`${name} option was not rejected: ${code}`);
    }
}
"#,
    )
    .expect("unsupported surface is explicit");
}

#[test]
fn a_missing_encoding_is_rejected_explicitly() {
    run(
        policy(&[]),
        r#"
import { spawnSync } from "node:child_process";
let code = null;
try {
    spawnSync("/bin/echo", ["hello"]);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED_ENCODING") {
    throw new Error(`encoding error: ${code}`);
}
"#,
    )
    .expect("encoding opt-in");
}

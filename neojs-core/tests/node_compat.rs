//! Opt-in Node compatibility through the V8 runtime and its real loader.
//!
//! These tests exercise the public `Runtime` seam: the default runtime must
//! reject a `node:` import, an opted-in runtime must serve the module, and the
//! process policy must reach the shared synchronous runner.

#![cfg(all(feature = "node-compat", unix))]

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

use neojs_core::ChildProcessPolicy;
use neojs_core::NodeOptions;
use neojs_core::Runtime;

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

fn run(options: NodeOptions, source: &str) -> Result<(), neojs_core::Error> {
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
fn the_default_runtime_rejects_node_path() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", "import \"node:path\";\n");
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("node:path is disabled");
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
fn a_bare_path_name_is_not_an_alias() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", "import \"path\";\n");
    let mut runtime = Runtime::with_node_compat(policy(&[])).expect("runtime");
    let error = runtime.run_file(&file).expect_err("bare path must fail");
    let text = format!("{error}");
    assert!(text.contains("node:` prefix"), "{text}");
    assert!(!text.contains("neo:path"), "{text}");
}

#[test]
fn node_compat_requires_the_neo_prefix_for_overlapping_names() {
    run(
        policy(&[]),
        r#"
import { randomBytes } from "neo:crypto";
let bare = null;
try {
    await import("crypto");
} catch (error) {
    bare = error;
}
if (!bare) throw new Error("bare crypto resolved in a Node-compat runtime");
const message = String(bare);
if (!message.includes("node:` prefix")) throw new Error(message);
if (typeof randomBytes !== "function") throw new Error("neo:crypto is missing");
"#,
    )
    .expect("overlapping names require a prefix");
}

#[test]
fn a_default_runtime_requires_canonical_neo_specifiers_in_a_node_compat_build() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(
        &dir,
        "main.js",
        r#"
import { randomBytes } from "neo:crypto";
if (typeof randomBytes !== "function") throw new Error("neo:crypto is missing");
let bare = null;
try {
    await import("crypto");
} catch (error) {
    bare = error;
}
if (!bare) throw new Error("bare crypto resolved");
const message = String(bare);
if (!message.includes("canonical `neo:`")) throw new Error(message);
"#,
    );
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&file)
        .expect("a default runtime requires canonical Neo specifiers");
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
if (list.includes("node:path")) throw new Error("node:path leaked into Neo");
if (Neo.getBuiltInModule("node:child_process") !== undefined) {
    throw new Error("node module leaked into Neo.getBuiltInModule");
}
if (Neo.getBuiltInModule("node:path") !== undefined) {
    throw new Error("node:path leaked into Neo.getBuiltInModule");
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
fn the_native_module_is_not_global_and_caps_large_inputs() {
    run(
        policy(&[]),
        r#"
const first = await import("node:child_process");
const second = await import("node:child_process");
if (first !== second) throw new Error("the module identity changed");
if (first.spawnSync !== second.spawnSync) throw new Error("the export identity changed");
if (typeof first.default !== "object" || first.default.spawnSync !== first.spawnSync) {
    throw new Error("the default export is not the frozen api object");
}
if (globalThis.__neojsNodeChildProcess !== undefined) {
    throw new Error("a bridge global exists");
}

let code = null;
try {
    first.execFileSync("/bin/echo", [], {
        encoding: "utf8",
        input: "x".repeat(1048577),
    });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") throw new Error(`input cap: ${code}`);

code = null;
try {
    first.execFileSync("/bin/echo", ["x".repeat(65537)], { encoding: "utf8" });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") throw new Error(`argument cap: ${code}`);

code = null;
try {
    first.execFileSync("x".repeat(4097), [], { encoding: "utf8" });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") throw new Error(`file cap: ${code}`);

code = null;
try {
    first.execFileSync("/bin/echo", [], {
        encoding: "utf8",
        env: { NEOJS_EXPANSION: "x".repeat(65537) },
    });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") throw new Error(`environment value cap: ${code}`);

code = null;
try {
    first.execFileSync("/bin/echo", [], {
        encoding: "utf8",
        ["x".repeat(100000)]: true,
    });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") throw new Error(`option key cap: ${code}`);
"#,
    )
    .expect("native module bounds");
}

#[test]
fn proxy_options_and_environment_are_rejected_before_enumeration() {
    run(
        policy(&[]),
        r#"
import { spawnSync } from "node:child_process";
let ownKeysCalls = 0;
const options = new Proxy({}, {
    ownKeys() {
        ownKeysCalls += 1;
        return [];
    },
});
let code = null;
try {
    spawnSync("/bin/echo", [], options);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") throw new Error(`Proxy options: ${code}`);
if (ownKeysCalls !== 0) throw new Error(`Proxy options invoked ownKeys ${ownKeysCalls} time(s)`);

const environment = new Proxy({}, {
    ownKeys() {
        ownKeysCalls += 1;
        return [];
    },
});
code = null;
try {
    spawnSync("/bin/echo", [], { encoding: "utf8", env: environment });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") throw new Error(`Proxy env: ${code}`);
if (ownKeysCalls !== 0) throw new Error(`Proxy env invoked ownKeys ${ownKeysCalls} time(s)`);
"#,
    )
    .expect("Proxy rejection");
}

#[test]
fn static_and_dynamic_imports_share_the_native_module() {
    require("/bin/echo");
    run(
        policy(&["/bin/echo"]),
        r#"
import * as staticModule from "node:child_process";
const dynamicModule = await import("node:child_process");
if (staticModule !== dynamicModule) throw new Error("the namespace identity changed");
if (staticModule.spawnSync !== dynamicModule.spawnSync) {
    throw new Error("the spawnSync identity changed");
}
if (staticModule.default.spawnSync !== staticModule.spawnSync) {
    throw new Error("the default export shares no function identity");
}
if (!Object.isFrozen(staticModule.default)) throw new Error("the api object is not frozen");
const output = staticModule.execFileSync("/bin/echo", ["ok"], { encoding: "utf8" });
if (output !== "ok\n") throw new Error(`allowed output: ${String(output)}`);
"#,
    )
    .expect("native module identity");
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
    if (output !== "ok\n") throw new Error(`output: ${JSON.stringify(output)}`);
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
    ["ChildProcess", () => new ChildProcess()],
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

#[test]
fn hostile_prototype_setters_do_not_break_native_results_or_errors() {
    require("/bin/echo");
    require("/bin/sh");
    run(
        policy(&["/bin/echo", "/bin/sh"]),
        r#"
import { execFileSync, spawnSync } from "node:child_process";
const boom = new Error("prototype setter ran");
for (const name of ["pid", "stdout", "stderr", "status", "output", "signal", "error"]) {
    Object.defineProperty(Object.prototype, name, {
        configurable: true,
        set() { throw boom; },
    });
}
for (const name of ["code", "stderr"]) {
    Object.defineProperty(Error.prototype, name, {
        configurable: true,
        set() { throw boom; },
    });
}
for (const name of ["0", "1", "2"]) {
    Object.defineProperty(Array.prototype, name, {
        configurable: true,
        set() { throw boom; },
    });
}
try {
    const result = spawnSync("/bin/echo", ["ok"], { encoding: "utf8" });
    if (result.status !== 0 || result.stdout !== "ok\n" || result.output[1] !== "ok\n") {
        throw new Error("spawn result changed");
    }
    if (!Object.prototype.hasOwnProperty.call(result, "pid")) throw new Error("pid is not own");
    let failed = null;
    try {
        execFileSync("/bin/sh", ["-c", "exit 3"], { encoding: "utf8" });
    } catch (error) {
        failed = error;
    }
    if (!failed || failed.code !== "ERR_NODE_CHILD_PROCESS_EXIT" || failed.status !== 3) {
        throw new Error(`exec error: ${failed && failed.code}`);
    }
    if (!Object.prototype.hasOwnProperty.call(failed, "code")) throw new Error("code is not own");
} finally {
    for (const name of ["pid", "stdout", "stderr", "status", "output", "signal", "error"]) {
        delete Object.prototype[name];
    }
    for (const name of ["code", "stderr"]) delete Error.prototype[name];
    for (const name of ["0", "1", "2"]) delete Array.prototype[name];
}
"#,
    )
    .expect("hostile prototype setters");
}

#[test]
fn throwing_getters_propagate_unchanged_from_options_env_and_args() {
    require("/bin/echo");
    run(
        policy(&["/bin/echo"]),
        r#"
import { spawnSync } from "node:child_process";
const boom = new Error("getter boom");
const option = { encoding: "utf8" };
Object.defineProperty(option, "cwd", {
    enumerable: true,
    get() { throw boom; },
});
let caught = null;
try {
    spawnSync("/bin/echo", [], option);
} catch (error) {
    caught = error;
}
if (caught !== boom) throw new Error(`option getter: ${caught && caught.message}`);

const environment = {};
Object.defineProperty(environment, "KEY", {
    enumerable: true,
    get() { throw boom; },
});
caught = null;
try {
    spawnSync("/bin/echo", [], { encoding: "utf8", env: environment });
} catch (error) {
    caught = error;
}
if (caught !== boom) throw new Error(`env getter: ${caught && caught.message}`);

const args = [];
Object.defineProperty(args, "0", { get() { throw boom; } });
args.length = 1;
caught = null;
try {
    spawnSync("/bin/echo", args, { encoding: "utf8" });
} catch (error) {
    caught = error;
}
if (caught !== boom) throw new Error(`argument getter: ${caught && caught.message}`);
"#,
    )
    .expect("getter propagation");
}

#[test]
fn proxy_argument_length_exceptions_propagate_through_both_sync_exports() {
    require("/bin/echo");
    run(
        policy(&["/bin/echo"]),
        r#"
import { execFileSync, spawnSync } from "node:child_process";
const boom = new Error("proxy length boom");
const args = new Proxy([], {
    get(target, key, receiver) {
        if (key === "length") throw boom;
        return Reflect.get(target, key, receiver);
    },
});
for (const [name, call] of [
    ["spawnSync", () => spawnSync("/bin/echo", args, { encoding: "utf8" })],
    ["execFileSync", () => execFileSync("/bin/echo", args, { encoding: "utf8" })],
]) {
    let caught = null;
    try {
        call();
    } catch (error) {
        caught = error;
    }
    if (caught !== boom) throw new Error(`${name} replaced the length error`);
}
"#,
    )
    .expect("proxy length exception propagation");
}

#[test]
fn bounded_engine_rejects_non_round_trippable_and_empty_keys() {
    require("/usr/bin/env");
    run(
        policy(&["/usr/bin/env"]),
        r#"
import { execFileSync, spawnSync } from "node:child_process";

let code = null;
try {
    spawnSync("/bin/echo", [], {
        encoding: "utf8",
        env: { "\uD800": "bad", "\uFFFD": "good" },
    });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_INVALID_ARG_VALUE") throw new Error(`unpaired surrogate: ${code}`);

const good = execFileSync("/usr/bin/env", [], {
    encoding: "utf8",
    env: { "\uFFFD": "replacement" },
});
if (!good.includes("\uFFFD=replacement")) {
    throw new Error(`replacement key lookup: ${good}`);
}

code = null;
try {
    spawnSync("/bin/echo", [], { encoding: "utf8", env: { "": "empty" } });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_INVALID_ARG_VALUE") throw new Error(`empty env key: ${code}`);

code = null;
try {
    spawnSync("/bin/echo", [], { encoding: "utf8", "\uD800": true });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") throw new Error(`surrogate option key: ${code}`);
"#,
    )
    .expect("key round-trip rejection");
}

#[test]
fn bounded_engine_rejects_huge_ordinary_shapes_before_enumeration() {
    run(
        policy(&[]),
        r#"
import { spawnSync } from "node:child_process";

const options = { encoding: "utf8" };
for (let index = 0; index < 20000; index += 1) options[`KEY_${index}`] = 1;
const started = Date.now();
let code = null;
try {
    spawnSync("/bin/echo", [], options);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") throw new Error(`option dictionary: ${code}`);
if (Date.now() - started > 2000) throw new Error("option dictionary enumeration was not bounded");

const environment = {};
for (let index = 0; index < 20000; index += 1) environment[`KEY_${index}`] = "value";
const envStarted = Date.now();
code = null;
try {
    spawnSync("/bin/echo", [], { encoding: "utf8", env: environment });
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_REQUEST_TOO_LARGE") throw new Error(`environment dictionary: ${code}`);
if (Date.now() - envStarted > 2000) throw new Error("environment dictionary enumeration was not bounded");

const hidden = { encoding: "utf8" };
for (let index = 0; index < 100; index += 1) {
    Object.defineProperty(hidden, `hidden_${index}`, { value: 1, enumerable: false });
    hidden[Symbol(`symbol_${index}`)] = 1;
}
code = null;
try {
    spawnSync("/bin/echo", [], hidden);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") throw new Error(`hidden slots: ${code}`);

const seven = { encoding: "utf8", a: 1, b: 2, c: 3, d: 4, e: 5, f: 6 };
code = null;
try {
    spawnSync("/bin/echo", [], seven);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") throw new Error(`seven keys: ${code}`);
"#,
    )
    .expect("bounded ordinary shapes");
}

#[test]
fn bounded_engine_stops_before_scanning_large_element_capacity() {
    run(
        policy(&[]),
        r#"
import { spawnSync } from "node:child_process";
const elements = { encoding: "utf8" };
for (let index = 0; index < 20000; index += 1) elements[index] = 1;
const started = Date.now();
let code = null;
try {
    spawnSync("/bin/echo", [], elements);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_UNSUPPORTED") throw new Error(`large elements: ${code}`);
if (Date.now() - started > 2000) throw new Error("element capacity scan was not bounded");
"#,
    )
    .expect("large element capacity");
}

#[test]
fn bounded_engine_supports_fast_index_elements() {
    require("/usr/bin/env");
    run(
        policy(&["/usr/bin/env"]),
        r#"
import { execFileSync } from "node:child_process";
const output = execFileSync("/usr/bin/env", [], {
    encoding: "utf8",
    env: { "0": "zero", A: "a" },
});
if (!output.includes("0=zero") || !output.includes("A=a")) {
    throw new Error(`index element environment: ${output}`);
}
"#,
    )
    .expect("fast index elements");
}

#[test]
fn node_path_vectors_and_identity() {
    run(
        policy(&[]),
        r#"
import path, {
    posix,
    win32,
    join,
    normalize,
    resolve,
    relative,
    dirname,
    basename,
    extname,
    parse,
    format,
    isAbsolute,
    toNamespacedPath,
    sep,
    delimiter,
    matchesGlob,
    _makeLong,
} from "node:path";

function eq(actual, expected, label) {
    if (actual !== expected) {
        throw new Error(`${label}: ${JSON.stringify(actual)} !== ${JSON.stringify(expected)}`);
    }
}
function own(object, key, label) {
    if (!Object.prototype.hasOwnProperty.call(object, key)) {
        throw new Error(`${label} is not own`);
    }
}

eq(sep, "/", "sep");
eq(delimiter, ":", "delimiter");
eq(path.posix, path, "default is posix");
if (path.win32 === path) throw new Error("default is win32 on Unix");
eq(path.join, join, "default join identity");
eq(path.posix.join, posix.join, "posix join identity");
eq(path.win32.join, win32.join, "win32 join identity");
if (path.win32.join === path.join) throw new Error("win32 join aliased default");
eq(posix.posix, posix, "posix self");
eq(win32.win32, win32, "win32 self");
eq(posix.win32, win32, "posix.win32");
eq(win32.posix, posix, "win32.posix");
eq(_makeLong, toNamespacedPath, "_makeLong identity");
const dynamicPath = await import("node:path");
eq(dynamicPath.default, path, "dynamic default identity");
eq(dynamicPath.posix, posix, "dynamic posix identity");
eq(dynamicPath.win32, win32, "dynamic win32 identity");

eq(normalize("/foo//bar/../baz/"), "/foo/baz/", "normalize");
eq(join("a", "b", "..", "c"), "a/c", "join");
eq(dirname("/foo/bar/"), "/foo", "dirname");
eq(basename("/foo/bar.txt", ".txt"), "bar", "basename suffix");
eq(extname("/foo/bar.txt"), ".txt", "extname");
eq(isAbsolute("/foo"), true, "isAbsolute");
eq(resolve("/a", "b"), "/a/b", "resolve absolute");
eq(relative("/a/b", "/a/c"), "../c", "relative");
const parsed = parse("/foo/bar.txt");
eq(parsed.root, "/", "parse.root");
eq(parsed.dir, "/foo", "parse.dir");
eq(parsed.base, "bar.txt", "parse.base");
eq(parsed.ext, ".txt", "parse.ext");
eq(parsed.name, "bar", "parse.name");
eq(format({ dir: "/foo", name: "bar", ext: ".txt" }), "/foo/bar.txt", "format");
eq(toNamespacedPath("/foo"), "/foo", "posix toNamespacedPath");

eq(posix.normalize("//a//b/../c"), "/a/c", "posix normalize");
eq(win32.normalize("C:\\foo\\..\\bar"), "C:\\bar", "win32 normalize");
eq(win32.join("C:\\a", "b"), "C:\\a\\b", "win32 join");
eq(win32.join("\\\\server\\share", "a"), "\\\\server\\share\\a", "win32 UNC join");
eq(win32.dirname("C:\\foo\\bar"), "C:\\foo", "win32 dirname");
eq(win32.basename("C:\\foo\\bar.txt", ".txt"), "bar", "win32 basename");
eq(win32.extname("C:\\foo\\bar.txt"), ".txt", "win32 extname");
eq(win32.isAbsolute("C:\\foo"), true, "win32 isAbsolute");
eq(win32.resolve("C:\\a", "b"), "C:\\a\\b", "win32 resolve");
eq(win32.relative("C:\\a\\b", "C:\\a\\c"), "..\\c", "win32 relative");
eq(win32.relative("C:\\a\\b", "c:\\a\\c"), "..\\c", "win32 relative case");
eq(win32.toNamespacedPath("C:\\foo"), "\\\\?\\C:\\foo", "win32 namespaced");
const wparsed = win32.parse("C:\\foo\\bar.txt");
eq(wparsed.root, "C:\\", "wparse.root");
eq(wparsed.dir, "C:\\foo", "wparse.dir");
eq(wparsed.base, "bar.txt", "wparse.base");
eq(wparsed.ext, ".txt", "wparse.ext");
eq(wparsed.name, "bar", "wparse.name");
eq(win32.format({ root: "C:\\", name: "foo", ext: ".txt" }), "C:\\foo.txt", "win32 format");

let unsupported = null;
try {
    matchesGlob("a", "*");
} catch (error) {
    unsupported = error && error.code;
}
eq(unsupported, "ERR_NODE_UNSUPPORTED", "matchesGlob unsupported");

let typeError = null;
try {
    normalize(1);
} catch (error) {
    typeError = error && error.code;
}
eq(typeError, "ERR_INVALID_ARG_TYPE", "type error");
let formatError = null;
try {
    format({ dir: 1 });
} catch (error) {
    formatError = error && error.code;
}
eq(formatError, "ERR_INVALID_ARG_TYPE", "format type error");

let tooLarge = null;
try {
    normalize("a".repeat(1_048_577));
} catch (error) {
    tooLarge = error && error.code;
}
eq(tooLarge, "ERR_NODE_REQUEST_TOO_LARGE", "large input");

for (const key of [
    "resolve",
    "normalize",
    "isAbsolute",
    "join",
    "relative",
    "toNamespacedPath",
    "dirname",
    "basename",
    "extname",
    "format",
    "parse",
    "sep",
    "delimiter",
    "posix",
    "win32",
    "matchesGlob",
    "_makeLong",
]) {
    own(path, key, `default ${key}`);
}
"#,
    )
    .expect("node:path vectors");
}

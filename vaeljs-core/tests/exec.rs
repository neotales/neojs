//! V8 integration tests for `vael:exec`.
#![cfg(feature = "vael-exec")]

use std::fs;
use std::time::{Duration, Instant};

use vaeljs_core::Runtime;
use vaeljs_exec_core::{FileLock, LockMode, LockOptions};

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn module_contract_and_exports() {
    run_js(
        r#"
import {
    ChildProcess,
    Command,
    CommandError,
    FileLock,
    NotFoundOnPathError,
    Output,
    cmd,
    exec,
    execSync,
    lockFile,
    lockFileSync,
    run,
    runSync,
    spawn,
    which,
    whichAll,
    whichAllSync,
    whichSync,
} from "vael:exec";
import * as ns from "vael:exec";

const list = Vael.getBuiltInModulesList();
if (!list.includes("vael:exec")) throw new Error("vael:exec missing from list");
if (list.includes("exec")) throw new Error("bare exec in list");

const reg = Vael.getBuiltInModule("vael:exec");
if (!reg || !Object.isFrozen(reg)) throw new Error("registry missing or unfrozen");
if (Vael.getBuiltInModule("exec") !== undefined) throw new Error("bare name accepted");

const dyn = await import("vael:exec");
if (dyn === reg) throw new Error("namespace must be distinct from registry");
if (Object.prototype.hasOwnProperty.call(ns, "default")) {
    throw new Error("vael:exec must not have a default export");
}

const expected = "ChildProcess,Command,CommandError,FileLock,NotFoundOnPathError,Output,cmd,exec,execSync,lockFile,lockFileSync,run,runSync,spawn,which,whichAll,whichAllSync,whichSync";
const keys = Object.keys(ns).sort().join(",");
if (keys !== expected) {
    throw new Error(`unexpected exports: ${keys}`);
}
if (Object.keys(reg).sort().join(",") !== expected) {
    throw new Error("registry keys drifted from namespace keys");
}
for (const k of expected.split(",")) {
    if (dyn[k] !== ns[k] || reg[k] !== ns[k]) {
        throw new Error(`identity mismatch for ${k}`);
    }
}
if (import.meta.resolve("vael:exec") !== "vael:exec") throw new Error("resolve failed");
if (import.meta.resolve("VAEL:exec") !== "vael:exec") throw new Error("case-fold failed");
"#,
    );
}

#[test]
fn command_builder_validation_and_prototype_safety() {
    run_js(
        r#"
import { Command, CommandError, cmd } from "vael:exec";

// Fluent methods return `this` and `toArgs()` returns `[program, ...args]`
const c = cmd("echo", ["-n"]);
if (c.arg("hello") !== c) throw new Error("arg must return this");
if (c.args(["world", "!"]) !== c) throw new Error("args must return this");
if (c.cwd("/tmp") !== c) throw new Error("cwd must return this");
if (c.env("A", "1") !== c) throw new Error("env must return this");
if (c.envs({ B: "2" }) !== c) throw new Error("envs must return this");
if (c.envRemove("A") !== c) throw new Error("envRemove must return this");
if (c.envClear() !== c) throw new Error("envClear must return this");
if (c.stdin("null") !== c) throw new Error("stdin must return this");
if (c.stdout("piped") !== c) throw new Error("stdout must return this");
if (c.stderr("piped") !== c) throw new Error("stderr must return this");
if (c.timeout(5000) !== c) throw new Error("timeout must return this");
if (c.maxOutputBytes(1024) !== c) throw new Error("maxOutputBytes must return this");
if (c.killSignal("SIGTERM") !== c) throw new Error("killSignal must return this");

if (JSON.stringify(c.toArgs()) !== JSON.stringify(["echo", "-n", "hello", "world", "!"])) {
    throw new Error(`unexpected toArgs: ${JSON.stringify(c.toArgs())}`);
}

// Argv constructor form
const c2 = new Command(["printf", "%s", "ok"], { cwd: "/tmp" });
if (JSON.stringify(c2.toArgs()) !== JSON.stringify(["printf", "%s", "ok"])) {
    throw new Error("argv form mismatch");
}

// Program + options constructor form
const c3 = new Command("echo", { args: ["hi"] });
if (JSON.stringify(c3.toArgs()) !== JSON.stringify(["echo", "hi"])) {
    throw new Error("program + options form mismatch");
}

// Reject non-string / empty program / NUL bytes
for (const bad of [123, null, undefined, {}]) {
    let threw = false;
    try { new Command(bad); } catch (e) { threw = e instanceof TypeError; }
    if (!threw) throw new Error(`expected TypeError for ${String(bad)}`);
}
for (const bad of ["", [], ["echo", "bad\0arg"], "bad\0prog"]) {
    let err = null;
    try { new Command(bad); } catch (e) { err = e; }
    if (!(err instanceof CommandError) || err.code !== "VAEL_EXEC_INVALID_ARGUMENT") {
        throw new Error(`expected VAEL_EXEC_INVALID_ARGUMENT for ${JSON.stringify(bad)}, got ${err}`);
    }
}

// Proxy and Object.prototype pollution safety
let trapRan = false;
const proxyOpts = new Proxy({ cwd: "/tmp" }, {
    get() { trapRan = true; return "/tmp"; },
    getOwnPropertyDescriptor() { trapRan = true; return undefined; },
});
let proxyThrew = false;
try { new Command("echo", proxyOpts); } catch (e) { proxyThrew = e instanceof TypeError; }
if (!proxyThrew || trapRan) throw new Error("Proxy options must throw TypeError without traps");

Object.prototype.cwd = "/should-be-ignored";
try {
    const clean = new Command("echo", { args: ["ok"] });
    if (clean.toArgs().length !== 2) throw new Error("polluted Object.prototype broke Command");
} finally {
    delete Object.prototype.cwd;
}
"#,
    );
}

#[cfg(unix)]
#[test]
fn exec_run_output_helpers_argv_immunity_and_not_found() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let cwd_json = serde_json::to_string(tmp.path().to_str().unwrap()).unwrap();

    run_js(&format!(
        r#"
import {{ CommandError, NotFoundOnPathError, cmd, exec, execSync, run, runSync }} from "vael:exec";

// 1. Argv-only shell injection immunity
const injected = execSync("/bin/echo", ["; echo INJECTED", "$(whoami)", "| cat"]);
if (!injected.success || injected.code !== 0) throw new Error("echo failed");
if (injected.text() !== "; echo INJECTED $(whoami) | cat\n") {{
    throw new Error(`shell injection check failed: ${{JSON.stringify(injected.text())}}`);
}}
if (JSON.stringify(injected.lines()) !== JSON.stringify(["; echo INJECTED $(whoami) | cat"])) {{
    throw new Error("lines mismatch");
}}

// 2. Output text/lines/json/errorText/errorLines/errorJson/validate
const outJson = await exec("/bin/sh", [
    "-c",
    'printf \'{{"ok":true,"n":7}}\'; printf \'{{"warn":1}}\\n\' >&2',
]);
if (outJson.json().n !== 7 || outJson.errorJson().warn !== 1) {{
    throw new Error("json/errorJson mismatch");
}}
if (outJson.validate() !== outJson) throw new Error("validate must return this");

let stderrErr = null;
try {{ outJson.validate(undefined, true); }} catch (e) {{ stderrErr = e; }}
if (!(stderrErr instanceof CommandError) || stderrErr.code !== "VAEL_EXEC_STDERR") {{
    throw new Error("failOnStderr must throw CommandError");
}}

let predErr = null;
try {{ outJson.validate((o) => o.json().n === 999); }} catch (e) {{ predErr = e; }}
if (!(predErr instanceof CommandError)) throw new Error("predicate false must throw CommandError");

// 3. Non-zero exit code: exec/run return Output/status with success=false, Command.text() throws CommandError
const failOut = await exec("/bin/sh", ["-c", "echo err-line >&2; exit 23"]);
if (failOut.code !== 23 || failOut.success !== false || failOut.errorLines()[0] !== "err-line") {{
    throw new Error("non-zero Output mismatch");
}}
const failStatus = runSync("/bin/sh", ["-c", "exit 17"], {{ stdout: "null", stderr: "null" }});
if (failStatus.code !== 17 || failStatus.success !== false) {{
    throw new Error("non-zero status mismatch");
}}
const asyncStatus = await run("/bin/sh", ["-c", "exit 0"], {{ stdout: "null", stderr: "null" }});
if (asyncStatus.code !== 0 || asyncStatus.success !== true) {{
    throw new Error("async run status mismatch");
}}

let cmdTextErr = null;
try {{
    await cmd("/bin/sh", ["-c", "exit 9"]).text();
}} catch (e) {{
    cmdTextErr = e;
}}
if (!(cmdTextErr instanceof CommandError) || cmdTextErr.exitCode !== 9) {{
    throw new Error(`Command.text() must throw CommandError with exitCode 9, got ${{cmdTextErr}}`);
}}

// 4. Stdin string & RAB Uint8Array + cwd + envClear + env
const rab = new ArrayBuffer(4, {{ maxByteLength: 16 }});
const view = new Uint8Array(rab, 1, 2);
view.set([0x68, 0x69]);
rab.resize(8);
const catOut = await cmd("cat").stdin(view).output();
if (catOut.text() !== "hi") throw new Error(`RAB stdin mismatch: ${{catOut.text()}}`);

const envOut = await cmd("/bin/sh", ["-c", "pwd; printf '%s|%s' \"$VAEL_KEEP\" \"$VAEL_CLEARED\""])
    .cwd({cwd_json})
    .env("VAEL_CLEARED", "should_be_cleared")
    .envClear()
    .env("VAEL_KEEP", "val123")
    .env("VAEL_REMOVED", "bye")
    .envRemove("VAEL_REMOVED")
    .text();
const lines = envOut.split("\n");
if (lines[1] !== "val123|") throw new Error(`envClear/env failed: ${{JSON.stringify(envOut)}}`);

// 5. Missing executable -> NotFoundOnPathError
let notFoundErr = null;
try {{
    await exec("definitely_missing_vael_exec_binary_987654");
}} catch (e) {{
    notFoundErr = e;
}}
if (!(notFoundErr instanceof NotFoundOnPathError) || !(notFoundErr instanceof Error)) {{
    throw new Error("expected NotFoundOnPathError");
}}
if (notFoundErr.code !== "VAEL_EXEC_NOT_FOUND" || notFoundErr.fileName !== "definitely_missing_vael_exec_binary_987654") {{
    throw new Error(`NotFoundOnPathError fields mismatch: ${{notFoundErr.code}} ${{notFoundErr.fileName}}`);
}}

// 6. Timeout & maxOutputBytes
const toOut = await cmd("/bin/sh", ["-c", "sleep 5"]).timeout(60).output();
if (!toOut.timedOut || toOut.success) throw new Error("timeout flag not set");

let bigErr = null;
try {{
    await cmd("/bin/sh", ["-c", "head -c 2048 /dev/zero"]).maxOutputBytes(512).output();
}} catch (e) {{
    bigErr = e;
}}
if (!(bigErr instanceof CommandError) || bigErr.code !== "VAEL_EXEC_OUTPUT_TOO_LARGE") {{
    throw new Error(`expected VAEL_EXEC_OUTPUT_TOO_LARGE, got ${{bigErr && bigErr.code}}`);
}}

// 7. Async exec does not block V8 event loop (fast command finishes before slow sleep)
const order = [];
const slowP = exec("/bin/sh", ["-c", "sleep 0.2"]).then((r) => {{ order.push("slow"); return r; }});
const fastP = exec("/bin/echo", ["fast"]).then((r) => {{ order.push("fast"); return r; }});
const [slowR, fastR] = await Promise.all([slowP, fastP]);
if (!slowR.success || !fastR.success) throw new Error("concurrent exec failed");
if (order.join(",") !== "fast,slow") {{
    throw new Error(`async exec blocked event loop: ${{order.join(",")}}`);
}}
"#,
    ));
}

#[cfg(unix)]
#[test]
fn spawn_streaming_child_kill_wait_output_and_async_dispose() {
    run_js(
        r#"
import { spawn } from "vael:exec";

// 1. Streaming writeStdin / readStdout / readStderr / wait
const child = spawn("/bin/sh", ["-c", "cat; printf 'err-msg\\n' >&2"]);
if (typeof child.pid !== "number" || child.pid <= 0) throw new Error("invalid pid");

await child.writeStdin("line-one\n");
await child.writeStdin(new Uint8Array([108, 105, 110, 101, 45, 50, 10])); // "line-2\n"
child.closeStdin();

let outBytes = [];
while (true) {
    const chunk = await child.readStdout(4);
    if (chunk.byteLength === 0) break;
    for (const b of chunk) outBytes.push(b);
}
const outStr = String.fromCharCode(...outBytes);
if (outStr !== "line-one\nline-2\n") {
    throw new Error(`streamed stdout mismatch: ${JSON.stringify(outStr)}`);
}

const errChunk = await child.readStderr(64);
const errStr = String.fromCharCode(...errChunk);
if (errStr !== "err-msg\n") throw new Error(`streamed stderr mismatch: ${JSON.stringify(errStr)}`);

const status = await child.wait();
if (!status.success || status.code !== 0) throw new Error("child wait failed");

// 2. child.output() collects remaining output
const child2 = spawn("/bin/sh", ["-c", "printf 'hello-out'; printf 'hello-err' >&2"]);
const out2 = await child2.output();
if (out2.text() !== "hello-out" || out2.errorText() !== "hello-err" || !out2.success) {
    throw new Error("child.output() mismatch");
}

// 3. kill() terminates sleeping child promptly
const sleeper = spawn("/bin/sh", ["-c", "sleep 10"]);
sleeper.kill("SIGTERM");
const killed = await sleeper.wait();
if (killed.success || killed.signal !== "SIGTERM") {
    throw new Error(`kill mismatch: ${JSON.stringify(killed)}`);
}

// 4. [Symbol.asyncDispose]() kills & reaps cleanly
if (typeof Symbol.asyncDispose !== "symbol") {
    throw new Error("Symbol.asyncDispose not supported");
}
const start = Date.now();
{
    await using disposable = spawn("/bin/sh", ["-c", "sleep 10"]);
    if (disposable.pid <= 0) throw new Error("bad disposable pid");
}
if (Date.now() - start > 2000) {
    throw new Error("asyncDispose took too long to kill & reap child");
}
"#,
    );
}

#[cfg(unix)]
#[test]
fn dropping_runtime_kills_and_reaps_undisposed_child() {
    let dir = tempfile::tempdir().expect("temp dir");
    let pid_file = dir.path().join("child.pid");
    let pid_json = serde_json::to_string(pid_file.to_str().unwrap()).unwrap();

    let start = Instant::now();
    {
        let main = dir.path().join("main.js");
        fs::write(
            &main,
            format!(
                r#"
import {{ writeTextFile }} from "vael:fs";
import {{ spawn }} from "vael:exec";

const child = spawn("/bin/sh", ["-c", "sleep 30"]);
writeTextFile({pid_json}, String(child.pid));
"#
            ),
        )
        .unwrap();
        let mut runtime = Runtime::new().expect("runtime");
        runtime.run_file(&main).expect("run");
        // Drop runtime without disposing `child`.
    }
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "runtime drop should kill & reap immediately"
    );
    let pid_str = fs::read_to_string(&pid_file).expect("pid file");
    let pid: u32 = pid_str.trim().parse().expect("numeric pid");
    // Give OS a tiny moment if needed, then verify process is gone (kill -0 fails).
    let status = std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("kill -0");
    assert!(
        !status.success(),
        "undisposed child process {pid} survived runtime drop"
    );
}

#[cfg(unix)]
#[test]
fn which_and_which_all_integration() {
    use std::os::unix::fs::PermissionsExt as _;

    let tmp = tempfile::tempdir().expect("temp dir");
    let bin1 = tmp.path().join("bin1");
    let bin2 = tmp.path().join("bin2");
    fs::create_dir_all(&bin1).unwrap();
    fs::create_dir_all(&bin2).unwrap();

    let non_exec = bin1.join("vael_test_tool");
    fs::write(&non_exec, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&non_exec, fs::Permissions::from_mode(0o644)).unwrap();

    let exec2 = bin2.join("vael_test_tool");
    fs::write(&exec2, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&exec2, fs::Permissions::from_mode(0o755)).unwrap();

    let prepend = format!("{}:{}", bin1.display(), bin2.display());
    let prepend_json = serde_json::to_string(&prepend).unwrap();
    let exec2_json = serde_json::to_string(exec2.to_str().unwrap()).unwrap();

    run_js(&format!(
        r#"
import {{ CommandError, which, whichAll, whichAllSync, whichSync }} from "vael:exec";

if (whichSync("vael_test_tool", {prepend_json}) !== {exec2_json}) {{
    throw new Error("whichSync did not skip non-executable and find bin2");
}}
if ((await which("vael_test_tool", {{ prependPath: {prepend_json} }})) !== {exec2_json}) {{
    throw new Error("async which with options object failed");
}}
const allSync = whichAllSync("vael_test_tool", {prepend_json});
const allAsync = await whichAll("vael_test_tool", {prepend_json});
if (JSON.stringify(allSync) !== JSON.stringify([{exec2_json}]) || JSON.stringify(allAsync) !== JSON.stringify([{exec2_json}])) {{
    throw new Error("whichAll mismatch");
}}
if (whichSync("nonexistent_tool_vael_xyz_99999") !== null) {{
    throw new Error("missing binary should return null");
}}
if ((await whichAll("nonexistent_tool_vael_xyz_99999")).length !== 0) {{
    throw new Error("missing binary whichAll should return []");
}}

let emptyErr = null;
try {{ whichSync(""); }} catch (e) {{ emptyErr = e; }}
if (!(emptyErr instanceof CommandError) || emptyErr.code !== "VAEL_EXEC_INVALID_ARGUMENT") {{
    throw new Error("empty which name should throw VAEL_EXEC_INVALID_ARGUMENT");
}}
"#,
    ));
}

#[test]
fn file_lock_sync_async_dispose_and_runtime_drop_release() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let lock_path = tmp.path().join("app.lock");
    let lock_json = serde_json::to_string(lock_path.to_str().unwrap()).unwrap();

    #[cfg(unix)]
    let symlink_path = {
        let target = tmp.path().join("real.lock");
        fs::write(&target, b"").unwrap();
        let sym = tmp.path().join("sym.lock");
        std::os::unix::fs::symlink(&target, &sym).unwrap();
        serde_json::to_string(sym.to_str().unwrap()).unwrap()
    };
    #[cfg(not(unix))]
    let symlink_path = "null";

    run_js(&format!(
        r#"
import {{ CommandError, FileLock, lockFile, lockFileSync }} from "vael:exec";

// 1. Exclusive lock blocks second exclusive nonBlocking & timeout
const l1 = lockFileSync({lock_json});
if (!(l1 instanceof FileLock) || l1.path !== {lock_json} || l1.mode !== "exclusive" || !l1.locked) {{
    throw new Error("l1 properties mismatch");
}}

let nbErr = null;
try {{
    lockFileSync({lock_json}, {{ nonBlocking: true }});
}} catch (e) {{
    nbErr = e;
}}
if (!(nbErr instanceof CommandError) || nbErr.code !== "VAEL_EXEC_LOCK_WOULD_BLOCK") {{
    throw new Error(`expected VAEL_EXEC_LOCK_WOULD_BLOCK, got ${{nbErr && nbErr.code}}`);
}}

let toErr = null;
try {{
    await lockFile({lock_json}, {{ timeoutMs: 30 }});
}} catch (e) {{
    toErr = e;
}}
if (!(toErr instanceof CommandError) || toErr.code !== "VAEL_EXEC_LOCK_TIMEOUT") {{
    throw new Error(`expected VAEL_EXEC_LOCK_TIMEOUT, got ${{toErr && toErr.code}}`);
}}

l1.unlock();
if (l1.locked !== false) throw new Error("locked must be false after unlock");
l1.unlock(); // idempotent

// 2. Shared locks allow concurrent shared, block exclusive
const s1 = await lockFile({lock_json}, {{ mode: "shared", nonBlocking: true }});
const s2 = lockFileSync({lock_json}, {{ mode: "shared", nonBlocking: true }});
if (s1.mode !== "shared" || s2.mode !== "shared") throw new Error("shared mode mismatch");

let exBlocked = null;
try {{
    lockFileSync({lock_json}, {{ mode: "exclusive", nonBlocking: true }});
}} catch (e) {{
    exBlocked = e;
}}
if (!(exBlocked instanceof CommandError) || exBlocked.code !== "VAEL_EXEC_LOCK_WOULD_BLOCK") {{
    throw new Error("exclusive should block while shared held");
}}

// 3. [Symbol.dispose] and [Symbol.asyncDispose]
if (typeof Symbol.dispose === "symbol") {{
    s1[Symbol.dispose]();
    if (s1.locked) throw new Error("Symbol.dispose did not unlock");
}} else {{
    s1.unlock();
}}
if (typeof Symbol.asyncDispose === "symbol") {{
    await s2[Symbol.asyncDispose]();
    if (s2.locked) throw new Error("Symbol.asyncDispose did not unlock");
}} else {{
    s2.unlock();
}}

// Now exclusive succeeds again
const l2 = await lockFile({lock_json}, {{ nonBlocking: true }});
l2.unlock();

// 4. Symlink lock rejected on Unix
if ({symlink_path} !== null) {{
    let symErr = null;
    try {{ lockFileSync({symlink_path}); }} catch (e) {{ symErr = e; }}
    if (!(symErr instanceof CommandError) || symErr.code !== "VAEL_EXEC_INVALID_ARGUMENT") {{
        throw new Error("symlink lock must fail with VAEL_EXEC_INVALID_ARGUMENT");
    }}
}}
"#,
    ));

    // 5. Dropping Runtime releases any undisposed FileLock
    {
        let main = tmp.path().join("leak_lock.js");
        fs::write(
            &main,
            format!(
                r#"
import {{ lockFileSync }} from "vael:exec";
const held = lockFileSync({lock_json});
if (!held.locked) throw new Error("expected locked");
// Intentionally not unlocked before runtime drop!
"#
            ),
        )
        .unwrap();
        let mut runtime = Runtime::new().expect("runtime");
        runtime.run_file(&main).expect("run");
    }

    // Lock must now be acquirable by Rust immediately
    let reacquired = FileLock::acquire(
        &lock_path,
        &LockOptions::new(LockMode::Exclusive).non_blocking(true),
    )
    .expect("runtime drop should release OS advisory lock");
    reacquired.unlock().expect("unlock");
}

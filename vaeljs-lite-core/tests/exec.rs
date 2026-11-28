//! Integration tests for the `vael:exec` built-in through the QuickJS [`Runtime`].

use vaeljs_lite_core::Runtime;

fn run_js(source: &str) {
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_source("exec-test.js", source)
        .expect("exec test module");
}

#[cfg(not(feature = "vael-exec"))]
#[test]
fn feature_off_rejects_vael_exec_import_and_registry() {
    run_js(
        r#"
const list = Vael.getBuiltInModulesList();
if (list.includes("vael:exec") || list.includes("exec")) {
    throw new Error(`vael:exec leaked into module list: ${JSON.stringify(list)}`);
}
if (Vael.getBuiltInModule("vael:exec") !== undefined) {
    throw new Error("Vael.getBuiltInModule('vael:exec') must be undefined when disabled");
}
let imported = false;
try {
    await import("vael:exec");
    imported = true;
} catch {
    // Expected when vael-exec is disabled.
}
if (imported) {
    throw new Error("import('vael:exec') succeeded while vael-exec feature was disabled");
}
"#,
    );
}

#[cfg(feature = "vael-exec")]
mod enabled {
    use std::fs::{self, File};

    use tempfile::tempdir;
    use vaeljs_exec_core::{ExecErrorCode, FileLock, LockOptions};
    use vaeljs_lite_core::Runtime;

    use super::run_js;

    fn js_str(value: &str) -> String {
        serde_json::to_string(value).expect("json string")
    }

    #[test]
    fn exports_identity_and_primordial_isolation() {
        run_js(
            r#"
const ns = await import("vael:exec");
const reg = Vael.getBuiltInModule("vael:exec");
const expected = [
    "Command",
    "CommandError",
    "FileLock",
    "NotFoundOnPathError",
    "Output",
    "PathFinder",
    "addLogger",
    "clearLoggers",
    "cmd",
    "execSync",
    "lockFileSync",
    "pathFinder",
    "runSync",
    "whichAllSync",
    "whichSync",
];
if (JSON.stringify(Object.keys(ns).sort()) !== JSON.stringify(expected)) {
    throw new Error(`unexpected exports: ${JSON.stringify(Object.keys(ns).sort())}`);
}
if ("default" in ns) {
    throw new Error("vael:exec must not have a default export");
}
if (!Object.isFrozen(reg)) {
    throw new Error("registry object must be frozen");
}
for (const name of expected) {
    if (ns[name] !== reg[name]) {
        throw new Error(`vael:exec.${name} namespace/registry identity mismatch`);
    }
}

// Mutate primordials after import and verify classes and functions still behave safely.
globalThis.Error = function FakeError() { return {}; };
globalThis.JSON = { parse: () => ({ hijacked: true }), stringify: () => "hijacked" };
Array.prototype.push = function () { throw new Error("Array.prototype.push hijacked"); };
Object.defineProperty(Object.prototype, "code", {
    set() { throw new Error("Object.prototype.code setter invoked"); },
    configurable: true,
});

const cmdErr = new ns.CommandError("VAEL_EXEC_IO", "failed", { exitCode: 2, fileName: "tool" });
if (!(cmdErr instanceof ns.CommandError) || cmdErr.name !== "CommandError" || cmdErr.code !== "VAEL_EXEC_IO" || cmdErr.exitCode !== 2 || cmdErr.fileName !== "tool") {
    throw new Error("CommandError construction failed under mutated primordials");
}
const nfErr = new ns.NotFoundOnPathError("missing-bin");
if (!(nfErr instanceof ns.NotFoundOnPathError) || nfErr.name !== "NotFoundOnPathError" || nfErr.code !== "VAEL_EXEC_NOT_FOUND" || nfErr.fileName !== "missing-bin") {
    throw new Error("NotFoundOnPathError construction failed under mutated primordials");
}
const c = ns.cmd("echo", ["hello"]).arg("world");
if (JSON.stringify(c.toArgs()) !== JSON.stringify(["echo", "hello", "world"])) {
    throw new Error("Command builder failed under mutated primordials");
}
delete Object.prototype.code;
"#,
        );
    }

    #[test]
    fn argv_only_validation_and_shell_metacharacters() {
        run_js(
            r#"
import { Command, CommandError, NotFoundOnPathError, cmd, execSync, runSync } from "vael:exec";

// Reject non-string / empty program / NUL bytes
for (const bad of [123, null, undefined, {}, []]) {
    let threw = false;
    try { new Command(bad); } catch { threw = true; }
    if (!threw) throw new Error(`expected rejection for program ${String(bad)}`);
    threw = false;
    try { execSync(bad); } catch { threw = true; }
    if (!threw) throw new Error(`expected execSync rejection for ${String(bad)}`);
    threw = false;
    try { runSync(bad); } catch { threw = true; }
    if (!threw) throw new Error(`expected runSync rejection for ${String(bad)}`);
}

for (const badStr of ["", "bad\0prog"]) {
    let caught = null;
    try { new Command(badStr); } catch (e) { caught = e; }
    if (!(caught instanceof CommandError) || caught.code !== "VAEL_EXEC_INVALID_ARGUMENT") {
        throw new Error(`expected VAEL_EXEC_INVALID_ARGUMENT for ${JSON.stringify(badStr)}, got ${caught}`);
    }
}

// Non-existent binary throws NotFoundOnPathError
let nf = null;
try {
    execSync("definitely_nonexistent_vael_exec_binary_xyz_99999");
} catch (e) {
    nf = e;
}
if (!(nf instanceof NotFoundOnPathError) || nf.name !== "NotFoundOnPathError" || nf.code !== "VAEL_EXEC_NOT_FOUND" || nf.fileName !== "definitely_nonexistent_vael_exec_binary_xyz_99999") {
    throw new Error(`expected NotFoundOnPathError, got ${nf && nf.name} ${nf && nf.code} ${nf && nf.fileName}`);
}

// Flexible signatures: (program, args?, options?) and (argv, options?)
const c1 = new Command("echo", ["a", "b"]);
const c2 = cmd(["echo", "a", "b"]);
if (JSON.stringify(c1.toArgs()) !== JSON.stringify(["echo", "a", "b"]) ||
    JSON.stringify(c2.toArgs()) !== JSON.stringify(["echo", "a", "b"])) {
    throw new Error("toArgs mismatch");
}
// Mutating toArgs() result does not mutate Command
const exportedArgs = c1.toArgs();
exportedArgs.push("mutated");
if (c1.toArgs().length !== 3) {
    throw new Error("toArgs() must return a defensive copy");
}
"#,
        );
    }

    #[cfg(unix)]
    #[test]
    fn command_fluent_execution_output_validate_env_stdin_and_timeout() {
        let dir = tempdir().expect("tempdir");
        let dir_js = js_str(dir.path().to_str().unwrap());

        run_js(&format!(
            r#"
import {{ Command, CommandError, Output, cmd, execSync, runSync }} from "vael:exec";

// 1. Shell metacharacters are passed literally via argv, never interpreted by a shell
const meta = execSync(["echo", "hello; echo INJECTED && $(uname) | cat"]);
if (!(meta instanceof Output)) throw new Error("execSync must return Output instance");
if (meta.text().trim() !== "hello; echo INJECTED && $(uname) | cat") {{
    throw new Error(`shell metacharacters were interpreted: ${{JSON.stringify(meta.text())}}`);
}}
if (!(meta.stdout instanceof Uint8Array) || !(meta.stderr instanceof Uint8Array)) {{
    throw new Error("stdout/stderr must be Uint8Array");
}}
// Defensive copy check on stdout
const copy = meta.stdout;
copy[0] = 0;
if (meta.stdout[0] === 0) {{
    throw new Error("mutating Output.stdout mutated internal state");
}}

// 2. Fluent builder returning this + cwd + env + envs + envRemove + envClear + stdin
const c = cmd("sh");
if (
    c.arg("-c") !== c ||
    c.args(["printf '%s|%s|%s' \"$VAEL_A\" \"$VAEL_B\" \"$(pwd)\"; cat"]) !== c ||
    c.cwd({dir_js}) !== c ||
    c.envClear() !== c ||
    c.env("PATH", "/usr/bin:/bin") !== c ||
    c.envs({{ VAEL_A: "alpha", VAEL_B: "dropme" }}) !== c ||
    c.envRemove("VAEL_B") !== c ||
    c.stdin("stdin-data\n") !== c ||
    c.stdout("piped") !== c ||
    c.stderr("piped") !== c ||
    c.timeout(5000) !== c ||
    c.maxOutputBytes(1024 * 1024) !== c ||
    c.killSignal("SIGTERM") !== c
) {{
    throw new Error("fluent methods must return this");
}}
const out = c.outputSync().validate();
if (out.code !== 0 || out.signal !== null || !out.success || out.timedOut || out.truncated) {{
    throw new Error(`unexpected Output fields: ${{JSON.stringify(out)}}`);
}}
const lines = out.lines();
if (lines.length !== 1 || !lines[0].startsWith("alpha||") || !lines[0].endsWith("stdin-data")) {{
    throw new Error(`unexpected stdout lines: ${{JSON.stringify(lines)}}`);
}}

// 3. Output json(), errorText(), errorLines(), errorJson(), validate()
const jOut = execSync("sh", ["-c", 'printf \'{{"ok":true,"n":7}}\'; printf \'{{"warn":"w"}}\\nline2\' >&2']);
if (jOut.json().ok !== true || jOut.json().n !== 7) throw new Error("Output.json() failed");
if (jOut.errorLines().length !== 2) throw new Error("Output.errorLines() failed");
const firstErr = new Command("sh", ["-c", 'printf \'{{"err":1}}\' >&2']).outputSync();
if (firstErr.errorJson().err !== 1) throw new Error("Output.errorJson() failed");

let stderrErr = null;
try {{
    jOut.validate(undefined, true);
}} catch (e) {{
    stderrErr = e;
}}
if (!(stderrErr instanceof CommandError) || stderrErr.exitCode !== 0) {{
    throw new Error("validate(..., true) should throw CommandError on non-empty stderr");
}}

const failOut = execSync(["sh", "-c", "exit 7"]);
if (failOut.success || failOut.code !== 7) throw new Error("expected exit code 7");
// Predicate allows exit code 7
failOut.validate((code) => code === 7);
failOut.validate((code) => code === 0 || code === 7);
let badPredErr = null;
try {{
    failOut.validate(7);
}} catch (e) {{
    badPredErr = e;
}}
if (!(badPredErr instanceof TypeError)) {{
    throw new Error("validate(nonFunction) must throw TypeError");
}}
let failCaught = null;
try {{
    failOut.validate();
}} catch (e) {{
    failCaught = e;
}}
if (!(failCaught instanceof CommandError) || failCaught.code !== "VAEL_EXEC_NON_ZERO" || failCaught.exitCode !== 7) {{
    throw new Error("validate() should throw CommandError(VAEL_EXEC_NON_ZERO) with exitCode 7");
}}

// 4. runSync / statusSync
const st = runSync("sh", ["-c", "exit 3"], {{ stdout: "null", stderr: "null" }});
if (st.code !== 3 || st.success !== false || st.timedOut !== false || st.signal !== null) {{
    throw new Error(`unexpected statusSync result: ${{JSON.stringify(st)}}`);
}}
const stOk = new Command(["sh", "-c", "exit 0"]).stdout("null").stderr("null").statusSync();
if (!stOk.success || stOk.code !== 0) throw new Error("statusSync exit 0 failed");

// 5. Uint8Array stdin
const binOut = new Command("cat").stdin(new Uint8Array([65, 66, 67])).outputSync();
if (binOut.text() !== "ABC") throw new Error("Uint8Array stdin failed");

// 6. Timeout and killSignal
const timed = new Command("sleep", ["2"]).timeout(50).killSignal("SIGKILL").outputSync();
if (!timed.timedOut || timed.success) {{
    throw new Error(`expected timeout, got ${{JSON.stringify(timed)}}`);
}}

// 7. maxOutputBytes exceeded -> VAEL_EXEC_OUTPUT_TOO_LARGE
let bigErr = null;
try {{
    new Command("sh", ["-c", "printf '0123456789abcdef'"]).maxOutputBytes(4).outputSync();
}} catch (e) {{
    bigErr = e;
}}
if (!(bigErr instanceof CommandError) || bigErr.code !== "VAEL_EXEC_OUTPUT_TOO_LARGE") {{
    throw new Error(`expected VAEL_EXEC_OUTPUT_TOO_LARGE, got ${{bigErr && bigErr.code}}`);
}}
"#
        ));
    }

    #[test]
    fn which_sync_and_which_all_sync() {
        let dir = tempdir().expect("tempdir");
        let bin_a = dir.path().join("bin_a");
        let bin_b = dir.path().join("bin_b");
        fs::create_dir_all(&bin_a).unwrap();
        fs::create_dir_all(&bin_b).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let exe_a = bin_a.join("vael_test_tool");
            let exe_b = bin_b.join("vael_test_tool");
            fs::write(&exe_a, b"#!/bin/sh\n").unwrap();
            fs::write(&exe_b, b"#!/bin/sh\n").unwrap();
            fs::set_permissions(&exe_a, fs::Permissions::from_mode(0o755)).unwrap();
            fs::set_permissions(&exe_b, fs::Permissions::from_mode(0o755)).unwrap();

            let prepend = format!("{}:{}", bin_a.display(), bin_b.display());
            let prepend_js = js_str(&prepend);
            let exe_a_js = js_str(exe_a.to_str().unwrap());
            let exe_b_js = js_str(exe_b.to_str().unwrap());

            run_js(&format!(
                r#"
import {{ CommandError, whichSync, whichAllSync }} from "vael:exec";

const first = whichSync("vael_test_tool", {{ prependPath: {prepend_js} }});
if (first !== {exe_a_js}) {{
    throw new Error(`whichSync returned ${{first}}, expected ${{{exe_a_js}}}`);
}}
const all = whichAllSync("vael_test_tool", {prepend_js});
if (all.length < 2 || all[0] !== {exe_a_js} || all[1] !== {exe_b_js}) {{
    throw new Error(`whichAllSync returned ${{JSON.stringify(all)}}`);
}}
if (whichSync("definitely_nonexistent_vael_tool_xyz_99999") !== null) {{
    throw new Error("whichSync for nonexistent tool must return null");
}}
if (whichAllSync("definitely_nonexistent_vael_tool_xyz_99999").length !== 0) {{
    throw new Error("whichAllSync for nonexistent tool must return []");
}}
let bad = null;
try {{
    whichSync("");
}} catch (e) {{
    bad = e;
}}
if (!(bad instanceof CommandError) || bad.code !== "VAEL_EXEC_INVALID_ARGUMENT") {{
    throw new Error(`expected VAEL_EXEC_INVALID_ARGUMENT, got ${{bad}}`);
}}
"#
            ));
        }
    }

    #[test]
    fn file_lock_unlock_using_dispose_modes_symlink_and_runtime_drop() {
        let dir = tempdir().expect("tempdir");
        let lock_path = dir.path().join("app.lock");
        let lock_js = js_str(lock_path.to_str().unwrap());
        let drop_lock_path = dir.path().join("drop.lock");
        let drop_lock_js = js_str(drop_lock_path.to_str().unwrap());

        let symlink_target = dir.path().join("real.lock");
        File::create(&symlink_target).unwrap();
        let symlink_path = dir.path().join("sym.lock");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&symlink_target, &symlink_path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&symlink_target, &symlink_path).unwrap();
        let symlink_js = js_str(symlink_path.to_str().unwrap());

        run_js(&format!(
            r#"
import {{ CommandError, FileLock, lockFileSync }} from "vael:exec";

// Cannot construct FileLock directly
let directThrew = false;
try {{ new FileLock(); }} catch {{ directThrew = true; }}
if (!directThrew) throw new Error("new FileLock() must throw");

// 1. Exclusive lock, contention, unlock, idempotent unlock
const lock1 = lockFileSync({lock_js});
if (!(lock1 instanceof FileLock) || lock1.path !== {lock_js} || lock1.mode !== "exclusive" || lock1.locked !== true) {{
    throw new Error("unexpected FileLock state");
}}
let errBlock = null;
try {{
    lockFileSync({lock_js}, {{ nonBlocking: true }});
}} catch (e) {{
    errBlock = e;
}}
if (!(errBlock instanceof CommandError) || errBlock.code !== "VAEL_EXEC_LOCK_WOULD_BLOCK") {{
    throw new Error(`expected VAEL_EXEC_LOCK_WOULD_BLOCK, got ${{errBlock && errBlock.code}}`);
}}
let errTimeout = null;
try {{
    lockFileSync({lock_js}, {{ timeoutMs: 30 }});
}} catch (e) {{
    errTimeout = e;
}}
if (!(errTimeout instanceof CommandError) || errTimeout.code !== "VAEL_EXEC_LOCK_TIMEOUT") {{
    throw new Error(`expected VAEL_EXEC_LOCK_TIMEOUT, got ${{errTimeout && errTimeout.code}}`);
}}
lock1.unlock();
if (lock1.locked !== false) throw new Error("locked should be false after unlock()");
lock1.unlock(); // idempotent

// 2. Explicit Resource Management (`using` + Symbol.dispose) even when throwing
let leaked = null;
try {{
    using scoped = lockFileSync({lock_js}, {{ mode: "exclusive", nonBlocking: true }});
    leaked = scoped;
    if (!scoped.locked) throw new Error("scoped lock should be held");
    throw new Error("inner failure");
}} catch (e) {{
    if (e.message !== "inner failure") throw e;
}}
if (!leaked || leaked.locked !== false) {{
    throw new Error("using / Symbol.dispose did not unlock on throw");
}}

// 3. Shared locks allow concurrent shared locks and block exclusive
const s1 = lockFileSync({lock_js}, {{ mode: "shared", nonBlocking: true }});
const s2 = lockFileSync({lock_js}, {{ mode: "shared", nonBlocking: true }});
if (s1.mode !== "shared" || !s1.locked || !s2.locked) throw new Error("shared locks failed");
let exContention = null;
try {{
    lockFileSync({lock_js}, {{ mode: "exclusive", nonBlocking: true }});
}} catch (e) {{
    exContention = e;
}}
if (!(exContention instanceof CommandError) || exContention.code !== "VAEL_EXEC_LOCK_WOULD_BLOCK") {{
    throw new Error("exclusive lock must fail while shared locks held");
}}
s1[Symbol.dispose]();
s2[Symbol.dispose]();

// 4. Symlink lock path must be rejected
let symErr = null;
try {{
    lockFileSync({symlink_js});
}} catch (e) {{
    symErr = e;
}}
if (!(symErr instanceof CommandError) || symErr.code !== "VAEL_EXEC_INVALID_ARGUMENT") {{
    throw new Error(`expected symlink rejection, got ${{symErr && symErr.code}}`);
}}
"#
        ));

        // 5. Runtime drop releases un-unlocked OS advisory locks!
        {
            let mut rt = Runtime::new().expect("runtime");
            rt.run_source(
                "leak-lock.js",
                &format!(
                    r#"
import {{ lockFileSync }} from "vael:exec";
globalThis.__held = lockFileSync({drop_lock_js}, {{ nonBlocking: true }});
if (!globalThis.__held.locked) throw new Error("expected lock held");
"#
                ),
            )
            .expect("acquire lock without unlocking");

            // While Runtime is alive, the OS lock is held.
            let contended =
                FileLock::acquire(&drop_lock_path, &LockOptions::default().non_blocking(true))
                    .unwrap_err();
            assert_eq!(contended.code(), ExecErrorCode::LockWouldBlock);
        }

        // After Runtime drops, the OS lock MUST be released immediately.
        let clean = FileLock::acquire(&drop_lock_path, &LockOptions::default().non_blocking(true))
            .expect("OS lock must be released on Runtime drop");
        clean.unlock().expect("unlock");
    }

    #[test]
    fn path_finder_and_command_log_hooks_lite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool_path = dir.path().join("custom-lite-tool");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::write(&tool_path, "#!/bin/sh\necho \"lite-pf:$1\"\n").unwrap();
            fs::set_permissions(&tool_path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(windows)]
        {
            fs::write(&tool_path, "@echo off\r\necho lite-pf:%1\r\n").unwrap();
        }
        let veto_marker = dir.path().join("should-not-exist-lite.txt");

        let tool_js = js_str(&tool_path.to_string_lossy());
        let veto_js = js_str(&veto_marker.to_string_lossy());

        run_js(&format!(
            r#"
import {{
    CommandError,
    NotFoundOnPathError,
    PathFinder,
    addLogger,
    clearLoggers,
    cmd,
    execSync,
    pathFinder,
    runSync,
}} from "vael:exec";

const pf = new PathFinder();
pf.set("MyLiteTool", {{
    name: "MyLiteTool",
    executable: "nonexistent-lite-bin",
    linux: ["${{VAEL_DEFINITELY_UNSET_999}}/tool", {tool_js}],
    darwin: ["${{VAEL_DEFINITELY_UNSET_999}}/tool", {tool_js}],
    windows: ["%VAEL_DEFINITELY_UNSET_999%\\tool.exe", {tool_js}],
}});
if (pf.find("mylitetool")?.name !== "MyLiteTool") {{
    throw new Error("PathFinder.find case-insensitive failed");
}}
if (pf.findExeSync("mylitetool") !== {tool_js}) {{
    throw new Error("PathFinder.findExeSync failed");
}}

pathFinder.set("lite-alias", {{
    name: "lite-alias",
    executable: "nonexistent-lite-alias",
    linux: [{tool_js}],
    darwin: [{tool_js}],
    windows: [{tool_js}],
}});

const seen = [];
const sub1 = addLogger((file, args, ev) => {{
    if (!Object.isFrozen(args) || !Object.isFrozen(ev) || Object.getPrototypeOf(ev) !== null) {{
        throw new Error("event payload must be frozen null-prototype");
    }}
    seen.push(`g1:${{ev.mode}}:${{file}}`);
}});
const sub2 = addLogger((_f, _a, ev) => seen.push(`g2:${{ev.mode}}`));

const c = cmd("lite-alias", ["ok"], {{ log: (f) => seen.push(`opt:${{f}}`) }});
const instSub = c.addLogger((f) => seen.push(`inst:${{f}}`));
const out = c.outputSync();
if (!out.text().includes("lite-pf:ok")) {{
    throw new Error(`unexpected output: ${{out.text()}}`);
}}
sub1.dispose();
instSub[Symbol.dispose]();
if (sub1.active || instSub.active || !sub2.active) {{
    throw new Error("subscription active flags wrong after partial dispose");
}}
c.log(null);
runSync(c);
clearLoggers();
if (sub2.active) throw new Error("clearLoggers must deactivate sub2");

if (
    JSON.stringify(seen) !==
    JSON.stringify([
        `g1:execSync:${{{tool_js}}}`,
        "g2:execSync",
        `opt:${{{tool_js}}}`,
        `inst:${{{tool_js}}}`,
        "g2:runSync",
    ])
) {{
    throw new Error(`unexpected hook log sequence: ${{JSON.stringify(seen)}}`);
}}

// Check reviewer fix #2: CommandError.signal is null on normal non-zero exit
const failOut = execSync(["sh", "-c", "exit 5"]);
let err5 = null;
try {{ failOut.validate(); }} catch (e) {{ err5 = e; }}
if (!(err5 instanceof CommandError) || err5.signal !== null || err5.exitCode !== 5) {{
    throw new Error(`expected signal===null on normal exit, got ${{err5 && err5.signal}}`);
}}

// Registered entry failing to resolve throws NotFoundOnPathError
pathFinder.set("lite-missing", {{
    name: "lite-missing",
    executable: "definitely-missing-vael-lite-xyz",
    linux: ["${{VAEL_DEFINITELY_UNSET_999}}/missing"],
}});
let nfCaught = false;
try {{ execSync("lite-missing"); }} catch (e) {{ nfCaught = e instanceof NotFoundOnPathError; }}
if (!nfCaught) throw new Error("expected NotFoundOnPathError");

// Audit throw veto prevents spawn
let vetoed = false;
try {{
    cmd("sh", ["-c", `touch ${{{veto_js}}}`])
        .log(() => {{ throw new Error("veto-lite"); }})
        .outputSync();
}} catch (e) {{
    vetoed = e.message === "veto-lite";
}}
if (!vetoed) throw new Error("audit throw must propagate");
pathFinder.clear();
"#
        ));

        assert!(
            !veto_marker.exists(),
            "audit veto throw must prevent process spawn in lite"
        );
    }
}

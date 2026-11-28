import test from "vael:test";
import assert from "vael:assert";
import process from "node:process";
import { tmpdir } from "node:os";
import { platform } from "vael:os";
import { join } from "vael:path";
import { exists, mkdir, realpath, remove, writeTextFile } from "vael:fs";
import { v4 } from "vael:uuid";
import * as execMod from "vael:exec";
import {
    Command,
    CommandError,
    FileLock,
    NotFoundOnPathError,
    Output,
    cmd,
    execSync,
    lockFileSync,
    runSync,
    whichAllSync,
    whichSync,
} from "vael:exec";

const EXPECTED_LITE_EXPORTS = [
    "Command",
    "CommandError",
    "FileLock",
    "NotFoundOnPathError",
    "Output",
    "cmd",
    "execSync",
    "lockFileSync",
    "runSync",
    "whichAllSync",
    "whichSync",
];

const EXPECTED_V8_EXPORTS = [
    "ChildProcess",
    "Command",
    "CommandError",
    "FileLock",
    "NotFoundOnPathError",
    "Output",
    "cmd",
    "exec",
    "execSync",
    "lockFile",
    "lockFileSync",
    "run",
    "runSync",
    "spawn",
    "which",
    "whichAll",
    "whichAllSync",
    "whichSync",
];

const VJS_BIN = process.argv[0];

function withTempDir(fn) {
    const root = join(tmpdir(), `vaeljs-exec-test-${v4()}`);
    mkdir(root, { recursive: true });
    let threw = false;
    try {
        return fn(root);
    } catch (err) {
        threw = true;
        throw err;
    } finally {
        if (exists(root)) {
            if (threw) {
                try {
                    remove(root, { recursive: true });
                } catch {
                    // Never mask the original failure thrown by `fn`.
                }
            } else {
                remove(root, { recursive: true });
            }
        }
    }
}

function assertExecError(fn, expectedCode) {
    let caught;
    try {
        fn();
    } catch (err) {
        caught = err;
    }
    assert.ok(caught instanceof Error, `expected Error, got ${caught}`);
    assert.strictEquals(
        caught.code,
        expectedCode,
        `expected ${expectedCode}, got ${caught.code} (${caught.message})`,
    );
    assert.strictEquals(typeof caught.message, "string");
    assert.ok(caught.message.length > 0, "error message must not be empty");
    return caught;
}

test("vael:exec registry, module namespace, and profile exports", async () => {
    const list = Vael.getBuiltInModulesList();
    assert.ok(list.includes("vael:exec"), "vael:exec missing from Vael.getBuiltInModulesList()");
    assert.ok(!list.includes("exec"), "bare exec must not be listed");

    const reg = Vael.getBuiltInModule("vael:exec");
    assert.ok(reg !== undefined && typeof reg === "object");
    assert.strictEquals(Vael.getBuiltInModule("vael:exec"), reg);
    assert.strictEquals(Vael.getBuiltInModule("exec"), undefined);
    assert.strictEquals(Object.isFrozen(reg), true);
    assert.strictEquals(Object.isExtensible(execMod), false);
    assert.ok(!("default" in execMod), "vael:exec must not have a default export");

    const dyn = await import("vael:exec");
    assert.strictEquals(dyn, execMod);
    assert.ok(execMod !== reg, "module namespace must be distinct from registry");

    const expected = Vael.engine === "vael" ? EXPECTED_V8_EXPORTS : EXPECTED_LITE_EXPORTS;
    assert.equals(Object.keys(execMod).sort(), expected);
    assert.equals(Object.keys(reg).sort(), expected);

    for (const name of EXPECTED_LITE_EXPORTS) {
        assert.strictEquals(execMod[name], reg[name], `${name} namespace identity mismatch`);
        assert.strictEquals(dyn[name], reg[name], `${name} dynamic import identity mismatch`);
    }
    assert.strictEquals(Command, reg.Command);
    assert.strictEquals(CommandError, reg.CommandError);
    assert.strictEquals(FileLock, reg.FileLock);
    assert.strictEquals(NotFoundOnPathError, reg.NotFoundOnPathError);
    assert.strictEquals(Output, reg.Output);
    assert.strictEquals(cmd, reg.cmd);
    assert.strictEquals(execSync, reg.execSync);
    assert.strictEquals(lockFileSync, reg.lockFileSync);
    assert.strictEquals(runSync, reg.runSync);
    assert.strictEquals(whichAllSync, reg.whichAllSync);
    assert.strictEquals(whichSync, reg.whichSync);
});

test("whichSync and whichAllSync resolve cross-platform executables", () => {
    const resolvedSelf = whichSync(VJS_BIN);
    assert.strictEquals(typeof resolvedSelf, "string");
    assert.ok(resolvedSelf.length > 0);
    const allSelf = whichAllSync(VJS_BIN);
    assert.ok(Array.isArray(allSelf) && allSelf.length >= 1);
    assert.strictEquals(allSelf[0], resolvedSelf);

    const portableCmd = platform === "windows" ? "cmd.exe" : "sh";
    const found = whichSync(portableCmd);
    assert.strictEquals(typeof found, "string");
    assert.ok(found.length > 0);
    const foundAll = whichAllSync(portableCmd);
    assert.ok(foundAll.length >= 1);
    assert.strictEquals(foundAll[0], found);

    assert.strictEquals(whichSync("vael-missing-exec-9f8e7d6c"), null);
    assert.equals(whichAllSync("vael-missing-exec-9f8e7d6c"), []);

    assertExecError(() => whichSync(""), "VAEL_EXEC_INVALID_ARGUMENT");
    assertExecError(() => whichSync("bad\0bin"), "VAEL_EXEC_INVALID_ARGUMENT");
});

test("Command fluent builder constructs argv and validates inputs", () => {
    const built = cmd(VJS_BIN)
        .arg("--alpha")
        .args(["--beta", "; echo BAD"])
        .cwd(".")
        .env("VAEL_ONE", "1")
        .envs({ VAEL_TWO: "2", VAEL_SKIP: undefined })
        .envRemove("VAEL_ONE")
        .envClear()
        .env("VAEL_FINAL", "ok");

    assert.ok(built instanceof Command);
    assert.equals(built.toArgs(), [VJS_BIN, "--alpha", "--beta", "; echo BAD"]);

    assertExecError(() => cmd(""), "VAEL_EXEC_INVALID_ARGUMENT");
    assertExecError(() => cmd(VJS_BIN).arg("bad\0arg"), "VAEL_EXEC_INVALID_ARGUMENT");
    assertExecError(() => cmd(VJS_BIN).env("BAD=KEY", "v"), "VAEL_EXEC_INVALID_ARGUMENT");
});

test("execSync and runSync execute binaries, decode Output, and resist shell injection", () => {
    const ver = execSync([VJS_BIN, "--version"]);
    assert.ok(ver instanceof Output);
    assert.strictEquals(ver.success, true);
    assert.strictEquals(ver.code, 0);
    assert.strictEquals(ver.signal, null);
    assert.strictEquals(ver.timedOut, false);
    assert.ok(ver.stdout instanceof Uint8Array);
    assert.ok(ver.stderr instanceof Uint8Array);
    assert.ok(ver.text().length > 0);
    assert.ok(ver.lines().length >= 1);
    assert.strictEquals(ver.validate(), ver);

    const status = runSync([VJS_BIN, "--version"], { stdout: "null", stderr: "null" });
    assert.strictEquals(status.success, true);
    assert.strictEquals(status.code, 0);
    assert.strictEquals(status.timedOut, false);

    withTempDir((dir) => {
        // Filenames with shell metacharacters must be passed verbatim as argv without shell execution.
        const dangerousScript = join(dir, "safe; echo INJECTED.js");
        writeTextFile(dangerousScript, 'console.log("SAFE_ARGV_ONLY");\n');

        const safeOut = execSync([VJS_BIN, dangerousScript]).validate();
        assert.strictEquals(safeOut.text().trim(), "SAFE_ARGV_ONLY");
        assert.ok(!safeOut.text().includes("INJECTED"));

        const badArgOut = execSync([VJS_BIN, "--version; echo INJECTED"]);
        assert.strictEquals(badArgOut.success, false);
        assert.ok(!badArgOut.text().includes("INJECTED"));
    });
});

test("Output.validate throws CommandError and missing binaries throw NotFoundOnPathError", () => {
    withTempDir((dir) => {
        const failScript = join(dir, "fail7.js");
        writeTextFile(
            failScript,
            'import process from "node:process";\nconsole.error("stderr-line");\nprocess.exit(7);\n',
        );

        const out = execSync([VJS_BIN, failScript]);
        assert.strictEquals(out.success, false);
        assert.strictEquals(out.code, 7);
        assert.strictEquals(out.errorText().trim(), "stderr-line");
        assert.equals(out.errorLines(), ["stderr-line"]);

        let cmdErr;
        try {
            out.validate();
        } catch (err) {
            cmdErr = err;
        }
        assert.ok(cmdErr instanceof Error);
        assert.ok(cmdErr instanceof CommandError);
        assert.strictEquals(cmdErr.name, "CommandError");
        assert.strictEquals(typeof cmdErr.message, "string");
        assert.ok(cmdErr.message.length > 0);
        if (cmdErr.exitCode !== undefined) {
            assert.strictEquals(cmdErr.exitCode, 7);
        }
        if (cmdErr.output !== undefined) {
            assert.strictEquals(cmdErr.output.code, 7);
        }
    });

    let notFoundErr;
    try {
        execSync(["vael-nonexistent-binary-9f8e7d6c"]);
    } catch (err) {
        notFoundErr = err;
    }
    assert.ok(notFoundErr instanceof Error);
    assert.ok(notFoundErr instanceof NotFoundOnPathError);
    assert.strictEquals(notFoundErr.code, "VAEL_EXEC_NOT_FOUND");
});

test("envClear, env, cwd, stdin piping, and timeout work deterministically", () => {
    withTempDir((dir) => {
        const workDir = join(dir, "work");
        mkdir(workDir, { recursive: true });

        const envScript = join(dir, "env_check.js");
        writeTextFile(
            envScript,
            [
                'import process from "node:process";',
                "console.log(JSON.stringify({",
                "  keep: process.env.VAEL_KEEP ?? null,",
                "  removed: process.env.VAEL_REMOVED ?? null,",
                "  inherited: process.env.VAEL_TEST_INHERITED ?? null,",
                "  cwd: process.cwd(),",
                "}));",
            ].join("\n"),
        );

        const prevInherited = process.env.VAEL_TEST_INHERITED;
        process.env.VAEL_TEST_INHERITED = "from-parent";
        try {
            const absBin = realpath(whichSync(VJS_BIN));
            assert.ok(typeof absBin === "string" && absBin.length > 0);
            const out = cmd(absBin)
                .arg(envScript)
                .cwd(workDir)
                .envClear()
                .env("VAEL_KEEP", "yes")
                .env("VAEL_REMOVED", "temp")
                .envRemove("VAEL_REMOVED")
                .execSync()
                .validate();

            const parsed = JSON.parse(out.text());
            assert.strictEquals(parsed.keep, "yes");
            assert.strictEquals(parsed.removed, null);
            assert.strictEquals(parsed.inherited, null);
            assert.strictEquals(realpath(parsed.cwd), realpath(workDir));
        } finally {
            if (prevInherited === undefined) {
                delete process.env.VAEL_TEST_INHERITED;
            } else {
                process.env.VAEL_TEST_INHERITED = prevInherited;
            }
        }

        const stdinTarget = platform === "windows" ? ["findstr.exe", "^"] : ["cat"];
        const stdinInput = platform === "windows" ? "vael-stdin-line\r\n" : "vael-stdin-line";
        const stdinOut = execSync(stdinTarget, { stdin: stdinInput }).validate();
        assert.ok(stdinOut.text().includes("vael-stdin-line"));

        const slowScript = join(dir, "slow.js");
        writeTextFile(slowScript, "const end = Date.now() + 5000;\nwhile (Date.now() < end) {}\n");

        const timedOut = execSync([VJS_BIN, slowScript], { timeout: 100, killGrace: 50 });
        assert.strictEquals(timedOut.timedOut, true);
        assert.strictEquals(timedOut.success, false);
        assert.throws(() => timedOut.validate(), CommandError);
    });
});

test("lockFileSync enforces exclusive/shared locks, nonBlocking, unlock, and Symbol.dispose", () => {
    withTempDir((dir) => {
        const lockPath = join(dir, "resource.lock");

        const exclusive = lockFileSync(lockPath, { mode: "exclusive", nonBlocking: true });
        assert.ok(exclusive instanceof FileLock);
        assert.strictEquals(exclusive.mode, "exclusive");

        assertExecError(
            () => lockFileSync(lockPath, { mode: "exclusive", nonBlocking: true }),
            "VAEL_EXEC_LOCK_WOULD_BLOCK",
        );
        assertExecError(
            () => lockFileSync(lockPath, { mode: "shared", nonBlocking: true }),
            "VAEL_EXEC_LOCK_WOULD_BLOCK",
        );

        exclusive.unlock();
        exclusive.unlock(); // idempotent

        const sharedA = lockFileSync(lockPath, { mode: "shared", nonBlocking: true });
        const sharedB = lockFileSync(lockPath, { mode: "shared", nonBlocking: true });
        assert.strictEquals(sharedA.mode, "shared");
        assert.strictEquals(sharedB.mode, "shared");

        assertExecError(
            () => lockFileSync(lockPath, { mode: "exclusive", nonBlocking: true }),
            "VAEL_EXEC_LOCK_WOULD_BLOCK",
        );

        sharedA.unlock();
        sharedB.unlock();

        const disposable = lockFileSync(lockPath, { mode: "exclusive", nonBlocking: true });
        if (typeof Symbol.dispose === "symbol") {
            assert.strictEquals(typeof disposable[Symbol.dispose], "function");
            disposable[Symbol.dispose]();
        } else {
            disposable.unlock();
        }

        const reacquired = lockFileSync(lockPath, { mode: "exclusive", nonBlocking: true });
        reacquired.unlock();
    });
});

if (Vael.engine === "vael") {
    test("V8 async exec, which, lockFile, and spawn APIs", async () => {
        const out = await execMod.exec([VJS_BIN, "--version"]);
        assert.ok(out instanceof Output);
        assert.strictEquals(out.success, true);

        const found = await execMod.which(VJS_BIN);
        assert.strictEquals(found, whichSync(VJS_BIN));

        const child = execMod.spawn([VJS_BIN, "--version"]);
        assert.ok(child instanceof execMod.ChildProcess);
        assert.strictEquals(typeof child.pid, "number");
        const childOut = await child.waitWithOutput();
        assert.strictEquals(childOut.success, true);

        const dir = join(tmpdir(), `vaeljs-exec-async-${v4()}`);
        mkdir(dir, { recursive: true });
        try {
            const lock = await execMod.lockFile(join(dir, "async.lock"), {
                mode: "exclusive",
                nonBlocking: true,
            });
            assert.ok(lock instanceof FileLock);
            lock.unlock();
        } finally {
            remove(dir, { recursive: true });
        }
    });
}

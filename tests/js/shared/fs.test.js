import test from "neo:test";
import assert from "neo:assert";
import { platform } from "neo:os";
import { v4 } from "neo:uuid";
import * as fsMod from "neo:fs";
import {
    copyFile,
    exists,
    lstat,
    mkdir,
    readDir,
    readFile,
    readTextFile,
    realpath,
    remove,
    rename,
    stat,
    writeFile,
    writeFileAtomic,
    writeTextFile,
} from "neo:fs";

const EXPECTED_EXPORTS = [
    "copyFile",
    "exists",
    "lstat",
    "mkdir",
    "readDir",
    "readFile",
    "readTextFile",
    "realpath",
    "remove",
    "rename",
    "stat",
    "writeFile",
    "writeFileAtomic",
    "writeTextFile",
];

function joinPath(a, b) {
    const sep = platform === "windows" ? "\\" : "/";
    return `${a}${sep}${b}`;
}

function withTempDir(fn) {
    const base = platform === "windows" ? "." : "/tmp";
    const root = joinPath(base, `.neojs-fs-test-${v4()}`);
    mkdir(root, { recursive: true });
    try {
        return fn(root);
    } finally {
        remove(root, { recursive: true });
    }
}

function assertFsError(fn, expectedCode) {
    let caught;
    try {
        fn();
    } catch (err) {
        caught = err;
    }
    assert.ok(caught instanceof Error, `expected Error, got ${caught}`);
    assert.strictEquals(caught.code, expectedCode, `expected ${expectedCode}, got ${caught.code} (${caught.message})`);
    assert.strictEquals(typeof caught.message, "string");
    assert.ok(caught.message.length > 0, "error message must not be empty");
    return caught;
}

test("neo:fs exposes frozen module object and identical function references", async () => {
    const list = Neo.getBuiltInModulesList();
    assert.ok(list.includes("neo:fs"), "neo:fs missing from Neo.getBuiltInModulesList()");
    assert.ok(!list.includes("fs"), "bare fs must not be listed");

    const reg = Neo.getBuiltInModule("neo:fs");
    assert.ok(reg !== undefined && typeof reg === "object");
    assert.strictEquals(Neo.getBuiltInModule("neo:fs"), reg);
    assert.strictEquals(Neo.getBuiltInModule("fs"), undefined);
    assert.strictEquals(Object.isFrozen(reg), true);
    assert.strictEquals(Object.isExtensible(fsMod), false);
    assert.ok(!("default" in fsMod), "neo:fs must not have a default export");

    const dyn = await import("neo:fs");
    assert.strictEquals(dyn, fsMod);
    assert.ok(fsMod !== reg, "namespace must be distinct from registry");

    assert.equals(Object.keys(fsMod).sort(), EXPECTED_EXPORTS);
    assert.equals(Object.keys(reg).sort(), EXPECTED_EXPORTS);

    for (const name of EXPECTED_EXPORTS) {
        assert.strictEquals(typeof reg[name], "function", `${name} must be a function`);
        assert.strictEquals(fsMod[name], reg[name], `${name} namespace identity mismatch`);
        assert.strictEquals(dyn[name], reg[name], `${name} dynamic import identity mismatch`);
    }

    assert.strictEquals(copyFile, reg.copyFile);
    assert.strictEquals(exists, reg.exists);
    assert.strictEquals(lstat, reg.lstat);
    assert.strictEquals(mkdir, reg.mkdir);
    assert.strictEquals(readDir, reg.readDir);
    assert.strictEquals(readFile, reg.readFile);
    assert.strictEquals(readTextFile, reg.readTextFile);
    assert.strictEquals(realpath, reg.realpath);
    assert.strictEquals(remove, reg.remove);
    assert.strictEquals(rename, reg.rename);
    assert.strictEquals(stat, reg.stat);
    assert.strictEquals(writeFile, reg.writeFile);
    assert.strictEquals(writeFileAtomic, reg.writeFileAtomic);
    assert.strictEquals(writeTextFile, reg.writeTextFile);

    assert.throws(() => {
        reg.readFile = () => {};
    }, TypeError);
    assert.throws(() => {
        fsMod.readFile = () => {};
    }, TypeError);
    await assert.rejects(() => import("fs"));
});

test("readFile, readTextFile, writeFile, writeTextFile, and maxBytes", () => {
    withTempDir((dir) => {
        const binPath = joinPath(dir, "data.bin");
        const bytes = new Uint8Array([0, 1, 2, 127, 255]);
        assert.strictEquals(writeFile(binPath, bytes), undefined);
        assert.strictEquals(exists(binPath), true);

        const readBack = readFile(binPath);
        assert.ok(readBack instanceof Uint8Array);
        assert.equals(Array.from(readBack), [0, 1, 2, 127, 255]);

        // Offset view writes only its own slice.
        const view = new Uint8Array([10, 20, 30, 40]).subarray(1, 3);
        const slicePath = joinPath(dir, "slice.bin");
        writeFile(slicePath, view);
        assert.equals(Array.from(readFile(slicePath)), [20, 30]);

        // maxBytes boundary
        assert.equals(Array.from(readFile(binPath, { maxBytes: 5 })), [0, 1, 2, 127, 255]);
        assertFsError(() => readFile(binPath, { maxBytes: 4 }), "NEO_FS_TOO_LARGE");

        const emptyPath = joinPath(dir, "empty.bin");
        writeFile(emptyPath, new Uint8Array(0));
        assert.equals(Array.from(readFile(emptyPath, { maxBytes: 0 })), []);
        assertFsError(() => readFile(binPath, { maxBytes: 0 }), "NEO_FS_TOO_LARGE");

        // writeTextFile / readTextFile
        const txtPath = joinPath(dir, "note.txt");
        assert.strictEquals(writeTextFile(txtPath, "hello "), undefined);
        writeTextFile(txtPath, "neo:fs", { append: true });
        assert.strictEquals(readTextFile(txtPath), "hello neo:fs");
        assert.strictEquals(readTextFile(txtPath, { maxBytes: 12 }), "hello neo:fs");
        assertFsError(() => readTextFile(txtPath, { maxBytes: 11 }), "NEO_FS_TOO_LARGE");

        // create: false on missing vs existing
        const missingPath = joinPath(dir, "missing.txt");
        assertFsError(() => writeTextFile(missingPath, "x", { create: false }), "NEO_FS_NOT_FOUND");
        writeTextFile(txtPath, "replaced", { create: false });
        assert.strictEquals(readTextFile(txtPath), "replaced");

        // createNew: true fails if exists
        assertFsError(() => writeFile(txtPath, bytes, { createNew: true }), "NEO_FS_ALREADY_EXISTS");
        assertFsError(() => writeTextFile(txtPath, "x", { createNew: true }), "NEO_FS_ALREADY_EXISTS");

        // createNew + append is invalid argument
        assertFsError(
            () => writeFile(missingPath, bytes, { createNew: true, append: true }),
            "NEO_FS_INVALID_ARGUMENT",
        );
        assertFsError(
            () => writeTextFile(missingPath, "x", { createNew: true, append: true }),
            "NEO_FS_INVALID_ARGUMENT",
        );

        // Invalid UTF-8 on readTextFile
        const badUtf8 = joinPath(dir, "bad.bin");
        writeFile(badUtf8, new Uint8Array([0xff, 0xfe, 0xfd]));
        assertFsError(() => readTextFile(badUtf8), "NEO_FS_INVALID_UTF8");
    });
});

test("writeFileAtomic handles Uint8Array, string, sync, and overwrite", () => {
    withTempDir((dir) => {
        const cfg = joinPath(dir, "config.json");
        assert.strictEquals(writeFileAtomic(cfg, '{"v":1}'), undefined);
        assert.strictEquals(readTextFile(cfg), '{"v":1}');

        // overwrite: false fails without clobbering
        assertFsError(
            () => writeFileAtomic(cfg, '{"v":2}', { overwrite: false, sync: false }),
            "NEO_FS_ALREADY_EXISTS",
        );
        assert.strictEquals(readTextFile(cfg), '{"v":1}');

        // overwrite: true (default) replaces with Uint8Array
        writeFileAtomic(cfg, new Uint8Array([65, 66, 67]), { sync: false });
        assert.strictEquals(readTextFile(cfg), "ABC");

        // new file with overwrite: false succeeds
        const fresh = joinPath(dir, "fresh.bin");
        writeFileAtomic(fresh, new Uint8Array([9, 8]), { overwrite: false });
        assert.equals(Array.from(readFile(fresh)), [9, 8]);

        // Trailing separator or dot rejected with NEO_FS_INVALID_PATH
        assertFsError(() => writeFileAtomic(`${dir}/`, "x"), "NEO_FS_INVALID_PATH");
        assertFsError(() => writeFileAtomic(".", "x"), "NEO_FS_INVALID_PATH");
    });
});

test("exists, stat, lstat, readDir, mkdir, remove, rename, copyFile, realpath", () => {
    withTempDir((dir) => {
        const sub = joinPath(dir, "nested/a/b");
        assert.strictEquals(exists(sub), false);
        assertFsError(() => mkdir(sub), "NEO_FS_NOT_FOUND");
        assert.strictEquals(mkdir(sub, { recursive: true }), undefined);
        assert.strictEquals(exists(sub), true);
        // mkdir recursive succeeds if already exists; non-recursive fails
        mkdir(sub, { recursive: true });
        assertFsError(() => mkdir(sub, { recursive: false }), "NEO_FS_ALREADY_EXISTS");

        // stat / lstat on dir and file
        const dStat = stat(sub);
        assert.strictEquals(Object.isFrozen(dStat), true);
        assert.strictEquals(dStat.isFile, false);
        assert.strictEquals(dStat.isDirectory, true);
        assert.strictEquals(dStat.isSymlink, false);
        assert.strictEquals(typeof dStat.size, "number");
        assert.ok(dStat.modifiedMs === null || typeof dStat.modifiedMs === "number");
        assert.strictEquals(typeof dStat.readonly, "boolean");

        const fileA = joinPath(sub, "z.txt");
        const fileB = joinPath(sub, "a.txt");
        const childDir = joinPath(sub, "m_dir");
        writeTextFile(fileA, "12345");
        writeTextFile(fileB, "12");
        mkdir(childDir);

        const fStat = stat(fileA);
        assert.strictEquals(Object.isFrozen(fStat), true);
        assert.strictEquals(fStat.isFile, true);
        assert.strictEquals(fStat.isDirectory, false);
        assert.strictEquals(fStat.isSymlink, false);
        assert.strictEquals(fStat.size, 5);

        const fLstat = lstat(fileA);
        assert.strictEquals(Object.isFrozen(fLstat), true);
        assert.strictEquals(fLstat.isFile, true);
        assert.strictEquals(fLstat.size, 5);

        // readDir sorted lexicographically with frozen entries
        const entries = readDir(sub);
        assert.ok(Array.isArray(entries));
        assert.equals(
            entries.map((e) => e.name),
            ["a.txt", "m_dir", "z.txt"],
        );
        for (const entry of entries) {
            assert.strictEquals(Object.isFrozen(entry), true);
            assert.equals(Object.keys(entry).sort(), ["isDirectory", "isFile", "isSymlink", "name"]);
        }
        assert.strictEquals(entries[0].isFile, true);
        assert.strictEquals(entries[1].isDirectory, true);
        assert.strictEquals(entries[2].isFile, true);

        // readDir on non-directory
        assertFsError(() => readDir(fileA), "NEO_FS_NOT_ADirectory".replace("ADirectory", "A_DIRECTORY"));

        // copyFile default overwrite: false vs overwrite: true vs self-copy
        const copied = joinPath(sub, "copy.txt");
        assert.strictEquals(copyFile(fileA, copied), undefined);
        assert.strictEquals(readTextFile(copied), "12345");
        assertFsError(() => copyFile(fileB, copied), "NEO_FS_ALREADY_EXISTS");
        assertFsError(() => copyFile(fileB, copied, { overwrite: false }), "NEO_FS_ALREADY_EXISTS");
        assert.strictEquals(readTextFile(copied), "12345");

        copyFile(fileB, copied, { overwrite: true });
        assert.strictEquals(readTextFile(copied), "12");

        // Self-copy with overwrite: true is a safe no-op
        copyFile(copied, copied, { overwrite: true });
        assert.strictEquals(readTextFile(copied), "12");
        assertFsError(() => copyFile(childDir, joinPath(sub, "bad")), "NEO_FS_IS_A_DIRECTORY");

        // rename
        const renamed = joinPath(sub, "renamed.txt");
        assert.strictEquals(rename(copied, renamed), undefined);
        assert.strictEquals(exists(copied), false);
        assert.strictEquals(readTextFile(renamed), "12");

        // realpath
        const viaDot = joinPath(sub, "./renamed.txt");
        assert.strictEquals(realpath(viaDot), realpath(renamed));
        assertFsError(() => realpath(joinPath(sub, "nope")), "NEO_FS_NOT_FOUND");

        // remove non-empty fails without recursive, succeeds with recursive
        const nestedRoot = joinPath(dir, "nested");
        assertFsError(() => remove(nestedRoot), "NEO_FS_DIRECTORY_NOT_EMPTY");
        assertFsError(() => remove(nestedRoot, { recursive: false }), "NEO_FS_DIRECTORY_NOT_EMPTY");
        assert.strictEquals(remove(nestedRoot, { recursive: true }), undefined);
        assert.strictEquals(exists(nestedRoot), false);
        assertFsError(() => remove(nestedRoot), "NEO_FS_NOT_FOUND");
    });
});

test("argument count, type validation, options validation, and detached Uint8Array", () => {
    withTempDir((dir) => {
        const file = joinPath(dir, "v.txt");
        writeTextFile(file, "ok");

        // Arity checks
        assert.throws(() => readFile(), TypeError);
        assert.throws(() => readFile(file, {}, "extra"), TypeError);
        assert.throws(() => readTextFile(), TypeError);
        assert.throws(() => readTextFile(file, {}, "extra"), TypeError);
        assert.throws(() => writeFile(file), TypeError);
        assert.throws(() => writeFile(file, new Uint8Array(0), {}, "extra"), TypeError);
        assert.throws(() => writeTextFile(file), TypeError);
        assert.throws(() => writeTextFile(file, "a", {}, "extra"), TypeError);
        assert.throws(() => writeFileAtomic(file), TypeError);
        assert.throws(() => writeFileAtomic(file, "a", {}, "extra"), TypeError);
        assert.throws(() => exists(), TypeError);
        assert.throws(() => exists(file, undefined), TypeError);
        assert.throws(() => stat(), TypeError);
        assert.throws(() => stat(file, undefined), TypeError);
        assert.throws(() => lstat(), TypeError);
        assert.throws(() => lstat(file, undefined), TypeError);
        assert.throws(() => readDir(), TypeError);
        assert.throws(() => readDir(dir, undefined), TypeError);
        assert.throws(() => mkdir(), TypeError);
        assert.throws(() => mkdir(dir, {}, "extra"), TypeError);
        assert.throws(() => remove(), TypeError);
        assert.throws(() => remove(dir, {}, "extra"), TypeError);
        assert.throws(() => rename(file), TypeError);
        assert.throws(() => rename(file, file, "extra"), TypeError);
        assert.throws(() => copyFile(file), TypeError);
        assert.throws(() => copyFile(file, file, {}, "extra"), TypeError);
        assert.throws(() => realpath(), TypeError);
        assert.throws(() => realpath(file, undefined), TypeError);

        // Wrong-type required arguments
        for (const bad of [undefined, null, 123, true, {}, [], new String(file)]) {
            assert.throws(() => readFile(bad), TypeError);
            assert.throws(() => readTextFile(bad), TypeError);
            assert.throws(() => exists(bad), TypeError);
            assert.throws(() => stat(bad), TypeError);
            assert.throws(() => lstat(bad), TypeError);
            assert.throws(() => readDir(bad), TypeError);
            assert.throws(() => mkdir(bad), TypeError);
            assert.throws(() => remove(bad), TypeError);
            assert.throws(() => realpath(bad), TypeError);
            assert.throws(() => rename(bad, file), TypeError);
            assert.throws(() => rename(file, bad), TypeError);
            assert.throws(() => copyFile(bad, file), TypeError);
            assert.throws(() => copyFile(file, bad), TypeError);
            assert.throws(() => writeFile(bad, new Uint8Array(0)), TypeError);
            assert.throws(() => writeTextFile(bad, "x"), TypeError);
            assert.throws(() => writeFileAtomic(bad, "x"), TypeError);
        }

        // Wrong data types
        for (const badData of ["str", 123, null, undefined, [], new Int8Array(2), new Uint8ClampedArray(2)]) {
            assert.throws(() => writeFile(file, badData), TypeError);
        }
        for (const badText of [new Uint8Array(2), 123, null, undefined, {}, new String("hi")]) {
            assert.throws(() => writeTextFile(file, badText), TypeError);
        }
        for (const badAtomic of [123, null, undefined, [], {}, new Int8Array(2), new String("hi")]) {
            assert.throws(() => writeFileAtomic(file, badAtomic), TypeError);
        }

        // Non-object / non-plain options rejected
        for (const badOpts of [null, 1, "x", true, [], () => {}, new Map(), new Proxy({}, {}), Object.create({ recursive: true })]) {
            assert.throws(() => readFile(file, badOpts), TypeError);
            assert.throws(() => readTextFile(file, badOpts), TypeError);
            assert.throws(() => writeFile(file, new Uint8Array(0), badOpts), TypeError);
            assert.throws(() => writeTextFile(file, "x", badOpts), TypeError);
            assert.throws(() => writeFileAtomic(file, "x", badOpts), TypeError);
            assert.throws(() => mkdir(dir, badOpts), TypeError);
            assert.throws(() => remove(dir, badOpts), TypeError);
            assert.throws(() => copyFile(file, joinPath(dir, "c"), badOpts), TypeError);
        }

        // Unknown keys, getters, non-enumerable, symbol keys rejected
        assert.throws(() => readFile(file, { unknown: 1 }), TypeError);
        assert.throws(() => writeFile(file, new Uint8Array(0), { overwrite: true }), TypeError);
        assert.throws(() => writeFileAtomic(file, "x", { append: true }), TypeError);
        assert.throws(() => mkdir(dir, { parents: true }), TypeError);
        assert.throws(() => remove(dir, { force: true }), TypeError);
        assert.throws(() => copyFile(file, joinPath(dir, "c"), { recursive: true }), TypeError);

        let getterRan = false;
        const getterOpts = {
            get recursive() {
                getterRan = true;
                return true;
            },
        };
        assert.throws(() => mkdir(joinPath(dir, "g"), getterOpts), TypeError);
        assert.strictEquals(getterRan, false);

        const symOpts = { [Symbol("k")]: true };
        assert.throws(() => mkdir(joinPath(dir, "s"), symOpts), TypeError);

        const nonEnumOpts = {};
        Object.defineProperty(nonEnumOpts, "recursive", { value: true, enumerable: false });
        assert.throws(() => mkdir(joinPath(dir, "ne"), nonEnumOpts), TypeError);

        // Invalid maxBytes values
        for (const badType of ["5", null, true, {}, 5n]) {
            assert.throws(() => readFile(file, { maxBytes: badType }), TypeError);
            assert.throws(() => readTextFile(file, { maxBytes: badType }), TypeError);
        }
        for (const badRange of [-1, -0.5, 1.5, NaN, Infinity, -Infinity, Number.MAX_SAFE_INTEGER + 1]) {
            assert.throws(() => readFile(file, { maxBytes: badRange }), RangeError);
            assert.throws(() => readTextFile(file, { maxBytes: badRange }), RangeError);
        }

        // Invalid path strings
        assertFsError(() => exists(""), "NEO_FS_INVALID_PATH");
        assertFsError(() => exists("bad\0path"), "NEO_FS_INVALID_PATH");
        assertFsError(() => exists("lone\ud800surrogate"), "NEO_FS_INVALID_PATH");

        // Detached Uint8Array buffer
        const ab = new ArrayBuffer(4);
        const u8 = new Uint8Array(ab);
        ab.transfer();
        assert.throws(() => writeFile(file, u8), TypeError);
        assert.throws(() => writeFileAtomic(file, u8), TypeError);
    });
});

test("hostile prototypes cannot hijack errors, options, or frozen return values", () => {
    withTempDir((dir) => {
        const file = joinPath(dir, "hostile.txt");
        writeTextFile(file, "safe");

        const origCodeDesc = Object.getOwnPropertyDescriptor(Error.prototype, "code");
        const origObjCodeDesc = Object.getOwnPropertyDescriptor(Object.prototype, "code");
        const origArr0Desc = Object.getOwnPropertyDescriptor(Array.prototype, "0");
        const origRecursiveDesc = Object.getOwnPropertyDescriptor(Object.prototype, "recursive");
        const origFreeze = Object.freeze;

        let traps = 0;
        try {
            Object.defineProperty(Error.prototype, "code", {
                configurable: true,
                set() {
                    traps += 1;
                },
                get() {
                    return "HIJACKED";
                },
            });
            Object.defineProperty(Object.prototype, "code", {
                configurable: true,
                set() {
                    traps += 1;
                },
            });
            Object.defineProperty(Array.prototype, "0", {
                configurable: true,
                set() {
                    traps += 1;
                },
            });
            Object.defineProperty(Object.prototype, "recursive", {
                configurable: true,
                get() {
                    traps += 1;
                    return false;
                },
            });
            Object.freeze = () => {
                traps += 1;
                throw new Error("hijacked freeze");
            };

            const err = assertFsError(() => stat(joinPath(dir, "missing")), "NEO_FS_NOT_FOUND");
            assert.strictEquals(Object.getOwnPropertyDescriptor(err, "code").value, "NEO_FS_NOT_FOUND");

            const info = stat(file);
            assert.strictEquals(info.isFile, true);
            assert.strictEquals(Object.isFrozen(info), true);

            const entries = readDir(dir);
            assert.strictEquals(entries.length, 1);
            assert.strictEquals(entries[0].name, "hostile.txt");
            assert.strictEquals(Object.isFrozen(entries[0]), true);

            // Inherited Object.prototype.recursive must be ignored
            const sub = joinPath(dir, "sub");
            mkdir(sub);
            assertFsError(() => remove(dir), "NEO_FS_DIRECTORY_NOT_EMPTY");

            assert.strictEquals(traps, 0, "hostile prototype traps must not fire");
        } finally {
            Object.freeze = origFreeze;
            if (origCodeDesc) Object.defineProperty(Error.prototype, "code", origCodeDesc);
            else delete Error.prototype.code;
            if (origObjCodeDesc) Object.defineProperty(Object.prototype, "code", origObjCodeDesc);
            else delete Object.prototype.code;
            if (origArr0Desc) Object.defineProperty(Array.prototype, "0", origArr0Desc);
            else delete Array.prototype["0"];
            if (origRecursiveDesc) Object.defineProperty(Object.prototype, "recursive", origRecursiveDesc);
            else delete Object.prototype.recursive;
        }
    });
});

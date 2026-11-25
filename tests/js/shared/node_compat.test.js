import test from "vael:test";
import assert from "vael:assert";

import pathDefault, {
    join,
    resolve,
    normalize,
    relative,
    dirname,
    basename,
    extname,
    parse,
    format,
    isAbsolute,
    sep,
    delimiter,
    posix,
    win32,
} from "node:path";
import * as pathPosix from "node:path/posix";
import * as pathWin32 from "node:path/win32";

import osDefault, {
    platform as osPlatform,
    arch as osArch,
    EOL,
    homedir,
    tmpdir,
    hostname,
    cpus,
    endianness,
    type as osType,
    machine,
} from "node:os";

import processDefault, {
    argv,
    cwd,
    chdir,
    env,
    exit,
    platform as procPlatform,
    arch as procArch,
    pid,
    version,
    versions,
    hrtime,
} from "node:process";

import fsDefault, {
    readFileSync,
    writeFileSync,
    appendFileSync,
    existsSync,
    statSync,
    lstatSync,
    readdirSync,
    mkdirSync,
    rmSync,
    unlinkSync,
    renameSync,
    copyFileSync,
    realpathSync,
    mkdtempSync,
    constants as fsConstants,
} from "node:fs";

import childDefault, { spawnSync, execFileSync } from "node:child_process";
import bufferDefault, { Buffer } from "node:buffer";
import urlDefault, {
    fileURLToPath,
    pathToFileURL,
    URL,
    URLSearchParams,
} from "node:url";
import cryptoDefault, {
    randomBytes,
    randomUUID,
    createHash,
    createHmac,
    pbkdf2Sync,
    timingSafeEqual,
} from "node:crypto";

test("node:* stays isolated from vael:* registry, globals, and bare names", async () => {
    const list = Vael.getBuiltInModulesList();
    for (const item of list) {
        assert.ok(item.startsWith("vael:"), `unexpected non-vael entry in list: ${item}`);
    }

    for (const nodeSpec of [
        "node:path",
        "node:path/posix",
        "node:path/win32",
        "node:os",
        "node:process",
        "node:fs",
        "node:child_process",
        "node:buffer",
        "node:url",
        "node:crypto",
    ]) {
        assert.strictEquals(
            Vael.getBuiltInModule(nodeSpec),
            undefined,
            `Vael.getBuiltInModule(${nodeSpec}) must return undefined`,
        );
    }

    assert.strictEquals(globalThis.process, undefined);
    assert.strictEquals(globalThis.Buffer, undefined);
    assert.strictEquals(globalThis.URL, undefined);

    for (const bare of [
        "fs",
        "path",
        "child_process",
        "os",
        "process",
        "crypto",
        "url",
        "buffer",
    ]) {
        await assert.rejects(
            () => import(bare),
            undefined,
            `bare specifier "${bare}" must be rejected`,
        );
    }
});

test("node:path, node:path/posix, and node:path/win32 work identically on both engines", () => {
    assert.strictEquals(typeof pathDefault.join, "function");
    assert.strictEquals(join, pathDefault.join);
    assert.strictEquals(sep, pathDefault.sep);
    assert.strictEquals(delimiter, pathDefault.delimiter);

    assert.strictEquals(posix.sep, "/");
    assert.strictEquals(posix.delimiter, ":");
    assert.strictEquals(posix.join("/a", "b", "../c"), "/a/c");
    assert.strictEquals(posix.resolve("/a/b", "./c"), "/a/b/c");
    assert.strictEquals(posix.normalize("/a/b/../c/"), "/a/c/");
    assert.strictEquals(posix.relative("/a/b", "/a/c/d"), "../c/d");
    assert.strictEquals(posix.dirname("/a/b/c.ts"), "/a/b");
    assert.strictEquals(posix.basename("/a/b/c.ts", ".ts"), "c");
    assert.strictEquals(posix.extname("file.tar.gz"), ".gz");
    assert.strictEquals(posix.isAbsolute("/tmp"), true);
    assert.strictEquals(posix.isAbsolute("tmp"), false);

    const parsed = posix.parse("/home/dev/file.js");
    assert.strictEquals(parsed.root, "/");
    assert.strictEquals(parsed.dir, "/home/dev");
    assert.strictEquals(parsed.base, "file.js");
    assert.strictEquals(parsed.ext, ".js");
    assert.strictEquals(parsed.name, "file");
    assert.strictEquals(posix.format(parsed), "/home/dev/file.js");

    assert.strictEquals(pathPosix.join("a", "b"), "a/b");
    assert.strictEquals(pathPosix.default.sep, "/");

    assert.strictEquals(win32.sep, "\\");
    assert.strictEquals(win32.delimiter, ";");
    assert.strictEquals(win32.join("C:\\foo", "bar", "..\\baz"), "C:\\foo\\baz");
    assert.strictEquals(win32.isAbsolute("C:\\foo"), true);
    assert.strictEquals(pathWin32.dirname("C:\\foo\\bar.txt"), "C:\\foo");
    assert.strictEquals(pathWin32.default.sep, "\\");

    assert.strictEquals(
        normalize(resolve("a", "b", "..", "c")),
        resolve("a", "c"),
    );
    assert.strictEquals(dirname("/a/b"), posix.dirname("/a/b"));
    assert.strictEquals(basename("/a/b.js"), "b.js");
    assert.strictEquals(extname("/a/b.js"), ".js");
    assert.strictEquals(isAbsolute("/a"), true);
    assert.strictEquals(relative("/a/b", "/a/b/c"), "c");
    assert.strictEquals(format(parse("/a/b.js")), "/a/b.js");
});

test("node:os exposes platform metadata", () => {
    assert.strictEquals(typeof osDefault.platform, "function");
    assert.ok(osPlatform().length > 0);
    assert.ok(osArch().length > 0);
    assert.ok(EOL === "\n" || EOL === "\r\n");
    assert.ok(homedir().length > 0);
    assert.ok(tmpdir().length > 0);
    assert.ok(hostname().length > 0);
    assert.ok(endianness() === "LE" || endianness() === "BE");
    assert.ok(osType().length > 0);
    assert.ok(machine().length > 0);

    const list = cpus();
    assert.ok(Array.isArray(list) && list.length >= 1);
    assert.strictEquals(typeof list[0].model, "string");
    assert.strictEquals(typeof list[0].speed, "number");
    assert.strictEquals(typeof list[0].times.user, "number");
});

test("node:process exposes argv, cwd/chdir, env proxy, exitCode, and hrtime", () => {
    assert.ok(Array.isArray(argv) && argv.length >= 1);
    assert.strictEquals(typeof exit, "function");
    assert.ok(procPlatform.length > 0);
    assert.ok(procArch.length > 0);
    assert.ok(typeof pid === "number" && pid > 0);
    assert.ok(typeof version === "string" && version.startsWith("v"));
    assert.strictEquals(typeof versions, "object");

    const origCwd = cwd();
    const tmp = mkdtempSync(join(tmpdir(), "vaeljs-proc-"));
    try {
        chdir(tmp);
        assert.strictEquals(realpathSync(cwd()), realpathSync(tmp));
    } finally {
        chdir(origCwd);
        rmSync(tmp, { recursive: true, force: true });
    }

    env.VAELJS_NODE_COMPAT_TEST = "hello";
    assert.strictEquals(env.VAELJS_NODE_COMPAT_TEST, "hello");
    assert.ok("VAELJS_NODE_COMPAT_TEST" in env);
    assert.ok(Object.keys(env).includes("VAELJS_NODE_COMPAT_TEST"));
    delete env.VAELJS_NODE_COMPAT_TEST;
    assert.strictEquals(env.VAELJS_NODE_COMPAT_TEST, undefined);

    assert.strictEquals(processDefault.exitCode, undefined);
    processDefault.exitCode = 5;
    assert.strictEquals(processDefault.exitCode, 5);
    processDefault.exitCode = undefined;
    assert.strictEquals(processDefault.exitCode, undefined);

    const t0 = hrtime();
    assert.ok(Array.isArray(t0) && t0.length === 2);
    const diff = hrtime(t0);
    assert.ok(Array.isArray(diff) && diff.length === 2 && diff[0] >= 0);
    const bi = hrtime.bigint();
    assert.strictEquals(typeof bi, "bigint");
    assert.ok(bi > 0n);
});

test("node:fs sync file operations roundtrip cleanly", () => {
    assert.strictEquals(typeof fsDefault.readFileSync, "function");
    assert.strictEquals(typeof fsConstants.F_OK, "number");
    assert.strictEquals(typeof fsConstants.COPYFILE_EXCL, "number");

    const dir = mkdtempSync(join(tmpdir(), "vaeljs-node-fs-"));
    try {
        const sub = join(dir, "nested", "sub");
        mkdirSync(sub, { recursive: true });
        assert.strictEquals(existsSync(sub), true);

        const file = join(sub, "hello.txt");
        writeFileSync(file, "alpha", "utf8");
        appendFileSync(file, "-beta", "utf8");

        assert.strictEquals(readFileSync(file, "utf8"), "alpha-beta");
        assert.strictEquals(readFileSync(file, { encoding: "utf-8" }), "alpha-beta");
        assert.strictEquals(readFileSync(pathToFileURL(file), "utf8"), "alpha-beta");

        const raw = readFileSync(file);
        assert.strictEquals(Buffer.isBuffer(raw), true);
        assert.strictEquals(raw.toString("utf8"), "alpha-beta");

        const st = statSync(file);
        assert.strictEquals(st.isFile(), true);
        assert.strictEquals(st.isDirectory(), false);
        assert.strictEquals(st.size, 10);
        assert.strictEquals(lstatSync(file).isFile(), true);
        assert.strictEquals(statSync(sub).isDirectory(), true);

        const names = readdirSync(sub);
        assert.equals(names, ["hello.txt"]);
        const dirents = readdirSync(sub, { withFileTypes: true });
        assert.strictEquals(dirents.length, 1);
        assert.strictEquals(dirents[0].name, "hello.txt");
        assert.strictEquals(dirents[0].isFile(), true);

        const copied = join(sub, "copied.txt");
        copyFileSync(file, copied);
        assert.throws(() => copyFileSync(file, copied, fsConstants.COPYFILE_EXCL));

        const moved = join(sub, "moved.txt");
        renameSync(copied, moved);
        assert.strictEquals(existsSync(copied), false);
        assert.strictEquals(readFileSync(moved, "utf8"), "alpha-beta");

        unlinkSync(moved);
        assert.strictEquals(existsSync(moved), false);
    } finally {
        rmSync(dir, { recursive: true, force: true });
    }
    assert.strictEquals(existsSync(dir), false);
});

test("node:child_process spawnSync and execFileSync handle stdout, stderr, exit codes, timeout, and maxBuffer", () => {
    assert.strictEquals(typeof childDefault.spawnSync, "function");

    const ok = spawnSync("sh", ["-c", "printf 'hello-out'; printf 'hello-err' >&2"], {
        encoding: "utf8",
    });
    assert.strictEquals(ok.status, 0);
    assert.strictEquals(ok.signal, null);
    assert.strictEquals(ok.timedOut, false);
    assert.strictEquals(ok.stdout, "hello-out");
    assert.strictEquals(ok.stderr, "hello-err");

    const stdinRes = spawnSync("sh", ["-c", "cat"], {
        input: "pipe-data",
        encoding: "utf8",
    });
    assert.strictEquals(stdinRes.status, 0);
    assert.strictEquals(stdinRes.stdout, "pipe-data");

    const rawRes = spawnSync("sh", ["-c", "printf 'xy'"]);
    assert.strictEquals(rawRes.status, 0);
    assert.strictEquals(Buffer.isBuffer(rawRes.stdout), true);
    assert.strictEquals(rawRes.stdout.toString("utf8"), "xy");

    assert.strictEquals(
        execFileSync("sh", ["-c", "printf 'exec-ok'"], { encoding: "utf8" }),
        "exec-ok",
    );

    let failedErr;
    try {
        execFileSync("sh", ["-c", "printf 'bad-err' >&2; exit 3"], { encoding: "utf8" });
    } catch (err) {
        failedErr = err;
    }
    assert.ok(failedErr instanceof Error);
    assert.strictEquals(failedErr.status, 3);
    assert.strictEquals(failedErr.stderr, "bad-err");

    const timedOut = spawnSync("sh", ["-c", "sleep 2"], { timeout: 50 });
    assert.strictEquals(timedOut.timedOut, true);

    const overflow = spawnSync("sh", ["-c", "head -c 256 /dev/zero"], { maxBuffer: 64 });
    assert.strictEquals(overflow.status, 0);
    assert.strictEquals(overflow.stdout.length, 64);
});

test("node:buffer encodes, decodes, allocates, and concatenates", () => {
    assert.strictEquals(bufferDefault.Buffer, Buffer);
    const b = Buffer.from("hello world", "utf8");
    assert.ok(b instanceof Uint8Array);
    assert.strictEquals(Buffer.isBuffer(b), true);
    assert.strictEquals(Buffer.isBuffer(new Uint8Array(2)), false);
    assert.strictEquals(b.toString("hex"), "68656c6c6f20776f726c64");
    assert.strictEquals(b.toString("base64"), "aGVsbG8gd29ybGQ=");
    assert.strictEquals(
        Buffer.from(b.toString("base64url"), "base64url").toString("utf8"),
        "hello world",
    );
    assert.strictEquals(
        Buffer.from("68656c6c6f", "hex").toString("latin1"),
        "hello",
    );
    assert.strictEquals(Buffer.from([65, 66]).toString("ascii"), "AB");

    const filled = Buffer.alloc(4, 0x41);
    assert.strictEquals(filled.toString("utf8"), "AAAA");
    assert.strictEquals(Buffer.allocUnsafe(5).length, 5);

    const joined = Buffer.concat([Buffer.from("foo"), Buffer.from("bar")]);
    assert.strictEquals(joined.toString("utf8"), "foobar");
    assert.strictEquals(joined.equals(Buffer.from("foobar")), true);
    assert.strictEquals(Buffer.byteLength("€", "utf8"), 3);
});

test("node:url provides URL, URLSearchParams, fileURLToPath, and pathToFileURL", () => {
    assert.strictEquals(urlDefault.URL, URL);
    assert.strictEquals(urlDefault.URLSearchParams, URLSearchParams);

    const u = new URL("https://example.com:8443/dir/page?a=1&b=two#top");
    assert.strictEquals(u.origin, "https://example.com:8443");
    assert.strictEquals(u.protocol, "https:");
    assert.strictEquals(u.hostname, "example.com");
    assert.strictEquals(u.port, "8443");
    assert.strictEquals(u.pathname, "/dir/page");
    assert.strictEquals(u.search, "?a=1&b=two");
    assert.strictEquals(u.hash, "#top");
    assert.strictEquals(u.searchParams.get("b"), "two");

    u.searchParams.set("a", "99");
    u.searchParams.append("c", "3");
    assert.strictEquals(u.search, "?a=99&b=two&c=3");

    const sp = new URLSearchParams([
        ["k", "2"],
        ["a", "1"],
        ["k", "3"],
    ]);
    assert.equals(sp.getAll("k"), ["2", "3"]);
    sp.sort();
    assert.strictEquals(sp.toString(), "a=1&k=2&k=3");

    const roundtripPath = "/tmp/hello world/test.txt";
    const fileUrl = pathToFileURL(roundtripPath);
    assert.strictEquals(fileUrl.protocol, "file:");
    assert.strictEquals(fileURLToPath(fileUrl), roundtripPath);
    assert.strictEquals(fileURLToPath(fileUrl.href), roundtripPath);
});

test("node:crypto provides randomBytes, randomUUID, createHash, createHmac, pbkdf2Sync, and timingSafeEqual", () => {
    assert.strictEquals(typeof cryptoDefault.createHash, "function");

    const rnd = randomBytes(16);
    assert.strictEquals(rnd.length, 16);
    assert.strictEquals(Buffer.isBuffer(rnd), true);

    const uuid = randomUUID();
    assert.ok(
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid),
        `unexpected uuid format: ${uuid}`,
    );

    const sha = createHash("sha256").update("abc").digest("hex");
    assert.strictEquals(
        sha,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );

    const mac = createHmac("sha256", "key")
        .update("The quick brown fox jumps over the lazy dog")
        .digest("hex");
    assert.strictEquals(
        mac,
        "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8",
    );

    const dk = pbkdf2Sync("password", "salt", 1, 32, "sha256");
    assert.strictEquals(
        Buffer.from(dk).toString("hex"),
        "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b",
    );

    const eqA = Buffer.from("secret");
    const eqB = Buffer.from("secret");
    const neq = Buffer.from("other!");
    assert.strictEquals(timingSafeEqual(eqA, eqB), true);
    assert.strictEquals(timingSafeEqual(eqA, neq), false);
    assert.throws(() => timingSafeEqual(eqA, Buffer.from("short")), RangeError);

    // Tampered own `byteLength` on TypedArray/DataView must not spoof lengths.
    const fakeZeroA = new Uint8Array([1, 2, 3, 4]);
    const fakeZeroB = new Uint8Array([9, 8]);
    Object.defineProperty(fakeZeroA, "byteLength", { value: 0 });
    Object.defineProperty(fakeZeroB, "byteLength", { value: 0 });
    assert.throws(() => timingSafeEqual(fakeZeroA, fakeZeroB), RangeError);
    const fakeZeroSameLen = new Uint8Array([9, 8, 7, 6]);
    Object.defineProperty(fakeZeroSameLen, "byteLength", { value: 0 });
    assert.strictEquals(timingSafeEqual(fakeZeroA, fakeZeroSameLen), false);
    assert.equals(Array.from(Buffer.from(fakeZeroA)), [1, 2, 3, 4]);

    // Iterations exceeding uint32 must throw RangeError on both engines.
    assert.throws(() => pbkdf2Sync("pw", "salt", 0x1_0000_0000, 16, "sha256"), RangeError);
});

test("node:child_process rejects truthy shell options and bounds arg/env tampering", () => {
    for (const shellOpt of [true, "sh", 1]) {
        let err;
        try {
            spawnSync("sh", ["-c", "echo nope"], { shell: shellOpt });
        } catch (caught) {
            err = caught;
        }
        assert.ok(err instanceof Error);
        assert.strictEquals(err.code, "ERR_UNSUPPORTED");
    }

    const tamperedProtoDesc = Object.getOwnPropertyDescriptor(Object.prototype, "code");
    Object.defineProperty(Object.prototype, "code", {
        configurable: true,
        set() {
            throw new Error("Object.prototype.code setter must not be invoked");
        },
    });
    try {
        let childErr;
        try {
            execFileSync("sh", ["-c", "exit 7"]);
        } catch (caught) {
            childErr = caught;
        }
        assert.ok(childErr instanceof Error);
        assert.strictEquals(childErr.status, 7);
    } finally {
        if (tamperedProtoDesc) {
            Object.defineProperty(Object.prototype, "code", tamperedProtoDesc);
        } else {
            delete Object.prototype.code;
        }
    }
});

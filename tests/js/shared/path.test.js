import test from "vael:test";
import assert from "vael:assert";
import { platform } from "vael:os";
import * as path from "vael:path";
import {
    DELIMITER,
    SEPARATOR,
    SEPARATOR_PATTERN,
    basename,
    common,
    dirname,
    extname,
    format,
    isAbsolute,
    join,
    normalize,
    parse,
    posix,
    relative,
    resolve,
    toNamespacedPath,
    win32,
} from "vael:path";

function expectPathError(fn, expectedCode) {
    let caught;
    try {
        fn();
    } catch (error) {
        caught = error;
    }
    assert.ok(caught instanceof Error, `expected PathError(${expectedCode}), nothing thrown`);
    assert.strictEquals(caught.name, "PathError");
    assert.strictEquals(caught.code, expectedCode);
    return caught;
}

test("vael:path registry, namespaces, and host constants share frozen identity", async () => {
    const list = Vael.getBuiltInModulesList();
    assert.ok(list.includes("vael:path"), `vael:path missing from list: ${JSON.stringify(list)}`);
    assert.ok(!list.includes("path"), "bare path must not be listed");

    const registry = Vael.getBuiltInModule("vael:path");
    assert.ok(registry !== undefined && typeof registry === "object");
    assert.strictEquals(Vael.getBuiltInModule("vael:path"), registry);
    assert.strictEquals(Object.isFrozen(registry), true);
    assert.strictEquals(Object.isFrozen(path.posix), true);
    assert.strictEquals(Object.isFrozen(path.win32), true);
    assert.strictEquals(Object.isExtensible(path), false);
    assert.ok(!("default" in path), "vael:path must not have a default export");

    const dynamic = await import("vael:path");
    assert.strictEquals(dynamic, path);
    assert.ok(path !== registry, "module namespace must be distinct from registry");

    const expectedNames = [
        "DELIMITER",
        "SEPARATOR",
        "SEPARATOR_PATTERN",
        "basename",
        "common",
        "dirname",
        "extname",
        "format",
        "isAbsolute",
        "join",
        "normalize",
        "parse",
        "posix",
        "relative",
        "resolve",
        "toNamespacedPath",
        "win32",
    ];
    assert.equals(Object.keys(path).sort(), expectedNames);
    assert.equals(Object.keys(registry).sort(), expectedNames);

    for (const name of expectedNames) {
        assert.strictEquals(path[name], registry[name], `${name} differs from registry`);
    }

    const isWindows = platform === "windows";
    assert.strictEquals(SEPARATOR, isWindows ? "\\" : "/");
    assert.strictEquals(DELIMITER, isWindows ? ";" : ":");
    assert.ok(SEPARATOR_PATTERN instanceof RegExp);
    assert.strictEquals(SEPARATOR_PATTERN.test("a//b"), true);
    assert.strictEquals(posix.sep, "/");
    assert.strictEquals(posix.delimiter, ":");
    assert.strictEquals(win32.sep, "\\");
    assert.strictEquals(win32.delimiter, ";");

    assert.strictEquals(Vael.getBuiltInModule("path"), undefined);
    await assert.rejects(() => import("path"));

    for (const leaked of ["path", "posix", "win32", "SEPARATOR", "DELIMITER", "__vaelPath", "__vaelPathCwd"]) {
        assert.ok(!(leaked in globalThis), `${leaked} leaked onto globalThis`);
    }

    assert.throws(() => {
        path.join = () => "replaced";
    }, TypeError);

    join.marker = 42;
    assert.strictEquals(registry.join.marker, 42);
    delete join.marker;

    const first = parse("/a/b.txt");
    const second = parse("/a/b.txt");
    assert.ok(first !== second, "parse must return a fresh object");
    first.base = "changed";
    assert.strictEquals(second.base, "b.txt");
});

test("vael:path.posix lexical operations match contract", () => {
    assert.strictEquals(posix.normalize(""), ".");
    assert.strictEquals(posix.normalize("//"), "/");
    assert.strictEquals(posix.normalize("/a//b/./c/.."), "/a/b");
    assert.strictEquals(posix.normalize("a/../../b"), "../b");
    assert.strictEquals(posix.normalize("a/../"), "./");

    assert.strictEquals(posix.join(), ".");
    assert.strictEquals(posix.join("a", "", "b"), "a/b");
    assert.strictEquals(posix.join("a", "/b"), "a/b");
    assert.strictEquals(posix.join("/foo", "bar", "baz/asdf", "quux", ".."), "/foo/bar/baz/asdf");
    assert.strictEquals(posix.join("/", ""), "/");

    assert.strictEquals(posix.basename("/"), "");
    assert.strictEquals(posix.basename("///"), "");
    assert.strictEquals(posix.basename("/a/b.txt", ".txt"), "b");
    assert.strictEquals(posix.basename("a.txt", "a.txt"), "a.txt");
    assert.strictEquals(posix.basename("a/b.txt/", ".txt"), "b");

    assert.strictEquals(posix.dirname(""), ".");
    assert.strictEquals(posix.dirname("a//b"), "a");
    assert.strictEquals(posix.dirname("///foo"), "/");
    assert.strictEquals(posix.dirname("/a/b.txt"), "/a");

    assert.strictEquals(posix.extname(".bashrc"), "");
    assert.strictEquals(posix.extname("a.tar.gz"), ".gz");
    assert.strictEquals(posix.extname("a/b.txt/"), ".txt");
    assert.strictEquals(posix.extname("."), "");
    assert.strictEquals(posix.extname(".."), "");
    assert.strictEquals(posix.extname("a."), ".");

    assert.strictEquals(posix.isAbsolute(""), false);
    assert.strictEquals(posix.isAbsolute("/a"), true);
    assert.strictEquals(posix.isAbsolute("//a"), true);
    assert.strictEquals(posix.isAbsolute("a"), false);
    assert.strictEquals(posix.isAbsolute("\\a"), false);

    assert.equals(posix.parse("/a//b/"), {
        root: "/",
        dir: "/a",
        base: "b",
        ext: "",
        name: "b",
    });
    assert.equals(posix.parse("/"), { root: "/", dir: "/", base: "", ext: "", name: "" });
    assert.equals(posix.parse(""), { root: "", dir: "", base: "", ext: "", name: "" });

    assert.strictEquals(posix.format({ dir: "foo", base: "/" }), "foo//");
    assert.strictEquals(posix.format({ dir: "some/dir" }), "some/dir/");
    assert.strictEquals(posix.format({}), "");
    assert.strictEquals(posix.format({ root: "/", name: "index", ext: ".html" }), "/index.html");

    assert.strictEquals(posix.common(["/a/b", "/a/c"]), "/a/");
    assert.strictEquals(posix.common([]), "");
    assert.strictEquals(posix.common(["/a/b"]), "/a/b");
    assert.strictEquals(posix.common(["/a/b", "/x/y"]), "/");

    assert.strictEquals(posix.resolve("/a", "b"), "/a/b");
    assert.strictEquals(posix.resolve("/a", "/b"), "/b");
    assert.strictEquals(posix.resolve("/a", "", "b"), "/a/b");
    assert.strictEquals(posix.resolve("/a", "..", "b"), "/b");

    assert.strictEquals(posix.relative("/a/b", "/a/c"), "../c");
    assert.strictEquals(posix.relative("/a", "/a"), "");
    assert.strictEquals(posix.relative("/a", "/a/b"), "b");
    assert.strictEquals(posix.relative("/a/b", "/a"), "..");

    assert.strictEquals(posix.toNamespacedPath("/a"), "/a");
});

test("vael:path.win32 lexical operations match contract", () => {
    assert.strictEquals(win32.normalize("C:/foo/.."), "C:\\");
    assert.strictEquals(win32.normalize("C:"), "C:.");
    assert.strictEquals(win32.normalize("C:..\\foo"), "C:..\\foo");
    assert.strictEquals(win32.normalize("\\\\server\\share"), "\\\\server\\share\\");
    assert.strictEquals(win32.normalize("\\\\server\\share\\..\\foo"), "\\\\server\\share\\foo");
    assert.strictEquals(win32.normalize("\\\\\\foo"), "\\foo");
    assert.strictEquals(win32.normalize("1:\\foo"), "1:\\foo");

    assert.strictEquals(win32.join("C:", "foo"), "C:\\foo");
    assert.strictEquals(win32.join("//server", "share"), "\\\\server\\share\\");
    assert.strictEquals(win32.join("C:\\a", "..", "b"), "C:\\b");
    assert.strictEquals(win32.join(), ".");

    assert.strictEquals(win32.dirname("C:/"), "C:/");
    assert.strictEquals(win32.dirname("\\\\server\\share\\foo"), "\\\\server\\share\\");
    assert.strictEquals(win32.dirname("C:\\foo\\bar"), "C:\\foo");

    assert.strictEquals(win32.basename("C:\\foo\\"), "foo");
    assert.strictEquals(win32.basename("C:\\"), "");
    assert.strictEquals(win32.basename("C:"), "");
    assert.strictEquals(win32.basename("\\\\server\\share.txt\\"), "");
    assert.strictEquals(win32.basename("\\\\server\\share.txt\\dir"), "dir");

    assert.strictEquals(win32.extname("C:\\foo\\a.."), ".");
    assert.strictEquals(win32.extname("\\\\server\\share.txt"), "");
    assert.strictEquals(win32.extname("C:\\foo\\bar.txt\\"), ".txt");

    assert.strictEquals(win32.isAbsolute("C:foo"), false);
    assert.strictEquals(win32.isAbsolute("C:\\foo"), true);
    assert.strictEquals(win32.isAbsolute("\\foo"), true);
    assert.strictEquals(win32.isAbsolute("\\\\server\\share\\foo"), true);

    assert.equals(win32.parse("\\\\?\\C:\\foo"), {
        root: "\\\\?\\C:\\",
        dir: "\\\\?\\C:\\",
        base: "foo",
        ext: "",
        name: "foo",
    });
    assert.equals(win32.parse("C:"), { root: "C:", dir: "C:", base: "", ext: "", name: "" });

    assert.strictEquals(win32.format({ dir: "foo", base: "\\" }), "foo\\\\");
    assert.strictEquals(win32.format({ root: "C:\\", name: "index", ext: ".html" }), "C:\\index.html");
    assert.strictEquals(win32.format({}), "");

    assert.strictEquals(win32.common(["C:\\a\\b", "C:\\a\\c"]), "C:\\a\\");
    assert.strictEquals(win32.common(["C:/a/b", "C:/a/c"]), "");
    assert.strictEquals(win32.common([]), "");

    assert.strictEquals(win32.resolve("C:\\work", "out\\..\\log"), "C:\\work\\log");
    assert.strictEquals(win32.resolve("C:\\base", "\\rooted"), "C:\\rooted");
    assert.strictEquals(win32.resolve("C:\\base", "D:\\absolute"), "D:\\absolute");

    assert.strictEquals(win32.relative("C:\\base", "c:\\base\\file"), "file");
    assert.strictEquals(win32.relative("C:\\base", "D:\\x"), "D:\\x");
    assert.strictEquals(win32.relative("C:\\a", "C:\\a"), "");

    assert.strictEquals(win32.toNamespacedPath("C:\\foo"), "\\\\?\\C:\\foo");
    assert.strictEquals(win32.toNamespacedPath("\\\\server\\share\\foo"), "\\\\?\\UNC\\server\\share\\foo");
    assert.strictEquals(win32.toNamespacedPath(""), "");
    assert.strictEquals(win32.toNamespacedPath("\\\\?\\C:\\foo"), "\\\\?\\C:\\foo");
});

test("vael:path validates arguments, error codes, cwd dialects, and resists intrinsic tampering", () => {
    for (const value of [undefined, null, 1, true, {}, [], new String("a"), Symbol("a")]) {
        assert.throws(() => posix.normalize(value), TypeError);
        assert.throws(() => win32.normalize(value), TypeError);
    }
    assert.throws(() => posix.join("a", undefined), TypeError);
    assert.throws(() => posix.basename("/a", 1), TypeError);
    assert.throws(() => posix.format(null), TypeError);
    assert.throws(() => posix.format([]), TypeError);
    assert.throws(() => posix.format({ dir: 1 }), TypeError);
    assert.throws(() => posix.format({ root: null }), TypeError);
    assert.throws(() => posix.common("a"), TypeError);
    assert.throws(() => posix.common([1]), TypeError);
    assert.throws(() => posix.common(["a", undefined]), TypeError);

    const nulError = assert.throws(() => posix.normalize("sentinel\u0000value"), TypeError);
    assert.ok(!nulError.message.includes("sentinel"), "NUL error echoed input");
    assert.ok(nulError.message.includes("NUL"), "NUL error must mention NUL");

    assert.throws(() => {
        posix.format({
            get root() {
                throw new RangeError("getter sentinel");
            },
        });
    }, RangeError);

    const originalCode = Object.getOwnPropertyDescriptor(Object.prototype, "code");
    Object.defineProperty(Object.prototype, "code", {
        configurable: true,
        get() {
            throw new Error("inherited code trap");
        },
    });
    try {
        expectPathError(() => win32.resolve("\\\\?\\UNC\\server"), "PATH_WINDOWS_ROOT");
    } finally {
        if (originalCode) Object.defineProperty(Object.prototype, "code", originalCode);
        else delete Object.prototype.code;
    }

    const native = SEPARATOR === "\\" ? win32 : posix;
    const foreign = native === posix ? win32 : posix;
    assert.strictEquals(native.join("a", "b"), join("a", "b"));
    assert.strictEquals(native.normalize("a/./b"), normalize("a/./b"));
    assert.strictEquals(native.basename("/a/b"), basename("/a/b"));
    assert.strictEquals(native.dirname("/a/b"), dirname("/a/b"));
    assert.strictEquals(native.extname("a.txt"), extname("a.txt"));
    assert.strictEquals(native.isAbsolute("/a"), isAbsolute("/a"));
    assert.equals(native.parse("/a/b.txt"), parse("/a/b.txt"));
    assert.strictEquals(native.format({ dir: "a", base: "b" }), format({ dir: "a", base: "b" }));
    assert.strictEquals(native.common(["/a/b", "/a/c"]), common(["/a/b", "/a/c"]));
    assert.strictEquals(native.toNamespacedPath("/a"), toNamespacedPath("/a"));

    const resolved = resolve("child");
    assert.strictEquals(isAbsolute(resolved), true);
    assert.strictEquals(relative(resolve("."), resolved), "child");
    assert.strictEquals(relative(".", "."), "");
    expectPathError(() => foreign.resolve("child"), "PATH_CWD_DIALECT");

    const originalCharCodeAt = String.prototype.charCodeAt;
    const originalSplit = String.prototype.split;
    const originalToLowerCase = String.prototype.toLowerCase;
    const originalJoin = Array.prototype.join;
    const originalMapGet = Map.prototype.get;
    String.prototype.charCodeAt = () => {
        throw new Error("tampered charCodeAt");
    };
    String.prototype.split = () => {
        throw new Error("tampered split");
    };
    String.prototype.toLowerCase = () => {
        throw new Error("tampered toLowerCase");
    };
    Array.prototype.join = () => {
        throw new Error("tampered join");
    };
    Map.prototype.get = () => {
        throw new Error("tampered map get");
    };
    try {
        assert.strictEquals(posix.normalize("/a//b/../c"), "/a/c");
        assert.strictEquals(posix.common(["/a/b", "/a/c"]), "/a/");
        assert.strictEquals(win32.relative("C:\\Base", "c:\\base\\File"), "File");
        assert.strictEquals(Vael.getBuiltInModule("vael:path"), Vael.getBuiltInModule("vael:path"));
    } finally {
        String.prototype.charCodeAt = originalCharCodeAt;
        String.prototype.split = originalSplit;
        String.prototype.toLowerCase = originalToLowerCase;
        Array.prototype.join = originalJoin;
        Map.prototype.get = originalMapGet;
    }
});

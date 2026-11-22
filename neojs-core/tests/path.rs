//! Integration tests for the `path` built-in module, driven through `Runtime`.
//!
//! The JavaScript modules assert on their own and throw on failure, so a test
//! is one module plus the Rust call that runs it. The POSIX and Windows vectors
//! run on every host through the explicit namespaces. Native Windows execution
//! is not exercised here; the Windows cases are pure string logic.
//!
//! `path` is lexical string manipulation. These tests deliberately assert that
//! it does not claim to enforce containment, resolve symlinks, validate Windows
//! device names, or authorize a filesystem operation.

use std::fs;

use neojs_core::Runtime;

/// Runs a JavaScript module whose own assertions throw on failure.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

/// Prepends the shared JavaScript assertion helpers to one module.
fn with_helpers(source: &str) -> String {
    format!("{HELPERS}{source}")
}

/// Runs a JavaScript module that must fail, returning the error text.
fn run_js_error(source: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    let error = runtime.run_file(&file).expect_err("module must fail");
    error.to_string()
}

/// JavaScript assertion and error helpers shared by the modules below.
///
/// They use only the language, not the module under test.
const HELPERS: &str = r#"
function equal(actual, expected, label = "value") {
    if (actual !== expected) {
        throw new Error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
    }
}
function deepEqual(actual, expected, label = "value") {
    equal(JSON.stringify(actual), JSON.stringify(expected), label);
}
function expectTypeError(action, label = "operation") {
    let error;
    try {
        action();
    } catch (thrown) {
        error = thrown;
    }
    if (!(error instanceof TypeError)) {
        throw new Error(`${label}: expected TypeError, got ${error && error.name}: ${error && error.message}`);
    }
}
function expectCode(action, expected) {
    let error;
    try {
        action();
    } catch (thrown) {
        error = thrown;
    }
    if (!error) throw new Error(`expected ${expected}, nothing was thrown`);
    if (error.name !== "PathError") {
        throw new Error(`expected PathError, got ${error && error.name}`);
    }
    if (error.code !== expected) {
        throw new Error(`expected ${expected}, got ${error && error.code}: ${error && error.message}`);
    }
}
"#;

#[test]
fn imports_are_cached_frozen_and_identity_shared() {
    run_js(&with_helpers(
        r#"
import * as path from "path";
import * as alias from "neo:path";

const registry = Neo.getBuiltInModule("path");

if (path !== alias) throw new Error("path and neo:path namespaces differ");
if (alias !== (await import("path"))) throw new Error("dynamic path namespace differs");
if (alias !== (await import("neo:path"))) throw new Error("dynamic neo:path namespace differs");
if (registry !== Neo.getBuiltInModule("path")) throw new Error("registry lookup is not cached");
if (Object.isFrozen(registry) !== true) throw new Error("registry is not frozen");
if (Object.isFrozen(path.posix) !== true) throw new Error("posix is not frozen");
if (Object.isFrozen(path.win32) !== true) throw new Error("win32 is not frozen");
if ("default" in path) throw new Error("unexpected default export");

for (const name of [
    "posix", "win32", "join", "normalize", "basename", "dirname", "extname",
    "isAbsolute", "parse", "format", "common", "resolve", "relative",
    "toNamespacedPath", "SEPARATOR", "DELIMITER", "SEPARATOR_PATTERN",
]) {
    if (!(name in path)) throw new Error(`missing export ${name}`);
    if (path[name] !== registry[name]) throw new Error(`${name} is not the registry value`);
}
if (Object.keys(path).length !== 17) throw new Error("unexpected export count");

equal(typeof path.join, "function", "join type");
equal(path.SEPARATOR, path.SEPARATOR === "\\" ? path.win32.sep : path.posix.sep, "separator");
equal(path.DELIMITER, path.SEPARATOR === "\\" ? ";" : ":", "delimiter");
if (!(path.SEPARATOR_PATTERN instanceof RegExp)) throw new Error("separator pattern type");
equal(path.SEPARATOR_PATTERN.test("a//b"), true, "separator pattern");
equal(path.posix.sep, "/", "posix sep");
equal(path.posix.delimiter, ":", "posix delimiter");
equal(path.win32.sep, "\\", "win32 sep");
equal(path.win32.delimiter, ";", "win32 delimiter");
equal(import.meta.resolve("path"), "neo:path", "canonical path");
equal(import.meta.resolve("neo:path"), "neo:path", "canonical neo:path");

for (const name of ["path", "posix", "win32", "SEPARATOR", "DELIMITER", "__neoPath", "__neoPathCwd"]) {
    if (name in globalThis) throw new Error(`${name} leaked onto globalThis`);
}
if (path.readCwd !== undefined || path.__neoPath !== undefined) {
    throw new Error("private cwd binding leaked onto the module");
}

// Namespace bindings are immutable. Function objects are shared, not frozen.
let assignmentFailed = false;
try {
    path.join = () => "replaced";
} catch (error) {
    assignmentFailed = error instanceof TypeError;
}
if (!assignmentFailed) throw new Error("namespace assignment did not throw TypeError");

path.join.marker = 42;
if (registry.join.marker !== 42) throw new Error("function identity is not shared");
delete path.join.marker;

const first = path.parse("/a/b.txt");
const second = path.parse("/a/b.txt");
if (first === second) throw new Error("parse reused an object");
first.base = "changed";
equal(second.base, "b.txt", "parse result was shared");
"#,
    ));
}

#[test]
fn posix_vectors() {
    run_js(&with_helpers(
        r#"
import { posix } from "path";

equal(posix.normalize(""), ".", "normalize empty");
equal(posix.normalize("//"), "/", "normalize slashes");
equal(posix.normalize("/a//b/./c/.."), "/a/b", "normalize traversal");
equal(posix.normalize("a/../../b"), "../b", "normalize above root");
equal(posix.normalize("a/../"), "./", "normalize trailing");
equal(posix.normalize("a", "/"), "a", "normalize trailing slash");

equal(posix.join(), ".", "join empty");
equal(posix.join("a", "", "b"), "a/b", "join empty segment");
equal(posix.join("a", "/b"), "a/b", "join does not reset");
equal(posix.join("/foo", "bar", "baz/asdf", "quux", ".."), "/foo/bar/baz/asdf", "join main");
equal(posix.join("/", ""), "/", "join root");

equal(posix.basename("/"), "", "basename root");
equal(posix.basename("///"), "", "basename separators");
equal(posix.basename("/a/b.txt", ".txt"), "b", "basename suffix");
equal(posix.basename("a.txt", "a.txt"), "a.txt", "basename equal suffix");
equal(posix.basename("a/b.txt/", ".txt"), "b", "basename trailing");

equal(posix.dirname(""), ".", "dirname empty");
equal(posix.dirname("a//b"), "a", "dirname double");
equal(posix.dirname("///foo"), "/", "dirname root");
equal(posix.dirname("/a/b.txt"), "/a", "dirname file");

equal(posix.extname(".bashrc"), "", "extname dotfile");
equal(posix.extname("a.tar.gz"), ".gz", "extname last dot");
equal(posix.extname("a/b.txt/"), ".txt", "extname trailing");
equal(posix.extname("."), "", "extname dot");
equal(posix.extname(".."), "", "extname dotdot");
equal(posix.extname("a."), ".", "extname empty extension");

equal(posix.isAbsolute(""), false, "isAbsolute empty");
equal(posix.isAbsolute("/a"), true, "isAbsolute root");
equal(posix.isAbsolute("//a"), true, "isAbsolute double root");
equal(posix.isAbsolute("a"), false, "isAbsolute relative");
equal(posix.isAbsolute("\\a"), false, "isAbsolute backslash");

deepEqual(posix.parse("/a//b/"), { root: "/", dir: "/a", base: "b", ext: "", name: "b" }, "parse");
deepEqual(posix.parse("/"), { root: "/", dir: "/", base: "", ext: "", name: "" }, "parse root");
deepEqual(posix.parse(""), { root: "", dir: "", base: "", ext: "", name: "" }, "parse empty");

equal(posix.format({ dir: "foo", base: "/" }), "foo//", "format separator base");
equal(posix.format({ dir: "some/dir" }), "some/dir/", "format dir only");
equal(posix.format({}), "", "format empty");
equal(posix.format({ root: "/", name: "index", ext: ".html" }), "/index.html", "format root");

equal(posix.common(["/a/b", "/a/c"]), "/a/", "common prefix");
equal(posix.common([]), "", "common empty");
equal(posix.common(["/a/b"]), "/a/b", "common single");
equal(posix.common(["/a/b", "/x/y"]), "/", "common root");

equal(posix.resolve("/a", "b"), "/a/b", "resolve base");
equal(posix.resolve("/a", "/b"), "/b", "resolve resets");
equal(posix.resolve("/a", "", "b"), "/a/b", "resolve empty");
equal(posix.resolve("/a", "..", "b"), "/b", "resolve dotdot");

equal(posix.relative("/a/b", "/a/c"), "../c", "relative sibling");
equal(posix.relative("/a", "/a"), "", "relative same");
equal(posix.relative("/a", "/a/b"), "b", "relative child");
equal(posix.relative("/a/b", "/a"), "..", "relative parent");

equal(posix.toNamespacedPath("/a"), "/a", "toNamespacedPath identity");
"#,
    ));
}

#[test]
fn win32_vectors() {
    run_js(&with_helpers(
        r#"
import { win32 } from "path";

equal(win32.normalize("C:/foo/.."), "C:\\", "normalize drive");
equal(win32.normalize("C:"), "C:.", "normalize bare drive");
equal(win32.normalize("C:..\\foo"), "C:..\\foo", "normalize drive relative");
equal(win32.normalize("\\\\server\\share"), "\\\\server\\share\\", "normalize UNC");
equal(win32.normalize("\\\\server\\share\\..\\foo"), "\\\\server\\share\\foo", "normalize UNC climb");
equal(win32.normalize("\\\\\\foo"), "\\foo", "normalize triple separator");
equal(win32.normalize("1:\\foo"), "1:\\foo", "normalize digit drive");

equal(win32.join("C:", "foo"), "C:\\foo", "join drive");
equal(win32.join("//server", "share"), "\\\\server\\share\\", "join UNC");
equal(win32.join("C:\\a", "..", "b"), "C:\\b", "join traversal");
equal(win32.join(), ".", "join empty");

equal(win32.dirname("C:/"), "C:/", "dirname preserves root spelling");
equal(win32.dirname("\\\\server\\share\\foo"), "\\\\server\\share\\", "dirname UNC");
equal(win32.dirname("C:\\foo\\bar"), "C:\\foo", "dirname drive path");

equal(win32.basename("C:\\foo\\"), "foo", "basename trailing");
equal(win32.basename("C:\\"), "", "basename drive root");
equal(win32.basename("C:"), "", "basename bare drive");
equal(win32.basename("\\\\server\\share.txt\\"), "", "basename UNC root");
equal(win32.basename("\\\\server\\share.txt\\dir"), "dir", "basename UNC child");

equal(win32.extname("C:\\foo\\a.."), ".", "extname dots");
equal(win32.extname("\\\\server\\share.txt"), "", "extname UNC root");
equal(win32.extname("C:\\foo\\bar.txt\\"), ".txt", "extname trailing");

equal(win32.isAbsolute("C:foo"), false, "isAbsolute drive relative");
equal(win32.isAbsolute("C:\\foo"), true, "isAbsolute drive root");
equal(win32.isAbsolute("\\foo"), true, "isAbsolute rooted");
equal(win32.isAbsolute("\\\\server\\share\\foo"), true, "isAbsolute UNC");

deepEqual(win32.parse("\\\\?\\C:\\foo"), {
    root: "\\\\?\\C:\\", dir: "\\\\?\\C:\\", base: "foo", ext: "", name: "foo",
}, "parse namespaced drive");
deepEqual(win32.parse("C:"), { root: "C:", dir: "C:", base: "", ext: "", name: "" }, "parse bare drive");

equal(win32.format({ dir: "foo", base: "\\" }), "foo\\\\", "format separator base");
equal(win32.format({ root: "C:\\", name: "index", ext: ".html" }), "C:\\index.html", "format root");
equal(win32.format({}), "", "format empty");

equal(win32.common(["C:\\a\\b", "C:\\a\\c"]), "C:\\a\\", "common prefix");
equal(win32.common(["C:/a/b", "C:/a/c"]), "", "common does not split slash");
equal(win32.common([]), "", "common empty");

equal(win32.resolve("C:\\work", "out\\..\\log"), "C:\\work\\log", "resolve base");
equal(win32.resolve("C:\\base", "\\rooted"), "C:\\rooted", "resolve rooted inherits drive");
equal(win32.resolve("C:\\base", "D:\\absolute"), "D:\\absolute", "resolve absolute resets");

equal(win32.relative("C:\\base", "c:\\base\\file"), "file", "relative case-insensitive");
equal(win32.relative("C:\\base", "D:\\x"), "D:\\x", "relative other drive");
equal(win32.relative("C:\\a", "C:\\a"), "", "relative same");

equal(win32.toNamespacedPath("C:\\foo"), "\\\\?\\C:\\foo", "namespaced drive");
equal(win32.toNamespacedPath("\\\\server\\share\\foo"), "\\\\?\\UNC\\server\\share\\foo", "namespaced UNC");
equal(win32.toNamespacedPath(""), "", "namespaced empty");
equal(win32.toNamespacedPath("\\\\?\\C:\\foo"), "\\\\?\\C:\\foo", "namespaced already");
"#,
    ));
}

#[test]
fn argument_errors_are_type_errors_and_do_not_echo_input() {
    run_js(&with_helpers(
        r#"
import { posix, win32 } from "path";

for (const value of [undefined, null, 1, true, {}, [], new String("a"), Symbol("a")]) {
    expectTypeError(() => posix.normalize(value), "posix normalize");
    expectTypeError(() => win32.normalize(value), "win32 normalize");
}
expectTypeError(() => posix.join("a", undefined), "join argument");
expectTypeError(() => posix.basename("/a", 1), "basename suffix");
expectTypeError(() => posix.format(null), "format null");
expectTypeError(() => posix.format([]), "format array");
expectTypeError(() => posix.format({ dir: 1 }), "format field");
expectTypeError(() => posix.format({ root: null }), "format null field");
expectTypeError(() => posix.common("a"), "common not array");
expectTypeError(() => posix.common([1]), "common non-string");
expectTypeError(() => posix.common(["a", undefined]), "common undefined");

try {
    posix.normalize("sentinel\u0000value");
    throw new Error("NUL did not throw");
} catch (error) {
    if (!(error instanceof TypeError)) throw new Error("NUL is not a TypeError");
    if (error.message.includes("sentinel")) throw new Error("NUL error echoed input");
    if (!error.message.includes("NUL")) throw new Error("NUL error does not name NUL");
}
for (const action of [
    () => win32.join("a", "b\u0000c"),
    () => posix.basename("/a", "x\u0000"),
    () => win32.format({ base: "x\u0000" }),
    () => posix.common(["a\u0000"]),
]) {
    try {
        action();
        throw new Error("NUL did not throw");
    } catch (error) {
        if (!(error instanceof TypeError)) throw new Error("NUL is not a TypeError");
        if (error.message.includes("\u0000")) throw new Error("NUL error echoed input");
    }
}

// A format property getter may throw its own error; it must not be masked.
let getterError;
try {
    posix.format({ get root() { throw new RangeError("getter sentinel"); } });
} catch (error) {
    getterError = error;
}
if (!(getterError instanceof RangeError) || getterError.message !== "getter sentinel") {
    throw new Error("format swallowed a getter error");
}
expectTypeError(() => posix.format(new Proxy({}, {
    get(target, key) { return key === "base" ? 1 : ""; },
})), "format proxy field");
"#,
    ));
}

#[test]
fn errors_have_own_codes_and_tampered_intrinsics_do_not_change_results() {
    run_js(&with_helpers(
        r#"
import { posix, win32 } from "path";

// A path error must not lose its code through an inherited accessor.
const originalCode = Object.getOwnPropertyDescriptor(Object.prototype, "code");
Object.defineProperty(Object.prototype, "code", {
    configurable: true,
    get() { throw new Error("inherited code trap"); },
});
try {
    expectCode(() => win32.resolve("\\\\?\\UNC\\server"), "PATH_WINDOWS_ROOT");
} finally {
    if (originalCode) Object.defineProperty(Object.prototype, "code", originalCode);
    else delete Object.prototype.code;
}

// The shim captured String/Array/Map intrinsics before user code. Replacing
// them afterwards must not change path operations or the registry.
const originalCharCodeAt = String.prototype.charCodeAt;
const originalSplit = String.prototype.split;
const originalToLowerCase = String.prototype.toLowerCase;
const originalJoin = Array.prototype.join;
const originalMapGet = Map.prototype.get;
String.prototype.charCodeAt = () => { throw new Error("tampered charCodeAt"); };
String.prototype.split = () => { throw new Error("tampered split"); };
String.prototype.toLowerCase = () => { throw new Error("tampered toLowerCase"); };
Array.prototype.join = () => { throw new Error("tampered join"); };
Map.prototype.get = () => { throw new Error("tampered map get"); };
try {
    equal(posix.normalize("/a//b/../c"), "/a/c", "normalize after tamper");
    equal(posix.common(["/a/b", "/a/c"]), "/a/", "common after tamper");
    equal(win32.relative("C:\\Base", "c:\\base\\File"), "File", "relative after tamper");
    equal(Neo.getBuiltInModule("path"), Neo.getBuiltInModule("path"), "registry after tamper");
} finally {
    String.prototype.charCodeAt = originalCharCodeAt;
    String.prototype.split = originalSplit;
    String.prototype.toLowerCase = originalToLowerCase;
    Array.prototype.join = originalJoin;
    Map.prototype.get = originalMapGet;
}

// Lexical results are not authorization. `..` is resolved as text, not against
// the filesystem, so this result can point outside the apparent parent.
equal(posix.normalize("a/../../b"), "../b", "lexical traversal");
equal(win32.normalize("C:\\a\\..\\..\\b"), "C:\\b", "rooted traversal");
if (posix.normalize("a/../../b") === "b") throw new Error("containment was assumed");
"#,
    ));
}

#[test]
fn cwd_operations_use_the_native_dialect_only_when_needed() {
    run_js(&with_helpers(
        r#"
import { posix, win32, SEPARATOR } from "path";

const native = SEPARATOR === "\\" ? win32 : posix;
const foreign = native === posix ? win32 : posix;
const absoluteBase = SEPARATOR === "\\" ? "C:\\base" : "/base";
const foreignBase = foreign === win32 ? "C:\\base" : "/base";

// Explicit absolute bases work in either dialect without cwd.
equal(native.resolve(absoluteBase, "child"), native.join(absoluteBase, "child"), "absolute native");
equal(foreign.resolve(foreignBase, "x"), foreign.join(foreignBase, "x"), "absolute foreign");
equal(win32.resolve("C:\\base", "child"), "C:\\base\\child", "explicit Windows base");
equal(posix.resolve("/base", "child"), "/base/child", "explicit POSIX base");

// A native relative base needs cwd and must produce an absolute result.
const resolved = native.resolve("child");
if (!native.isAbsolute(resolved)) throw new Error("resolve did not produce an absolute path");
equal(native.relative(native.resolve("."), resolved), "child", "relative to cwd");
equal(native.relative(".", "."), "", "relative cwd identity");

// The non-native dialect cannot use cwd, even when the path is rooted.
expectCode(() => foreign.resolve("child"), "PATH_CWD_DIALECT");
if (SEPARATOR !== "\\") {
    expectCode(() => win32.resolve("C:child"), "PATH_CWD_DIALECT");
    expectCode(() => win32.toNamespacedPath("child"), "PATH_CWD_DIALECT");
    equal(win32.toNamespacedPath("C:\\foo"), "\\\\?\\C:\\foo", "absolute namespaced");
} else {
    expectCode(() => posix.resolve("child"), "PATH_CWD_DIALECT");
}
expectCode(() => win32.resolve("\\\\?\\UNC\\server"), "PATH_WINDOWS_ROOT");
"#,
    ));
}

#[test]
fn unknown_neo_path_names_are_rejected() {
    let bare = run_js_error("import { nope } from \"path\";\n");
    assert!(bare.contains("path"), "{bare}");
    assert!(bare.contains("nope"), "{bare}");

    let alias = run_js_error("import { nope } from \"neo:path\";\n");
    assert!(alias.contains("neo:path"), "{alias}");
    assert!(alias.contains("nope"), "{alias}");
}

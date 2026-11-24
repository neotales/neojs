//! Platform-independent `node:path` tests through the QuickJS runtime.
//!
//! This file deliberately does not require Unix. It uses absolute paths for
//! every host-dependent vector, and it exercises `posix` and `win32` explicitly
//! so the same test can run on a Windows host.

#![cfg(feature = "node-compat")]

use std::fs;
use std::path::PathBuf;

use neojs_lite_core::ChildProcessPolicy;
use neojs_lite_core::NodeOptions;
use neojs_lite_core::Runtime;

fn policy() -> NodeOptions {
    NodeOptions::new(ChildProcessPolicy::new())
}

fn write_module(dir: &tempfile::TempDir, name: &str, source: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, source).expect("write module");
    path
}

fn run(source: &str) -> Result<(), neojs_lite_core::Error> {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", source);
    let mut runtime = Runtime::with_node_compat(policy()).expect("runtime");
    runtime.run_file(&file)
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
fn a_bare_path_name_is_not_an_alias() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = write_module(&dir, "main.js", "import \"path\";\n");
    let mut runtime = Runtime::with_node_compat(policy()).expect("runtime");
    let error = runtime.run_file(&file).expect_err("bare path must fail");
    let text = format!("{error}");
    assert!(text.contains("node:` prefix"), "{text}");
    assert!(!text.contains("neo:path"), "{text}");
}

#[test]
fn node_path_is_not_in_the_neo_registry() {
    run(r#"
if (Neo.getBuiltInModulesList().includes("node:path")) {
    throw new Error("node:path leaked into Neo.getBuiltInModulesList");
}
if (Neo.getBuiltInModule("node:path") !== undefined) {
    throw new Error("node:path leaked into Neo.getBuiltInModule");
}
"#)
    .expect("node:path registry isolation");
}

#[test]
fn node_path_vectors_and_identity() {
    run(r#"
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

if (path.sep === "/") {
    eq(path.delimiter, ":", "host delimiter");
    eq(path.posix, path, "host is posix");
} else if (path.sep === "\\") {
    eq(path.delimiter, ";", "host delimiter");
    eq(path.win32, path, "host is win32");
} else {
    throw new Error(`unexpected host separator: ${JSON.stringify(path.sep)}`);
}
eq(path.sep, sep, "named sep identity");
eq(path.delimiter, delimiter, "named delimiter identity");
eq(path.join, join, "default join identity");
eq(path.posix.join, posix.join, "posix join identity");
eq(path.win32.join, win32.join, "win32 join identity");
eq(posix.posix, posix, "posix self");
eq(win32.win32, win32, "win32 self");
eq(posix.win32, win32, "posix.win32");
eq(win32.posix, posix, "win32.posix");
eq(_makeLong, toNamespacedPath, "_makeLong identity");
const dynamicPath = await import("node:path");
eq(dynamicPath.default, path, "dynamic default identity");
eq(dynamicPath.posix, posix, "dynamic posix identity");
eq(dynamicPath.win32, win32, "dynamic win32 identity");

eq(posix.normalize("/foo//bar/../baz/"), "/foo/baz/", "posix normalize");
eq(posix.join("a", "b", "..", "c"), "a/c", "posix join");
eq(posix.dirname("/foo/bar/"), "/foo", "posix dirname");
eq(posix.basename("/foo/bar.txt", ".txt"), "bar", "posix basename");
eq(posix.extname("/foo/bar.txt"), ".txt", "posix extname");
eq(posix.isAbsolute("/foo"), true, "posix isAbsolute");
eq(posix.resolve("/a", "b"), "/a/b", "posix resolve");
eq(posix.relative("/a/b", "/a/c"), "../c", "posix relative");
eq(posix.relative("/a", "/b"), "../b", "posix relative root child");
eq(posix.relative("/a/b", "/x/y"), "../../x/y", "posix relative no common");
const parsed = posix.parse("/foo/bar.txt");
eq(parsed.root, "/", "posix parse.root");
eq(parsed.dir, "/foo", "posix parse.dir");
eq(parsed.base, "bar.txt", "posix parse.base");
eq(parsed.ext, ".txt", "posix parse.ext");
eq(parsed.name, "bar", "posix parse.name");
eq(posix.format({ dir: "/foo", name: "bar", ext: ".txt" }), "/foo/bar.txt", "posix format");
eq(posix.format({ root: "/", dir: "/", base: "a" }), "/a", "posix format dir root");
eq(posix.format({ name: "foo", ext: "txt" }), "foo.txt", "posix format ext dot");
eq(posix.toNamespacedPath("/foo"), "/foo", "posix namespaced");

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
eq(win32.relative("C:\\a", "C:\\a\\BAR"), "BAR", "win32 relative spelling");
eq(win32.relative("C:\\Ä", "C:\\ä\\B"), "B", "win32 relative unicode case");
eq(win32.relative("C:\\a", "C:\\a\\Ä"), "Ä", "win32 relative unicode child");
eq(win32.relative("C:\\Ä\\x", "C:\\ä\\y"), "..\\y", "win32 relative unicode expansion");
eq(win32.relative("C:\\İ", "C:\\İ\\x"), "x", "win32 relative unicode expansion child");
eq(win32.toNamespacedPath("C:\\foo"), "\\\\?\\C:\\foo", "win32 namespaced");
eq(win32.normalize("é:\\x"), ".\\é:\\x", "win32 unicode colon normalize");
eq(win32.normalize("é:\\foo\\..\\bar"), ".\\é:\\bar", "win32 unicode colon traversal");
const wparsed = win32.parse("C:\\foo\\bar.txt");
eq(wparsed.root, "C:\\", "win32 parse.root");
eq(wparsed.dir, "C:\\foo", "win32 parse.dir");
eq(wparsed.base, "bar.txt", "win32 parse.base");
eq(wparsed.ext, ".txt", "win32 parse.ext");
eq(wparsed.name, "bar", "win32 parse.name");
eq(win32.format({ root: "C:\\", name: "foo", ext: ".txt" }), "C:\\foo.txt", "win32 format");
eq(win32.format({ root: "C:\\", dir: "C:\\", base: "a" }), "C:\\a", "win32 format dir root");
eq(win32.format({ name: "foo", ext: "txt" }), "foo.txt", "win32 format ext dot");

let unsupported = null;
try {
    matchesGlob("a", "*");
} catch (error) {
    unsupported = error && error.code;
}
eq(unsupported, "ERR_NODE_UNSUPPORTED", "matchesGlob unsupported");

let typeError = null;
try {
    posix.normalize(1);
} catch (error) {
    typeError = error && error.code;
}
eq(typeError, "ERR_INVALID_ARG_TYPE", "type error");
let formatError = null;
try {
    posix.format({ dir: 1 });
} catch (error) {
    formatError = error && error.code;
}
eq(formatError, "ERR_INVALID_ARG_TYPE", "format type error");

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
"#)
    .expect("node:path vectors");
}

#[test]
fn node_path_limits_are_path_specific() {
    run(r#"
import { posix } from "node:path";
const oneMiB = "a".repeat(1_048_576);
const tooLarge = "a".repeat(1_048_577);

if (posix.normalize(oneMiB).length !== 1_048_576) {
    throw new Error("the exact single-path bound was rejected");
}

let code = null;
try {
    posix.normalize(tooLarge);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_PATH_TOO_LARGE") throw new Error(`single limit: ${code}`);

code = null;
try {
    posix.join(oneMiB, oneMiB);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_PATH_TOO_LARGE") throw new Error(`join aggregate: ${code}`);

code = null;
try {
    posix.resolve(oneMiB, oneMiB);
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_PATH_TOO_LARGE") throw new Error(`resolve aggregate: ${code}`);

code = null;
try {
    posix.join(...Array.from({ length: 257 }, () => "a"));
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_PATH_TOO_LARGE") throw new Error(`argument count: ${code}`);
"#)
    .expect("node:path limits");
}

#[test]
fn node_path_rejects_lone_surrogates() {
    run(r#"
import { posix, win32 } from "node:path";
for (const fn of [posix.normalize, win32.normalize, posix.join, posix.resolve]) {
    let code = null;
    try {
        fn("\uD800");
    } catch (error) {
        code = error && error.code;
    }
    if (code !== "ERR_INVALID_ARG_VALUE") {
        throw new Error(`lone surrogate: ${code}`);
    }
}
"#)
    .expect("lone surrogate rejection");
}

struct ResetCwdFailure;

impl Drop for ResetCwdFailure {
    fn drop(&mut self) {
        neojs_node_builtins::set_host_cwd_failure_for_testing(false);
    }
}

#[test]
fn node_path_cwd_failure_is_coded_and_only_acquired_when_needed() {
    run(r#"
import { win32 } from "node:path";
const bare = win32.resolve("C:");
if (!bare.startsWith("C:\\")) throw new Error(`bare drive: ${bare}`);
if (bare !== win32.resolve("C:.")) throw new Error("bare drive changed");
"#)
    .expect("bare drive resolve");

    neojs_node_builtins::set_host_cwd_failure_for_testing(true);
    let _reset = ResetCwdFailure;
    run(
        r#"
import { posix, win32 } from "node:path";
if (posix.resolve("/a", "b") !== "/a/b") throw new Error("absolute resolve needed cwd");
if (posix.relative("/a", "/b") !== "../b") throw new Error("absolute relative needed cwd");
if (posix.toNamespacedPath("/a") !== "/a") throw new Error("posix namespaced needed cwd");
if (win32.resolve("C:\\a", "b") !== "C:\\a\\b") throw new Error("win32 absolute needed cwd");
if (win32.relative("C:\\a", "C:\\b") !== "..\\b") throw new Error("win32 relative needed cwd");
if (win32.toNamespacedPath("C:\\a") !== "\\\\?\\C:\\a") throw new Error("win32 namespaced needed cwd");

let code = null;
try {
    posix.resolve("relative");
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_PATH_CWD") throw new Error(`cwd failure: ${code}`);

code = null;
try {
    win32.resolve("C:foo");
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_PATH_CWD") throw new Error(`drive-relative resolve: ${code}`);

code = null;
try {
    win32.toNamespacedPath("C:foo");
} catch (error) {
    code = error && error.code;
}
if (code !== "ERR_NODE_PATH_CWD") throw new Error(`drive-relative namespaced: ${code}`);
"#,
    )
    .expect("cwd failure");
}

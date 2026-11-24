//! Integration tests for the `fs` built-in module through the V8 `Runtime`.
//!
//! Core filesystem behaviour is covered in `neojs-fs-core` and shared
//! cross-engine JavaScript behaviour is covered in `tests/js/shared/fs.test.js`.
//! These tests cover V8-specific module identity (`NEO:fs` case-insensitive
//! scheme, `import.meta.resolve`, leak checks) and Unix symlink / permission
//! scenarios using `tempfile`.

use std::fs;

use neojs_core::Runtime;

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

#[test]
fn imports_are_cached_frozen_and_identity_shared() {
    run_js(
        r#"
import * as fsMod from "neo:fs";
import * as alias from "NEO:fs";

const registry = Neo.getBuiltInModule("neo:fs");

if (fsMod !== alias) throw new Error("neo:fs and NEO:fs namespaces differ");
if (alias !== (await import("neo:fs"))) throw new Error("dynamic neo:fs namespace differs");
if (registry !== Neo.getBuiltInModule("neo:fs")) throw new Error("registry lookup is not cached");
if (Object.isFrozen(registry) !== true) throw new Error("registry is not frozen");
if (Object.isExtensible(fsMod) !== false) throw new Error("namespace is extensible");
if ("default" in fsMod) throw new Error("unexpected default export");

const expected = [
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
const exported = Object.keys(fsMod).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(expected)) {
    throw new Error(`unexpected exports: ${JSON.stringify(exported)}`);
}
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error("module exports and registry properties differ");
}
for (const name of exported) {
    if (fsMod[name] !== registry[name]) throw new Error(`${name} is not the registry value`);
    if (typeof fsMod[name] !== "function") throw new Error(`${name} is not a function`);
}

if (import.meta.resolve("neo:fs") !== "neo:fs") throw new Error("canonical fs specifier");
if (import.meta.resolve("NEO:fs") !== "neo:fs") throw new Error("scheme is not case-insensitive");

if (Neo.getBuiltInModule("fs") !== undefined) {
    throw new Error("registry accepted bare fs");
}
let bareRejected = false;
try {
    await import("fs");
} catch (error) {
    bareRejected = error instanceof TypeError;
}
if (!bareRejected) throw new Error("bare fs import was accepted");

for (const name of ["fs", "__neoFs"]) {
    if (name in globalThis) throw new Error(`${name} leaked onto globalThis`);
}
"#,
    );
}

#[cfg(unix)]
#[test]
fn stat_lstat_realpath_and_remove_handle_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("temp dir");
    let target_dir = dir.path().join("target_dir");
    fs::create_dir(&target_dir).expect("create target dir");
    let target_file = target_dir.join("hello.txt");
    fs::write(&target_file, b"hello").expect("write target file");

    let link_dir = dir.path().join("link_dir");
    symlink(&target_dir, &link_dir).expect("symlink dir");

    let link_file = dir.path().join("link_file");
    symlink(&target_file, &link_file).expect("symlink file");

    let dangling = dir.path().join("dangling");
    symlink(dir.path().join("missing"), &dangling).expect("dangling symlink");

    let script = format!(
        r#"
import {{ exists, lstat, realpath, remove, stat }} from "neo:fs";

const targetFile = {target_file:?};
const linkFile = {link_file:?};
const targetDir = {target_dir:?};
const linkDir = {link_dir:?};
const dangling = {dangling:?};

const sFile = stat(linkFile);
if (!sFile.isFile || sFile.isSymlink || sFile.size !== 5) {{
    throw new Error(`unexpected stat(linkFile): ${{JSON.stringify(sFile)}}`);
}}
const lFile = lstat(linkFile);
if (!lFile.isSymlink || lFile.isFile) {{
    throw new Error(`unexpected lstat(linkFile): ${{JSON.stringify(lFile)}}`);
}}

const lDirSlash = lstat(linkDir + "/");
if (!lDirSlash.isSymlink || lDirSlash.isDirectory) {{
    throw new Error(`unexpected lstat(linkDir + "/"): ${{JSON.stringify(lDirSlash)}}`);
}}

if (exists(dangling) !== false) throw new Error("exists(dangling) must be false");
const lDangling = lstat(dangling);
if (!lDangling.isSymlink) throw new Error("lstat(dangling) must report isSymlink");

if (realpath(linkFile) !== realpath(targetFile)) {{
    throw new Error("realpath(linkFile) did not resolve to targetFile");
}}

// Removing symlink with trailing slash must unlink only the symlink, not targetDir contents.
remove(linkDir + "/", {{ recursive: true }});
if (!exists(targetFile)) throw new Error("removing linkDir/ deleted targetFile!");
if (exists(linkDir)) throw new Error("linkDir still exists after remove");
"#
    );

    let main = dir.path().join("test.js");
    fs::write(&main, script).expect("write script");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&main).expect("symlink test");
}

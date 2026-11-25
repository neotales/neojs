//! V8 integration tests for `node:*` compatibility modules, async
//! `node:fs/promises` and `node:child_process.execFile`, `RuntimeOptions`,
//! `process.exit` / `process.exitCode`, and compile-time `node-compat` gating.

use std::fs;

use neojs_core::{Runtime, RuntimeOptions};

fn run_with_options(
    source: &str,
    options: RuntimeOptions,
) -> Result<Option<i32>, neojs_core::Error> {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::with_options(options).expect("runtime");
    runtime.run_file(&file)?;
    Ok(runtime.exit_code())
}

#[test]
#[cfg(feature = "node-compat")]
fn v8_node_fs_promises_supports_full_async_surface() {
    run_with_options(
        r#"
import fs from "node:fs";
import fsp, {
    readFile,
    writeFile,
    appendFile,
    stat,
    lstat,
    readdir,
    mkdir,
    rm,
    unlink,
    rename,
    copyFile,
    realpath,
    mkdtemp,
    constants,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { Buffer } from "node:buffer";
import { pathToFileURL } from "node:url";

if (fs.promises.readFile !== readFile || fsp.readFile !== readFile) {
    throw new Error("node:fs.promises must match node:fs/promises");
}
if (typeof constants.F_OK !== "number" || typeof constants.COPYFILE_EXCL !== "number") {
    throw new Error("fs/promises constants missing");
}

const root = await mkdtemp(path.join(os.tmpdir(), "neojs-fsp-"));
try {
    const sub = path.join(root, "deep", "dir");
    await mkdir(sub, { recursive: true });
    const file = path.join(sub, "async.txt");

    await writeFile(file, "hello", "utf8");
    await appendFile(file, "-async", "utf8");

    if (await readFile(file, "utf8") !== "hello-async") {
        throw new Error("readFile utf8 mismatch");
    }
    if (await readFile(pathToFileURL(file), { encoding: "utf-8" }) !== "hello-async") {
        throw new Error("readFile URL mismatch");
    }
    const raw = await readFile(file);
    if (!Buffer.isBuffer(raw) || raw.toString("utf8") !== "hello-async") {
        throw new Error("readFile buffer mismatch");
    }

    const st = await stat(file);
    if (!st.isFile() || st.isDirectory() || st.size !== 11) {
        throw new Error("stat mismatch");
    }
    if (!(await lstat(file)).isFile()) {
        throw new Error("lstat mismatch");
    }
    const resolved = await realpath(file);
    if (!resolved.endsWith("async.txt")) {
        throw new Error(`unexpected realpath: ${resolved}`);
    }

    const names = await readdir(sub);
    if (names.length !== 1 || names[0] !== "async.txt") {
        throw new Error(`unexpected readdir names: ${JSON.stringify(names)}`);
    }
    const dirents = await readdir(sub, { withFileTypes: true });
    if (dirents.length !== 1 || dirents[0].name !== "async.txt" || !dirents[0].isFile()) {
        throw new Error("readdir withFileTypes mismatch");
    }

    const copied = path.join(sub, "copy.txt");
    await copyFile(file, copied);
    let exclFailed = false;
    try {
        await copyFile(file, copied, constants.COPYFILE_EXCL);
    } catch (err) {
        exclFailed = err instanceof Error && err.code === "EEXIST";
    }
    if (!exclFailed) {
        throw new Error("COPYFILE_EXCL must reject with EEXIST");
    }

    const moved = path.join(sub, "moved.txt");
    await rename(copied, moved);
    await unlink(moved);

    let enoent = false;
    try {
        await readFile(moved, "utf8");
    } catch (err) {
        enoent = err instanceof Error && err.code === "ENOENT";
    }
    if (!enoent) {
        throw new Error("missing file must throw ENOENT");
    }
} finally {
    await rm(root, { recursive: true, force: true });
}
"#,
        RuntimeOptions::new(),
    )
    .expect("node:fs/promises works on V8");
}

#[test]
#[cfg(feature = "node-compat")]
fn v8_node_child_process_exec_file_supports_promise_and_callback() {
    run_with_options(
        r#"
import { execFile } from "node:child_process";

const ok = await execFile("sh", ["-c", "printf 'async-out'; printf 'async-err' >&2"], {
    encoding: "utf8",
});
if (ok.stdout !== "async-out" || ok.stderr !== "async-err") {
    throw new Error(`unexpected execFile result: ${JSON.stringify(ok)}`);
}

let rejected = null;
try {
    await execFile("sh", ["-c", "printf 'fail-err' >&2; exit 6"], { encoding: "utf8" });
} catch (err) {
    rejected = err;
}
if (!(rejected instanceof Error) || rejected.status !== 6 || rejected.stderr !== "fail-err") {
    throw new Error(`unexpected execFile rejection: ${rejected}`);
}

const cbResult = await new Promise((resolve, reject) => {
    execFile("sh", ["-c", "printf 'cb-ok'"], { encoding: "utf8" }, (err, stdout, stderr) => {
        if (err) {
            reject(err);
        } else {
            resolve({ stdout, stderr });
        }
    });
});
if (cbResult.stdout !== "cb-ok" || cbResult.stderr !== "") {
    throw new Error(`unexpected callback result: ${JSON.stringify(cbResult)}`);
}
"#,
        RuntimeOptions::new(),
    )
    .expect("node:child_process execFile works on V8");
}

#[test]
#[cfg(feature = "node-compat")]
fn v8_runtime_options_no_node_compat_refuses_all_node_specifiers() {
    for spec in [
        "node:path",
        "node:path/posix",
        "node:path/win32",
        "node:os",
        "node:process",
        "node:fs",
        "node:fs/promises",
        "node:child_process",
        "node:buffer",
        "node:url",
        "node:crypto",
    ] {
        let err = run_with_options(
            &format!("import \"{spec}\";\n"),
            RuntimeOptions::new().with_node_compat(false),
        )
        .expect_err("node:* must be rejected when RuntimeOptions::with_node_compat(false)");
        assert!(
            err.to_string().contains("Node.js built-in"),
            "{spec}: {err}"
        );
    }

    // `neo:*` modules stay unaffected when `node_compat` is disabled.
    run_with_options(
        r#"
if (Neo.engine !== "neo") throw new Error("unexpected engine");
const list = Neo.getBuiltInModulesList();
if (list.some((s) => s.startsWith("node:"))) {
    throw new Error("node: leaked into Neo.getBuiltInModulesList()");
}
"#,
        RuntimeOptions::new().with_node_compat(false),
    )
    .expect("runtime still boots with node_compat disabled");
}

#[test]
#[cfg(feature = "node-compat")]
fn v8_process_exit_and_exit_code_work_cleanly() {
    let dir = tempfile::tempdir().expect("temp dir");
    let marker = dir.path().join("marker.txt");
    let marker_lit = serde_json::to_string(marker.to_str().unwrap()).unwrap();

    let src = format!(
        r#"
import process from "node:process";
import fs from "node:fs";
process.exit(17);
fs.writeFileSync({marker_lit}, "should-not-run");
"#
    );
    let exit_code = run_with_options(&src, RuntimeOptions::new())
        .expect("process.exit must terminate script cleanly without Runtime error");
    assert_eq!(exit_code, Some(17));
    assert!(
        !marker.exists(),
        "code after process.exit() must not execute"
    );

    let code_exit = run_with_options(
        r#"
import process from "node:process";
process.exitCode = 23;
"#,
        RuntimeOptions::new(),
    )
    .expect("process.exitCode script finishes cleanly");
    assert_eq!(code_exit, Some(23));
}

#[test]
#[cfg(feature = "node-compat")]
fn v8_node_compat_accepts_resizable_array_buffer_views() {
    run_with_options(
        r#"
import fs from "node:fs";
import fsp from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawnSync, execFile } from "node:child_process";
import { Buffer } from "node:buffer";
import { timingSafeEqual } from "node:crypto";

const rab = new ArrayBuffer(4, { maxByteLength: 16 });
const view = new Uint8Array(rab);
view.set([65, 66, 67, 68]);
rab.resize(6);
view.set([69, 70], 4);

const root = fs.mkdtempSync(path.join(os.tmpdir(), "neojs-rab-"));
try {
    const syncFile = path.join(root, "sync.bin");
    fs.writeFileSync(syncFile, view);
    if (fs.readFileSync(syncFile, "utf8") !== "ABCDEF") {
        throw new Error("fs.writeFileSync RAB mismatch");
    }

    const asyncFile = path.join(root, "async.bin");
    await fsp.writeFile(asyncFile, view);
    await fsp.appendFile(asyncFile, view.subarray(0, 2));
    if (await fsp.readFile(asyncFile, "utf8") !== "ABCDEFAB") {
        throw new Error("fs.promises write/append RAB mismatch");
    }

    const syncSpawn = spawnSync("sh", ["-c", "cat"], { input: view, encoding: "utf8" });
    if (syncSpawn.status !== 0 || syncSpawn.stdout !== "ABCDEF") {
        throw new Error(`spawnSync RAB mismatch: ${JSON.stringify(syncSpawn)}`);
    }

    const asyncSpawn = await execFile("sh", ["-c", "cat"], { input: view, encoding: "utf8" });
    if (asyncSpawn.stdout !== "ABCDEF") {
        throw new Error(`execFile RAB mismatch: ${JSON.stringify(asyncSpawn)}`);
    }

    if (Buffer.from(view).toString("utf8") !== "ABCDEF") {
        throw new Error("Buffer.from RAB mismatch");
    }
    if (!timingSafeEqual(view, Buffer.from("ABCDEF"))) {
        throw new Error("timingSafeEqual RAB mismatch");
    }
} finally {
    fs.rmSync(root, { recursive: true, force: true });
}
"#,
        RuntimeOptions::new(),
    )
    .expect(
        "RAB-backed Uint8Array works across fs, fs/promises, child_process, buffer, and crypto",
    );
}

#[test]
#[cfg(not(feature = "node-compat"))]
fn v8_node_compat_is_absent_when_feature_is_disabled() {
    for spec in ["node:path", "node:fs", "node:fs/promises", "node:process"] {
        let err = run_with_options(&format!("import \"{spec}\";\n"), RuntimeOptions::new())
            .expect_err("node:* must fail when node-compat feature is disabled");
        assert!(
            err.to_string().contains("Node.js built-in"),
            "{spec}: {err}"
        );
    }
    run_with_options(
        r#"
if (Neo.getBuiltInModule("node:path") !== undefined) {
    throw new Error("Neo.getBuiltInModule(node:path) must stay undefined");
}
"#,
        RuntimeOptions::new(),
    )
    .expect("runtime boots without node-compat");
}

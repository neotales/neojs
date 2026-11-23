//! End-to-end tests for the `neojs` command.

use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::io::Write;
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::sync::atomic::AtomicBool;
#[cfg(unix)]
use std::sync::atomic::AtomicUsize;
#[cfg(unix)]
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use neojs_core::RemoteOptions;
use neojs_core::Runtime;
use neojs_core::test_support::TestRemoteResponse;
use tempfile::TempDir;

struct Project {
    dir: TempDir,
}

impl Project {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn write(&self, relative: &str, source: &str) -> PathBuf {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(&path, source).expect("write source");
        path
    }
}

fn run_file(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_neojs"))
        .arg(path)
        .output()
        .expect("spawn neojs")
}

/// Runs the command and fails the test if it is still alive at `timeout`.
///
/// A regression in the V8/Tokio lifecycle aborts the process, which `output`
/// would report as a signal status. A hang would block the test suite, so the
/// child is killed at the deadline instead.
fn run_file_with_timeout(path: &Path, timeout: Duration) -> Output {
    let child = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .arg(path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn neojs");
    run_child(child, timeout)
}

/// Runs `neojs` with an isolated `SSH_AUTH_SOCK`, for agent tests.
#[cfg(unix)]
fn run_file_with_agent(path: &Path, socket: &Path, timeout: Duration) -> Output {
    let child = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .arg(path)
        .env("SSH_AUTH_SOCK", socket)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn neojs");
    run_child(child, timeout)
}

fn run_child(mut child: std::process::Child, timeout: Duration) -> Output {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("wait for neojs") {
            Some(status) => {
                let mut output = Output {
                    status,
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                };
                child
                    .stdout
                    .take()
                    .expect("stdout")
                    .read_to_end(&mut output.stdout)
                    .expect("read stdout");
                child
                    .stderr
                    .take()
                    .expect("stderr")
                    .read_to_end(&mut output.stderr)
                    .expect("read stderr");
                return output;
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("neojs did not finish within {timeout:?}");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn run_args(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_neojs"))
        .args(args)
        .output()
        .expect("spawn neojs")
}

/// Runs the command and fails the test if it is still alive at `timeout`.
///
/// Used by the test-runner lifecycle cases: a leaked refed timer must not keep
/// the test command alive after the runner has returned its verdict.
fn run_args_with_timeout(args: &[&str], timeout: Duration) -> Output {
    let child = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn neojs");
    run_child(child, timeout)
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// True only when the complete output line matches, so `ok ...` cannot be
/// satisfied by a `not ok ...` line.
fn contains_line(output: &str, expected: &str) -> bool {
    output.lines().any(|line| line == expected)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn runs_a_module_and_prints_to_stdout() {
    let project = Project::new();
    let file = project.write("hello.js", "console.log(\"hello from neojs\");\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "hello from neojs\n");
    assert_eq!(stderr(&output), "");
}

/// The stdio writers must reach the real process descriptors, preserve binary
/// bytes, and stay writable through `console` after their wrapper is closed.
#[test]
fn stdio_built_in_writes_binary_streams_to_the_process() {
    let project = Project::new();
    let file = project.write(
        "main.js",
        r#"
import { stdout, stderr } from "stdio";

if (Neo.getBuiltInModule("stdio").stdout !== stdout) {
    throw new Error("the registry and the import disagree");
}
const payload = Uint8Array.from([0x00, 0xff, 0x80, 0x0a]);
if (stdout.writeSync(payload) !== 4) throw new Error("binary stdout count");
if (stdout.writeSync("out-é\n") !== 7) throw new Error("text stdout count");
if (stderr.writeSync(Uint8Array.from([0x65, 0x00, 0xff, 0x0a])) !== 4) {
    throw new Error("binary stderr count");
}

stdout.close();
stderr.close();
console.log("after-close");
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let mut expected = vec![0x00, 0xff, 0x80, 0x0a];
    expected.extend_from_slice("out-é\nafter-close\n".as_bytes());
    assert_eq!(output.stdout, expected);
    assert_eq!(output.stderr, vec![0x65, 0x00, 0xff, 0x0a]);
}

/// The path module must be resolvable from the CLI through both spellings and
/// must keep its lexical results independent of the host dialect.
#[test]
fn path_built_in_is_available_from_the_cli() {
    let project = Project::new();
    let file = project.write(
        "main.js",
        r#"
import * as path from "path";
import * as alias from "neo:path";

if (path !== alias) throw new Error("path and neo:path namespaces differ");
if (Neo.getBuiltInModule("path").join !== path.join) {
    throw new Error("registry and import disagree");
}
console.log(path.posix.join("/a", "b"));
console.log(path.win32.normalize("C:/foo/../bar"));
console.log(path.posix.normalize("a/../../b"));
console.log(path.posix.sep + path.win32.sep);
console.log(path.relative(path.resolve("."), path.resolve("child")));
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "/a/b\nC:\\bar\n../b\n/\\\nchild\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn resolves_relative_imports_from_each_module() {
    let project = Project::new();
    project.write(
        "main.js",
        "import { area } from \"./lib/math.js\";\nconsole.log(area(3));\n",
    );
    project.write(
        "lib/math.js",
        "import { square } from \"./square.js\";\nexport function area(n) { return square(n); }\n",
    );
    project.write(
        "lib/square.js",
        "export function square(n) { return n * n; }\n",
    );

    let output = run_file(&project.path("main.js"));

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "9\n");
}

#[test]
fn resolves_a_file_url_import() {
    let project = Project::new();
    let target = project.write("target.js", "export const value = 8;\n");
    let url = url::Url::from_file_path(&target).expect("file URL");
    let file = project.write(
        "main.js",
        &format!("import {{ value }} from \"{url}\";\nconsole.log(value);\n"),
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "8\n");
}

#[test]
fn waits_for_top_level_await() {
    let project = Project::new();
    let file = project.write(
        "await.js",
        "const value = await Promise.resolve(42);\nconsole.log(value);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "42\n");
}

#[test]
fn resolves_dynamic_imports() {
    let project = Project::new();
    project.write("greet.js", "export function greet() { return \"hi\"; }\n");
    let file = project.write(
        "main.js",
        "const module = await import(\"./greet.js\");\nconsole.log(module.greet());\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "hi\n");
}

#[test]
fn console_errors_write_to_stderr() {
    let project = Project::new();
    let file = project.write(
        "console.js",
        "console.log(\"out\");\nconsole.warn(\"warn\");\nconsole.error(\"err\");\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "out\n");
    assert_eq!(stderr(&output), "warn\nerr\n");
}

#[test]
fn formats_console_arguments() {
    let project = Project::new();
    let file = project.write(
        "format.js",
        "console.log(\"number\", 1, true, null, undefined);\nconsole.log(new Uint8Array([1, 2, 3]));\nconsole.log({ a: 1 });\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        "number 1 true null undefined\nUint8Array(3) [1, 2, 3]\n{\n  \"a\": 1\n}\n"
    );
}

#[test]
fn console_module_writes_to_the_same_streams() {
    let project = Project::new();
    let file = project.write(
        "console-module.js",
        r#"
import defaultConsole, { log, debug, info, warn, error } from "neo:console";
log("log");
info("info");
debug("debug");
warn("warn");
error("error");
defaultConsole.log("default-out");
defaultConsole.error("default-err");
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "log\ninfo\ndebug\ndefault-out\n");
    assert_eq!(stderr(&output), "warn\nerror\ndefault-err\n");
}

#[test]
fn console_module_shares_identity_with_the_global_console() {
    let project = Project::new();
    let file = project.write(
        "console-identity.js",
        r#"
import defaultConsole from "console";
import * as namespace from "neo:console";
const registry = Neo.getBuiltInModule("console");
const dynamic = await import("console");
const uppercase = await import("NEO:console");
console.log(registry === globalThis.console);
console.log(defaultConsole === registry);
console.log(namespace.default === registry);
console.log(dynamic === namespace);
console.log(uppercase === namespace);
console.log(namespace.log === globalThis.console.log);
console.log(Neo.getBuiltInModule("neo:console") === undefined);
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        "true\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\n"
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn console_module_keeps_the_live_console_and_captures_named_exports() {
    let project = Project::new();
    let file = project.write(
        "console-mutation.js",
        r#"
import defaultConsole, { log } from "console";
const original = globalThis.console.log;
const replacement = () => {};
defaultConsole.log = replacement;
try {
    if (globalThis.console.log !== replacement) throw new Error("global did not change");
    if (defaultConsole.log !== replacement) throw new Error("default export did not change");
    if (log !== original) throw new Error("named export followed the replacement");
} finally {
    defaultConsole.log = original;
}
if (log !== globalThis.console.log) throw new Error("restore failed");
console.log("console mutation ok");
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "console mutation ok\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn console_module_ignores_a_global_replacement_before_dynamic_import() {
    let project = Project::new();
    let file = project.write(
        "console-wholesale.js",
        r#"
const installed = Neo.getBuiltInModule("console");
const print = installed.log;
const installedLog = installed.log;
globalThis.console = { log() {}, error() {} };

const dynamic = await import("console");
print(Neo.getBuiltInModule("console") === installed);
print(dynamic.default === installed);
print(dynamic.log === installedLog);
print(dynamic.default === globalThis.console);
print(dynamic.log === globalThis.console.log);

globalThis.console = installed;
print(dynamic.log === globalThis.console.log);
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "true\ntrue\ntrue\nfalse\nfalse\ntrue\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn console_module_captures_a_method_replacement_before_dynamic_import() {
    let project = Project::new();
    let file = project.write(
        "console-pre-evaluation.js",
        r#"
const installed = Neo.getBuiltInModule("console");
const print = installed.log;
const original = installed.log;
const replacement = () => {};

installed.log = replacement;
const dynamic = await import("neo:console");
print(dynamic.log === replacement);
print(dynamic.default === installed);
print(dynamic.default.log === replacement);

installed.log = original;
print(dynamic.log === original);
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "true\ntrue\ntrue\nfalse\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn console_module_runs_from_typescript() {
    let project = Project::new();
    let file = project.write(
        "console-module.ts",
        r#"
import defaultConsole, { log, warn } from "console";

const name: string = "typescript";
log(`console ${name}`);
defaultConsole.warn(`warned by ${name}`);
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "console typescript\n");
    assert_eq!(stderr(&output), "warned by typescript\n");
}

#[test]
fn crypto_builtin_roundtrips_and_rejects_a_wrong_key() {
    let project = Project::new();
    let file = project.write(
        "crypto.js",
        r#"
import { decrypt, encrypt, generateEnvelopeKey, randomBytes } from "crypto";
const key = generateEnvelopeKey();
const plaintext = Uint8Array.from([0, 1, 2, 255, 128]);
const envelope = encrypt(key, plaintext);
const out = decrypt(key, envelope);
let hex = "";
for (const byte of out) hex += byte.toString(16).padStart(2, "0");
console.log(hex, randomBytes(4).length, typeof Neo.getBuiltInModule("crypto").generateKey);
try {
    randomBytes(1.5);
} catch (error) {
    console.log(error.constructor.name);
}
let callbackCalled = false;
try {
    randomBytes(4, () => { callbackCalled = true; });
} catch (error) {
    console.log(error.constructor.name, callbackCalled);
}
try {
    decrypt(new Uint8Array(32), envelope);
} catch (error) {
    console.log(error.constructor.name, error.code);
}
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        "000102ff80 4 undefined\nRangeError\nTypeError false\nError NEO_CRYPTO_AUTH\n"
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn crypto_shamir_work_from_the_cli() {
    let project = Project::new();
    let file = project.write(
        "shamir.js",
        r#"
import {
    shamirCombine,
    shamirCombineAsync,
    shamirSplit,
    shamirSplitAsync,
} from "crypto";

function hex(bytes) {
    let text = "";
    for (const byte of bytes) text += byte.toString(16).padStart(2, "0");
    return text;
}

const secret = Uint8Array.from([0x00, 0x01, 0x80, 0xff, 0x42]);
const shares = shamirSplit(secret, 3, 5);
console.log(shares.length, shares[4].index, hex(shamirCombine([shares[0], shares[2], shares[4]])));

const asyncShares = await shamirSplitAsync(secret, 2, 3);
const recovered = await shamirCombineAsync([asyncShares[0], asyncShares[1]], 2);
console.log(recovered.length, hex(recovered));

try {
    shamirCombine([shares[0], shares[0]]);
} catch (error) {
    console.log(error.constructor.name, error.code);
}
try {
    shamirSplit(secret, 1, 3);
} catch (error) {
    console.log(error.constructor.name);
}
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        "5 5 000180ff42\n5 000180ff42\nError NEO_CRYPTO_SHAMIR_DUPLICATE\nRangeError\n"
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn crypto_stateful_cbc_and_gcm_work_from_the_cli() {
    let project = Project::new();
    let file = project.write(
        "cipher.js",
        r#"
import { createDecryptor, createEncryptor } from "crypto";

function fromHex(text) {
    const bytes = new Uint8Array(text.length / 2);
    for (let i = 0; i < bytes.length; i++) {
        bytes[i] = parseInt(text.slice(i * 2, i * 2 + 2), 16);
    }
    return bytes;
}
function hex(bytes) {
    let text = "";
    for (const byte of bytes) text += byte.toString(16).padStart(2, "0");
    return text;
}

const cbcKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const cbcIv = fromHex("101112131415161718191a1b1c1d1e1f");
const cbcPlaintext = fromHex("6e656f6a73206369706865722063626320696e7465726f70");
const cbc = createEncryptor("aes-256-cbc", cbcKey, cbcIv);
if (cbc.update(cbcPlaintext).length !== 0) throw Error("update returned bytes");
console.log(hex(cbc.final()));

const gcmKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const gcmIv = fromHex("a1b2c3d4e5f60718293a4b5c");
const gcmAad = fromHex("6e656f6a732d6369706865722d636f6e74657874");
const gcmPlaintext = fromHex("6e656f6a73206369706865722067636d20696e7465726f70");
const gcm = createEncryptor("aes-256-gcm", gcmKey, gcmIv);
gcm.setAAD(gcmAad);
gcm.update(gcmPlaintext);
const ciphertext = gcm.final();
const tag = gcm.getAuthTag();
console.log(hex(ciphertext), hex(tag));

const decryptor = createDecryptor("aes-256-gcm", gcmKey, gcmIv);
decryptor.setAAD(gcmAad);
decryptor.setAuthTag(tag);
decryptor.update(ciphertext);
console.log(hex(decryptor.final()));

try {
    const bad = createDecryptor("aes-256-gcm", gcmKey, gcmIv);
    bad.setAAD(gcmAad);
    const wrong = tag.slice();
    wrong[0] ^= 1;
    bad.setAuthTag(wrong);
    bad.update(ciphertext);
    bad.final();
} catch (error) {
    console.log(error.code);
}
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        concat!(
            "5226084e220371598b38cb20eaa1b85c2ff1b252517261b6f0054232236daa85\n",
            "6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1 9ea6507bfbc87455b1f24edc000cc662\n",
            "6e656f6a73206369706865722067636d20696e7465726f70\n",
            "NEO_CRYPTO_AUTH\n",
        )
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn crypto_hash_hmac_and_async_hash_work_from_the_cli() {
    let project = Project::new();
    let file = project.write(
        "hash.js",
        r#"
import { createHash, createHmac, hashAsync, hmacAsync } from "crypto";

console.log(createHash("sha256").update("abc").digest("hex"));
console.log(createHash("sha384").update("abc").digest("hex"));
console.log(createHmac("sha256", "Jefe").update("what do ya want for nothing?").digest("hex"));
console.log(await hashAsync("sha256", "abc", "hex"));
console.log(await hmacAsync("sha512", "Jefe", "what do ya want for nothing?", "hex"));
try {
    createHash("sha1");
} catch (error) {
    console.log(error.code);
}
"#,
    );

    let output = run_file_with_timeout(&file, Duration::from_secs(30));

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        concat!(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n",
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7\n",
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843\n",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\n",
            "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737\n",
            "NEO_CRYPTO_ALGORITHM\n",
        )
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn crypto_pbkdf2_and_bcrypt_work_from_the_cli() {
    let project = Project::new();
    let file = project.write(
        "password.js",
        r#"
import {
    bcryptHash,
    bcryptVerify,
    bcryptVerifyAsync,
    pbkdf2,
    pbkdf2Async,
} from "crypto";

function hex(bytes) {
    let text = "";
    for (const byte of bytes) text += byte.toString(16).padStart(2, "0");
    return text;
}

console.log(hex(pbkdf2("sha256", "passwd", "salt", 1, 16)));
console.log(hex(await pbkdf2Async("sha256", "passwd", "salt", 1, 16)));

const hash = bcryptHash("pässwörd", 4);
console.log(hash.length, hash.startsWith("$2b$04$"));
console.log(bcryptVerify("pässwörd", hash), bcryptVerify("wrong", hash));

const fixed = "$2y$04$10NU4r76Q4nBoXH7Sw4RKu9LjsXUslmRavZFzvlPUCYI3Hkl4xX/W";
console.log(bcryptVerify("password", fixed));
console.log(await bcryptVerifyAsync("password", fixed));
"#,
    );

    let output = run_file_with_timeout(&file, Duration::from_secs(30));

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        concat!(
            "55ac046e56e3089fec1691c22544b605\n",
            "55ac046e56e3089fec1691c22544b605\n",
            "60 true\n",
            "true false\n",
            "true\n",
            "true\n",
        )
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn age_builtin_roundtrips_and_rejects_a_wrong_file() {
    let project = Project::new();
    let file = project.write(
        "age.js",
        r#"
import { decrypt, encrypt, generateIdentity, identityToRecipient } from "age";
const pair = generateIdentity();
const plaintext = Uint8Array.from([0, 1, 2, 255, 128]);
const ciphertext = encrypt(pair.recipient, plaintext);
const out = decrypt(pair.identity, ciphertext);
let hex = "";
for (const byte of out) hex += byte.toString(16).padStart(2, "0");
console.log(
    pair.identity.startsWith("AGE-SECRET-KEY-1"),
    pair.recipient.startsWith("age1"),
    identityToRecipient(pair.identity) === pair.recipient,
    hex,
    ciphertext[0] === 0x61,
    typeof Neo.getBuiltInModule("age").generateKey,
);
let callbackCalled = false;
try {
    encrypt(pair.recipient, plaintext, () => { callbackCalled = true; });
} catch (error) {
    console.log(error.constructor.name, callbackCalled);
}
try {
    decrypt(pair.identity, new Uint8Array(0));
} catch (error) {
    console.log(error.constructor.name, error.code);
}
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        "true true true 000102ff80 true undefined\nTypeError false\nError NEO_AGE_FORMAT\n"
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn a_local_module_named_age_is_not_shadowed() {
    let project = Project::new();
    project.write("age.js", "export const value = 7;\n");
    let file = project.write(
        "main.js",
        "import { value } from \"./age.js\";\nimport { generateIdentity } from \"age\";\nconsole.log(value, generateIdentity().identity.startsWith(\"AGE-SECRET-KEY-1\"));\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "7 true\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn a_local_module_named_crypto_is_not_shadowed() {
    let project = Project::new();
    project.write("crypto.js", "export const value = 7;\n");
    let file = project.write(
        "main.js",
        "import { value } from \"./crypto.js\";\nimport { generateEnvelopeKey } from \"crypto\";\nconsole.log(value, generateEnvelopeKey().length);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "7 32\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn a_local_module_named_console_is_not_shadowed() {
    let project = Project::new();
    project.write("console.js", "export const value = 7;\n");
    let file = project.write(
        "main.js",
        "import { value } from \"./console.js\";\nimport * as builtin from \"console\";\nconsole.log(value, builtin.default === globalThis.console);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "7 true\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn reports_an_unknown_console_export_and_exits_1() {
    let project = Project::new();
    let file = project.write(
        "bad-console-export.js",
        "import { nope } from \"neo:console\";\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(
        message.contains("does not provide an export named 'nope'"),
        "{message}"
    );
    assert!(message.contains("neo:console"), "{message}");
}

#[test]
fn exposes_the_cached_chars_builtin_to_a_module() {
    let project = Project::new();
    let file = project.write(
        "chars.js",
        r#"
const first = Neo.getBuiltInModule("chars");
const second = Neo.getBuiltInModule("chars");
console.log(first === second);
console.log(Neo.getBuiltInModule("missing") === undefined);
console.log(first.isLetter("λ".codePointAt(0)));
console.log(first.isDigit("٣".codePointAt(0)));
console.log(first.isSpace("\u3000".codePointAt(0)));
console.log(first.toUpper("a".codePointAt(0)));
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "true\ntrue\ntrue\ntrue\ntrue\n65\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn exposes_the_cached_strings_builtin_to_a_module() {
    let project = Project::new();
    let file = project.write(
        "strings.js",
        r#"
const strings = Neo.getBuiltInModule("strings");
console.log(strings === Neo.getBuiltInModule("strings"));
console.log(strings.equalFold("K", "\u212A"));
console.log(strings.indexOfFold("\u{1F600}a", "A"));
console.log(strings.lastIndexOfFold("aaa", "aa"));
console.log(strings.trim("...hi...", "."));
console.log(strings.endsWithFold("a\n", "a"));
console.log(strings.trim("\uFEFF a \uFEFF"));
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "true\ntrue\n2\n1\nhi\nfalse\na\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn exposes_the_cached_os_builtin_to_a_module() {
    let project = Project::new();
    let file = project.write(
        "os.js",
        r#"
import * as os from "os";
import { arch, platform } from "neo:os";
const registry = Neo.getBuiltInModule("os");
console.log(registry === Neo.getBuiltInModule("os"));
console.log(os.platform === registry.platform && os.arch === registry.arch);
console.log(platform === os.platform && arch === os.arch);
console.log(`${os.platform}/${os.arch}`);
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        stdout(&output),
        format!(
            "true\ntrue\ntrue\n{}/{}\n",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    );
    assert_eq!(stderr(&output), "");
}

#[test]
fn slices_built_in_is_available_from_the_cli() {
    let project = Project::new();
    let file = project.write(
        "slices.js",
        r#"
import * as slices from "slices";
import * as alias from "neo:slices";

if (slices !== alias) throw new Error("slices and neo:slices namespaces differ");
if (Neo.getBuiltInModule("slices").slice !== slices.slice) throw new Error("registry differs");

const base = [1, 2, 3];
const view = slices.slice(base, 1, 3);
view.set(0, 20);
const nested = view.slice(1, 2);
nested.set(0, 30);

const bytes = new Uint8Array([1, 2, 3, 4]);
const bytesView = slices.slice(bytes, 1, 3);
bytesView.set(0, 9);

const text = slices.CharSlice.fromString("Hi");
text.toUpper();

console.log(base.join(","));
console.log(view.at(0), nested.at(0));
console.log(bytes.join(","));
console.log(text.toString());
console.log(typeof slices.readonlySlice(base).set);
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "1,20,30\n20 30\n1,9,3,4\nHI\nundefined\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn assert_builtin_passes_from_the_cli() {
    let project = Project::new();
    let file = project.write(
        "assert-pass.js",
        r#"
import assert, { equals, strictEquals } from "assert";
import { equals as aliased } from "neo:assert";

if (Neo.getBuiltInModule("assert") !== assert) throw new Error("registry identity");
if (aliased !== equals) throw new Error("specifier alias identity");
if (assert.ok !== assert) throw new Error("ok identity");

const shared = { value: 1 };
strictEquals(shared, shared);
strictEquals(NaN, NaN);
equals({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] });
equals(new Map([["a", 1], [{ b: 2 }, 3]]), new Map([[{ b: 2 }, 3], ["a", 1]]));
equals(new Set([1, { a: 2 }]), new Set([{ a: 2 }, 1]));
equals(new Uint8Array([1, 2]), new Uint8Array([1, 2]));
equals(new DataView(new Uint8Array([9, 1, 2]).buffer, 1, 2), new DataView(new Uint8Array([1, 2]).buffer));
assert.throws(() => {
    throw new TypeError("expected");
}, TypeError);
await assert.rejects(async () => {
    throw new RangeError("expected");
}, RangeError);

let subclassThenCalls = 0;
class TestPromise extends Promise {
    then() {
        subclassThenCalls += 1;
        throw new Error("overridden then was called");
    }
}
const subclassValue = await assert.doesNotReject(() =>
    new TestPromise((resolve) => resolve(7)));
if (subclassValue !== 7 || subclassThenCalls !== 0) {
    throw new Error("the CLI did not observe a genuine Promise subclass safely");
}

const hadOwnThen = Object.prototype.hasOwnProperty.call(Object.prototype, "then");
const savedThen = Object.prototype.then;
Object.prototype.then = function () {
    throw new Error("polluted then");
};
try {
    let rejectedPrimitive = false;
    try {
        await assert.rejects(() => 42);
    } catch (error) {
        rejectedPrimitive = error instanceof assert.AssertionError;
    }
    if (!rejectedPrimitive) throw new Error("polluted then caused a false pass");
} finally {
    if (hadOwnThen) Object.prototype.then = savedThen;
    else delete Object.prototype.then;
}
console.log("assert ok");
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "assert ok\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn assert_failure_exits_1_with_an_informative_assertion_error() {
    let project = Project::new();
    let file = project.write(
        "assert-fail.js",
        r#"
import { equals } from "assert";

equals({ a: { b: [0, 1] } }, { a: { b: [0, 2] } });
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("AssertionError"), "{message}");
    assert!(message.contains("values are not equal"), "{message}");
    assert!(message.contains("$.a.b[1]"), "{message}");
    assert!(message.contains("actual:"), "{message}");
    assert!(message.contains("expected:"), "{message}");
}

#[test]
fn assert_has_no_assert_equals_export_from_the_cli() {
    let project = Project::new();
    let file = project.write(
        "assert-no-alias.js",
        "import { assertEquals } from \"assert\";\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(
        message.contains("does not provide an export named 'assertEquals'"),
        "{message}"
    );
    assert!(message.contains("assert"), "{message}");
}

#[test]
fn reports_bad_built_in_module_names_and_exits_1() {
    let project = Project::new();
    let file = project.write("bad-name.js", "Neo.getBuiltInModule(42);\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("TypeError"), "{message}");
    assert!(
        message.contains("module name must be a string"),
        "{message}"
    );
}

#[test]
fn imports_a_built_in_module_by_bare_name() {
    let project = Project::new();
    let file = project.write(
        "chars-import.js",
        r#"
import { isLetter, isDigit, toUpper } from "chars";
console.log(isLetter("λ".codePointAt(0)));
console.log(isDigit("٣".codePointAt(0)));
console.log(toUpper("a".codePointAt(0)));
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "true\ntrue\n65\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn imports_a_built_in_module_by_neo_specifier() {
    let project = Project::new();
    let file = project.write(
        "chars-neo-import.js",
        r#"
import { isLetter } from "neo:chars";
const registry = Neo.getBuiltInModule("chars");
const dynamic = await import("chars");
console.log(isLetter === registry.isLetter);
console.log(dynamic.isLetter === isLetter);
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "true\ntrue\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn reports_an_unknown_built_in_export_and_exits_1() {
    let project = Project::new();
    let file = project.write("bad-export.js", "import { nope } from \"chars\";\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(
        message.contains("does not provide an export named 'nope'"),
        "{message}"
    );
    assert!(message.contains("chars"), "{message}");
}

#[test]
fn reports_an_unknown_neo_module_and_lists_the_implemented_ones() {
    let project = Project::new();
    let file = project.write("bad-neo.js", "import \"neo:nope\";\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("neo:nope"), "{message}");
    assert!(message.contains("not implemented"), "{message}");
    assert!(
        message.contains(
            "neojs implements age, neo:age, assert, neo:assert, chars, neo:chars, console, neo:console, crypto, neo:crypto, os, neo:os, path, neo:path, slices, neo:slices, ssh, neo:ssh, stdio, neo:stdio, strings, neo:strings"
        ),
        "{message}"
    );
}

#[test]
fn a_local_module_named_chars_is_not_shadowed() {
    let project = Project::new();
    project.write("chars.js", "export const value = 7;\n");
    let file = project.write(
        "main.js",
        "import { value } from \"./chars.js\";\nconsole.log(value);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "7\n");
}

#[test]
fn a_local_module_named_path_is_not_shadowed() {
    let project = Project::new();
    project.write("path.js", "export const value = 7;\n");
    let file = project.write(
        "main.js",
        "import { value } from \"./path.js\";\nimport { posix } from \"path\";\nconsole.log(value, posix.join(\"/a\", \"b\"));\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "7 /a/b\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn reports_an_uncaught_exception_and_exits_1() {
    let project = Project::new();
    let file = project.write(
        "throw.js",
        "function fail() {\n    throw new Error(\"boom\");\n}\nfail();\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let message = stderr(&output);
    assert!(message.contains("boom"), "{message}");
    assert!(message.contains("throw.js:2"), "{message}");
}

#[test]
fn reports_a_syntax_error_and_exits_1() {
    let project = Project::new();
    let file = project.write("broken.js", "const = 1;\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("SyntaxError"), "{message}");
    assert!(message.contains("broken.js:1"), "{message}");
}

#[test]
fn reports_a_missing_import_and_exits_1() {
    let project = Project::new();
    let file = project.write("main.js", "import \"./missing.js\";\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("cannot load"), "{message}");
    assert!(message.contains("missing.js"), "{message}");
}

#[test]
fn reports_a_missing_entry_file_and_exits_2() {
    let project = Project::new();
    let missing = project.path("missing.js");

    let output = run_file(&missing);

    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(message.contains("cannot read"), "{message}");
    assert!(message.contains("missing.js"), "{message}");
}

#[test]
fn runs_typescript_and_mixed_imports() {
    let project = Project::new();
    project.write(
        "main.ts",
        r#"
import type { Named } from "./lib/types.ts";
import { area } from "./lib/math.ts";
import { label } from "./lib/label.js";

const shape: Named = { name: "square", width: 3 };
const text = `${shape.name} ${area(shape)}` as string;
console.log(text satisfies string);
console.log(label(text));
const dynamic = await import("./lib/dynamic.mts");
console.log(dynamic.twice(21));
"#,
    );
    project.write(
        "lib/types.ts",
        "export interface Named {\n    name: string;\n    width: number;\n}\n",
    );
    project.write(
        "lib/math.ts",
        r#"
import { square } from "./square.mts";
export function area(shape: { width: number }): number {
    return square(shape.width);
}
"#,
    );
    project.write(
        "lib/square.mts",
        "export const square = (n: number): number => n * n;\n",
    );
    project.write(
        "lib/label.js",
        "export const label = (value) => `[${value}]`;\n",
    );
    project.write(
        "lib/dynamic.mts",
        "export const twice = (n: number): number => n * 2;\n",
    );

    let mut file = project.path("main.ts");
    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "square 9\n[square 9]\n42\n");
    assert_eq!(stderr(&output), "");

    // A JavaScript entry reaches TypeScript modules too.
    file = project.write(
        "main.js",
        "import { area } from \"./lib/math.ts\";\nconsole.log(area({ width: 4 }));\n",
    );
    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "16\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn does_not_type_check_typescript() {
    let project = Project::new();
    let file = project.write(
        "unchecked.ts",
        "const value: number = \"text\";\nconsole.log(value, typeof value);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "text string\n");
    assert_eq!(stderr(&output), "");
}

#[test]
fn reports_a_typescript_syntax_error_with_its_position_and_exits_1() {
    let project = Project::new();
    let file = project.write("invalid.ts", "const x: = 1;\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let message = stderr(&output);
    assert!(message.contains("cannot compile"), "{message}");
    assert!(message.contains("SyntaxError"), "{message}");
    assert!(message.contains("invalid.ts:1:"), "{message}");
}

#[test]
fn reports_an_unsupported_typescript_construct_and_exits_1() {
    let project = Project::new();
    let file = project.write("enum.ts", "enum Color { Red }\nconsole.log(Color.Red);\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "");
    let message = stderr(&output);
    assert!(message.contains("cannot compile"), "{message}");
    assert!(message.contains("enum.ts"), "{message}");
    assert!(
        message.contains("enum is not supported in strip-only mode"),
        "{message}"
    );
}

#[test]
fn reports_typescript_stack_traces_at_the_source_line() {
    let project = Project::new();
    let file = project.write(
        "throws.ts",
        r#"
interface Marker {
    label: string;
}
const marker: Marker = { label: "boom" };
throw new Error(marker.label);
"#,
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("boom"), "{message}");
    assert!(message.contains("throws.ts:6"), "{message}");
}

#[test]
fn refuses_tsx_cts_and_declaration_entries_and_exits_2() {
    let project = Project::new();
    let cases = [
        ("view.tsx", "export const x = <div />;\n", "JSX and TSX"),
        (
            "old.cts",
            "export = 1;\n",
            "CommonJS TypeScript (.cts) is not supported",
        ),
        (
            "types.d.ts",
            "export declare const x: number;\n",
            "declaration files",
        ),
    ];

    for (name, source, expected) in cases {
        let file = project.write(name, source);
        let output = run_file(&file);
        assert_eq!(output.status.code(), Some(2), "{name}");
        let message = stderr(&output);
        assert!(message.contains(expected), "{name}: {message}");
    }
}

#[test]
fn refuses_a_tsx_import_and_exits_1() {
    let project = Project::new();
    project.write("view.tsx", "export const x = <div />;\n");
    let file = project.write(
        "main.ts",
        "import { x } from \"./view.tsx\";\nconsole.log(x);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("view.tsx"), "{message}");
    assert!(message.contains("JSX and TSX"), "{message}");
}

#[test]
fn reports_a_typescript_import_that_cannot_be_stripped_and_exits_1() {
    let project = Project::new();
    project.write("bad.ts", "export enum Color { Red }\n");
    let file = project.write(
        "main.js",
        "import { Color } from \"./bad.ts\";\nconsole.log(Color.Red);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("bad.ts"), "{message}");
    assert!(
        message.contains("enum is not supported in strip-only mode"),
        "{message}"
    );
}

#[test]
fn refuses_commonjs_by_extension_only() {
    let project = Project::new();
    let commonjs = project.write("old.cjs", "module.exports = 1;\n");

    let output = run_file(&commonjs);

    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(
        message.contains(".cjs extension is not supported"),
        "{message}"
    );

    // A .js file is not rejected for its contents. It fails as ECMAScript when
    // module-level CommonJS bindings are used.
    let mislabelled = project.write("old.js", "module.exports = 1;\n");
    let output = run_file(&mislabelled);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("module is not defined"), "{message}");
    assert!(!message.contains("CommonJS"), "{message}");
}

#[test]
fn truncates_large_typed_arrays_to_ten_elements() {
    let project = Project::new();
    let file = project.write(
        "big.js",
        "const big = new Uint8Array(1000000);\nbig[0] = 7;\nbig[9] = 9;\nbig[10] = 11;\nconsole.log(big);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(0));
    let printed = stdout(&output);
    assert_eq!(
        printed,
        "Uint8Array(1000000) [7, 0, 0, 0, 0, 0, 0, 0, 0, 9]\n"
    );
    assert!(printed.len() < 100, "{printed}");
}

#[test]
fn help_describes_the_exit_codes_the_cli_uses() {
    let help = run_args(&["--help"]);

    assert_eq!(help.status.code(), Some(0));
    let text = stdout(&help);
    assert!(
        text.contains("1  the module or a test failed, or a test file registered no tests"),
        "{text}"
    );
    assert!(text.contains("2  neojs could not start"), "{text}");
}

#[test]
fn refuses_a_bare_import_and_exits_1() {
    let project = Project::new();
    let file = project.write(
        "main.js",
        "import value from \"lodash\";\nconsole.log(value);\n",
    );

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("bare module specifiers"), "{message}");
    assert!(message.contains("lodash"), "{message}");
}

#[test]
fn refuses_a_neo_import_and_exits_1() {
    let project = Project::new();
    let file = project.write("main.js", "import fs from \"neo:fs\";\nconsole.log(fs);\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("neo:fs"), "{message}");
    assert!(message.contains("not implemented"), "{message}");
}

#[test]
fn refuses_a_jsr_import_and_exits_1() {
    let project = Project::new();
    let file = project.write("main.js", "import \"jsr:@scope/pkg@1\";\n");

    let output = run_file(&file);

    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("jsr:@scope/pkg@1"), "{message}");
    assert!(
        message.contains("jsr: modules are not implemented"),
        "{message}"
    );
}

/// Golden help text. The default help is deliberately short: details live in
/// the README, and every option named here is one the current dev parser
/// accepts. Runtime behavior for those options is covered by the remote and
/// test-command regressions below.
#[test]
fn prints_the_compact_help() {
    let help = run_args(&["--help"]);

    assert_eq!(help.status.code(), Some(0));
    assert_eq!(stdout(&help), expected_help());

    let version = run_args(&["--version"]);
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        stdout(&version),
        format!("neojs {}\n", env!("CARGO_PKG_VERSION"))
    );

    // `test` has no separate help page; it prints the same text.
    let test_help = run_args(&["test", "--help"]);
    assert_eq!(test_help.status.code(), Some(0));
    assert_eq!(stdout(&test_help), expected_help());
}

fn expected_help() -> String {
    format!(
        "\
neojs {}
Run an ECMAScript module or erasable TypeScript file, or run test files.

Usage:
  neojs <file>                    run one module
  neojs --allow-remote <file-or-https-url>
  neojs test <file-or-glob>...    run test files
  neojs test --allow-remote <file-or-glob>...
  neojs --allow-remote test <file-or-glob>...

Options:
  --allow-remote   allow https: entry and imports; off by default
  -h, --help       show this help; alone, after `test`, or after `--allow-remote`
  -V, --version    show the version; alone, or after `--allow-remote`
  --               end option parsing, for a file or pattern beginning with `-`

Modules are ECMAScript modules, and relative imports resolve from the importing
file. .js, .mjs, .ts, and .mts run. TypeScript is stripped, not type checked:
only erasable syntax is accepted (no enum, namespace, or other transforms), and
.tsx, .jsx, .cts, and declaration files are rejected.

The age, assert, chars, console, crypto, os, path, slices,
ssh, stdio, strings, test, and uuid built-in modules are implemented, including
the uuid/v4 and uuid/v7 subpaths. Import one by bare name or with the `neo:`
prefix, or use `Neo.getBuiltInModule(\"test\")` and
`Neo.getBuiltInModule(\"uuid\")`; the README lists every specifier.

Remote loading is disabled by default. `--allow-remote` enables an https:
entry and absolute https: imports; it may appear before the module, or before or
after `test`. It is not SSRF protection and not a sandbox, and downloads are
cached under `neojs/packages` in the user cache.

`neojs test` expands each argument as an existing file or a case-sensitive
shell-style glob. Files are sorted, deduplicated, and each runs in a fresh
runtime. There is no default discovery, and a file that registers no tests fails.
Node modules (`node:`) are rejected; there is no Node-module opt-in.

Exit codes:
  0  the module finished, or every test file passed
  1  the module or a test failed, or a test file registered no tests
  2  neojs could not start: bad arguments, an unreadable or unsupported entry
     file, or a test file or glob that could not be expanded

Docs: https://github.com/neotales/neojs#readme
",
        env!("CARGO_PKG_VERSION")
    )
}

/// The help names `--allow-remote` for a module run and in both test
/// placements accepted by the current dev parser. The assertions are on the
/// documented contract; the remote tests below exercise the runtime.
#[test]
fn help_documents_allow_remote_for_modules_and_both_test_positions() {
    for args in [["--help"].as_slice(), ["test", "--help"].as_slice()] {
        let output = run_args(args);
        assert_eq!(output.status.code(), Some(0), "{args:?}");
        let text = stdout(&output);

        assert!(
            text.contains("neojs --allow-remote <file-or-https-url>"),
            "{text}"
        );
        assert!(
            text.contains("neojs test --allow-remote <file-or-glob>..."),
            "{text}"
        );
        assert!(
            text.contains("neojs --allow-remote test <file-or-glob>..."),
            "{text}"
        );
        assert!(
            text.contains("--allow-remote   allow https: entry and imports"),
            "{text}"
        );
        assert!(text.contains("disabled by default"), "{text}");
    }
}

/// The built-in list names UUID and its subpaths, and points at the README as
/// the complete list, so adding a built-in does not require a help edit here.
#[test]
fn help_lists_uuid_and_points_at_the_readme_for_every_specifier() {
    let help = run_args(&["--help"]);
    assert_eq!(help.status.code(), Some(0));
    let text = stdout(&help);

    for expected in [
        "uuid built-in modules are implemented",
        "uuid/v4 and uuid/v7 subpaths",
        "Neo.getBuiltInModule(\"uuid\")",
        "README lists every specifier",
    ] {
        assert!(text.contains(expected), "missing {expected:?}: {text}");
    }
}

#[test]
fn help_and_version_options_succeed_alone() {
    for option in ["-h", "--help", "-V", "--version"] {
        let output = run_args(&[option]);
        assert_eq!(output.status.code(), Some(0), "{option}");
    }

    // No long help is claimed or implemented. Adding one is a deliberate
    // change to this contract, not a silent extra page.
    let long_help = run_args(&["--help-long"]);
    assert_eq!(long_help.status.code(), Some(2));
    assert!(stderr(&long_help).contains("unknown option `--help-long`"));
}

/// A leading `--allow-remote` is skipped before the command is classified, so
/// help and version still work with it. The parser also accepts `--help` after
/// `test`, but not `--version` there.
#[test]
fn help_and_version_accept_a_leading_allow_remote() {
    for args in [
        ["--allow-remote", "--help"].as_slice(),
        ["--allow-remote", "-h"].as_slice(),
        ["--allow-remote", "test", "--help"].as_slice(),
        ["--allow-remote", "test", "-h"].as_slice(),
        ["--allow-remote", "--version"].as_slice(),
        ["--allow-remote", "-V"].as_slice(),
    ] {
        let output = run_args(args);
        assert_eq!(
            output.status.code(),
            Some(0),
            "args: {args:?}, stderr: {}",
            stderr(&output)
        );
    }

    let help = expected_help();
    for args in [
        ["--allow-remote", "--help"].as_slice(),
        ["--allow-remote", "-h"].as_slice(),
        ["--allow-remote", "test", "--help"].as_slice(),
        ["--allow-remote", "test", "-h"].as_slice(),
    ] {
        assert_eq!(stdout(&run_args(args)), help, "args: {args:?}");
    }

    let version = format!("neojs {}\n", env!("CARGO_PKG_VERSION"));
    for args in [
        ["--allow-remote", "--version"].as_slice(),
        ["--allow-remote", "-V"].as_slice(),
    ] {
        assert_eq!(stdout(&run_args(args)), version, "args: {args:?}");
    }
}

#[test]
fn reports_bad_arguments_and_exits_2() {
    let no_args = run_args(&[]);
    assert_eq!(no_args.status.code(), Some(2));
    assert_eq!(stdout(&no_args), "");
    assert_eq!(
        stderr(&no_args),
        "neojs: no input file\nTry 'neojs --help'.\n"
    );

    let unknown = run_args(&["--unknown"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert_eq!(
        stderr(&unknown),
        "neojs: unknown option `--unknown`\nTry 'neojs --help'.\n"
    );

    // Apart from a leading `--allow-remote`, help and version are accepted only
    // as the sole argument; `--` is the exception for naming a file.
    let help_with_file = run_args(&["--help", "main.js"]);
    assert_eq!(help_with_file.status.code(), Some(2));
    assert_eq!(
        stderr(&help_with_file),
        "neojs: expected one input file, got 2\nTry 'neojs --help'.\n"
    );

    let two_files = run_args(&["a.js", "b.js"]);
    assert_eq!(two_files.status.code(), Some(2));
    assert!(stderr(&two_files).contains("expected one input file"));

    let no_patterns = run_args(&["test"]);
    assert_eq!(no_patterns.status.code(), Some(2));
    assert_eq!(
        stderr(&no_patterns),
        "neojs: no test files or globs given\nTry 'neojs --help'.\n"
    );

    let test_unknown = run_args(&["test", "--unknown"]);
    assert_eq!(test_unknown.status.code(), Some(2));
    assert_eq!(
        stderr(&test_unknown),
        "neojs: unknown option `--unknown`\nTry 'neojs --help'.\n"
    );
}

#[test]
fn remote_urls_are_refused_without_the_allow_remote_flag() {
    let output = run_args(&["https://mods.test/main.js"]);

    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(
        message.contains("remote module loading is disabled"),
        "{message}"
    );
}

#[test]
fn allow_remote_still_refuses_http_and_credential_bearing_urls() {
    let http = run_args(&["--allow-remote", "http://mods.test/main.js"]);
    assert_eq!(http.status.code(), Some(2));
    assert!(stderr(&http).contains("https"), "{}", stderr(&http));

    let credentials = run_args(&["--allow-remote", "https://user:secret@mods.test/main.js"]);
    assert_eq!(credentials.status.code(), Some(2));
    let message = stderr(&credentials);
    assert!(message.contains("username or password"), "{message}");
    assert!(!message.contains("secret"), "{message}");
}

#[test]
fn allow_remote_still_runs_local_files() {
    let project = Project::new();
    let file = project.write("main.js", "console.log(\"local with remote enabled\");\n");
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["--allow-remote", path]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "local with remote enabled\n");
}

#[test]
fn help_documents_remote_loading_without_promising_isolation() {
    let help = run_args(&["--help"]);

    assert_eq!(help.status.code(), Some(0));
    let text = stdout(&help);
    assert!(text.contains("--allow-remote"), "{text}");
    assert!(text.contains("disabled by default"), "{text}");
    assert!(text.contains("not SSRF"), "{text}");
    assert!(text.contains("not a sandbox"), "{text}");
    assert!(text.contains("neojs/packages"), "{text}");
}

#[test]
fn accepts_a_path_after_a_double_dash() {
    let project = Project::new();
    let file = project.write("dash.js", "console.log(\"after dash\");\n");
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["--", path]);

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout(&output), "after dash\n");
}

/// The original lifecycle regression: enough heap activity to make V8 post a
/// delayed GC task. An isolate created outside a Tokio context aborts the
/// process when one arrives, so this runs in a child process.
const HEAP_PRESSURE_SOURCE: &str = r#"
const strings: string[] = [];
let s = "abc";
for (let i = 0; i < 22; i += 1) {
    s += s;
    strings.push(s.slice(0, 4096));
    if (/abc/.test(strings[strings.length - 1] ?? "")) {
        continue;
    }
}
const numbers: number[] = [];
for (let i = 0; i < 2_000_000; i += 1) numbers.push(i);
console.log(strings.length, numbers.length);
"#;

#[test]
fn survives_delayed_v8_gc_tasks_under_heap_pressure() {
    let project = Project::new();
    let file = project.write("pressure.ts", HEAP_PRESSURE_SOURCE);

    // The abort is deterministic in the unfixed build. Two rounds also exercise
    // starting and stopping the runtime's driver more than once.
    for round in 0..2 {
        let output = run_file_with_timeout(&file, Duration::from_secs(120));
        assert_eq!(
            output.status.code(),
            Some(0),
            "round {round} exited abnormally; stderr: {}",
            stderr(&output)
        );
        assert_eq!(stdout(&output), "22 2000000\n", "round {round}");
        assert_eq!(stderr(&output), "", "round {round}");
    }
}

#[test]
fn rejects_import_attributes_and_exits_1() {
    let project = Project::new();
    project.write("data.js", "export const value = 1;\n");
    let cases = [
        (
            "static.js",
            "import value from \"./data.js\" with { type: \"json\" };\nconsole.log(value);\n",
            "type: \"json\"",
        ),
        (
            "dynamic.js",
            "const data = await import(\"./data.js\", { with: { type: \"json\" } });\nconsole.log(data.value);\n",
            "type: \"json\"",
        ),
        (
            "builtin.js",
            "const chars = await import(\"chars\", { with: { type: \"text\" } });\nconsole.log(chars.isLetter(65));\n",
            "type: \"text\"",
        ),
    ];

    for (name, source, expected) in cases {
        let file = project.write(name, source);
        let output = run_file(&file);
        assert_eq!(output.status.code(), Some(1), "{name}");
        let message = stderr(&output);
        assert!(
            message.contains("module type attributes are not implemented"),
            "{name}: {message}"
        );
        assert!(message.contains(expected), "{name}: {message}");
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push_str(&format!("{byte:02x}"));
    }
    text
}

fn decode_hex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0, "hex length");
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex byte"))
        .collect()
}

/// Locates the reference `age` or `rage` binary, or fails with instructions.
///
/// `AGE_BIN` wins when set. Otherwise `age` and `rage` are tried in that order,
/// matching the README and the test's ignore reason. A configured binary that
/// does not run is a failure, not a reason to try another one silently.
fn reference_age() -> PathBuf {
    let candidates: Vec<PathBuf> = match std::env::var_os("AGE_BIN") {
        Some(bin) => vec![PathBuf::from(bin)],
        None => vec![PathBuf::from("age"), PathBuf::from("rage")],
    };
    let mut tried = Vec::with_capacity(candidates.len());
    for bin in &candidates {
        tried.push(bin.display().to_string());
        if let Ok(output) = Command::new(bin).arg("--version").output()
            && output.status.success()
        {
            return bin.clone();
        }
    }
    panic!(
        "this interop test needs a reference age CLI; install age or rage, or set AGE_BIN (tried {})",
        tried.join(", ")
    );
}

/// Two-way binary age v1 interoperation with a real `age` executable.
///
/// Ignored by default because it depends on a tool outside the workspace. Run
/// it with:
///
/// ```text
/// cargo test -p neojs --test cli -- --ignored age_cli_interop
/// ```
///
/// It fails, rather than skipping, when the tool is missing. The identity and
/// recipient are the public X25519 test vector, never a credential.
#[test]
#[ignore = "requires age or rage on PATH (or AGE_BIN)"]
fn age_cli_interop_with_external_age() {
    const IDENTITY: &str =
        "AGE-SECRET-KEY-1GQ9778VQXMMJVE8SK7J6VT8UJ4HDQAJUVSFCWCM02D8GEWQ72PVQ2Y5J33";
    const RECIPIENT: &str = "age1t7rxyev2z3rw82stdlrrepyc39nvn86l5078zqkf5uasdy86jp6svpy7pa";

    let age = reference_age();
    let project = Project::new();

    // Our encryption, the reference tool's decryption. The payload crosses
    // several 64 KiB STREAM chunks.
    let plaintext: Vec<u8> = (0..150_000).map(|index| (index * 37 + 11) as u8).collect();
    let encrypt_file = project.write(
        "encrypt.js",
        &format!(
            r#"
import {{ encrypt }} from "age";
const plaintext = new Uint8Array({});
for (let i = 0; i < plaintext.length; i++) plaintext[i] = (i * 37 + 11) & 255;
const ciphertext = encrypt("{RECIPIENT}", plaintext);
let hex = "";
for (const byte of ciphertext) hex += byte.toString(16).padStart(2, "0");
console.log(hex);
"#,
            plaintext.len()
        ),
    );
    let emitted = run_file(&encrypt_file);
    assert_eq!(emitted.status.code(), Some(0), "{}", stderr(&emitted));
    let ciphertext = decode_hex(stdout(&emitted).trim());
    let ciphertext_path = project.path("ours.age");
    fs::write(&ciphertext_path, &ciphertext).expect("write ciphertext");
    let identity_path = project.path("identity.txt");
    fs::write(&identity_path, format!("{IDENTITY}\n")).expect("write identity");

    let decrypted = Command::new(&age)
        .args(["--decrypt", "--identity"])
        .arg(&identity_path)
        .stdin(fs::File::open(&ciphertext_path).expect("open ciphertext"))
        .output()
        .expect("run reference age");
    assert!(
        decrypted.status.success(),
        "reference age failed: {}",
        String::from_utf8_lossy(&decrypted.stderr)
    );
    assert_eq!(decrypted.stdout, plaintext);

    // The reference tool's encryption, our decryption. This payload also
    // crosses several 64 KiB STREAM chunks.
    let reverse_plaintext: Vec<u8> = (0..120_000).map(|index| (index * 13 + 5) as u8).collect();
    let reverse_plain_path = project.path("reverse.bin");
    fs::write(&reverse_plain_path, &reverse_plaintext).expect("write reverse plaintext");
    let reverse = project.path("theirs.age");
    let reverse_status = Command::new(&age)
        .args(["--encrypt", "--recipient", RECIPIENT])
        .stdin(fs::File::open(&reverse_plain_path).expect("open reverse plaintext"))
        .stdout(fs::File::create(&reverse).expect("create reference output"))
        .status()
        .expect("run reference age encrypt");
    assert!(reverse_status.success());
    let reverse_ciphertext = fs::read(&reverse).expect("read reference output");
    let decrypt_file = project.write(
        "decrypt.js",
        &format!(
            r#"
import {{ decrypt }} from "age";
const hex = "{}";
const ciphertext = new Uint8Array(hex.length / 2);
for (let i = 0; i < ciphertext.length; i++) {{
    ciphertext[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
}}
const plaintext = decrypt("{IDENTITY}", ciphertext);
let sum = 0;
for (const byte of plaintext) sum = (sum + byte) % 1000003;
console.log(plaintext.length, sum);
"#,
            encode_hex(&reverse_ciphertext)
        ),
    );
    let decrypted_by_us = run_file(&decrypt_file);
    assert_eq!(
        decrypted_by_us.status.code(),
        Some(0),
        "{}",
        stderr(&decrypted_by_us)
    );
    let expected_sum = reverse_plaintext
        .iter()
        .fold(0u32, |sum, byte| (sum + u32::from(*byte)) % 1_000_003);
    assert_eq!(
        stdout(&decrypted_by_us),
        format!("{} {expected_sum}\n", reverse_plaintext.len())
    );

    // The empty file also interoperates in both directions.
    let empty = project.path("empty.age");
    let empty_status = Command::new(&age)
        .args(["--encrypt", "--recipient", RECIPIENT])
        .stdin(Stdio::null())
        .stdout(fs::File::create(&empty).expect("create empty output"))
        .status()
        .expect("run reference age encrypt");
    assert!(empty_status.success());
    let empty_ciphertext = fs::read(&empty).expect("read empty output");
    let empty_decrypt = project.write(
        "empty-decrypt.js",
        &format!(
            r#"
import {{ decrypt }} from "age";
const hex = "{}";
const ciphertext = new Uint8Array(hex.length / 2);
for (let i = 0; i < ciphertext.length; i++) {{
    ciphertext[i] = parseInt(hex.substring(i * 2, i * 2 + 2), 16);
}}
console.log(decrypt("{IDENTITY}", ciphertext).length);
"#,
            encode_hex(&empty_ciphertext)
        ),
    );
    let empty_output = run_file(&empty_decrypt);
    assert_eq!(
        empty_output.status.code(),
        Some(0),
        "{}",
        stderr(&empty_output)
    );
    assert_eq!(stdout(&empty_output), "0\n");
}

/// A deterministic loopback SSH server for the CLI smoke test.
///
/// The host key and password are fixed test values. No external host, key
/// file, agent, or real credential is used.
struct CliFinished {
    commands: Vec<Vec<u8>>,
    attempts: usize,
}

struct SshFixture {
    port: u16,
    host_key: Vec<u8>,
    observed: mpsc::Receiver<Vec<u8>>,
    attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    handle: thread::JoinHandle<()>,
}

enum CliAuth {
    Password,
    PublicKey(Vec<u8>),
}

impl SshFixture {
    fn start() -> Self {
        Self::start_with(CliAuth::Password)
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    fn start_public_key(blob: Vec<u8>) -> Self {
        Self::start_with(CliAuth::PublicKey(blob))
    }

    fn start_with(auth: CliAuth) -> Self {
        use neotales_ssh::ServerAuthPolicy;
        use neotales_ssh::ServerConfig;
        use neotales_ssh::ServerSessions;
        use neotales_ssh::authenticate_server;
        use neotales_ssh::hostkey::Ed25519Signer;
        use neotales_ssh::session::SessionDecision;
        use neotales_ssh::session::SessionEvent;
        use neotales_ssh::session::SessionRequest;
        use std::sync::Arc;
        use std::sync::atomic::AtomicUsize;
        use std::sync::atomic::Ordering;

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
        let port = listener.local_addr().expect("address").port();
        let mut seed = [21; 32];
        let host_key = Ed25519Signer::from_seed(&mut seed)
            .public_key_blob()
            .expect("host key blob");
        let (observed, observed_rx) = mpsc::channel();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempts_for_auth = Arc::clone(&attempts);
        let handle = thread::spawn(move || {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(15)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(15)));
            let mut seed = [21; 32];
            let config = ServerConfig::new(Ed25519Signer::from_seed(&mut seed));
            let Ok(mut transport) = config.accept(stream) else {
                return;
            };
            let policy = match auth {
                CliAuth::Password => ServerAuthPolicy::new(4).map(|policy| {
                    policy.with_password_verifier(move |candidate: &str| {
                        attempts_for_auth.fetch_add(1, Ordering::SeqCst);
                        candidate == "cli-password"
                    })
                }),
                CliAuth::PublicKey(authorized) => {
                    let expected = authorized.clone();
                    ServerAuthPolicy::new(4)
                        .and_then(|policy| policy.with_authorized_key(&authorized))
                        .map(|policy| {
                            policy.with_public_key_verifier(
                                move |_algorithm, key, _signed, _proof| {
                                    attempts_for_auth.fetch_add(1, Ordering::SeqCst);
                                    key == expected.as_slice()
                                },
                            )
                        })
                }
            };
            let Ok(policy) = policy else {
                return;
            };
            if authenticate_server(&mut transport, &policy).is_err() {
                return;
            }
            let mut sessions = ServerSessions::with_policy(transport, move |request| {
                if let SessionRequest::Exec { command } = request {
                    let _ = observed.send(command.to_vec());
                    SessionDecision::accept(0)
                } else {
                    SessionDecision::reject()
                }
            });
            loop {
                match sessions.dispatch_one() {
                    Ok(SessionEvent::Request {
                        session,
                        accepted: true,
                        ..
                    }) => {
                        if sessions.send_data(session, b"cli fixture\n").is_ok() {
                            let _ = sessions.send_exit_status(session);
                            let _ = sessions.send_eof(session);
                            let _ = sessions.send_close(session);
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            host_key,
            observed: observed_rx,
            attempts,
            handle,
        }
    }

    fn finish(self) -> CliFinished {
        self.handle.join().expect("server thread");
        CliFinished {
            commands: self.observed.try_iter().collect(),
            attempts: self.attempts.load(std::sync::atomic::Ordering::SeqCst),
        }
    }
}

/// A deterministic synthetic SSH agent over a private Unix socket.
///
/// It uses no real agent, environment, or credential. The signing key is
/// derived from a fixed test seed and exists only in this process.
#[cfg(unix)]
#[derive(Clone, Copy)]
enum AgentMode {
    Sign,
    Empty,
    RefuseSign,
    /// Advertise more identities than the client's documented bound.
    ManyIdentities,
}

#[cfg(unix)]
struct AgentFixture {
    _dir: TempDir,
    path: PathBuf,
    connects: Arc<AtomicUsize>,
    signs: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
    public_blob: Vec<u8>,
}

#[cfg(unix)]
impl AgentFixture {
    fn start(mode: AgentMode, seed: u8) -> Self {
        use neotales_ssh::hostkey::Ed25519Signer;

        let dir = tempfile::tempdir().expect("agent temp dir");
        let path = dir.path().join("agent.sock");
        let listener = UnixListener::bind(&path).expect("bind agent socket");
        listener
            .set_nonblocking(true)
            .expect("nonblocking agent socket");
        let mut key_seed = [seed; 32];
        let public_blob = Ed25519Signer::from_seed(&mut key_seed)
            .public_key_blob()
            .expect("agent public key");
        let connects = Arc::new(AtomicUsize::new(0));
        let signs = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let connects_for_thread = Arc::clone(&connects);
        let signs_for_thread = Arc::clone(&signs);
        let stop_for_thread = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !stop_for_thread.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        connects_for_thread.fetch_add(1, Ordering::SeqCst);
                        serve_agent(stream, mode, seed, &signs_for_thread);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            _dir: dir,
            path,
            connects,
            signs,
            stop,
            handle: Some(handle),
            public_blob,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn public_blob(&self) -> &[u8] {
        &self.public_blob
    }

    /// Stop the agent and return `(connections, sign requests)`.
    fn finish(mut self) -> (usize, usize) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle.join().expect("agent thread");
        }
        (
            self.connects.load(Ordering::SeqCst),
            self.signs.load(Ordering::SeqCst),
        )
    }
}

#[cfg(unix)]
fn serve_agent(mut stream: UnixStream, mode: AgentMode, seed: u8, signs: &AtomicUsize) {
    use neotales_ssh::agent_server::AgentServer;
    use neotales_ssh::auth::PublicKeySigner;
    use neotales_ssh::hostkey::Ed25519Signer;

    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut server = match mode {
        AgentMode::Sign => {
            let mut key_seed = [seed; 32];
            let signer = PublicKeySigner::ed25519(Ed25519Signer::from_seed(&mut key_seed));
            Some(AgentServer::from_signer(signer).expect("synthetic agent signer"))
        }
        AgentMode::Empty => Some(AgentServer::default()),
        AgentMode::RefuseSign => None,
        AgentMode::ManyIdentities => {
            let signers = (0..17_u8)
                .map(|offset| {
                    let mut key_seed = [seed.wrapping_add(offset); 32];
                    PublicKeySigner::ed25519(Ed25519Signer::from_seed(&mut key_seed))
                })
                .collect();
            Some(AgentServer::from_signers(signers).expect("many-identity agent"))
        }
    };
    let mut refuse_blob = Vec::new();
    if matches!(mode, AgentMode::RefuseSign) {
        let mut key_seed = [seed; 32];
        refuse_blob = Ed25519Signer::from_seed(&mut key_seed)
            .public_key_blob()
            .expect("refusal public key");
    }
    while let Some(request) = read_agent_frame(&mut stream) {
        if request.first() == Some(&13) {
            signs.fetch_add(1, Ordering::SeqCst);
        }
        let response = match mode {
            AgentMode::Sign | AgentMode::Empty | AgentMode::ManyIdentities => server
                .as_mut()
                .expect("agent server")
                .answer_read_only(&request),
            AgentMode::RefuseSign => match request.first() {
                Some(11) => agent_identities_answer(&refuse_blob),
                _ => vec![5],
            },
        };
        if !write_agent_frame(&mut stream, &response) {
            break;
        }
    }
}

#[cfg(unix)]
fn read_agent_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).ok()?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > 1_048_576 {
        return None;
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

#[cfg(unix)]
fn write_agent_frame(stream: &mut UnixStream, body: &[u8]) -> bool {
    let Ok(length) = u32::try_from(body.len()) else {
        return false;
    };
    stream.write_all(&length.to_be_bytes()).is_ok() && stream.write_all(body).is_ok()
}

#[cfg(unix)]
fn agent_identities_answer(blob: &[u8]) -> Vec<u8> {
    let mut body = vec![12];
    body.extend_from_slice(&1_u32.to_be_bytes());
    write_ssh_string(&mut body, blob);
    write_ssh_string(&mut body, b"");
    body
}

#[cfg(unix)]
fn write_ssh_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&u32::try_from(bytes.len()).unwrap_or(0).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn ssh_bytes(bytes: &[u8]) -> String {
    let items = bytes
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!("Uint8Array.from([{items}])")
}

#[test]
fn ssh_builtin_connects_to_a_loopback_fixture_through_the_cli() {
    let fixture = SshFixture::start();
    let project = Project::new();
    let script = r#"
import { connect } from "neo:ssh";

const registry = Neo.getBuiltInModule("ssh");
if (typeof registry !== "object" || registry === null) throw Error("missing ssh registry module");
if (registry.connect !== connect) throw Error("registry and import identity");

let refused = false;
try {
    connect({ host: "127.0.0.1", port: 1, username: "fixture", password: "fixture" });
} catch (error) {
    if (error.name !== "SshError" || error.code !== "NEO_SSH_HOST_KEY_REQUIRED") throw error;
    refused = true;
}
if (!refused) throw Error("a connection without host trust was accepted");

const connection = connect({
    host: "127.0.0.1",
    port: __PORT__,
    username: "fixture",
    password: "cli-password",
    hostKey: __HOST_KEY__,
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
});
const result = connection.exec("cli-smoke");
if (result.status !== 0) throw Error("status " + result.status);
let text = "";
for (let index = 0; index < result.stdout.length; index += 1) {
    text += String.fromCharCode(result.stdout[index]);
}
if (text !== "cli fixture\n") throw Error("stdout " + JSON.stringify(text));
connection.close();
console.log("ssh ok");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__HOST_KEY__", &ssh_bytes(&fixture.host_key));
    let file = project.write("ssh-cli.js", &script);

    let output = run_file_with_timeout(&file, Duration::from_secs(30));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "ssh ok\n");
    assert_eq!(stderr(&output), "");
    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"cli-smoke".to_vec()]);
    assert_eq!(finished.attempts, 1);
}

/// The deterministic host key used by `SshFixture`.
#[cfg(unix)]
fn cli_host_key(seed: u8) -> Vec<u8> {
    use neotales_ssh::hostkey::Ed25519Signer;
    let mut seed = [seed; 32];
    Ed25519Signer::from_seed(&mut seed)
        .public_key_blob()
        .expect("host key blob")
}

#[cfg(unix)]
#[test]
fn ssh_agent_auth_uses_the_synthetic_agent_and_runs_the_command() {
    let agent = AgentFixture::start(AgentMode::Sign, 31);
    let fixture = SshFixture::start_public_key(agent.public_blob().to_vec());
    let project = Project::new();
    let script = r#"
import { connect } from "neo:ssh";
const connection = connect({
    host: "127.0.0.1",
    port: __PORT__,
    username: "fixture",
    agent: true,
    hostKey: __HOST_KEY__,
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
});
const result = connection.exec("agent-smoke");
if (result.status !== 0) throw Error("status " + result.status);
let text = "";
for (let index = 0; index < result.stdout.length; index += 1) {
    text += String.fromCharCode(result.stdout[index]);
}
if (text !== "cli fixture\n") throw Error("stdout " + JSON.stringify(text));
connection.close();
console.log("agent ok");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__HOST_KEY__", &ssh_bytes(&fixture.host_key));
    let file = project.write("ssh-agent.js", &script);

    let output = run_file_with_agent(&file, agent.path(), Duration::from_secs(30));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "agent ok\n");
    assert_eq!(stderr(&output), "");

    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"agent-smoke".to_vec()]);
    assert_eq!(finished.attempts, 1);
    let (connections, signs) = agent.finish();
    assert!(connections >= 1, "the agent was not contacted");
    assert!(signs >= 1, "the agent was not asked to sign");
}

#[cfg(unix)]
#[test]
fn ssh_agent_is_not_contacted_when_the_host_key_is_rejected() {
    let agent = AgentFixture::start(AgentMode::Sign, 31);
    let fixture = SshFixture::start_public_key(agent.public_blob().to_vec());
    let wrong_host_key = cli_host_key(7);
    let project = Project::new();
    let script = r#"
import { connect } from "neo:ssh";
let failed = false;
try {
    connect({
        host: "127.0.0.1",
        port: __PORT__,
        username: "fixture",
        agent: true,
        hostKey: __WRONG_HOST_KEY__,
        connectTimeoutMs: 5000,
        execTimeoutMs: 5000,
    });
} catch (error) {
    if (error.name !== "SshError" || error.code !== "NEO_SSH_HOST_KEY") throw error;
    failed = true;
}
if (!failed) throw Error("the untrusted host key was accepted");
console.log("host rejected");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__WRONG_HOST_KEY__", &ssh_bytes(&wrong_host_key));
    let file = project.write("ssh-agent-host-reject.js", &script);

    let output = run_file_with_agent(&file, agent.path(), Duration::from_secs(30));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "host rejected\n");
    let finished = fixture.finish();
    assert!(finished.commands.is_empty());
    assert_eq!(
        finished.attempts, 0,
        "SSH authentication ran before host-key verification"
    );
    let (connections, signs) = agent.finish();
    assert_eq!(connections, 0, "the agent was contacted before host trust");
    assert_eq!(signs, 0, "the agent was asked to sign before host trust");
}

#[cfg(unix)]
#[test]
fn ssh_agent_without_identities_is_a_fixed_error() {
    let agent = AgentFixture::start(AgentMode::Empty, 31);
    let fixture = SshFixture::start_public_key(agent.public_blob().to_vec());
    let project = Project::new();
    let script = r#"
import { connect } from "neo:ssh";
let failed = false;
try {
    connect({
        host: "127.0.0.1",
        port: __PORT__,
        username: "fixture",
        agent: true,
        hostKey: __HOST_KEY__,
        connectTimeoutMs: 5000,
        execTimeoutMs: 5000,
    });
} catch (error) {
    if (error.name !== "SshError" || error.code !== "NEO_SSH_AGENT") throw error;
    failed = true;
}
if (!failed) throw Error("an empty agent was accepted");
console.log("empty agent");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__HOST_KEY__", &ssh_bytes(&fixture.host_key));
    let file = project.write("ssh-agent-empty.js", &script);

    let output = run_file_with_agent(&file, agent.path(), Duration::from_secs(30));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "empty agent\n");
    let finished = fixture.finish();
    assert!(finished.commands.is_empty());
    assert_eq!(finished.attempts, 0);
    let (connections, signs) = agent.finish();
    assert!(connections >= 1);
    assert_eq!(signs, 0);
}

#[cfg(unix)]
#[test]
fn ssh_agent_sign_refusal_is_a_fixed_error() {
    let agent = AgentFixture::start(AgentMode::RefuseSign, 31);
    let fixture = SshFixture::start_public_key(agent.public_blob().to_vec());
    let project = Project::new();
    let script = r#"
import { connect } from "neo:ssh";
let failed = false;
try {
    connect({
        host: "127.0.0.1",
        port: __PORT__,
        username: "fixture",
        agent: true,
        hostKey: __HOST_KEY__,
        connectTimeoutMs: 5000,
        execTimeoutMs: 5000,
    });
} catch (error) {
    if (error.name !== "SshError" || error.code !== "NEO_SSH_AGENT") throw error;
    failed = true;
}
if (!failed) throw Error("the refusing agent was accepted");
console.log("agent refused");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__HOST_KEY__", &ssh_bytes(&fixture.host_key));
    let file = project.write("ssh-agent-refuse.js", &script);

    let output = run_file_with_agent(&file, agent.path(), Duration::from_secs(30));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "agent refused\n");
    let finished = fixture.finish();
    assert!(finished.commands.is_empty());
    assert_eq!(finished.attempts, 0);
    let (connections, signs) = agent.finish();
    assert!(connections >= 1);
    assert_eq!(signs, 1, "the agent was not asked to sign exactly once");
}

#[cfg(unix)]
#[test]
fn ssh_password_mode_never_contacts_the_agent() {
    let agent = AgentFixture::start(AgentMode::Sign, 31);
    let fixture = SshFixture::start();
    let project = Project::new();
    let script = r#"
import { connect } from "neo:ssh";
const connection = connect({
    host: "127.0.0.1",
    port: __PORT__,
    username: "fixture",
    password: "cli-password",
    agent: false,
    hostKey: __HOST_KEY__,
    connectTimeoutMs: 5000,
    execTimeoutMs: 5000,
});
const result = connection.exec("password-mode");
if (result.status !== 0) throw Error("status");
connection.close();
console.log("password ok");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__HOST_KEY__", &ssh_bytes(&fixture.host_key));
    let file = project.write("ssh-password-no-agent.js", &script);

    let output = run_file_with_agent(&file, agent.path(), Duration::from_secs(30));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "password ok\n");
    let finished = fixture.finish();
    assert_eq!(finished.commands, vec![b"password-mode".to_vec()]);
    assert_eq!(finished.attempts, 1);
    let (connections, signs) = agent.finish();
    assert_eq!(connections, 0, "password mode touched SSH_AUTH_SOCK");
    assert_eq!(signs, 0);
}

#[cfg(not(unix))]
#[test]
fn ssh_agent_is_rejected_on_non_unix_platforms() {
    let fixture = SshFixture::start();
    let project = Project::new();
    let script = r#"
import { connect } from "neo:ssh";
let failed = false;
try {
    connect({
        host: "127.0.0.1",
        port: __PORT__,
        username: "fixture",
        agent: true,
        hostKey: __HOST_KEY__,
        connectTimeoutMs: 5000,
        execTimeoutMs: 5000,
    });
} catch (error) {
    if (error.name !== "SshError" || error.code !== "NEO_SSH_AGENT") throw error;
    failed = true;
}
if (!failed) throw Error("agent authentication is not rejected on this platform");
console.log("agent unsupported");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__HOST_KEY__", &ssh_bytes(&fixture.host_key));
    let file = project.write("ssh-agent-unsupported.js", &script);

    let output = run_file_with_timeout(&file, Duration::from_secs(30));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "agent unsupported\n");
    let finished = fixture.finish();
    assert!(finished.commands.is_empty());
}

#[cfg(unix)]
#[test]
fn ssh_agent_identity_bound_is_a_fixed_error() {
    let agent = AgentFixture::start(AgentMode::ManyIdentities, 31);
    let fixture = SshFixture::start_public_key(agent.public_blob().to_vec());
    let project = Project::new();
    let script = r#"
import { connect } from "neo:ssh";
let failed = false;
try {
    connect({
        host: "127.0.0.1",
        port: __PORT__,
        username: "fixture",
        agent: true,
        hostKey: __HOST_KEY__,
        connectTimeoutMs: 5000,
        execTimeoutMs: 5000,
    });
} catch (error) {
    if (error.name !== "SshError" || error.code !== "NEO_SSH_AGENT") throw error;
    failed = true;
}
if (!failed) throw Error("an oversized agent identity set was accepted");
console.log("agent bounded");
"#
    .replace("__PORT__", &fixture.port.to_string())
    .replace("__HOST_KEY__", &ssh_bytes(&fixture.host_key));
    let file = project.write("ssh-agent-many.js", &script);

    let output = run_file_with_agent(&file, agent.path(), Duration::from_secs(30));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert_eq!(stdout(&output), "agent bounded\n");
    let finished = fixture.finish();
    assert!(finished.commands.is_empty());
    assert_eq!(finished.attempts, 0);
    let (connections, signs) = agent.finish();
    assert!(connections >= 1);
    assert_eq!(signs, 0);
}

/// `neojs test` must expand files and globs deterministically and run an
/// overlapping selection once. The order is canonical-path order, so the
/// three file headers are the only stable way to count the selection.
#[test]
fn test_command_expands_files_and_globs_without_duplicates() {
    let project = Project::new();
    project.write(
        "a.test.js",
        "import test from \"test\";\ntest(\"a\", () => {});\n",
    );
    project.write(
        "b.test.js",
        "import test from \"neo:test\";\ntest(\"b\", () => {});\n",
    );
    project.write(
        "sub/c.test.js",
        "import test from \"test\";\ntest(\"c\", () => {});\n",
    );
    let dir = project.dir.path().to_str().expect("utf-8 path");
    let literal = format!("{dir}/a.test.js");
    let glob = format!("{dir}/**/*.test.js");

    let output = run_args(&["test", &literal, &glob]);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert_eq!(
        printed
            .lines()
            .filter(|line| line.starts_with("test "))
            .count(),
        3,
        "{printed}"
    );
    assert_eq!(
        printed
            .lines()
            .filter(|line| line.ends_with("a.test.js:"))
            .count(),
        1,
        "the literal and the glob overlap; a.test.js ran twice\n{printed}"
    );
    assert!(
        printed.contains("# 3 selected file(s): 3 test(s): 3 passed"),
        "{printed}"
    );
}

/// A loopback address held for the lifetime of a test.
///
/// The listener keeps the port from being reused by another process, so the
/// URL is stable for the whole test. The cache fixture below means a passing run
/// never opens a connection; if a regression misses the cache, the request
/// reaches this loopback socket rather than an external network.
struct HeldLoopback {
    url: String,
    _listener: TcpListener,
}

impl HeldLoopback {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        Self {
            url: format!("https://127.0.0.1:{port}/dep.js"),
            _listener: listener,
        }
    }

    fn url(&self) -> &str {
        &self.url
    }
}

fn write_remote_importing_test(project: &Project, name: &str, url: &str) -> PathBuf {
    project.write(
        name,
        &format!(
            "import test from \"test\";\n\
             import {{ value }} from \"{url}\";\n\
             test(\"remote import\", () => {{ if (value !== 1) throw new Error(\"bad value\"); }});\n"
        ),
    )
}

/// Runs the CLI with a private cache home and a deadline.
///
/// Setting `XDG_CACHE_HOME` keeps the spawned process away from the real
/// user-profile cache, and the deadline fails a cache miss or hang without
/// waiting for the remote timeout.
fn run_args_with_cache_home(args: &[&str], cache_home: &Path, timeout: Duration) -> Output {
    let child = Command::new(env!("CARGO_BIN_EXE_neojs"))
        .args(args)
        .env("XDG_CACHE_HOME", cache_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn neojs");
    run_child(child, timeout)
}

/// Publishes one deterministic module into `cache_home`'s package cache.
///
/// The scripted transport is used only to seed the fixture; the spawned CLI
/// then exercises the production cache path. No connection is made, so the test
/// is deterministic and needs no local TLS certificate or external network.
fn seed_remote_cache(cache_home: &Path, url: &str) {
    let cache_dir = cache_home.join("neojs").join("packages");
    let expected = url.to_string();
    let mut runtime = Runtime::with_remote_transport(
        RemoteOptions::new().with_cache_dir(&cache_dir),
        move |requested, _timeout, _if_none_match, _if_modified_since| {
            if requested != expected {
                return Err("unexpected remote URL".to_string());
            }
            Ok(TestRemoteResponse {
                status: 200,
                cache_control: Some("max-age=3600".to_string()),
                body: b"export const value = 1;\n".to_vec(),
                ..TestRemoteResponse::default()
            })
        },
    )
    .expect("seed runtime");
    runtime.run_url(url).expect("seed remote cache");
    assert!(cache_dir.is_dir(), "cache directory was not created");
}

/// `--allow-remote` must reach every fresh test runtime, whether it is written
/// before or after `test`, and a selected test must be able to import and use a
/// remote module end to end.
#[test]
fn test_command_accepts_allow_remote_for_remote_imports() {
    let project = Project::new();
    let cache_home = project.path("cache-home");
    let loopback = HeldLoopback::new();
    let file = write_remote_importing_test(&project, "remote.test.js", loopback.url());
    seed_remote_cache(&cache_home, loopback.url());
    let path = file.to_str().expect("utf-8 path");

    for prefix in [
        ["--allow-remote", "test"].as_slice(),
        ["test", "--allow-remote"].as_slice(),
    ] {
        let mut args = prefix.to_vec();
        args.push(path);
        let output = run_args_with_cache_home(&args, &cache_home, Duration::from_secs(10));
        assert_eq!(
            output.status.code(),
            Some(0),
            "args: {args:?}, stderr: {}",
            stderr(&output)
        );
        let printed = stdout(&output);
        assert!(
            contains_line(&printed, "ok 1 - remote import"),
            "args: {args:?}, stdout: {printed}"
        );
        assert!(
            printed.contains("# 1 selected file(s): 1 test(s): 1 passed"),
            "args: {args:?}, stdout: {printed}"
        );
    }
}

/// `--allow-remote` must reach every per-file runtime, not just the first.
///
/// The files are passed as two separate arguments and each imports the same
/// module from the seeded cache. If the flag reached only one runtime, the
/// other file would fail with remote loading disabled.
#[test]
fn test_command_allow_remote_reaches_every_selected_file() {
    let project = Project::new();
    let cache_home = project.path("cache-home");
    let loopback = HeldLoopback::new();
    let first = write_remote_importing_test(&project, "first-remote.test.js", loopback.url());
    let second = write_remote_importing_test(&project, "second-remote.test.js", loopback.url());
    seed_remote_cache(&cache_home, loopback.url());
    let first = first.to_str().expect("utf-8 path");
    let second = second.to_str().expect("utf-8 path");

    let output = run_args_with_cache_home(
        &["--allow-remote", "test", first, second],
        &cache_home,
        Duration::from_secs(20),
    );

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    for name in ["first-remote.test.js", "second-remote.test.js"] {
        let header = format!("{name}:");
        assert!(
            printed
                .lines()
                .any(|line| line.starts_with("test ") && line.ends_with(header.as_str())),
            "missing a header for {name}: {printed}"
        );
    }
    assert_eq!(
        printed
            .lines()
            .filter(|line| *line == "ok 1 - remote import")
            .count(),
        2,
        "{printed}"
    );
    assert!(
        printed.contains("# 2 selected file(s): 2 test(s): 2 passed"),
        "{printed}"
    );
}

/// Without the flag the same selected test file must fail with remote loading
/// disabled even when the remote module is already in the shared package cache,
/// so the cache cannot mask a missing opt-in.
#[test]
fn test_command_refuses_remote_imports_without_allow_remote() {
    let project = Project::new();
    let cache_home = project.path("cache-home");
    let loopback = HeldLoopback::new();
    let file = write_remote_importing_test(&project, "remote.test.js", loopback.url());
    seed_remote_cache(&cache_home, loopback.url());
    let path = file.to_str().expect("utf-8 path");

    let output = run_args_with_cache_home(&["test", path], &cache_home, Duration::from_secs(10));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let message = stderr(&output);
    assert!(
        message.contains("remote module loading is disabled"),
        "{message}"
    );
}

/// Enabling remote loading must not change the per-file runtime isolation: a
/// glob still gets one fresh runtime per file.
#[test]
fn test_command_allow_remote_keeps_globbed_files_isolated() {
    let project = Project::new();
    project.write(
        "a-isolation.test.js",
        r#"
import test from "test";
globalThis.leaked = "a-isolation";
test("a", () => {});
"#,
    );
    project.write(
        "b-isolation.test.js",
        r#"
import test from "test";
test("b", () => {
    if (globalThis.leaked !== undefined) throw new Error("state leaked from a-isolation");
});
"#,
    );
    let dir = project.dir.path().to_str().expect("utf-8 path");
    let glob = format!("{dir}/*.test.js");

    let output = run_args(&["--allow-remote", "test", &glob]);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(
        printed.contains("# 2 selected file(s): 2 test(s): 2 passed"),
        "{printed}"
    );
}

/// A failure in one file must not stop the next file, and the next file must
/// not see the failing file's globals.
#[test]
fn test_command_continues_after_a_failure_and_isolates_files() {
    let project = Project::new();
    project.write(
        "a-fail.test.js",
        r#"
import test from "test";
globalThis.leaked = "a-fail";
test("fails", () => { throw new Error("first file failed"); });
"#,
    );
    project.write(
        "b-pass.test.js",
        r#"
import test from "test";
test("isolated", () => {
    if (globalThis.leaked !== undefined) throw new Error("state leaked from a-fail");
});
"#,
    );
    let dir = project.dir.path().to_str().expect("utf-8 path");
    let glob = format!("{dir}/*.test.js");

    let output = run_args(&["test", &glob]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.contains("not ok 1 - fails"), "{printed}");
    assert!(printed.contains("b-pass.test.js:"), "{printed}");
    assert!(printed.contains("ok 1 - isolated"), "{printed}");
    assert!(
        printed.contains("# 2 selected file(s): 2 test(s): 1 passed, 1 failed"),
        "{printed}"
    );
}

/// A pending test and an unhandled rejection are both failures. A run that
/// printed only "ok" lines for other tests must still be non-zero.
#[test]
fn test_command_fails_for_pending_and_unhandled_cases() {
    let project = Project::new();
    let file = project.write(
        "problems.test.js",
        r#"
import test from "test";
test("pending case");
test("unhandled case", () => { Promise.reject(new Error("boom")); });
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(
        printed.contains("not ok 1 - pending case # PENDING"),
        "{printed}"
    );
    assert!(
        printed.contains("unhandled rejection: Error: boom"),
        "{printed}"
    );
    assert!(printed.contains("1 pending, 1 unhandled"), "{printed}");
}

/// A callback that returns a Promise which never settles is failed by the
/// per-test timeout rather than blocking the command forever.
#[test]
fn test_command_fails_a_pending_promise_at_its_timeout() {
    let project = Project::new();
    let file = project.write(
        "timeout.test.js",
        r#"
import test from "test";
test("never settles", () => new Promise(() => {}), { timeout: 10 });
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("did not settle within 10 ms"),
        "{}",
        stdout(&output)
    );
}

/// Registration after a top-level await must still be found by the host.
#[test]
fn test_command_runs_tests_registered_after_top_level_await() {
    let project = Project::new();
    let file = project.write(
        "await.test.js",
        r#"
import test from "test";
const value = await Promise.resolve(41);
test("after await", () => {
    if (value + 1 !== 42) throw new Error("wrong value");
});
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(0), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("ok 1 - after await"),
        "{}",
        stdout(&output)
    );
}

/// A file that imports the runner but registers nothing is not a pass.
#[test]
fn test_command_fails_a_file_with_no_tests() {
    let project = Project::new();
    let file = project.write("empty.test.js", "console.log(\"nothing registered\");\n");
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stderr(&output).contains("no tests registered"),
        "{}",
        stderr(&output)
    );
}

/// Literal misses, malformed globs, and globs with no file are startup errors,
/// not a clean run with zero tests.
#[test]
fn test_command_reports_unmatched_and_malformed_selections() {
    let project = Project::new();
    let dir = project.dir.path().to_str().expect("utf-8 path");

    let unmatched = run_args(&["test", &format!("{dir}/missing*.test.js")]);
    assert_eq!(unmatched.status.code(), Some(2));
    assert!(
        stderr(&unmatched).contains("matched no files"),
        "{}",
        stderr(&unmatched)
    );

    let malformed = run_args(&["test", "["]);
    assert_eq!(malformed.status.code(), Some(2));
    assert!(
        stderr(&malformed).contains("invalid test glob"),
        "{}",
        stderr(&malformed)
    );

    let missing = run_args(&["test", &format!("{dir}/missing.test.js")]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(
        stderr(&missing).contains("does not exist or is not a regular file"),
        "{}",
        stderr(&missing)
    );

    let no_args = run_args(&["test"]);
    assert_eq!(no_args.status.code(), Some(2));
    assert!(
        stderr(&no_args).contains("no test files or globs"),
        "{}",
        stderr(&no_args)
    );
}

/// An async rejection is a failure, and a later test in the same file runs.
#[test]
fn test_command_reports_an_async_rejection_and_continues_in_the_file() {
    let project = Project::new();
    let file = project.write(
        "async.test.js",
        r#"
import test from "test";
test("rejects", async () => {
    await Promise.resolve();
    throw new Error("async failure");
});
test("later", () => {});
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.contains("not ok 1 - rejects"), "{printed}");
    assert!(printed.contains("async failure"), "{printed}");
    assert!(printed.contains("ok 2 - later"), "{printed}");
}

/// Calling `run()` yourself and catching its `TestFailureError` must not hide
/// the failure from the host's final report.
#[test]
fn test_command_does_not_let_a_caught_run_failure_pass() {
    let project = Project::new();
    let file = project.write(
        "caught.test.js",
        r#"
import test, { run, TestFailureError } from "test";
test("fails once", () => { throw new Error("caught failure"); });
try {
    await run();
} catch (error) {
    if (!(error instanceof TestFailureError)) throw error;
    if (error.code !== "NEO_TEST_FAILURES") throw error;
}
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.contains("not ok 1 - fails once"), "{printed}");
    assert!(printed.contains("1 failed"), "{printed}");
}

/// `node:test` is not an alias. Node modules need an explicit opt-in, which
/// this build does not have.
#[test]
fn test_command_rejects_node_test_without_an_opt_in() {
    let project = Project::new();
    let file = project.write(
        "node.test.js",
        "import test from \"node:test\";\ntest(\"x\", () => {});\n",
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let message = stderr(&output);
    assert!(
        message.contains("Node.js built-in modules are not implemented"),
        "{message}"
    );
    assert!(message.contains("node:test"), "{message}");
}

/// Replacing Promise.resolve or Promise.prototype.then after the shim runs
/// must not make a never-settling test look settled.
#[test]
fn test_command_timeout_survives_replaced_promise_intrinsics() {
    let project = Project::new();
    let file = project.write(
        "promise-ops.test.js",
        r#"
import test from "test";
Promise.resolve = () => ({ then: (fulfilled) => fulfilled() });
Promise.prototype.then = function (fulfilled) { fulfilled(); return this; };
test("never settles", () => new Promise(() => {}), { timeout: 20 });
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.contains("not ok 1 - never settles"), "{printed}");
    assert!(printed.contains("did not settle within 20 ms"), "{printed}");
}

/// A refed timer left by a test is discoverable event-loop work. The settle
/// window is bounded, the run is reported unsettled, and the runtime is
/// dropped without waiting for the timer.
#[test]
fn test_command_fails_unsettled_detached_timer_without_hanging() {
    let project = Project::new();
    let file = project.write(
        "detached.test.js",
        r#"
import test from "test";
test("leaves a long timer", () => {
    Deno.core.createTimer(() => {}, 60_000, undefined, false, true, false);
});
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args_with_timeout(&["test", path], Duration::from_secs(10));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.contains("unsettled work"), "{printed}");
    assert!(printed.contains("1 unsettled"), "{printed}");
}

/// A thenable that resolves to a never-settling native Promise must not be
/// handed to native resolution, where a replaced `Promise.prototype.then`
/// could make the pending Promise look fulfilled.
#[test]
fn test_command_nested_pending_thenable_uses_internal_promise_state() {
    let project = Project::new();
    let file = project.write(
        "nested-pending.test.js",
        r#"
import test from "test";
Promise.prototype.then = function (fulfilled) { fulfilled("fake"); return this; };
test("nested pending", () => ({
    then(resolve) {
        resolve(new Promise(() => {}));
    },
}), { timeout: 20 });
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("did not settle within 20 ms"),
        "{}",
        stdout(&output)
    );
}

/// Native Promise subclasses with throwing `then`, `constructor`, and
/// `Symbol.species` are observed by their real state, not by a prototype
/// getter. Fulfilled, rejected, and pending states all behave normally.
#[test]
fn test_command_native_promise_subclass_ignores_throwing_prototype_getters() {
    let project = Project::new();
    let file = project.write(
        "subclass.test.js",
        r#"
import test from "test";
class EvilPromise extends Promise {}
Object.defineProperty(EvilPromise, Symbol.species, {
    get() { throw new Error("species touched"); },
});
Object.defineProperty(EvilPromise.prototype, "constructor", {
    get() { throw new Error("constructor touched"); },
});
Object.defineProperty(EvilPromise.prototype, "then", {
    get() { throw new Error("then touched"); },
});
test("fulfilled subclass", () => new EvilPromise((resolve) => resolve("ok")));
test("rejected subclass", () => new EvilPromise((resolve, reject) => {
    reject(new Error("real rejection"));
}));
test("pending subclass", () => new EvilPromise(() => {}), { timeout: 20 });
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(printed.contains("ok 1 - fulfilled subclass"), "{printed}");
    assert!(
        printed.contains("not ok 2 - rejected subclass"),
        "{printed}"
    );
    assert!(printed.contains("real rejection"), "{printed}");
    assert!(printed.contains("not ok 3 - pending subclass"), "{printed}");
    assert!(printed.contains("did not settle within 20 ms"), "{printed}");
    assert!(
        !printed.contains("touched"),
        "a prototype getter ran:\n{printed}"
    );
}

/// Thenable behavior is first-settlement, cycle-guarded, and error-safe.
#[test]
fn test_command_thenable_settlement_cycles_and_errors() {
    let project = Project::new();
    let file = project.write(
        "thenables.test.js",
        r#"
import test from "test";
test("first resolution wins", () => ({
    then(resolve, reject) {
        resolve("first");
        reject(new Error("second"));
        resolve("third");
    },
}));
const cycle = { then(resolve) { resolve(cycle); } };
test("cycle", () => cycle);
test("then throws", () => ({ then() { throw new Error("then exploded"); } }));
test("then getter throws", () => ({
    get then() { throw new Error("getter exploded"); },
}));
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(
        printed.contains("ok 1 - first resolution wins"),
        "{printed}"
    );
    assert!(printed.contains("not ok 2 - cycle"), "{printed}");
    assert!(printed.contains("cyclic thenable chain"), "{printed}");
    assert!(printed.contains("not ok 3 - then throws"), "{printed}");
    assert!(printed.contains("then exploded"), "{printed}");
    assert!(
        printed.contains("not ok 4 - then getter throws"),
        "{printed}"
    );
    assert!(printed.contains("getter exploded"), "{printed}");
}

/// A chain longer than the assimilation limit fails quickly rather than
/// starving the timeout with an unbounded microtask walk.
#[test]
fn test_command_bounds_a_long_thenable_chain_without_hanging() {
    let project = Project::new();
    let file = project.write(
        "chain.test.js",
        r#"
import test from "test";
test("deep chain", () => {
    let value = 1;
    for (let index = 0; index < 1001; index += 1) {
        const next = value;
        value = { then(resolve) { resolve(next); } };
    }
    return value;
});
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args_with_timeout(&["test", path], Duration::from_secs(10));

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    assert!(
        stdout(&output).contains("thenable chain limit"),
        "{}",
        stdout(&output)
    );
}

/// A stateful `then` getter must not be read a second time to discover a
/// cycle. The first read makes the object a thenable; resolving to itself is
/// a cycle, whether a second read would return undefined or throw.
#[test]
fn test_command_stateful_then_getter_cycle_is_not_reread() {
    let project = Project::new();
    let file = project.write(
        "stateful.test.js",
        r#"
import test from "test";
let undefinedReads = 0;
const undefinedCycle = {
    get then() {
        undefinedReads += 1;
        if (undefinedReads === 1) {
            return function (resolve) { resolve(undefinedCycle); };
        }
        if (undefinedReads === 2) return undefined;
        throw new Error("undefined cycle read more than twice");
    },
};
let throwingReads = 0;
const throwingCycle = {
    get then() {
        throwingReads += 1;
        if (throwingReads === 1) {
            return function (resolve) { resolve(throwingCycle); };
        }
        throw new Error("throwing cycle reread");
    },
};
const plain = { ok: true };
test("stateful undefined cycle", () => undefinedCycle);
test("stateful throwing cycle", () => throwingCycle);
test("getters read once", () => {
    if (undefinedReads !== 1) throw new Error(`undefined getter read ${undefinedReads}`);
    if (throwingReads !== 1) throw new Error(`throwing getter read ${throwingReads}`);
});
test("plain object", () => plain);
test("plain through thenable", () => ({ then(resolve) { resolve(plain); } }));
test("nested native", () => ({
    then(resolve) { resolve(Promise.resolve("ok")); },
}));
"#,
    );
    let path = file.to_str().expect("utf-8 path");

    let output = run_args(&["test", path]);

    assert_eq!(output.status.code(), Some(1), "stderr: {}", stderr(&output));
    let printed = stdout(&output);
    assert!(
        contains_line(&printed, "not ok 1 - stateful undefined cycle"),
        "{printed}"
    );
    assert!(
        contains_line(&printed, "not ok 2 - stateful throwing cycle"),
        "{printed}"
    );
    assert_eq!(printed.matches("NEO_TEST_CYCLE").count(), 2, "{printed}");
    assert!(
        contains_line(&printed, "ok 3 - getters read once"),
        "{printed}"
    );
    assert!(contains_line(&printed, "ok 4 - plain object"), "{printed}");
    assert!(
        contains_line(&printed, "ok 5 - plain through thenable"),
        "{printed}"
    );
    assert!(contains_line(&printed, "ok 6 - nested native"), "{printed}");
    assert!(
        !printed.contains("undefined cycle read more than twice"),
        "{printed}"
    );
    assert!(!printed.contains("throwing cycle reread"), "{printed}");
}

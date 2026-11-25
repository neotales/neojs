//! Integration tests for `vael:crypto` PBKDF2 and bcrypt.
//!
//! Each test runs a real ECMAScript module against a fresh `Runtime`, so it
//! exercises the registry, the ESM bindings, the host ops, the shared worker
//! budget, and the JavaScript error mapping together. The Rust unit tests in
//! `vaeljs-core/src/crypto_hash/password.rs` cover the primitives and batched
//! vector set.

use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

use vaeljs_core::Runtime;

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

const PBKDF2_SHA256: &str = "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783";
const PBKDF2_SHA384: &str = "cd3443723a41cf1460cca9efeede428a8898a82d2ad4d1fc5cca08ed3f4d3cb47a62a70b3cb9ce65dcbfb9fb9d425027a8be69b53e2a22674b0939e5e0a682f7";
const PBKDF2_SHA512: &str = "c74319d99499fc3e9013acff597c23c5baf0a0bec5634c46b8352b793e324723d55caa76b2b25c43402dcfdc06cdcf66f95b7d0429420b39520006749c51a04e";

const BCRYPT_2Y: &str = "$2y$04$10NU4r76Q4nBoXH7Sw4RKu9LjsXUslmRavZFzvlPUCYI3Hkl4xX/W";
const BCRYPT_2A_EMPTY: &str = "$2a$06$DCq7YPn5Rq63x1Lad4cll.TV4S6ytwfsfvkgY8jIucDrjc8deX1s.";
const BCRYPT_2A_GO: &str = "$2a$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga";
const BCRYPT_2B_GO: &str = "$2b$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga";

#[test]
fn pbkdf2_matches_published_and_independent_vectors() {
    run_js(&format!(
        r#"
import {{ pbkdf2 }} from "vael:crypto";

function hex(bytes) {{
    let text = "";
    for (let index = 0; index < bytes.length; index += 1) {{
        text += bytes[index].toString(16).padStart(2, "0");
    }}
    return text;
}}
function expectBytes(label, actual, expected) {{
    if (!(actual instanceof Uint8Array)) throw new Error(label + ": not Uint8Array");
    if (typeof actual.then === "function") throw new Error(label + ": thenable");
    const got = hex(actual);
    if (got !== expected) throw new Error(label + ": " + got);
}}

expectBytes("sha256 rfc7914", pbkdf2("sha256", "passwd", "salt", 1, 64), "{sha256}");
expectBytes("sha384", pbkdf2("SHA384", "passwd", "salt", 1, 64), "{sha384}");
expectBytes("sha512", pbkdf2("sha512", "passwd", "salt", 1, 64), "{sha512}");

// A string is UTF-8; a NUL inside it is a data byte, not a terminator.
expectBytes("unicode nul", pbkdf2("sha256", "pässwörd\0", "sält\0", 2048, 48),
    "e6635e1a12f44a9bfe0ea9fbf1ca3b46197ab653a51824dd04ae991232b28d9155ab1b190a9cbba124272bee01a6d70b");

// Uint8Array input is exact, including NUL and 0xff.
expectBytes("raw bytes", pbkdf2("sha256",
    Uint8Array.from([0, 255, 112, 97, 115, 115]),
    Uint8Array.from([0, 115, 97, 108, 116, 255]), 2, 32),
    "60d38d8523b876fa767fb807eec4c919d8c75512bb89ad7954bc0328812eb4cc");

// The same input via the registry and the named export.
if (pbkdf2 !== Vael.getBuiltInModule("vael:crypto").pbkdf2) throw new Error("identity");
"#,
        sha256 = PBKDF2_SHA256,
        sha384 = PBKDF2_SHA384,
        sha512 = PBKDF2_SHA512,
    ));
}

#[test]
fn pbkdf2_rejects_bad_types_ranges_and_callbacks() {
    run_js(
        r#"
import { pbkdf2 } from "vael:crypto";

function expectTypeError(label, fn) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
        if ("code" in error) throw new Error(label + ": unexpected code");
        return;
    }
    throw new Error(label + ": expected TypeError");
}
function expectRange(label, fn) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof RangeError)) throw error;
        if ("code" in error) throw new Error(label + ": unexpected code");
        return;
    }
    throw new Error(label + ": expected RangeError");
}
function expectCode(label, code, fn) {
    try {
        fn();
    } catch (error) {
        if (error.code !== code) throw new Error(label + ": " + error.code);
        return;
    }
    throw new Error(label + ": expected " + code);
}

expectTypeError("missing", () => pbkdf2("sha256", "p", "s", 1));
expectTypeError("extra", () => pbkdf2("sha256", "p", "s", 1, 16, () => {}));
expectTypeError("algorithm type", () => pbkdf2(1, "p", "s", 1, 16));
expectTypeError("password type", () => pbkdf2("sha256", null, "s", 1, 16));
expectTypeError("salt type", () => pbkdf2("sha256", "p", {}, 1, 16));
expectTypeError("iterations type", () => pbkdf2("sha256", "p", "s", "1", 16));
expectTypeError("length type", () => pbkdf2("sha256", "p", "s", 1, "16"));

expectRange("iterations zero", () => pbkdf2("sha256", "p", "s", 0, 16));
expectRange("iterations high", () => pbkdf2("sha256", "p", "s", 1000001, 16));
expectRange("iterations fraction", () => pbkdf2("sha256", "p", "s", 1.5, 16));
expectRange("length zero", () => pbkdf2("sha256", "p", "s", 1, 0));
expectRange("length high", () => pbkdf2("sha256", "p", "s", 1, 1025));
expectRange("length fraction", () => pbkdf2("sha256", "p", "s", 1, 1.25));

expectCode("algorithm", "VAEL_CRYPTO_ALGORITHM",
    () => pbkdf2("sha1", "p", "s", 1, 16));
expectCode("algorithm alias", "VAEL_CRYPTO_ALGORITHM",
    () => pbkdf2("sha-256", "p", "s", 1, 16));
expectCode("password limit", "VAEL_CRYPTO_LIMIT",
    () => pbkdf2("sha256", new Uint8Array(1024 * 1024 + 1), "s", 1, 16));
expectCode("salt limit", "VAEL_CRYPTO_LIMIT",
    () => pbkdf2("sha256", "p", new Uint8Array(1024 * 1024 + 1), 1, 16));

// A detached view is rejected before its bytes are read.
const detached = new Uint8Array(16);
detached.buffer.transfer();
expectCode("detached", "VAEL_CRYPTO_DETACHED",
    () => pbkdf2("sha256", detached, "s", 1, 16));
"#,
    );
}

#[test]
fn bcrypt_verifies_published_fixtures_and_rejects_malicious_hashes() {
    run_js(&format!(
        r#"
import {{ bcryptVerify }} from "vael:crypto";

// Apache htpasswd, jBCrypt, and Go bcrypt fixtures. These are public test
// values, not credentials.
if (!bcryptVerify("password", "{two_y}")) throw new Error("2y fixture");
if (bcryptVerify("wrong password", "{two_y}")) throw new Error("2y wrong");
if (!bcryptVerify("", "{two_a_empty}")) throw new Error("2a empty");
if (!bcryptVerify("allmine", "{two_a_go}")) throw new Error("2a go");
if (!bcryptVerify("allmine", "{two_b_go}")) throw new Error("2b go");

// 2x is not accepted, even though the bcrypt crate can parse it.
if (bcryptVerify("allmine", "$2x$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga")) {{
    throw new Error("2x accepted");
}}
// Costs outside 4..=16 return false without computing a hash.
if (bcryptVerify("allmine", "$2b$03$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga")) {{
    throw new Error("cost 3 accepted");
}}
if (bcryptVerify("allmine", "$2b$17$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga")) {{
    throw new Error("cost 17 accepted");
}}
if (bcryptVerify("allmine", "$2b$31$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga")) {{
    throw new Error("cost 31 accepted");
}}

const valid = "{two_b_go}";
for (const bad of [
    "",
    valid.slice(0, 59),
    valid + "0",
    valid.slice(0, 6) + "!1" + valid.slice(8),
    valid.slice(0, 7) + "\u012f" + valid.slice(8),
    valid.slice(0, 59) + "!",
]) {{
    if (bcryptVerify("allmine", bad)) throw new Error("malformed accepted: " + bad);
}}

// A password above 72 bytes is a false result, not a partial match.
const tooLong = new Uint8Array(73).fill(97);
if (bcryptVerify(tooLong, "{two_y}")) throw new Error("oversized password accepted");
"#,
        two_y = BCRYPT_2Y,
        two_a_empty = BCRYPT_2A_EMPTY,
        two_a_go = BCRYPT_2A_GO,
        two_b_go = BCRYPT_2B_GO,
    ));
}

#[test]
fn bcrypt_hashes_are_2b_bounded_and_roundtrip_unicode_nul_and_72_bytes() {
    run_js(
        r#"
import { bcryptHash, bcryptVerify } from "vael:crypto";

function expectTypeError(label, fn) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
        return;
    }
    throw new Error(label + ": expected TypeError");
}
function expectRange(label, fn) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof RangeError)) throw error;
        return;
    }
    throw new Error(label + ": expected RangeError");
}
function expectCode(label, code, fn) {
    try {
        fn();
    } catch (error) {
        if (error.code !== code) throw new Error(label + ": " + error.code);
        return;
    }
    throw new Error(label + ": expected " + code);
}

const defaultHash = bcryptHash("password");
if (!defaultHash.startsWith("$2b$10$")) throw new Error("default cost: " + defaultHash);
if (defaultHash.length !== 60) throw new Error("length " + defaultHash.length);
if (!bcryptVerify("password", defaultHash)) throw new Error("default verify");
if (bcryptVerify("password ", defaultHash)) throw new Error("default wrong verify");

for (const password of ["pässwörd", "a\0b", "\u{1f600}"]) {
    const hash = bcryptHash(password, 4);
    if (!hash.startsWith("$2b$04$")) throw new Error("prefix " + hash);
    if (!bcryptVerify(password, hash)) throw new Error("unicode/nul verify " + password);
    if (bcryptVerify(password + "x", hash)) throw new Error("unicode/nul wrong " + password);
}

// Exactly 72 bytes is accepted and hashed as 72 bytes. 73 is rejected by
// hashing and returns false from verification.
const seventyTwo = "a".repeat(72);
const hash72 = bcryptHash(seventyTwo, 4);
if (!bcryptVerify(seventyTwo, hash72)) throw new Error("72 bytes");
const seventyThree = "a".repeat(73);
expectCode("73 hash", "VAEL_CRYPTO_LIMIT", () => bcryptHash(seventyThree, 4));
if (bcryptVerify(seventyThree, hash72)) throw new Error("73 bytes verified");

// A multi-byte character counts by UTF-8 bytes, not JS code units.
const thirtySixEAcute = "é".repeat(36);
if (thirtySixEAcute.length !== 36) throw new Error("test setup");
const hash72Unicode = bcryptHash(thirtySixEAcute, 4);
if (!bcryptVerify(thirtySixEAcute, hash72Unicode)) throw new Error("72 utf8 bytes");
expectCode("73 utf8 bytes", "VAEL_CRYPTO_LIMIT", () =>
    bcryptHash("é".repeat(36) + "a", 4));

expectTypeError("missing", () => bcryptHash());
expectTypeError("extra", () => bcryptHash("p", 4, () => {}));
expectTypeError("password type", () => bcryptHash(null, 4));
expectTypeError("cost type", () => bcryptHash("p", "4"));
expectRange("cost low", () => bcryptHash("p", 3));
expectRange("cost high", () => bcryptHash("p", 17));
expectRange("cost fraction", () => bcryptHash("p", 4.5));

// Synchronous results are not thenables.
const value = bcryptHash("p", 4);
if (typeof value.then === "function") throw new Error("sync thenable");
"#,
    );
}

#[test]
fn long_two_byte_strings_with_an_unpaired_surrogate_use_utf8_replacement() {
    run_js(
        r#"
import {
    bcryptHash,
    bcryptHashAsync,
    bcryptVerify,
    bcryptVerifyAsync,
    pbkdf2,
    pbkdf2Async,
} from "vael:crypto";

function expectBytes(label, actual, expected) {
    if (!(actual instanceof Uint8Array)) throw new Error(label + ": not Uint8Array");
    if (actual.length !== expected.length) {
        throw new Error(label + ": length " + actual.length + " != " + expected.length);
    }
    for (let index = 0; index < actual.length; index += 1) {
        if (actual[index] !== expected[index]) {
            throw new Error(label + ": byte " + index + " " + actual[index] + " != " + expected[index]);
        }
    }
}

// A long two-byte string ending in an unpaired high surrogate. Its explicit
// UTF-8 form is "é" repeated n times followed by U+FFFD, not a CESU-8
// surrogate encoding. This checks encoding semantics only; it does not prove
// that memory was wiped.
const n = 5000;
const text = "é".repeat(n) + "\uD800";
const replacement = new Uint8Array(2 * n + 3);
for (let index = 0; index < n; index += 1) {
    replacement[2 * index] = 0xc3;
    replacement[2 * index + 1] = 0xa9;
}
replacement[2 * n] = 0xef;
replacement[2 * n + 1] = 0xbf;
replacement[2 * n + 2] = 0xbd;

const fromBytes = pbkdf2("sha256", replacement, "salt", 1, 32);
expectBytes("pbkdf2 string", pbkdf2("sha256", text, "salt", 1, 32), fromBytes);
expectBytes("pbkdf2Async string", await pbkdf2Async("sha256", text, "salt", 1, 32), fromBytes);

// bcrypt accepts at most 72 bytes. This is 34 two-byte characters (68 bytes),
// one U+FFFD (3 bytes), and one ASCII byte: exactly 72.
const text72 = "é".repeat(34) + "\uD800" + "a";
const replacement72 = new Uint8Array(72);
for (let index = 0; index < 34; index += 1) {
    replacement72[2 * index] = 0xc3;
    replacement72[2 * index + 1] = 0xa9;
}
replacement72[68] = 0xef;
replacement72[69] = 0xbf;
replacement72[70] = 0xbd;
replacement72[71] = 0x61;

const syncHash = bcryptHash(text72, 4);
if (!bcryptVerify(replacement72, syncHash)) throw new Error("bcrypt sync replacement bytes");
if (!bcryptVerify(text72, syncHash)) throw new Error("bcrypt sync string");
const asyncHash = await bcryptHashAsync(text72, 4);
if (!(await bcryptVerifyAsync(replacement72, asyncHash))) {
    throw new Error("bcrypt async replacement bytes");
}
if (!(await bcryptVerifyAsync(text72, asyncHash))) throw new Error("bcrypt async string");

// 35 two-byte characters plus U+FFFD is 73 bytes, one over bcrypt's limit.
let rejected = false;
try {
    bcryptHash("é".repeat(35) + "\uD800", 4);
} catch (error) {
    if (error.code !== "VAEL_CRYPTO_LIMIT") throw error;
    rejected = true;
}
if (!rejected) throw new Error("73-byte replacement password was accepted");
"#,
    );
}

#[test]
fn password_readers_reject_detached_views_on_sync_and_async_paths() {
    run_js(
        r#"
import {
    bcryptHash,
    bcryptHashAsync,
    pbkdf2,
    pbkdf2Async,
} from "vael:crypto";

function detached(n) {
    const view = new Uint8Array(n);
    view.buffer.transfer();
    return view;
}
function expectSync(label, fn) {
    try {
        fn();
    } catch (error) {
        if (error.code !== "VAEL_CRYPTO_DETACHED") {
            throw new Error(label + ": " + error.code);
        }
        return;
    }
    throw new Error(label + ": expected VAEL_CRYPTO_DETACHED");
}
async function expectAsync(label, promise) {
    try {
        await promise;
    } catch (error) {
        if (error.code !== "VAEL_CRYPTO_DETACHED") {
            throw new Error(label + ": " + error.code);
        }
        return;
    }
    throw new Error(label + ": expected VAEL_CRYPTO_DETACHED");
}

expectSync("pbkdf2 password", () =>
    pbkdf2("sha256", detached(8), "salt", 1, 16));
expectSync("pbkdf2 salt", () =>
    pbkdf2("sha256", "password", detached(8), 1, 16));
expectSync("bcryptHash password", () => bcryptHash(detached(8), 4));

await expectAsync("pbkdf2Async password",
    pbkdf2Async("sha256", detached(8), "salt", 1, 16));
await expectAsync("pbkdf2Async salt",
    pbkdf2Async("sha256", "password", detached(8), 1, 16));
await expectAsync("bcryptHashAsync password", bcryptHashAsync(detached(8), 4));
"#,
    );
}

#[test]
fn bcrypt_verify_validates_password_before_a_malformed_hash() {
    run_js(&format!(
        r#"
import {{ bcryptVerify, bcryptVerifyAsync }} from "vael:crypto";

const malformed = "not a bcrypt hash";
const valid = "{two_y}";

function detached(n) {{
    const view = new Uint8Array(n);
    view.buffer.transfer();
    return view;
}}
function expectSyncArg(label, fn, name, code) {{
    try {{
        fn();
    }} catch (error) {{
        if (error.name !== name) throw new Error(label + ": " + error.name);
        if (code === undefined) {{
            if ("code" in error) throw new Error(label + ": unexpected code");
        }} else if (error.code !== code) {{
            throw new Error(label + ": " + error.code);
        }}
        return;
    }}
    throw new Error(label + ": expected " + name);
}}
async function expectAsyncArg(label, promise, name, code) {{
    try {{
        await promise;
    }} catch (error) {{
        if (error.name !== name) throw new Error(label + ": " + error.name);
        if (code === undefined) {{
            if ("code" in error) throw new Error(label + ": unexpected code");
        }} else if (error.code !== code) {{
            throw new Error(label + ": " + error.code);
        }}
        return;
    }}
    throw new Error(label + ": expected " + name);
}}

// Invalid password values are argument errors even when the hash is
// malformed. The async form has the same precedence as the sync form.
for (const value of [null, 1, {{}}, []]) {{
    expectSyncArg("sync invalid password", () => bcryptVerify(value, malformed), "TypeError");
    await expectAsyncArg("async invalid password", bcryptVerifyAsync(value, malformed), "TypeError");
    expectSyncArg("sync invalid password valid hash", () => bcryptVerify(value, valid), "TypeError");
    await expectAsyncArg("async invalid password valid hash",
        bcryptVerifyAsync(value, valid), "TypeError");
}}

const detachedView = detached(8);
expectSyncArg("sync detached malformed", () => bcryptVerify(detachedView, malformed),
    "Error", "VAEL_CRYPTO_DETACHED");
await expectAsyncArg("async detached malformed",
    bcryptVerifyAsync(detachedView, malformed), "Error", "VAEL_CRYPTO_DETACHED");

const detachedValid = detached(8);
expectSyncArg("sync detached valid", () => bcryptVerify(detachedValid, valid),
    "Error", "VAEL_CRYPTO_DETACHED");
await expectAsyncArg("async detached valid",
    bcryptVerifyAsync(detachedValid, valid), "Error", "VAEL_CRYPTO_DETACHED");

// The hash type is still checked first, so a non-string hash is TypeError even
// with an invalid password.
expectSyncArg("sync hash type", () => bcryptVerify(null, 1), "TypeError");
await expectAsyncArg("async hash type", bcryptVerifyAsync(null, 1), "TypeError");

// A valid string password and malformed hash is false in both forms.
if (bcryptVerify("p", malformed)) throw new Error("sync malformed true");
if (await bcryptVerifyAsync("p", malformed)) throw new Error("async malformed true");
"#,
        two_y = BCRYPT_2Y,
    ));
}

#[test]
fn bcrypt_verify_rejects_bad_argument_shapes() {
    run_js(
        r#"
import { bcryptVerify } from "vael:crypto";
function expectTypeError(label, fn) {
    try { fn(); } catch (error) {
        if (!(error instanceof TypeError)) throw error;
        return;
    }
    throw new Error(label + ": expected TypeError");
}
expectTypeError("missing", () => bcryptVerify("p"));
expectTypeError("extra", () => bcryptVerify("p", "h", () => {}));
expectTypeError("hash type", () => bcryptVerify("p", 1));
expectTypeError("password type", () => bcryptVerify(null, "h"));
"#,
    );
}

#[test]
fn async_password_forms_are_real_promises_and_release_resources() {
    run_js(&format!(
        r#"
import {{
    bcryptHashAsync,
    bcryptVerifyAsync,
    pbkdf2,
    pbkdf2Async,
}} from "vael:crypto";

const expected = "{sha256}";

// Input is copied before the promise is returned.
const password = Uint8Array.from([112, 97, 115, 115, 119, 100]);
const salt = Uint8Array.from([115, 97, 108, 116]);
const pending = pbkdf2Async("sha256", password, salt, 1, 64);
if (!(pending instanceof Promise)) throw new Error("pbkdf2Async is not a Promise");
password.fill(0);
salt.fill(0);
const derived = await pending;
let text = "";
for (let index = 0; index < derived.length; index += 1) {{
    text += derived[index].toString(16).padStart(2, "0");
}}
if (text !== expected) throw new Error("async pbkdf2: " + text);
if (pbkdf2("sha256", "passwd", "salt", 1, 64).length !== 64) {{
    throw new Error("sync pbkdf2 length");
}}

const hashPromise = bcryptHashAsync("pässwörd", 4);
if (!(hashPromise instanceof Promise)) throw new Error("bcryptHashAsync is not a Promise");
const hash = await hashPromise;
if (typeof hash !== "string" || hash.length !== 60 || !hash.startsWith("$2b$04$")) {{
    throw new Error("async bcrypt hash: " + hash);
}}
const verifyPromise = bcryptVerifyAsync("pässwörd", hash);
if (!(verifyPromise instanceof Promise)) throw new Error("bcryptVerifyAsync is not a Promise");
if (!(await verifyPromise)) throw new Error("async bcrypt verify");
if (await bcryptVerifyAsync("wrong", hash)) throw new Error("async bcrypt false verify");

// A malformed hash resolves false without a worker, but still returns a
// Promise from the async function.
const malformed = bcryptVerifyAsync("p", "not a hash");
if (!(malformed instanceof Promise)) throw new Error("malformed not a Promise");
if (await malformed) throw new Error("malformed true");

// Jobs are removed from the resource table by the join op.
const resources = Object.values(Deno.core.resources());
for (const name of resources) {{
    if (name === "vael::crypto::AsyncPassword") {{
        throw new Error("an async password job resource was retained");
    }}
}}
"#,
        sha256 = PBKDF2_SHA256,
    ));
}

#[test]
fn async_pbkdf2_without_a_tokio_event_loop_reports_no_executor() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { pbkdf2Async } from "vael:crypto";
let code = "none";
try {
    await pbkdf2Async("sha256", "p", "s", 1, 16);
} catch (error) {
    code = error.code;
}
if (code !== "VAEL_CRYPTO_NO_EXECUTOR") {
    throw new Error("unexpected executor result: " + code);
}
"#,
    )
    .expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    block_on_std(runtime.run_file_async(&file)).expect("module assertions");
}

#[test]
fn bcrypt_error_messages_do_not_echo_the_password() {
    run_js(
        r#"
import { bcryptHash } from "vael:crypto";
const password = "super-secret-password-" + "x".repeat(100);
try {
    bcryptHash(password, 4);
} catch (error) {
    if (error.code !== "VAEL_CRYPTO_LIMIT") throw error;
    if (error.message.includes("super-secret")) throw new Error("message echoed password");
    if (error.message.includes(password)) throw new Error("message echoed password");
    if (error.message.length > 128) throw new Error("message is too large");
}
"#,
    );
}

/// Polls a future to completion on the current thread, without a Tokio runtime.
struct WakeSignal(mpsc::Sender<()>);

impl Wake for WakeSignal {
    fn wake(self: Arc<Self>) {
        let _ = self.0.send(());
    }
}

fn block_on_std<F: Future>(future: F) -> F::Output {
    let (sender, receiver) = mpsc::channel();
    let waker = Waker::from(Arc::new(WakeSignal(sender)));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => receiver.recv().expect("the future must wake the thread"),
        }
    }
}

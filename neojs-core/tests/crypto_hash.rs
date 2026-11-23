//! Integration tests for the `crypto` hash and HMAC surface through the public
//! `Runtime`.
//!
//! Each test runs a real ECMAScript module with its own assertions, so it
//! exercises the module registry, the ESM bindings, the host ops, the
//! typed-array handling, the bounded worker pool, and the JavaScript lifecycle
//! together. The Rust unit tests in `neojs-core/src/crypto_hash.rs` cover the
//! SHA-2/HMAC primitives, the encoders, and the pending-byte counter.

use std::fs;
use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc;
use std::task::Context;
use std::task::Poll;
use std::task::Wake;
use std::task::Waker;

use neojs_core::Runtime;

/// Writes a module into a fresh temporary directory and runs it.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const SHA384_ABC: &str = "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7";
const SHA512_ABC: &str = "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f";

#[test]
fn hash_matches_published_vectors_incrementally_and_one_shot() {
    run_js(&format!(
        r#"
import {{ createHash }} from "neo:crypto";

const vectors = [
    ["sha256", "", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"],
    ["sha256", "abc", "{sha256}"],
    ["sha256", "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"],
    ["sha384", "abc", "{sha384}"],
    ["sha384", "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
        "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039"],
    ["sha384", "", "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"],
    ["sha512", "abc", "{sha512}"],
    ["sha512", "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
        "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909"],
    ["sha512", "", "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"],
];

for (const [algorithm, message, expected] of vectors) {{
    const oneShot = createHash(algorithm).update(message).digest("hex");
    if (oneShot !== expected) throw new Error(algorithm + " one-shot: " + oneShot);

    // Feed the same message in uneven chunks, including a zero-length update.
    const incremental = createHash(algorithm);
    incremental.update("");
    let offset = 0;
    let width = 1;
    while (offset < message.length) {{
        const end = Math.min(offset + width, message.length);
        incremental.update(message.slice(offset, end));
        offset = end;
        width = (width * 3 + 1) % 97 + 1;
    }}
    const split = incremental.digest("hex");
    if (split !== expected) throw new Error(algorithm + " incremental: " + split);
}}

// Digest encodings for SHA-256("abc"), independently generated with Python's
// hashlib and base64 for this repository.
const digest = createHash("sha256").update("abc").digest("base64");
if (digest !== "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=") throw new Error("base64: " + digest);
const url = createHash("sha256").update("abc").digest("base64url");
if (url !== "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0") throw new Error("base64url: " + url);
const latin1 = createHash("sha256").update("abc").digest("latin1");
if (latin1.length !== 32) throw new Error("latin1 length");
const raw = createHash("sha256").update("abc").digest();
if (!(raw instanceof Uint8Array) || raw.length !== 32) throw new Error("raw digest");
if (raw.buffer === undefined) throw new Error("raw digest has no buffer");
for (let index = 0; index < raw.length; index += 1) {{
    if (latin1.charCodeAt(index) !== raw[index]) throw new Error("latin1 byte " + index);
}}
const binaryAlias = createHash("sha256").update("abc").digest("binary");
if (binaryAlias !== latin1) throw new Error("binary alias");

// Case-insensitive algorithm and encoding names.
if (createHash("SHA256").update("abc").digest("HEX") !== "{sha256}") {{
    throw new Error("case-insensitive names");
}}

// The million-'a' FIPS vector exercises the internal block boundary.
const million = createHash("sha256");
for (let index = 0; index < 1000; index += 1) million.update("a".repeat(1000));
if (million.digest("hex") !== "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0") {{
    throw new Error("million-a vector");
}}
"#,
        sha256 = SHA256_ABC,
        sha384 = SHA384_ABC,
        sha512 = SHA512_ABC,
    ));
}

#[test]
fn hash_accepts_strings_binary_views_and_resizable_buffers() {
    run_js(
        r#"
import { createHash } from "neo:crypto";

// UTF-8 for 'Grüße, 世界, 🦀'. The string path must agree with the byte path.
const text = "Grüße, 世界, 🦀";
const utf8 = Uint8Array.from([
    0x47, 0x72, 0xc3, 0xbc, 0xc3, 0x9f, 0x65, 0x2c, 0x20,
    0xe4, 0xb8, 0x96, 0xe7, 0x95, 0x8c, 0x2c, 0x20,
    0xf0, 0x9f, 0xa6, 0x80,
]);
const fromText = createHash("sha256").update(text).digest("hex");
const fromBytes = createHash("sha256").update(utf8).digest("hex");
if (fromText !== fromBytes) throw new Error("string and bytes differ");

// Lone surrogates are replaced by U+FFFD, as Node's UTF-8 encoding does.
const replacement = createHash("sha256").update("\ud800").digest("hex");
const expectedReplacement = createHash("sha256").update(Uint8Array.from([0xef, 0xbf, 0xbd])).digest("hex");
if (replacement !== expectedReplacement) throw new Error("lone surrogate");

// Every byte value, including NUL.
const all = new Uint8Array(256);
for (let index = 0; index < 256; index += 1) all[index] = index;
const allDigest = createHash("sha512").update(all).digest("hex");
if (allDigest.length !== 128) throw new Error("all bytes");

// An offset view hashes only its own range.
const backing = Uint8Array.from([0xaa, 0x62, 0x63, 0xff]);
const view = backing.subarray(1, 3);
const viewDigest = createHash("sha256").update(view).digest("hex");
const abc = createHash("sha256").update("bc").digest("hex");
if (viewDigest !== abc) throw new Error("offset view");

// A SharedArrayBuffer-backed view.
const shared = new SharedArrayBuffer(8);
const sharedView = new Uint8Array(shared, 2, 3);
sharedView.set([0x61, 0x62, 0x63]);
if (createHash("sha256").update(sharedView).digest("hex") !==
    createHash("sha256").update("abc").digest("hex")) {
    throw new Error("shared view");
}

// A resizable buffer, including growth and a length-tracking view.
const resizable = new ArrayBuffer(4, { maxByteLength: 16 });
const resizableView = new Uint8Array(resizable, 1, 2);
resizableView.set([0x61, 0x62]);
resizable.resize(8);
if (createHash("sha256").update(resizableView).digest("hex") !==
    createHash("sha256").update("ab").digest("hex")) {
    throw new Error("resizable view");
}
const tracking = new Uint8Array(resizable);
resizable.resize(12);
if (tracking.length !== 12) throw new Error("tracking view did not grow");
const trackingDigest = createHash("sha256").update(tracking).digest("hex");
if (trackingDigest.length !== 64) throw new Error("tracking view");

// Shrinking until a fixed-length view is out of bounds exposes a zero-length
// view; it hashes as empty rather than reading stale memory.
resizable.resize(1);
if (resizableView.length !== 0) throw new Error("out-of-bounds view is not empty");
if (createHash("sha256").update(resizableView).digest("hex") !==
    createHash("sha256").update("").digest("hex")) {
    throw new Error("out-of-bounds view");
}

// Empty input still produces the empty-message digest.
const empty = createHash("sha384");
if (empty.digest("hex") !== "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b") {
    throw new Error("empty digest");
}
"#,
    );
}

#[test]
fn hash_has_a_strict_lifecycle_and_argument_contract() {
    run_js(
        r#"
import { createHash, createHmac } from "neo:crypto";

function expectTypeError(fn, label) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw new Error(label + ": not TypeError: " + error);
        if ("code" in error) throw new Error(label + ": TypeError carried a code");
        return;
    }
    throw new Error(label + ": expected TypeError");
}
function expectCode(fn, code, label) {
    try {
        fn();
    } catch (error) {
        if (error.code !== code) throw new Error(label + ": expected " + code + ", got " + error.code);
        return;
    }
    throw new Error(label + ": expected " + code);
}

// Factories are strict about arity and primitive types, and never coerce.
let coerced = false;
const hook = { toString() { coerced = true; return "sha256"; } };
expectTypeError(() => createHash(), "createHash no args");
expectTypeError(() => createHash("sha256", "extra"), "createHash extra");
expectTypeError(() => createHash(123), "createHash number");
expectTypeError(() => createHash(null), "createHash null");
expectTypeError(() => createHash(hook), "createHash object");
expectTypeError(() => createHash(new String("sha256")), "createHash boxed string");
if (coerced) throw new Error("createHash coerced its argument");
expectTypeError(() => createHmac("sha256"), "createHmac no key");
expectTypeError(() => createHmac("sha256", "k", "extra"), "createHmac extra");
expectTypeError(() => createHmac("sha256", 42), "createHmac number key");
expectTypeError(() => createHmac("sha256", null), "createHmac null key");
expectTypeError(() => createHmac(42, "k"), "createHmac number algorithm");

expectCode(() => createHash("sha1"), "NEO_CRYPTO_ALGORITHM", "sha1");
expectCode(() => createHash("sha224"), "NEO_CRYPTO_ALGORITHM", "sha224");
expectCode(() => createHash("sha2-256"), "NEO_CRYPTO_ALGORITHM", "sha2-256");
expectCode(() => createHash(""), "NEO_CRYPTO_ALGORITHM", "empty name");

const hash = createHash("sha256");
expectTypeError(() => hash.update(), "update no args");
expectTypeError(() => hash.update(undefined), "update undefined");
expectTypeError(() => hash.update(null), "update null");
expectTypeError(() => hash.update(42), "update number");
expectTypeError(() => hash.update([1, 2]), "update array");
expectTypeError(() => hash.update("abc", "hex"), "update hex input");
expectTypeError(() => hash.update("abc", 5), "update numeric encoding");
expectTypeError(() => hash.update(new Uint8Array([1]), "utf8"), "update encoding with bytes");
expectTypeError(() => hash.update("abc", "UTF-8", "extra"), "update extra");
let callbackCalled = false;
expectTypeError(() => hash.update("abc", () => { callbackCalled = true; }), "update callback");
if (callbackCalled) throw new Error("update invoked a callback");

// `update` chains, an invalid digest encoding does not consume the state, and
// an explicit undefined encoding means raw bytes.
if (hash.update("a").update("b").update("c") !== hash) throw new Error("update did not chain");
expectTypeError(() => hash.digest("bogus"), "digest bogus encoding");
expectTypeError(() => hash.digest("hex", "extra"), "digest extra");
const beforeSeal = createHash("sha256");
beforeSeal.update("abc");
const first = beforeSeal.digest("hex");
expectCode(() => beforeSeal.digest("hex"), "NEO_CRYPTO_DIGESTED", "second digest");
expectCode(() => beforeSeal.update("x"), "NEO_CRYPTO_DIGESTED", "update after digest");

// close()/dispose() release the state without a digest, are idempotent, and
// make later operations fail with a distinct code.
const closable = createHash("sha256");
closable.update("abc");
closable.close();
expectCode(() => closable.update("x"), "NEO_CRYPTO_CLOSED", "update after close");
expectCode(() => closable.digest(), "NEO_CRYPTO_CLOSED", "digest after close");
closable.close();
expectTypeError(() => closable.close("x"), "close argument");
const disposable = createHash("sha256");
disposable.dispose();
expectCode(() => disposable.digest(), "NEO_CRYPTO_CLOSED", "digest after dispose");
expectTypeError(() => disposable.dispose("x"), "dispose argument");
const finished = createHash("sha256").update("abc");
finished.digest("hex");
finished.close();
finished.close();

// The hash class is not constructible without the private token.
const Hash = Object.getPrototypeOf(createHash("sha256")).constructor;
if (Hash.name !== "Hash") throw new Error("unexpected class name");
expectTypeError(() => new Hash(), "direct construction");
expectTypeError(() => new Hash(Symbol("t"), 1), "forged token");

// HMAC with an empty key is defined and produces a digest.
if (createHmac("sha256", "").update("").digest("hex").length !== 64) throw new Error("empty hmac key");
if (createHmac("sha256", new Uint8Array(0)).update("").digest("hex").length !== 64) throw new Error("empty hmac bytes");
"#,
    );
}

#[test]
fn hash_rejects_non_uint8_arrays_detached_views_and_javascript_hooks() {
    run_js(
        r#"
import { createHash, createHmac } from "neo:crypto";

function expectTypeError(fn, label) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw new Error(label + ": " + error);
        return;
    }
    throw new Error(label + ": expected TypeError");
}

const key = new Uint8Array(32);
const empty = new Uint8Array(0);

// These have a Uint8Array-looking shape but are not one; V8's internal check
// rejects each without consulting Symbol.hasInstance or a getter.
for (const fake of [
    Object.create(Uint8Array.prototype),
    new Proxy(new Uint8Array(1), {}),
    new Uint8ClampedArray(1),
    new Int8Array(1),
    new DataView(new ArrayBuffer(1)),
    [1, 2, 3],
]) {
    expectTypeError(() => createHash("sha256").update(fake), "fake input");
    expectTypeError(() => createHmac("sha256", fake), "fake key");
}

// Detached views are rejected rather than treated as empty.
function detached(length) {
    const array = new Uint8Array(length);
    array.buffer.transfer();
    return array;
}
for (const call of [
    () => createHash("sha256").update(detached(4)),
    () => createHmac("sha256", detached(32)),
]) {
    try {
        call();
        throw new Error("accepted a detached view");
    } catch (error) {
        if (error.code !== "NEO_CRYPTO_DETACHED") throw error;
    }
}

// Validation hooks on the real prototype and instances are not consulted.
let calls = 0;
const RealUint8Array = Uint8Array;
Object.defineProperty(RealUint8Array, Symbol.hasInstance, {
    value() { calls += 1; return true; },
});
const real = new RealUint8Array([0x61, 0x62, 0x63]);
Object.defineProperty(real, "byteLength", { get() { calls += 1; return 3; } });
const digest = createHash("sha256").update(real).digest("hex");
if (calls !== 0) throw new Error("validation hook was invoked");
if (digest !== "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") {
    throw new Error("hooked digest");
}

// Replacing Deno.core.ops after the shim captured them changes nothing.
let replaced = 0;
Deno.core.ops.op_neo_crypto_hash_create = function () {
    replaced += 1;
    return { ok: true, id: 1 };
};
Deno.core.ops.op_neo_crypto_hash_update = function () {
    replaced += 1;
    return { ok: true };
};
if (createHash("sha256").update("abc").digest("hex") !==
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") {
    throw new Error("op replacement");
}
if (replaced) throw new Error("a replaced op was called");

// Stable codes survive an inherited `code` setter.
Object.defineProperty(Object.prototype, "code", {
    __proto__: null,
    set() { throw new Error("inherited code setter ran"); },
    configurable: true,
});
try {
    createHash("sha1");
    throw new Error("unexpected success");
} catch (error) {
    if (error.code !== "NEO_CRYPTO_ALGORITHM") throw new Error("stable code lost");
}
delete Object.prototype.code;
"#,
    );
}

#[test]
fn hash_enforces_input_and_key_bounds_without_partial_updates() {
    run_js(
        r#"
import { createHash, createHmac } from "neo:crypto";

const MAX = 16 * 1024 * 1024;
const KEY_MAX = 1024 * 1024;
const key = new Uint8Array(32);

// Exactly at the input bound is accepted; one byte over is rejected before the
// bytes are added.
const big = new Uint8Array(MAX);
const atLimit = createHash("sha256").update(big).digest("hex");
if (atLimit.length !== 64) throw new Error("input at limit");
try {
    createHash("sha256").update(new Uint8Array(MAX + 1));
    throw new Error("oversized input accepted");
} catch (error) {
    if (error.code !== "NEO_CRYPTO_LIMIT") throw error;
}

// The aggregate across updates is bounded, and a rejected update does not add
// a prefix.
const half = new Uint8Array(MAX / 2);
const state = createHash("sha256").update(half);
try {
    state.update(new Uint8Array(MAX / 2 + 1));
    throw new Error("over-aggregate update accepted");
} catch (error) {
    if (error.code !== "NEO_CRYPTO_LIMIT") throw error;
}
const expected = createHash("sha256").update(half).digest("hex");
if (state.digest("hex") !== expected) throw new Error("partial update changed the digest");

// HMAC keys are bounded at 1 MiB.
if (createHmac("sha256", new Uint8Array(KEY_MAX)).update("x").digest("hex").length !== 64) {
    throw new Error("key at limit");
}
try {
    createHmac("sha256", new Uint8Array(KEY_MAX + 1));
    throw new Error("oversized key accepted");
} catch (error) {
    if (error.code !== "NEO_CRYPTO_LIMIT") throw error;
}
try {
    createHmac("sha256", "x".repeat(KEY_MAX + 1));
    throw new Error("oversized string key accepted");
} catch (error) {
    if (error.code !== "NEO_CRYPTO_LIMIT") throw error;
}
"#,
    );
}

#[test]
fn incremental_objects_are_capped_and_explicitly_disposable() {
    run_js(&format!(
        r#"
import {{ createHash, createHmac }} from "neo:crypto";

const cap = 1024;
const objects = [];
for (let index = 0; index < cap; index += 1) {{
    objects.push(createHash("sha256"));
}}

function expectTooMany(fn, label) {{
    try {{
        fn();
    }} catch (error) {{
        if (error.code !== "NEO_CRYPTO_TOO_MANY_OBJECTS") throw error;
        return;
    }}
    throw new Error(label + ": expected NEO_CRYPTO_TOO_MANY_OBJECTS");
}}
expectTooMany(() => createHash("sha256"), "hash over cap");
expectTooMany(() => createHmac("sha256", "key"), "hmac over cap");

// Closing one releases exactly one slot, for a hash or an HMAC.
objects[0].close();
const replacement = createHmac("sha256", "key");
replacement.close();

// Close the rest and confirm the cap is fully released.
for (const object of objects) object.close();
const after = createHash("sha256").update("abc").digest("hex");
if (after !== "{sha256}") throw new Error("create after closing all: " + after);

// Closed objects cannot be updated or digested.
let closed = createHash("sha256");
closed.close();
for (const call of [() => closed.update("x"), () => closed.digest()]) {{
    try {{
        call();
        throw new Error("closed object was usable");
    }} catch (error) {{
        if (error.code !== "NEO_CRYPTO_CLOSED") throw error;
    }}
}}
"#,
        sha256 = SHA256_ABC,
    ));
}

#[test]
fn hmac_matches_rfc4231_and_copies_its_key_and_input() {
    run_js(
        r#"
import { createHash, createHmac } from "neo:crypto";

function repeat(byte, count) {
    return new Uint8Array(count).fill(byte);
}
function expect(label, actual, expected) {
    if (actual !== expected) throw new Error(label + ": " + actual);
}

// RFC 4231 test cases 1-3, for all three hashes.
const cases = [
    ["sha256", repeat(0x0b, 20), "Hi There",
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"],
    ["sha256", "Jefe", "what do ya want for nothing?",
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"],
    ["sha256", repeat(0xaa, 20), repeat(0xdd, 50),
        "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"],
    ["sha384", repeat(0x0b, 20), "Hi There",
        "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6"],
    ["sha384", "Jefe", "what do ya want for nothing?",
        "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e8e2240ca5e69e2c78b3239ecfab21649"],
    ["sha384", repeat(0xaa, 20), repeat(0xdd, 50),
        "88062608d3e6ad8a0aa2ace014c8a86f0aa635d947ac9febe83ef4e55966144b2a5ab39dc13814b94e3ab6e101a34f27"],
    ["sha512", repeat(0x0b, 20), "Hi There",
        "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"],
    ["sha512", "Jefe", "what do ya want for nothing?",
        "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"],
    ["sha512", repeat(0xaa, 20), repeat(0xdd, 50),
        "fa73b0089d56a284efb0f0756c890be9b1b5dbdd8ee81a3655f83e33b2279d39bf3e848279a722c806b485a47e67c807b946a337bee8942674278859e13292fb"],
];
for (const [algorithm, key, data, expected] of cases) {
    const oneShot = createHmac(algorithm, key).update(data).digest("hex");
    expect(algorithm + " hmac", oneShot, expected);
}

// The key is copied at construction: mutating the caller's view does not
// change the MAC.
const keyBytes = repeat(0x0b, 20);
const mac = createHmac("sha256", keyBytes);
const expected = createHmac("sha256", repeat(0x0b, 20)).update("Hi There").digest("hex");
keyBytes.fill(0xff);
expect("key copy", mac.update("Hi There").digest("hex"), expected);

// Input is copied at update: mutating it afterwards does not change the MAC.
const data = new Uint8Array([0x48, 0x69]);
const copied = createHmac("sha256", "Jefe");
copied.update(data);
data.fill(0);
expect(
    "input copy",
    copied.digest("hex"),
    createHmac("sha256", "Jefe").update("Hi").digest("hex"),
);

// Incremental HMAC equals the one-shot HMAC.
const incremental = createHmac("sha512", "Jefe");
incremental.update("what do ");
incremental.update("ya want ");
incremental.update("for nothing?");
expect("incremental hmac", incremental.digest("hex"),
    "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737");

// A string key is UTF-8, not Latin-1.
const unicodeKey = createHmac("sha256", "Grüße").update("x").digest("hex");
const unicodeBytes = createHash("sha256").update("Grüße").digest("hex");
if (unicodeKey === unicodeBytes) throw new Error("hmac and hash collided");
"#,
    );
}

#[test]
fn async_hash_and_hmac_match_the_sync_results() {
    run_js(&format!(
        r#"
import {{ createHash, createHmac, hashAsync, hmacAsync }} from "neo:crypto";

function expect(label, actual, expected) {{
    if (actual !== expected) throw new Error(label + ": " + actual);
}}

// The return value is a real Promise, not a thenable over a blocking call.
const promise = hashAsync("sha256", "abc", "hex");
if (!(promise instanceof Promise)) throw new Error("hashAsync did not return a Promise");
expect("async sha256", await promise, "{sha256}");

expect("async sha384", await hashAsync("sha384", "abc", "hex"), "{sha384}");
expect("async sha512 base64", await hashAsync("sha512", "abc", "base64"),
    "3a81oZNherrMQXNJriBBMRLm+k6JqX6iCp7u5ktV05ohkpkqJ0/BqDa6PCOj/uu9RU1EI2Q86A4qmslPpUyknw==");

// Raw bytes and latin1 decoding agree with the synchronous encoder.
const raw = await hashAsync("sha256", "abc");
if (!(raw instanceof Uint8Array) || raw.length !== 32) throw new Error("async raw digest");
const latin1 = await hashAsync("sha256", "abc", "latin1");
for (let index = 0; index < raw.length; index += 1) {{
    if (latin1.charCodeAt(index) !== raw[index]) throw new Error("async latin1 " + index);
}}

// Async HMAC matches the sync path, including a binary key and a binary input.
const key = new Uint8Array(20).fill(0x0b);
const data = Uint8Array.from([0x48, 0x69, 0x20, 0x54, 0x68, 0x65, 0x72, 0x65]); // "Hi There"
const expected = createHmac("sha256", key).update(data).digest("hex");
expect("async hmac", await hmacAsync("sha256", key, data, "hex"), expected);

// The input is copied before the promise is returned, so mutation after the
// call does not change the result.
const big = new Uint8Array(1024 * 1024);
for (let index = 0; index < big.length; index += 1) big[index] = index & 255;
const expectedBig = createHash("sha256").update(big).digest("hex");
const pending = hashAsync("sha256", big, "hex");
big.fill(0);
expect("async input copy", await pending, expectedBig);

// Argument failures reject with the documented code, and a later call still
// works (the runtime recovers).
try {{
    await hashAsync("sha1", "abc");
    throw new Error("unsupported algorithm accepted");
}} catch (error) {{
    if (error.code !== "NEO_CRYPTO_ALGORITHM") throw error;
}}
try {{
    await hashAsync("sha256", "abc", "utf8");
    throw new Error("input encoding accepted as an output encoding");
}} catch (error) {{
    if (!(error instanceof TypeError)) throw error;
}}
try {{
    await hmacAsync("sha256", "k", "data", 42);
    throw new Error("numeric encoding accepted");
}} catch (error) {{
    if (!(error instanceof TypeError)) throw error;
}}
try {{
    await hashAsync("sha256", new Uint8Array(16 * 1024 * 1024 + 1));
    throw new Error("oversized async input accepted");
}} catch (error) {{
    if (error.code !== "NEO_CRYPTO_LIMIT") throw error;
}}
try {{
    await hmacAsync("sha256", new Uint8Array(1024 * 1024 + 1), "data");
    throw new Error("oversized async key accepted");
}} catch (error) {{
    if (error.code !== "NEO_CRYPTO_LIMIT") throw error;
}}
expect("recovery", await hashAsync("sha256", "abc", "hex"), "{sha256}");
"#,
        sha256 = SHA256_ABC,
        sha384 = SHA384_ABC,
    ));
}

#[test]
fn async_hash_is_nonblocking_and_runs_concurrently() {
    run_js(
        r#"
import { createHash, hashAsync } from "neo:crypto";

// Responsiveness: the hash promise must still be pending when the first
// microtask queued after the call runs. A Promise.resolve(blockingHash())
// wrapper would already be resolved, queueing its reaction first.
let resolved = false;
const big = new Uint8Array(8 * 1024 * 1024);
for (let index = 0; index < big.length; index += 1) big[index] = (index * 31 + 7) & 255;
const pending = hashAsync("sha256", big, "hex").then((value) => {
    resolved = true;
    return value;
});
const order = [];
queueMicrotask(() => { order.push(resolved ? "after" : "before"); });
const value = await pending;
if (order[0] !== "before") throw new Error("the hash blocked the isolate: " + order[0]);
if (value !== createHash("sha256").update(big).digest("hex")) throw new Error("responsive digest");

// Several jobs run concurrently and every result matches the sync path.
const inputs = [
    "a",
    "bc",
    "def",
    "ghij",
    "klmno",
    new Uint8Array([0, 1, 2, 255]),
    "Grüße, 世界, 🦀",
];
const algorithms = ["sha256", "sha384", "sha512"];
const promises = [];
for (let index = 0; index < inputs.length; index += 1) {
    promises.push(hashAsync(algorithms[index % 3], inputs[index], "hex"));
}
const results = await Promise.all(promises);
for (let index = 0; index < inputs.length; index += 1) {
    const expected = createHash(algorithms[index % 3]).update(inputs[index]).digest("hex");
    if (results[index] !== expected) throw new Error("concurrent result " + index);
}

// An abandoned promise does not wedge the event loop or the worker pool.
hashAsync("sha512", new Uint8Array(1024 * 1024));
if (await hashAsync("sha256", "abc", "hex") !==
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") {
    throw new Error("hash after abandonment");
}
"#,
    );
}

#[test]
fn async_hash_reports_the_pending_bound_without_hanging() {
    run_js(
        r#"
import { hashAsync } from "neo:crypto";

// Four 16 MiB jobs fill the 64 MiB pending allowance. The fifth may race with
// a completed worker, so accept either success or the documented busy error,
// but never another failure and never a hang.
const big = new Uint8Array(16 * 1024 * 1024);
const outcomes = [];
for (let index = 0; index < 5; index += 1) {
    outcomes.push(hashAsync("sha512", big, "hex").then(
        (value) => ({ value }),
        (error) => ({ error }),
    ));
}
let successes = 0;
let busy = 0;
for (const outcome of await Promise.all(outcomes)) {
    if (outcome.error) {
        if (outcome.error.code !== "NEO_CRYPTO_BUSY") throw outcome.error;
        busy += 1;
    } else {
        if (outcome.value.length !== 128) throw new Error("unexpected digest length");
        successes += 1;
    }
}
if (successes + busy !== 5) throw new Error("lost a submission");
if (successes < 1) throw new Error("no submission completed");
"#,
    );
}

#[test]
fn async_reservations_release_between_batches() {
    run_js(
        r#"
import { hashAsync } from "neo:crypto";

const EMPTY = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

// Eight batches of sixteen empty jobs exceed the 64-job cap in total. Every
// batch must succeed, which proves the job reservation is released when a
// worker finishes.
for (let round = 0; round < 8; round += 1) {
    const promises = [];
    for (let index = 0; index < 16; index += 1) {
        promises.push(hashAsync("sha256", new Uint8Array(0), "hex"));
    }
    const values = await Promise.all(promises);
    for (const value of values) {
        if (value !== EMPTY) throw new Error("empty batch result");
    }
}

// Eight sequential 16 MiB jobs exceed the 64 MiB byte allowance in total.
const big = new Uint8Array(16 * 1024 * 1024);
for (let round = 0; round < 8; round += 1) {
    const value = await hashAsync("sha256", big, "hex");
    if (value.length !== 64) throw new Error("big batch result");
}
"#,
    );
}

#[test]
fn unawaited_async_result_is_not_retained_in_the_resource_table() {
    run_js(
        r#"
import { hashAsync } from "neo:crypto";

// The wrapper calls the join op immediately, which removes the job resource.
// The completed digest belongs to the promise, not the resource table.
const abandoned = hashAsync("sha256", "abc", "hex");
abandoned.catch(() => {});
await hashAsync("sha256", "abc", "hex");
const names = Object.values(Deno.core.resources());
for (const name of names) {
    if (name === "neo::crypto::AsyncDigest") {
        throw new Error("an async job resource was retained");
    }
}
"#,
    );
}

#[test]
fn hash_exports_are_the_registry_functions_and_resolve_canonically() {
    run_js(
        r#"
import * as crypto from "neo:crypto";
import { createHash, createHmac, hashAsync, hmacAsync } from "neo:crypto";

const registry = Neo.getBuiltInModule("neo:crypto");
for (const name of ["createHash", "createHmac", "hashAsync", "hmacAsync"]) {
    if (typeof registry[name] !== "function") throw new Error(name + " missing");
    if (crypto[name] !== registry[name]) throw new Error(name + " identity");
}
if (createHash !== registry.createHash) throw new Error("named createHash identity");
if (createHmac !== registry.createHmac) throw new Error("named createHmac identity");
if (hashAsync !== registry.hashAsync) throw new Error("named hashAsync identity");
if (hmacAsync !== registry.hmacAsync) throw new Error("named hmacAsync identity");
if ("default" in crypto) throw new Error("crypto must not have a default export");
if (import.meta.resolve("neo:crypto") !== "neo:crypto") throw new Error("resolve");
if (import.meta.resolve("neo:crypto") !== "neo:crypto") throw new Error("alias resolve");
if (Object.keys(registry).length !== 20) throw new Error("unexpected export count");
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

#[test]
fn async_hash_without_a_tokio_event_loop_reports_no_executor() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { createHash, hashAsync } from "neo:crypto";
// The synchronous API still works.
if (createHash("sha256").update("abc").digest("hex") !==
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") {
    throw new Error("sync hash failed");
}
try {
    await hashAsync("sha256", "abc", "hex");
    throw new Error("unexpected success");
} catch (error) {
    if (error.code !== "NEO_CRYPTO_NO_EXECUTOR") throw new Error("wrong code: " + error.code);
}
"#,
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    block_on_std(runtime.run_file_async(&file)).expect("module assertions");
}

#[test]
fn async_hash_rejects_a_multi_thread_caller_instead_of_aborting() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { hashAsync } from "neo:crypto";
try {
    await hashAsync("sha256", "abc", "hex");
    throw new Error("unexpected success");
} catch (error) {
    if (error.code !== "NEO_CRYPTO_NO_EXECUTOR") throw new Error("wrong code: " + error.code);
}
"#,
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    let caller = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_time()
        .build()
        .expect("caller runtime");
    caller
        .block_on(async { runtime.run_file_async(&file).await })
        .expect("module assertions");
}

#[test]
fn async_hash_works_under_a_tokio_current_thread_caller() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
import { hmacAsync, hashAsync } from "neo:crypto";
const value = await hashAsync("sha256", "abc", "hex");
if (value !== "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") {
    throw new Error("async sha256: " + value);
}
const mac = await hmacAsync("sha256", "Jefe", "what do ya want for nothing?", "hex");
if (mac !== "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843") {
    throw new Error("async hmac: " + mac);
}
"#,
    )
    .expect("write source");

    let mut runtime = Runtime::new().expect("runtime");
    let caller = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("caller runtime");
    caller
        .block_on(async { runtime.run_file_async(&file).await })
        .expect("module assertions");
}

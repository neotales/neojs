//! Integration tests for the `crypto` built-in through the public `Runtime`.
//!
//! Every test runs a real ECMAScript module with its own assertions, so it
//! exercises the registry, the module source, the host ops, and the browser
//! style `Uint8Array` handling together. The Rust unit tests in
//! `neojs-core/src/crypto.rs` cover the AES-256-GCM primitive and the envelope
//! internals.

use std::fs;

use neojs_core::Runtime;

/// Writes a module into a fresh temporary directory and runs it.
fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

/// Runs a module that must fail, returning the reported error.
fn run_js_error(source: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime
        .run_file(&file)
        .expect_err("module must fail")
        .to_string()
}

#[test]
fn crypto_is_available_and_roundtrips_bytes() {
    run_js(
        r#"
import { decrypt, encrypt, generateEnvelopeKey } from "neo:crypto";

const registry = Neo.getBuiltInModule("crypto");
if (typeof registry !== "object" || registry === null) throw Error("missing registry module");
if (Neo.getBuiltInModule("crypto") !== registry) throw Error("registry lookup is not cached");
if (!Object.isFrozen(registry)) throw Error("registry module is not frozen");
for (const name of ["decrypt", "encrypt", "generateEnvelopeKey", "randomBytes"]) {
    if (typeof registry[name] !== "function") throw Error(`${name} is not a function`);
}
if ("__neoCryptoEncrypt" in globalThis || "__neoCryptoDecrypt" in globalThis ||
    "__neoCryptoGenerateKey" in globalThis) {
    throw Error("global callback exposed");
}
if (typeof fs !== "undefined" || typeof env !== "undefined") throw Error("capability leaked");

const key = generateEnvelopeKey();
if (!(key instanceof Uint8Array)) throw Error("key is not a Uint8Array");
if (key.length !== 32) throw Error("key length");

const plaintext = Uint8Array.from([0, 1, 2, 0, 255, 128, 7]);
const envelope = encrypt(key, plaintext);
if (!(envelope instanceof Uint8Array)) throw Error("envelope is not a Uint8Array");
if (envelope.length !== plaintext.length + 34) throw Error("envelope length");

const returned = decrypt(key, envelope);
if (!(returned instanceof Uint8Array)) throw Error("plaintext is not a Uint8Array");
if (returned.length !== plaintext.length) throw Error("plaintext length");
for (let i = 0; i < plaintext.length; i++) {
    if (returned[i] !== plaintext[i]) throw Error("plaintext byte " + i);
}
if (key.length !== 32) throw Error("key was mutated");

// This slice is synchronous. None of these results may be a thenable, because
// Promise-wrapping blocking work would be a fake async API.
for (const value of [key, envelope, returned, registry.randomBytes(4)]) {
    if (typeof value.then === "function" || value instanceof Promise) {
        throw Error("a synchronous function returned a thenable");
    }
}
"#,
    );
}

#[test]
fn crypto_rejects_unsupported_argument_counts_and_reserves_generate_key() {
    run_js(
        r#"
import * as crypto from "neo:crypto";
const registry = Neo.getBuiltInModule("crypto");

// The envelope generator is not Node's generateKey; the name is reserved for
// a future Node-compatible surface and must not appear here.
if ("generateKey" in registry) throw Error("generateKey must not be exported");
if ("generateKey" in crypto) throw Error("generateKey appeared in the namespace");
if (typeof registry.generateEnvelopeKey !== "function") {
    throw Error("missing generateEnvelopeKey");
}

function expectTypeError(fn, label) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
        if ("code" in error) throw Error(label + ": unexpected code");
        return;
    }
    throw Error(label + ": expected TypeError");
}

expectTypeError(() => registry.generateEnvelopeKey(undefined), "generateEnvelopeKey");
expectTypeError(() => registry.generateEnvelopeKey(1), "generateEnvelopeKey");
expectTypeError(() => registry.randomBytes(), "randomBytes missing");
expectTypeError(() => registry.randomBytes(4, undefined), "randomBytes second argument");
let called = false;
expectTypeError(
    () => registry.randomBytes(4, () => { called = true; }),
    "randomBytes callback",
);
if (called) throw Error("randomBytes invoked a callback");

const key = registry.generateEnvelopeKey();
const plaintext = new Uint8Array([1]);
expectTypeError(() => registry.encrypt(key), "encrypt missing");
expectTypeError(
    () => registry.encrypt(key, plaintext, undefined, undefined),
    "encrypt fourth argument",
);
expectTypeError(
    () => registry.encrypt(key, plaintext, undefined, () => {}),
    "encrypt callback",
);
expectTypeError(() => registry.decrypt(key), "decrypt missing");
expectTypeError(
    () => registry.decrypt(key, registry.encrypt(key, plaintext), undefined, undefined),
    "decrypt fourth argument",
);

// The documented arities still work, including explicit undefined AAD.
if (registry.encrypt(key, plaintext).length !== 35) throw Error("encrypt two args");
if (registry.encrypt(key, plaintext, undefined).length !== 35) {
    throw Error("encrypt three args");
}
if (registry.decrypt(key, registry.encrypt(key, plaintext)).length !== 1) {
    throw Error("decrypt two args");
}
if (registry.decrypt(key, registry.encrypt(key, plaintext), undefined).length !== 1) {
    throw Error("decrypt three args");
}
"#,
    );
}

#[test]
fn crypto_generate_key_is_not_exported() {
    let message = run_js_error("import { generateKey } from \"neo:crypto\";\n");
    assert!(
        message.contains("does not provide an export named 'generateKey'"),
        "{message}"
    );
    assert!(message.contains("neo:crypto"), "{message}");
}

#[test]
fn crypto_module_specifiers_share_one_module_instance() {
    run_js(
        r#"
import * as staticCrypto from "crypto";

const registry = Neo.getBuiltInModule("crypto");
const first = await import("crypto");
const alias = await import("neo:crypto");
const uppercase = await import("NEO:crypto");
const again = await import("crypto");

if (staticCrypto !== first) throw Error("static and dynamic import differ");
if (first !== alias) throw Error("crypto and neo:crypto differ");
if (alias !== uppercase) throw Error("scheme matching is not case-insensitive");
if (alias !== again) throw Error("repeated dynamic import was not cached");
if (staticCrypto.encrypt !== registry.encrypt) throw Error("registry function differs");
if ("default" in staticCrypto) throw Error("crypto must not have a default export");
if (import.meta.resolve("crypto") !== "neo:crypto") throw Error("crypto did not resolve canonically");
if (import.meta.resolve("neo:crypto") !== "neo:crypto") throw Error("alias did not resolve canonically");
"#,
    );
    // A second importer in a different module must see the same function
    // object, because both modules read the one registry object.
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(
        dir.path().join("consumer.js"),
        "import { encrypt } from \"neo:crypto\";\nexport { encrypt };\n",
    )
    .expect("write consumer");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        "import { encrypt as a } from \"crypto\";\nimport { encrypt as b } from \"./consumer.js\";\nif (a !== b) throw new Error(\"cross-module identity failed\");\n",
    )
    .expect("write main");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("cross-module identity");
}

#[test]
fn crypto_handles_empty_unicode_binary_and_large_payloads() {
    run_js(
        r#"
import { decrypt, encrypt, generateEnvelopeKey } from "neo:crypto";
const key = generateEnvelopeKey();
const aad = Uint8Array.from([0x63, 0x6f, 0x6e, 0x74, 0x65, 0x78, 0x74]);

// UTF-8 for 'Grüße, 世界, 🦀', including two-, three-, and four-byte sequences.
const unicode = Uint8Array.from([
    0x47, 0x72, 0xc3, 0xbc, 0xc3, 0x9f, 0x65, 0x2c, 0x20,
    0xe4, 0xb8, 0x96, 0xe7, 0x95, 0x8c, 0x2c, 0x20,
    0xf0, 0x9f, 0xa6, 0x80,
]);
const unicodeOut = decrypt(key, encrypt(key, unicode, aad), aad);
if (unicodeOut.length !== unicode.length) throw Error("unicode length");
for (let i = 0; i < unicode.length; i++) {
    if (unicodeOut[i] !== unicode[i]) throw Error("unicode byte " + i);
}

// Empty plaintext still has a full header, nonce, and tag.
const empty = encrypt(key, new Uint8Array(), aad);
if (empty.length !== 34) throw Error("empty envelope length");
if (decrypt(key, empty, aad).length !== 0) throw Error("empty plaintext");

// Every byte value survives, including NUL.
const all = new Uint8Array(256);
for (let i = 0; i < 256; i++) all[i] = i;
const allOut = decrypt(key, encrypt(key, all, aad), aad);
if (allOut.length !== 256) throw Error("binary length");
for (let i = 0; i < 256; i++) {
    if (allOut[i] !== i) throw Error("binary byte " + i);
}

// 1 MiB plus a few bytes.
const large = new Uint8Array(1024 * 1024 + 3);
for (let i = 0; i < large.length; i++) large[i] = (i * 31 + 7) & 255;
const largeOut = decrypt(key, encrypt(key, large), new Uint8Array());
if (largeOut.length !== large.length) throw Error("large length");
for (let i = 0; i < large.length; i++) {
    if (largeOut[i] !== large[i]) throw Error("large byte " + i);
}

// The returned arrays own their backing store and are not shared with the
// inputs.
if (largeOut.buffer === large.buffer) throw Error("output shares an input buffer");
"#,
    );
}

#[test]
fn crypto_respects_typed_array_views() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";

// A key view at a non-zero byte offset in a larger buffer.
const keyBytes = new Uint8Array(34);
keyBytes[0] = 0xff;
for (let i = 1; i <= 32; i++) keyBytes[i] = i;
keyBytes[33] = 0xff;
const key = keyBytes.subarray(1, 33);
if (key.length !== 32) throw Error("key view length");

// A plaintext view that starts after a sentinel byte and excludes the
// trailing byte.
const backing = Uint8Array.from([0xaa, 0xbb, 0x00, 0x01, 0xfe, 0xcc]);
const plaintext = backing.subarray(2, 5);
const envelope = encrypt(key, plaintext);
const out = decrypt(key, envelope);
if (out.length !== 3) throw Error("view length");
if (out[0] !== 0x00 || out[1] !== 0x01 || out[2] !== 0xfe) throw Error("view bytes");

// The same for associated data.
const aadBacking = Uint8Array.from([0x11, 0x61, 0x62, 0x63, 0x22]);
const aad = aadBacking.subarray(1, 4);
const withAad = encrypt(key, plaintext, aad);
if (decrypt(key, withAad, aad).length !== 3) throw Error("aad view");

// The sentinel bytes around the views are not authenticated as part of them.
let rejected = false;
try {
    decrypt(key, withAad, aadBacking.subarray(1, 3));
} catch (error) {
    rejected = error.code === "NEO_CRYPTO_AUTH";
}
if (!rejected) throw Error("a different aad view was accepted");
"#,
    );
}

#[test]
fn crypto_accepts_shared_and_resizable_backing_buffers() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const key = new Uint8Array(32).fill(0x2a);

// A view over a SharedArrayBuffer, with an offset and an explicit length.
const shared = new SharedArrayBuffer(8);
const sharedView = new Uint8Array(shared, 2, 3);
sharedView.set([7, 8, 9]);
const sharedOut = decrypt(key, encrypt(key, sharedView));
if (sharedOut.length !== 3 || sharedOut[0] !== 7 || sharedOut[2] !== 9) {
    throw Error("shared view roundtrip");
}
if (shared.byteLength !== 8) throw Error("shared buffer changed");

// A view over a resizable ArrayBuffer.
const resizable = new ArrayBuffer(4, { maxByteLength: 16 });
const resizableView = new Uint8Array(resizable, 1, 2);
resizableView.set([5, 6]);
resizable.resize(8);
const resizableOut = decrypt(key, encrypt(key, resizableView));
if (resizableOut.length !== 2 || resizableOut[0] !== 5 || resizableOut[1] !== 6) {
    throw Error("resizable grow roundtrip");
}

// A length-tracking view follows a grow, and the extra bytes are copied.
const tracking = new Uint8Array(resizable);
resizable.resize(12);
if (tracking.length !== 12) throw Error("tracking view did not grow");
const trackingOut = decrypt(key, encrypt(key, tracking));
if (trackingOut.length !== 12) throw Error("tracking view length");

// Shrinking a resizable buffer until a fixed-length view is out of bounds is
// exposed by V8 as a zero-length view. The module encrypts exactly that view:
// it does not read stale bytes and does not fail. Encrypting the view before
// and after the shrink must therefore differ in length.
resizable.resize(1);
if (resizableView.length !== 0) throw Error("out-of-bounds view is not empty");
const afterShrink = encrypt(key, resizableView);
if (afterShrink.length !== 34) throw Error("out-of-bounds view was not empty");
"#,
    );
}

#[test]
fn crypto_random_bytes_is_bounded_synchronous_and_validated() {
    run_js(
        r#"
import { randomBytes } from "neo:crypto";
const MAX = 8 * 1024 * 1024;

function expectTypeError(fn) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
        if ("code" in error) throw Error("TypeError must not carry a code");
        return;
    }
    throw Error("expected TypeError");
}
function expectRangeError(fn) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof RangeError)) throw error;
        if ("code" in error) throw Error("RangeError must not carry a code");
        return;
    }
    throw Error("expected RangeError");
}

const empty = randomBytes(0);
if (!(empty instanceof Uint8Array) || empty.length !== 0) throw Error("zero length");
if (randomBytes(-0).length !== 0) throw Error("negative zero");
if (randomBytes(1).length !== 1) throw Error("one byte");
const thirtyTwo = randomBytes(32);
if (!(thirtyTwo instanceof Uint8Array) || thirtyTwo.length !== 32) {
    throw Error("32 bytes");
}

// This slice is synchronous. A Promise-wrapped blocking call would be a fake
// async API, so the return value must not be thenable.
if (typeof thirtyTwo.then === "function" || thirtyTwo instanceof Promise) {
    throw Error("randomBytes returned a thenable");
}

// Two calls must not return the same bytes. This is a functional check, not a
// statistical test and not a proof of cryptographic security.
const first = randomBytes(32);
const second = randomBytes(32);
let identical = true;
for (let i = 0; i < 32; i++) {
    if (first[i] !== second[i]) identical = false;
}
if (identical) throw Error("two calls returned identical bytes");

const largest = randomBytes(MAX);
if (largest.length !== MAX) throw Error("largest allowed length");

for (const value of [undefined, null, "4", {}, [], true, Symbol("4"), new Number(4)]) {
    expectTypeError(() => randomBytes(value));
}
for (const value of [-1, 1.5, NaN, Infinity, -Infinity, MAX + 1, 2 ** 53, 1e100]) {
    expectRangeError(() => randomBytes(value));
}
"#,
    );
}

#[test]
fn crypto_uses_a_fresh_nonce_per_encryption() {
    run_js(
        r#"
import { encrypt, generateEnvelopeKey } from "neo:crypto";
const key = generateEnvelopeKey();
const plaintext = Uint8Array.from([1, 2, 3]);
const seen = new Set();
for (let i = 0; i < 500; i++) {
    const envelope = encrypt(key, plaintext);
    if (envelope.length !== 37) throw Error("length");
    let nonce = "";
    let zero = true;
    for (let j = 6; j < 18; j++) {
        nonce += String.fromCharCode(envelope[j]);
        if (envelope[j] !== 0) zero = false;
    }
    if (zero) throw Error("nonce was all zero");
    if (seen.has(nonce)) throw Error("nonce repeated at " + i);
    seen.add(nonce);
}
"#,
    );
}

#[test]
fn crypto_rejects_wrong_keys_tampering_and_bad_arguments() {
    run_js(
        r#"
import { decrypt, encrypt, generateEnvelopeKey } from "neo:crypto";
const key = generateEnvelopeKey();
const other = generateEnvelopeKey();
const plaintext = Uint8Array.from([1, 2, 3]);
const aad = Uint8Array.from([9, 8, 7]);
const envelope = encrypt(key, plaintext, aad);

function expectCode(fn, code) {
    try {
        fn();
    } catch (error) {
        if (error.code !== code) {
            throw Error("expected " + code + ", got " + error.code + ": " + error.message);
        }
        return;
    }
    throw Error("expected " + code + " but no error was thrown");
}

function expectTypeError(fn) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw Error("expected TypeError: " + error);
        if ("code" in error) throw Error("TypeError must not carry a code");
        return;
    }
    throw Error("expected TypeError but no error was thrown");
}

// A wrong key of the same length and different AAD both look like
// authentication failures; no plaintext is returned.
expectCode(() => decrypt(other, envelope, aad), "NEO_CRYPTO_AUTH");
expectCode(() => decrypt(key, envelope, Uint8Array.from([9, 8, 6])), "NEO_CRYPTO_AUTH");
expectCode(() => decrypt(key, envelope, new Uint8Array()), "NEO_CRYPTO_AUTH");
expectCode(() => decrypt(key, envelope), "NEO_CRYPTO_AUTH");

// Every single-bit change in the envelope is rejected.
for (let i = 0; i < envelope.length; i++) {
    for (let bit = 0; bit < 8; bit++) {
        const copy = envelope.slice();
        copy[i] ^= 1 << bit;
        try {
            decrypt(key, copy, aad);
            throw Error("tamper at " + i + "/" + bit + " was accepted");
        } catch (error) {
            if (error.code !== "NEO_CRYPTO_AUTH" && error.code !== "NEO_CRYPTO_FORMAT") {
                throw error;
            }
        }
    }
}

// Key length is enforced before any cryptography.
expectCode(() => encrypt(new Uint8Array(31), plaintext), "NEO_CRYPTO_KEY");
expectCode(() => encrypt(new Uint8Array(33), plaintext), "NEO_CRYPTO_KEY");
expectCode(() => decrypt(new Uint8Array(16), envelope, aad), "NEO_CRYPTO_KEY");

// Truncated and unknown-format envelopes are malformed, not authentication
// failures.
expectCode(() => decrypt(key, new Uint8Array()), "NEO_CRYPTO_FORMAT");
expectCode(() => decrypt(key, new Uint8Array(33)), "NEO_CRYPTO_FORMAT");
const badMagic = envelope.slice();
badMagic[0] = 0;
expectCode(() => decrypt(key, badMagic, aad), "NEO_CRYPTO_FORMAT");
const badVersion = envelope.slice();
badVersion[4] = 2;
expectCode(() => decrypt(key, badVersion, aad), "NEO_CRYPTO_FORMAT");
const badAlgorithm = envelope.slice();
badAlgorithm[5] = 2;
expectCode(() => decrypt(key, badAlgorithm, aad), "NEO_CRYPTO_FORMAT");

// A structurally valid empty-payload envelope with a wrong tag is an
// authentication failure, not a format failure.
const validEmpty = encrypt(key, new Uint8Array(), aad);
const badTag = validEmpty.slice();
badTag[badTag.length - 1] ^= 1;
expectCode(() => decrypt(key, badTag, aad), "NEO_CRYPTO_AUTH");

// The host rejects non-Uint8Array arguments. `undefined` associated data is
// the documented way to supply none; `null` is not.
expectTypeError(() => encrypt("key", plaintext));
expectTypeError(() => encrypt(key, [1, 2, 3]));
expectTypeError(() => encrypt(key, plaintext, "aad"));
expectTypeError(() => encrypt(key, plaintext, null));
expectTypeError(() => decrypt(key, "envelope"));
expectTypeError(() => decrypt(key, envelope, null));
expectTypeError(() => decrypt(key, envelope, "aad"));
if (decrypt(key, encrypt(key, plaintext)).length !== 3) throw Error("missing aad");
"#,
    );
}

#[test]
fn crypto_rejects_proxies_and_detached_views() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const key = new Uint8Array(32);
const empty = new Uint8Array(0);
const envelope = encrypt(key, empty, empty);
if (decrypt(key, envelope, empty).length !== 0) throw Error("empty");

function detached(n) {
    const array = new Uint8Array(n);
    array.buffer.transfer();
    return array;
}
for (const call of [
    () => encrypt(detached(32), empty),
    () => encrypt(key, detached(0)),
    () => encrypt(key, empty, detached(1)),
    () => decrypt(key, detached(34)),
    () => decrypt(key, envelope, detached(0)),
]) {
    try {
        call();
        throw Error("accepted a detached view");
    } catch (error) {
        if (error.code !== "NEO_CRYPTO_DETACHED") throw error;
    }
}

// These all have a `Uint8Array`-looking shape but are not one. The host's
// internal check rejects each with TypeError.
for (const fake of [
    Object.create(Uint8Array.prototype),
    new Proxy(new Uint8Array(1), {}),
    new Uint8ClampedArray(1),
    new Int8Array(1),
    [1, 2, 3],
]) {
    try {
        encrypt(key, fake);
        throw Error("accepted a non-Uint8Array");
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
    }
}
"#,
    );
}

#[test]
fn crypto_does_not_invoke_javascript_validation_hooks() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const RealUint8Array = Uint8Array;
let calls = 0;
Object.defineProperty(RealUint8Array, Symbol.hasInstance, {
    value() { calls += 1; return true; },
});
const key = new RealUint8Array(32);
const plain = new RealUint8Array([42]);
const aad = new RealUint8Array([7]);
Object.defineProperty(key, "byteLength", { get() { calls += 1; return 32; } });
Object.defineProperty(plain, "byteLength", { get() { calls += 1; return 1; } });
Object.defineProperty(aad, "byteLength", { get() { calls += 1; return 1; } });
const envelope = encrypt(key, plain, aad);
Object.defineProperty(envelope, "byteLength", { get() { calls += 1; return 34; } });
const result = decrypt(key, envelope, aad);
if (calls) throw Error("observable validation hook was invoked");
if (result.length !== 1 || result[0] !== 42) throw Error("roundtrip");
"#,
    );
}

#[test]
fn crypto_globals_cannot_intercept_before_or_after_import() {
    run_js(
        r#"
const RealUint8Array = Uint8Array;
const RealError = Error;
const RealTypeError = TypeError;
let encryptCalls = 0;
let decryptCalls = 0;
let keyCalls = 0;
let randomCalls = 0;
// The built-in registry and its ops are captured before user code runs, so
// replacing either later must not change what the module calls.
Deno.core.ops.op_neo_crypto_encrypt = function () {
    encryptCalls += 1;
    return { ok: true, value: new RealUint8Array(34) };
};
Deno.core.ops.op_neo_crypto_decrypt = function () {
    decryptCalls += 1;
    return { ok: true, value: new RealUint8Array(0) };
};
Deno.core.ops.op_neo_crypto_generate_envelope_key = function () {
    keyCalls += 1;
    return { ok: true, value: new RealUint8Array(32) };
};
Deno.core.ops.op_neo_crypto_random_bytes = function () {
    randomCalls += 1;
    return { ok: true, value: new RealUint8Array(0) };
};
let fakeErrorCalled = false;
let fakeTypeErrorCalled = false;
globalThis.Uint8Array = function FakeUint8Array() {};
globalThis.Error = function FakeError() { fakeErrorCalled = true; };
globalThis.TypeError = function FakeTypeError() { fakeTypeErrorCalled = true; };

const { decrypt, encrypt, generateEnvelopeKey, randomBytes } = await import("neo:crypto");
const key = new RealUint8Array(32);
for (let i = 0; i < 32; i++) key[i] = i;
const plaintext = RealUint8Array.from([0, 1, 2, 255]);
const envelope = encrypt(key, plaintext);
const out = decrypt(key, envelope);
if (out.length !== 4) throw new RealError("length");
for (let i = 0; i < 4; i++) {
    if (out[i] !== plaintext[i]) throw new RealError("bytes");
}
const generated = generateEnvelopeKey();
if (!(generated instanceof RealUint8Array) || generated.length !== 32) {
    throw new RealError("generated key");
}
const random = randomBytes(4);
if (!(random instanceof RealUint8Array) || random.length !== 4) {
    throw new RealError("random bytes");
}
if (encryptCalls || decryptCalls || keyCalls || randomCalls) {
    throw new RealError("replaced op was called");
}
try {
    decrypt(key, new RealUint8Array(34));
    throw new RealError("unexpected success");
} catch (error) {
    if (fakeErrorCalled) throw new RealError("fake Error used");
    if (!(error instanceof RealError)) throw new RealError("wrong error type");
    if (error.code !== "NEO_CRYPTO_FORMAT" && error.code !== "NEO_CRYPTO_AUTH") {
        throw new RealError("wrong code");
    }
}
try {
    encrypt(key, [1, 2, 3]);
    throw new RealError("unexpected success");
} catch (error) {
    if (fakeTypeErrorCalled) throw new RealError("fake TypeError used");
    if (!(error instanceof RealTypeError)) throw new RealTypeError("wrong type error");
}
"#,
    );
}

#[test]
fn crypto_error_codes_survive_inherited_code_setters() {
    run_js(
        r#"
import { decrypt } from "neo:crypto";
const RealError = Error;

function expectStableKeyError(phase) {
    let failure;
    try {
        decrypt(new Uint8Array(16), new Uint8Array(34));
    } catch (error) {
        failure = error;
    }
    if (!(failure instanceof RealError)) {
        throw new RealError(phase + ": wrong error type: " + failure);
    }
    if (failure.code !== "NEO_CRYPTO_KEY") {
        throw new RealError(phase + ": stable code lost: " + failure.code);
    }
}

expectStableKeyError("no mutation");
Object.defineProperty(Object.prototype, "code", {
    __proto__: null,
    set() { throw new RealError("inherited code setter ran"); },
    configurable: true,
});
expectStableKeyError("throwing setter");
Object.defineProperty(Object.prototype, "code", {
    __proto__: null,
    set() {},
    configurable: true,
});
expectStableKeyError("no-op setter");
delete Object.prototype.code;
"#,
    );
}

#[test]
fn crypto_errors_do_not_echo_inputs() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const key = new Uint8Array(32).fill(0x41);
const secret = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x00, 0x99]);
const envelope = encrypt(key, secret);
let failure;
try {
    decrypt(new Uint8Array(32).fill(0x42), envelope);
} catch (error) {
    failure = error;
}
if (!failure) throw Error("expected failure");
if (failure.code !== "NEO_CRYPTO_AUTH") throw Error("wrong code");
const text = String(failure.message);
if (text.includes("AAAA") || text.toLowerCase().includes("dead") ||
    text.includes("\\u0000") || text.includes("99")) {
    throw Error("error message echoed input");
}
"#,
    );
}

#[test]
fn crypto_rejects_inputs_above_the_documented_limits() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const MAX_PLAINTEXT = 8 * 1024 * 1024;
const MAX_AAD = 8 * 1024 * 1024;
const MAX_ENVELOPE = MAX_PLAINTEXT + 34;
const key = new Uint8Array(32);
const small = new Uint8Array(1);
// One allocation, viewed at the three public bounds.
const big = new Uint8Array(MAX_ENVELOPE + 1);
const overPlaintext = big.subarray(0, MAX_PLAINTEXT + 1);
const overAad = big.subarray(0, MAX_AAD + 1);
const boundaryEnvelope = big.subarray(0, MAX_ENVELOPE);
function expectLimit(fn) {
    try {
        fn();
    } catch (error) {
        if (error.code !== "NEO_CRYPTO_LIMIT") throw error;
        return;
    }
    throw Error("expected NEO_CRYPTO_LIMIT");
}
expectLimit(() => encrypt(key, overPlaintext));
expectLimit(() => encrypt(key, small, overAad));
expectLimit(() => decrypt(key, big));
// Exactly at the envelope bound is not a limit error; the zero-filled header
// is a format error.
let format = false;
try {
    decrypt(key, boundaryEnvelope);
} catch (error) {
    format = error.code === "NEO_CRYPTO_FORMAT";
}
if (!format) throw Error("boundary envelope not format-checked");
"#,
    );
}

#[test]
fn crypto_decrypts_independent_fixtures_and_roundtrips_emitted_envelopes() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
function fromHex(value) {
    const out = new Uint8Array(value.length / 2);
    for (let i = 0; i < out.length; i++) {
        out[i] = parseInt(value.substring(i * 2, i * 2 + 2), 16);
    }
    return out;
}
function expectBytes(label, actual, expected) {
    if (actual.length !== expected.length) throw Error(label + " length");
    for (let i = 0; i < expected.length; i++) {
        if (actual[i] !== expected[i]) throw Error(label + " byte " + i);
    }
}

// Fixture from the reference implementation, produced independently with
// Node's OpenSSL-backed aes-256-gcm using this module's header/nonce/AAD
// layout. The key is a public test vector.
const referenceKey =
    "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const referenceEnvelope =
    "4e454f310101000102030405060708090a0b2967b921a697bb6bf92eb7e2df9d1d1feca6a75299032b094a0230ed39f87f14b520fd7a392dc47ab999";
const referenceAad = "636f6e746578742d6279746573";
const expected = "neo:crypto interop fixture";
const referenceOut = decrypt(fromHex(referenceKey), fromHex(referenceEnvelope), fromHex(referenceAad));
if (referenceOut.length !== expected.length) throw Error("reference length");
for (let i = 0; i < expected.length; i++) {
    if (referenceOut[i] !== expected.charCodeAt(i)) throw Error("reference byte " + i);
}

// A second fixture produced for this repository with Node 26 / OpenSSL 3.6.
const key = fromHex("8f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0");
const aad = fromHex("00010203040506070809");
const envelope = fromHex(
    "4e454f310101a1b2c3d4e5f60718293a4b5c6da183558bece799386fca37bf3f217b09605d495c551d38c28e68bd5cbdd804db19b8ed0a913ffff9b52d67946d74dd47c5b93bdec06d68df",
);
const plaintext = fromHex(
    "6e656f6a732063727970746f2063726f73732d696d706c656d656e746174696f6e2066697874757265",
);
expectBytes("node fixture", decrypt(key, envelope, aad), plaintext);

// The fixture has the documented layout.
if (envelope.length !== plaintext.length + 34) throw Error("fixture layout");
if (envelope[0] !== 0x4e || envelope[3] !== 0x31) throw Error("magic");
if (envelope[4] !== 1 || envelope[5] !== 1) throw Error("version or algorithm");

// An envelope emitted by the public API decrypts back to the same plaintext.
// The public API draws its own nonce, so this is a roundtrip, not a
// byte-for-byte comparison; the Rust unit test compares the envelope path
// byte-for-byte with these independent fixtures.
expectBytes(
    "emitted envelope",
    decrypt(key, encrypt(key, plaintext, aad), aad),
    plaintext,
);
"#,
    );
}

#[test]
fn crypto_named_exports_are_the_registry_functions() {
    run_js(
        r#"
import * as crypto from "crypto";
const registry = Neo.getBuiltInModule("crypto");
const exported = Object.keys(crypto).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error(`export list ${exported} does not match ${registered}`);
}
for (const name of registered) {
    if (typeof crypto[name] !== "function") throw new Error(`${name} is not a function`);
    if (crypto[name] !== registry[name]) throw new Error(`${name} is not the registry function`);
}
if ("default" in crypto) throw new Error("crypto must not have a default export");
"#,
    );
}

#[test]
fn crypto_is_not_available_under_an_unknown_registry_name() {
    run_js(
        r#"
if (Neo.getBuiltInModule("neo:crypto") !== undefined) {
    throw new Error("prefixed registry name must be unknown");
}
if (Neo.getBuiltInModule("Crypto") !== undefined) {
    throw new Error("bare registry name must match exactly");
}
"#,
    );
}

#[test]
fn crypto_missing_export_is_a_link_error() {
    let message = run_js_error("import { nope } from \"neo:crypto\";\n");
    assert!(
        message.contains("does not provide an export named 'nope'"),
        "{message}"
    );
    assert!(message.contains("neo:crypto"), "{message}");
}

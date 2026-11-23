//! Integration tests for the optional `neo:crypto` built-in on QuickJS.
//!
//! These compile only with the `crypto` feature. The disabled-build tests are
//! in `crypto_disabled.rs`.

#![cfg(feature = "crypto")]

use std::fs;

use neojs_lite_core::Runtime;

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
fn enabled_list_names_only_the_crypto_module() {
    assert_eq!(neojs_lite_core::enabled_builtin_modules(), &["neo:crypto"]);
}

#[test]
fn crypto_is_a_native_module_with_exactly_four_exports() {
    run_js(
        r#"
import * as fromScheme from "neo:crypto";
import * as fromUpper from "NEO:crypto";

if (fromScheme !== fromUpper) throw new Error("scheme matching is not case-insensitive");

const names = Object.keys(fromScheme).sort();
const expected = ["decrypt", "encrypt", "generateEnvelopeKey", "randomBytes"];
if (names.length !== expected.length) throw new Error(`exports: ${names}`);
for (let i = 0; i < expected.length; i++) {
    if (names[i] !== expected[i]) throw new Error(`export ${i}: ${names[i]}`);
    if (typeof fromScheme[names[i]] !== "function") throw new Error(`${names[i]} is not a function`);
}
if ("default" in fromScheme) throw new Error("crypto must not have a default export");
if (fromScheme.generateEnvelopeKey.length !== 0) throw new Error("generateEnvelopeKey arity");
if (fromScheme.randomBytes.length !== 1) throw new Error("randomBytes arity");
if (fromScheme.encrypt.length !== 2) throw new Error("encrypt arity");
if (fromScheme.decrypt.length !== 2) throw new Error("decrypt arity");
if (fromScheme.encrypt.name !== "encrypt") throw new Error("encrypt name");
if ("generateKey" in fromScheme) throw new Error("generateKey must not be exported");
if ("hashAsync" in fromScheme || "createHash" in fromScheme || "subtle" in fromScheme) {
    throw new Error("an unimplemented crypto export is present");
}
if ("__neojsLiteCrypto" in globalThis) throw new Error("internal host global leaked");
if (typeof globalThis.Neo !== "undefined") throw new Error("unexpected Neo registry");
"#,
    );
}

#[test]
fn crypto_roundtrips_types_and_the_neo1_layout() {
    run_js(
        r#"
import { decrypt, encrypt, generateEnvelopeKey, randomBytes } from "neo:crypto";

const key = generateEnvelopeKey();
if (!(key instanceof Uint8Array)) throw new Error("key is not a Uint8Array");
if (key.length !== 32) throw new Error(`key length ${key.length}`);

const plaintext = Uint8Array.from([0, 1, 2, 0, 255, 128, 7]);
const envelope = encrypt(key, plaintext);
if (!(envelope instanceof Uint8Array)) throw new Error("envelope is not a Uint8Array");
if (envelope.length !== plaintext.length + 34) throw new Error("envelope length");
if (envelope[0] !== 0x4e || envelope[1] !== 0x45 || envelope[2] !== 0x4f || envelope[3] !== 0x31) {
    throw new Error("envelope magic is not NEO1");
}
if (envelope[4] !== 1 || envelope[5] !== 1) throw new Error("envelope version or algorithm");

const returned = decrypt(key, envelope);
if (!(returned instanceof Uint8Array)) throw new Error("plaintext is not a Uint8Array");
if (returned.length !== plaintext.length) throw new Error("plaintext length");
for (let i = 0; i < plaintext.length; i++) {
    if (returned[i] !== plaintext[i]) throw new Error(`plaintext byte ${i}`);
}
if (returned.buffer === plaintext.buffer) throw new Error("output shares an input buffer");

const aad = Uint8Array.from([9, 8, 7]);
const withAad = encrypt(key, plaintext, aad);
if (decrypt(key, withAad, aad).length !== plaintext.length) throw new Error("aad roundtrip");

const empty = encrypt(key, new Uint8Array());
if (empty.length !== 34) throw new Error("empty envelope length");
if (decrypt(key, empty).length !== 0) throw new Error("empty plaintext");

for (const value of [key, envelope, returned, randomBytes(4), empty]) {
    if (typeof value.then === "function" || value instanceof Promise) {
        throw new Error("a synchronous function returned a thenable");
    }
}
"#,
    );
}

#[test]
fn crypto_decrypts_the_independently_produced_fixtures() {
    run_js(
        r#"
import { decrypt } from "neo:crypto";

function hex(text) {
    if (text.length % 2 !== 0) throw new Error("odd hex");
    const out = new Uint8Array(text.length / 2);
    for (let i = 0; i < out.length; i++) {
        out[i] = parseInt(text.slice(i * 2, i * 2 + 2), 16);
    }
    return out;
}
function bytesAt(text, index) { return text.charCodeAt(index); }
function expectText(actual, expected) {
    if (actual.length !== expected.length) throw new Error(`length ${actual.length}`);
    for (let i = 0; i < expected.length; i++) {
        if (actual[i] !== expected.charCodeAt(i)) throw new Error(`byte ${i}`);
    }
}

// The neolitejs reference fixture, produced independently with Node's
// OpenSSL-backed AES-256-GCM.
const referenceKey = hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const referenceAad = hex("636f6e746578742d6279746573");
const referenceEnvelope = hex("4e454f310101000102030405060708090a0b2967b921a697bb6bf92eb7e2df9d1d1feca6a75299032b094a0230ed39f87f14b520fd7a392dc47ab999");
expectText(decrypt(referenceKey, referenceEnvelope, referenceAad), "neo:crypto interop fixture");

// A second fixture produced for this repository with Node 26 / OpenSSL 3.6.
const key = hex("8f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0");
const aad = hex("00010203040506070809");
const plaintext = hex("6e656f6a732063727970746f2063726f73732d696d706c656d656e746174696f6e2066697874757265");
const envelope = hex("4e454f310101a1b2c3d4e5f60718293a4b5c6da183558bece799386fca37bf3f217b09605d495c551d38c28e68bd5cbdd804db19b8ed0a913ffff9b52d67946d74dd47c5b93bdec06d68df");
const opened = decrypt(key, envelope, aad);
if (opened.length !== plaintext.length) throw new Error("node fixture length");
for (let i = 0; i < plaintext.length; i++) {
    if (opened[i] !== plaintext[i]) throw new Error(`node fixture byte ${i}`);
}
// The fixture has the documented layout.
if (envelope.length !== plaintext.length + 34) throw new Error("fixture layout");
"#,
    );
}

#[test]
fn crypto_accepts_typed_array_views_and_ignores_surrounding_bytes() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";

const keyBytes = new Uint8Array(34);
keyBytes[0] = 0xff;
for (let i = 1; i <= 32; i++) keyBytes[i] = i;
keyBytes[33] = 0xff;
const key = keyBytes.subarray(1, 33);

const backing = Uint8Array.from([0xaa, 0xbb, 0x00, 0x01, 0xfe, 0xcc]);
const plaintext = backing.subarray(2, 5);
const envelope = encrypt(key, plaintext);
const out = decrypt(key, envelope);
if (out.length !== 3 || out[0] !== 0 || out[1] !== 1 || out[2] !== 0xfe) {
    throw new Error("view bytes");
}

const aadBacking = Uint8Array.from([0x11, 0x61, 0x62, 0x63, 0x22]);
const aad = aadBacking.subarray(1, 4);
const withAad = encrypt(key, plaintext, aad);
if (decrypt(key, withAad, aad).length !== 3) throw new Error("aad view");
let rejected = false;
try {
    decrypt(key, withAad, aadBacking.subarray(1, 3));
} catch (error) {
    rejected = error.code === "NEO_CRYPTO_AUTH";
}
if (!rejected) throw new Error("a different aad view was accepted");
"#,
    );
}

#[test]
fn crypto_accepts_shared_and_resizable_backing_buffers() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const key = new Uint8Array(32).fill(0x2a);

const shared = new SharedArrayBuffer(8);
const sharedView = new Uint8Array(shared, 2, 3);
sharedView.set([7, 8, 9]);
const sharedOut = decrypt(key, encrypt(key, sharedView));
if (sharedOut.length !== 3 || sharedOut[0] !== 7 || sharedOut[2] !== 9) {
    throw new Error("shared view roundtrip");
}
if (shared.byteLength !== 8) throw new Error("shared buffer changed");

const resizable = new ArrayBuffer(4, { maxByteLength: 16 });
const resizableView = new Uint8Array(resizable, 1, 2);
resizableView.set([5, 6]);
resizable.resize(8);
const resizableOut = decrypt(key, encrypt(key, resizableView));
if (resizableOut.length !== 2 || resizableOut[0] !== 5 || resizableOut[1] !== 6) {
    throw new Error("resizable grow roundtrip");
}

// QuickJS exposes a fixed view that no longer fits as zero length with no
// readable byte range. The adapter fails closed with NEO_CRYPTO_DETACHED
// instead of guessing that it is an empty view.
resizable.resize(1);
if (resizableView.length !== 0) throw new Error("out-of-bounds view is not empty");
let outOfBoundsRejected = false;
try {
    encrypt(key, resizableView);
} catch (error) {
    outOfBoundsRejected = error.code === "NEO_CRYPTO_DETACHED";
}
if (!outOfBoundsRejected) throw new Error("out-of-bounds view was not rejected");
"#,
    );
}

#[test]
fn crypto_rejects_resized_tracking_views_instead_of_truncating() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const key = new Uint8Array(32).fill(0x2a);
const plaintext = Uint8Array.from([1, 2, 3]);

function expectDetached(fn, label) {
    try {
        fn();
    } catch (error) {
        if (error.code !== "NEO_CRYPTO_DETACHED") throw error;
        return;
    }
    throw new Error(`${label}: expected NEO_CRYPTO_DETACHED`);
}

// Plaintext: a tracking view grown after creation must not be truncated to
// the creation length.
const plainBuffer = new ArrayBuffer(4, { maxByteLength: 16 });
const plainTracking = new Uint8Array(plainBuffer);
plainBuffer.resize(8);
if (plainTracking.length !== 8) throw new Error("plain tracking did not grow");
expectDetached(() => encrypt(key, plainTracking), "grown plaintext");

// Key: the same check applies to the key range.
const keyBuffer = new ArrayBuffer(32, { maxByteLength: 64 });
const keyTracking = new Uint8Array(keyBuffer);
keyBuffer.resize(33);
if (keyTracking.length !== 33) throw new Error("key tracking did not grow");
expectDetached(() => encrypt(keyTracking, plaintext), "grown key");

// Associated data: and to the AAD range.
const aadBuffer = new ArrayBuffer(4, { maxByteLength: 16 });
const aadTracking = new Uint8Array(aadBuffer);
aadBuffer.resize(8);
expectDetached(() => encrypt(key, plaintext, aadTracking), "grown aad");

// Envelope input: and to the envelope range.
const envelopeBuffer = new ArrayBuffer(34, { maxByteLength: 64 });
const envelopeTracking = new Uint8Array(envelopeBuffer);
envelopeBuffer.resize(40);
expectDetached(() => decrypt(key, envelopeTracking), "grown envelope");

// A tracking view shrunk below its creation length is also unprovable.
const shrinkBuffer = new ArrayBuffer(8, { maxByteLength: 16 });
const shrinkTracking = new Uint8Array(shrinkBuffer);
shrinkBuffer.resize(4);
expectDetached(() => encrypt(key, shrinkTracking), "shrunk plaintext");

// A fixed view over the same buffer is exact and still works.
plainBuffer.resize(12);
const fixed = new Uint8Array(plainBuffer, 0, 3);
fixed.set([7, 8, 9]);
const fixedOut = decrypt(key, encrypt(key, fixed));
if (fixedOut.length !== 3 || fixedOut[0] !== 7 || fixedOut[2] !== 9) {
    throw new Error("fixed view over resizable buffer");
}

// Hostile own `length` properties cannot make a grown tracking view look
// smaller, and cannot make a fixed view look wrong.
Object.defineProperty(plainTracking, "length", { value: 4 });
expectDetached(() => encrypt(key, plainTracking), "own length shadow");
Object.defineProperty(fixed, "length", { value: 99 });
if (decrypt(key, encrypt(key, fixed)).length !== 3) {
    throw new Error("own length shadow broke a fixed view");
}

// A replaced prototype getter is equally irrelevant: the adapter uses the
// intrinsic captured before user code.
const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
const realLength = Object.getOwnPropertyDescriptor(typedArrayPrototype, "length");
Object.defineProperty(typedArrayPrototype, "length", {
    get() { return 0; },
    configurable: true,
});
expectDetached(() => encrypt(key, plainTracking), "replaced prototype getter");
let fixedAfter;
try {
    fixedAfter = decrypt(key, encrypt(key, fixed));
} catch (error) {
    throw new Error(`replaced prototype getter broke a fixed view: ${error.code} ${error.message}`);
}
if (fixedAfter.byteLength !== 3) throw new Error("replaced prototype getter changed a fixed view");
Object.defineProperty(typedArrayPrototype, "length", realLength);
"#,
    );
}

#[test]
fn crypto_rejects_detached_views() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const key = new Uint8Array(32);
const empty = new Uint8Array(0);
const envelope = encrypt(key, empty);

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
        throw new Error("accepted a detached view");
    } catch (error) {
        if (error.code !== "NEO_CRYPTO_DETACHED") throw error;
    }
}
"#,
    );
}

#[test]
fn crypto_rejects_wrong_keys_tampering_and_malformed_envelopes() {
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
            throw new Error(`expected ${code}, got ${error.code}: ${error.message}`);
        }
        if (!(error instanceof Error)) throw new Error("not an Error");
        if ("code" in error === false) throw new Error("code is not an own property");
        return;
    }
    throw new Error(`expected ${code} but no error was thrown`);
}

expectCode(() => decrypt(other, envelope, aad), "NEO_CRYPTO_AUTH");
expectCode(() => decrypt(key, envelope, Uint8Array.from([9, 8, 6])), "NEO_CRYPTO_AUTH");
expectCode(() => decrypt(key, new Uint8Array(), aad), "NEO_CRYPTO_FORMAT");
expectCode(() => decrypt(key, new Uint8Array(33), aad), "NEO_CRYPTO_FORMAT");
expectCode(() => encrypt(new Uint8Array(31), plaintext), "NEO_CRYPTO_KEY");
expectCode(() => encrypt(new Uint8Array(33), plaintext), "NEO_CRYPTO_KEY");
expectCode(() => decrypt(new Uint8Array(16), envelope), "NEO_CRYPTO_KEY");

const badMagic = envelope.slice();
badMagic[0] = 0;
expectCode(() => decrypt(key, badMagic, aad), "NEO_CRYPTO_FORMAT");
const badVersion = envelope.slice();
badVersion[4] = 2;
expectCode(() => decrypt(key, badVersion, aad), "NEO_CRYPTO_FORMAT");
const badAlgorithm = envelope.slice();
badAlgorithm[5] = 2;
expectCode(() => decrypt(key, badAlgorithm, aad), "NEO_CRYPTO_FORMAT");

for (let i = 0; i < envelope.length; i++) {
    for (let bit = 0; bit < 8; bit++) {
        const copy = envelope.slice();
        copy[i] ^= 1 << bit;
        try {
            decrypt(key, copy, aad);
            throw new Error(`tamper at ${i}/${bit} was accepted`);
        } catch (error) {
            if (error.code !== "NEO_CRYPTO_AUTH" && error.code !== "NEO_CRYPTO_FORMAT") throw error;
        }
    }
}
"#,
    );
}

#[test]
fn crypto_rejects_wrong_argument_types_and_counts() {
    run_js(
        r#"
import * as crypto from "neo:crypto";
const key = crypto.generateEnvelopeKey();

function expectTypeError(fn, label) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
        if ("code" in error) throw new Error(`${label}: unexpected code`);
        return;
    }
    throw new Error(`${label}: expected TypeError`);
}
function expectRangeError(fn, label) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof RangeError)) throw error;
        if ("code" in error) throw new Error(`${label}: unexpected code`);
        return;
    }
    throw new Error(`${label}: expected RangeError`);
}

expectTypeError(() => crypto.generateEnvelopeKey(undefined), "generateEnvelopeKey undefined");
expectTypeError(() => crypto.generateEnvelopeKey(1), "generateEnvelopeKey number");
expectTypeError(() => crypto.randomBytes(), "randomBytes missing");
expectTypeError(() => crypto.randomBytes(4, undefined), "randomBytes second argument");
expectTypeError(() => crypto.randomBytes("4"), "randomBytes string");
expectTypeError(() => crypto.randomBytes(null), "randomBytes null");
expectTypeError(() => crypto.randomBytes(new Number(4)), "randomBytes boxed");
expectTypeError(() => crypto.randomBytes(Symbol("4")), "randomBytes symbol");
for (const value of [-1, 1.5, NaN, Infinity, -Infinity, 8 * 1024 * 1024 + 1, 2 ** 53, 1e100]) {
    expectRangeError(() => crypto.randomBytes(value), `randomBytes ${value}`);
}

const plaintext = new Uint8Array([1]);
expectTypeError(() => crypto.encrypt(key), "encrypt missing");
expectTypeError(() => crypto.encrypt(key, plaintext, undefined, undefined), "encrypt fourth argument");
expectTypeError(() => crypto.encrypt("key", plaintext), "encrypt string key");
expectTypeError(() => crypto.encrypt(key, [1, 2, 3]), "encrypt array plaintext");
expectTypeError(() => crypto.encrypt(key, plaintext, "aad"), "encrypt string aad");
expectTypeError(() => crypto.encrypt(key, plaintext, null), "encrypt null aad");
expectTypeError(() => crypto.decrypt(key), "decrypt missing");
expectTypeError(() => crypto.decrypt(key, "envelope"), "decrypt string envelope");
expectTypeError(() => crypto.decrypt(key, crypto.encrypt(key, plaintext), null), "decrypt null aad");

// The documented arities still work, including explicit undefined AAD.
if (crypto.encrypt(key, plaintext).length !== 35) throw new Error("encrypt two args");
if (crypto.encrypt(key, plaintext, undefined).length !== 35) throw new Error("encrypt three args");
if (crypto.decrypt(key, crypto.encrypt(key, plaintext)).length !== 1) throw new Error("decrypt two args");
if (crypto.decrypt(key, crypto.encrypt(key, plaintext), undefined).length !== 1) throw new Error("decrypt three args");
if (crypto.randomBytes(0).length !== 0) throw new Error("randomBytes zero");
if (crypto.generateEnvelopeKey().length !== 32) throw new Error("generateEnvelopeKey length");
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
Object.defineProperty(envelope, "byteLength", {
    get() { calls += 1; return 34; },
});
const result = decrypt(key, envelope, aad);
if (calls) throw new Error("observable validation hook was invoked");
if (result.length !== 1 || result[0] !== 42) throw new Error("roundtrip");
"#,
    );
}

#[test]
fn crypto_rejects_non_uint8array_shapes() {
    run_js(
        r#"
import { encrypt } from "neo:crypto";
const key = new Uint8Array(32);
const empty = new Uint8Array(0);
const envelope = encrypt(key, empty);
if (envelope.length !== 34) throw new Error("empty");

for (const fake of [
    Object.create(Uint8Array.prototype),
    new Uint8ClampedArray(1),
    new Int8Array(1),
    new DataView(new ArrayBuffer(1)),
    [1, 2, 3],
]) {
    try {
        encrypt(key, fake);
        throw new Error("accepted a non-Uint8Array");
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
    }
}
"#,
    );
}

#[test]
fn crypto_error_codes_are_own_properties_and_do_not_echo_inputs() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const RealError = Error;
const key = new Uint8Array(32).fill(0x41);
const secret = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x00, 0x99]);
const envelope = encrypt(key, secret);

let setterRan = false;
Object.defineProperty(Object.prototype, "code", {
    set() { setterRan = true; },
    configurable: true,
});
let messageSetterRan = false;
Object.defineProperty(Error.prototype, "message", {
    set() { messageSetterRan = true; },
    configurable: true,
});
let failure;
try {
    decrypt(new Uint8Array(32).fill(0x42), envelope);
} catch (error) {
    failure = error;
}
delete Object.prototype.code;
delete Error.prototype.message;

if (!failure) throw new RealError("expected failure");
if (!(failure instanceof RealError)) throw new RealError("wrong error type");
if (failure.code !== "NEO_CRYPTO_AUTH") throw new RealError("wrong code");
if (setterRan) throw new RealError("inherited code setter ran");
if (messageSetterRan) throw new RealError("inherited message setter ran");
const descriptor = Object.getOwnPropertyDescriptor(failure, "code");
if (!descriptor || descriptor.value !== "NEO_CRYPTO_AUTH") throw new RealError("code is not own data");
const text = String(failure.message);
if (text.includes("AAAA") || text.toLowerCase().includes("dead") ||
    text.includes("\u0000") || text.includes("99")) {
    throw new RealError("error message echoed input");
}
"#,
    );
}

#[test]
fn crypto_respects_the_documented_limits() {
    run_js(
        r#"
import { decrypt, encrypt } from "neo:crypto";
const MAX = 8 * 1024 * 1024;
const key = new Uint8Array(32);

function expectLimit(fn, label) {
    try {
        fn();
    } catch (error) {
        if (error.code !== "NEO_CRYPTO_LIMIT") throw error;
        return;
    }
    throw new Error(`${label}: expected NEO_CRYPTO_LIMIT`);
}

expectLimit(() => encrypt(key, new Uint8Array(MAX + 1)), "plaintext");
expectLimit(() => encrypt(key, new Uint8Array(1), new Uint8Array(MAX + 1)), "aad");
expectLimit(() => decrypt(key, new Uint8Array(MAX + 35)), "envelope");

// A small payload still roundtrips while the limit checks are present.
const small = encrypt(key, new Uint8Array([1, 2, 3]));
if (decrypt(key, small).length !== 3) throw new Error("small roundtrip");
"#,
    );
}

#[test]
fn unknown_crypto_exports_are_link_errors() {
    for source in [
        "import { hashAsync } from \"neo:crypto\";\n",
        "import { generateKey } from \"neo:crypto\";\n",
        "import { createHash } from \"neo:crypto\";\n",
        "import def from \"neo:crypto\";\n",
    ] {
        let message = run_js_error(source);
        assert!(
            message.contains("Could not find export")
                || message.contains("does not provide an export named")
                || message.contains("does not have a default export")
                || message.contains("default"),
            "{message}"
        );
    }
}

#[test]
fn an_unknown_builtin_specifier_is_rejected() {
    let message = run_js_error("import \"neo:os\";\n");
    assert!(
        message.contains("not enabled") || message.contains("not implemented"),
        "{message}"
    );
}

#[test]
fn a_bare_crypto_specifier_is_not_an_alias() {
    let message = run_js_error("import \"crypto\";\n");
    assert!(message.contains("bare module specifiers"), "{message}");
    let message = run_js_error("import { encrypt } from \"crypto\";\n");
    assert!(message.contains("bare module specifiers"), "{message}");
}

#[test]
fn a_local_module_named_crypto_is_not_shadowed() {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(dir.path().join("crypto.js"), "export const value = 7;\n")
        .expect("write local module");
    let main = dir.path().join("main.js");
    fs::write(
        &main,
        r#"
import { value } from "./crypto.js";
import { generateEnvelopeKey } from "neo:crypto";
if (value !== 7) throw new Error("local module");
if (generateEnvelopeKey().length !== 32) throw new Error("built-in");
"#,
    )
    .expect("write main");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&main).expect("local and built-in modules");
}

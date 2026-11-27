//! Integration tests for the `age` built-in through the public `Runtime`.
//!
//! Every test runs a real ECMAScript module with its own assertions, so it
//! exercises the registry, the module source, the host ops, and the
//! `Uint8Array` handling together. The Rust unit tests in
//! `vaeljs-core/src/age.rs` cover the age crate interop, the binary-only policy,
//! and the core bounds.

use std::fs;

use vaeljs_core::Runtime;

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
fn age_is_available_and_roundtrips_bytes() {
    run_js(
        r#"
import { decrypt, encrypt, generateIdentity, identityToRecipient } from "vael:age";

const registry = Vael.getBuiltInModule("vael:age");
if (typeof registry !== "object" || registry === null) throw Error("missing registry module");
if (Vael.getBuiltInModule("vael:age") !== registry) throw Error("registry lookup is not cached");
if (!Object.isFrozen(registry)) throw Error("registry module is not frozen");
for (const name of ["generateIdentity", "identityToRecipient", "encrypt", "decrypt"]) {
    if (typeof registry[name] !== "function") throw Error(`${name} is not a function`);
}
for (const name of ["__vaelAgeEncrypt", "__vaelAgeDecrypt", "__vaelAgeGenerateIdentity"]) {
    if (name in globalThis) throw Error("global callback exposed");
}
if (typeof fs !== "undefined" || typeof env !== "undefined") throw Error("capability leaked");

const pair = generateIdentity();
if (typeof pair !== "object" || pair === null) throw Error("generateIdentity result type");
if (typeof pair.identity !== "string" || typeof pair.recipient !== "string") {
    throw Error("generateIdentity result fields");
}
if (!pair.identity.startsWith("AGE-SECRET-KEY-1")) throw Error("identity prefix");
if (!pair.recipient.startsWith("age1")) throw Error("recipient prefix");
if (identityToRecipient(pair.identity) !== pair.recipient) {
    throw Error("identity does not derive the returned recipient");
}
const second = generateIdentity();
if (second.identity === pair.identity) throw Error("identity repeated");
if (second.recipient === pair.recipient) throw Error("recipient repeated");

const plaintext = Uint8Array.from([0, 1, 2, 0, 255, 128, 7]);
const ciphertext = encrypt(pair.recipient, plaintext);
if (!(ciphertext instanceof Uint8Array)) throw Error("ciphertext is not a Uint8Array");
if (ciphertext.length <= plaintext.length) throw Error("ciphertext does not contain a header");
const magic = "age-encryption.org/v1\n";
if (ciphertext.length < magic.length) throw Error("ciphertext is too short");
for (let i = 0; i < magic.length; i++) {
    if (ciphertext[i] !== magic.charCodeAt(i)) throw Error("binary age magic at " + i);
}

const returned = decrypt(pair.identity, ciphertext);
if (!(returned instanceof Uint8Array)) throw Error("plaintext is not a Uint8Array");
if (returned.length !== plaintext.length) throw Error("plaintext length");
for (let i = 0; i < plaintext.length; i++) {
    if (returned[i] !== plaintext[i]) throw Error("plaintext byte " + i);
}
if (plaintext[0] !== 0 || plaintext[3] !== 0) throw Error("input plaintext was mutated");
if (returned.buffer === plaintext.buffer) throw Error("output shares the plaintext buffer");

// This slice is synchronous. None of these results may be a thenable, because
// Promise-wrapping blocking work would be a fake async API.
for (const value of [pair.identity, pair.recipient, ciphertext, returned]) {
    if (typeof value === "object" && (typeof value.then === "function" || value instanceof Promise)) {
        throw Error("a synchronous function returned a thenable");
    }
}
"#,
    );
}

#[test]
fn age_rejects_unsupported_argument_counts_and_types() {
    run_js(
        r#"
import * as age from "vael:age";
const registry = Vael.getBuiltInModule("vael:age");
const pair = registry.generateIdentity();

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

expectTypeError(() => registry.generateIdentity(undefined), "generateIdentity undefined");
expectTypeError(() => registry.generateIdentity(1), "generateIdentity number");
expectTypeError(() => registry.identityToRecipient(), "identityToRecipient missing");
expectTypeError(() => registry.identityToRecipient(pair.identity, pair.identity), "identityToRecipient extra");
expectTypeError(() => registry.identityToRecipient(null), "identityToRecipient null");
expectTypeError(() => registry.identityToRecipient(1), "identityToRecipient number");
expectTypeError(() => registry.identityToRecipient(new String(pair.identity)), "identityToRecipient boxed");

expectTypeError(() => registry.encrypt(pair.recipient), "encrypt missing plaintext");
expectTypeError(() => registry.encrypt(), "encrypt missing both");
expectTypeError(
    () => registry.encrypt(pair.recipient, new Uint8Array(), undefined),
    "encrypt third argument",
);
let callbackCalled = false;
expectTypeError(
    () => registry.encrypt(pair.recipient, new Uint8Array(), () => { callbackCalled = true; }),
    "encrypt callback",
);
if (callbackCalled) throw Error("encrypt invoked a callback");
expectTypeError(() => registry.encrypt(1, new Uint8Array()), "encrypt non-string recipient");
expectTypeError(() => registry.encrypt(null, new Uint8Array()), "encrypt null recipient");
expectTypeError(() => registry.encrypt(pair.recipient, [1, 2, 3]), "encrypt array plaintext");
expectTypeError(() => registry.encrypt(pair.recipient, "plaintext"), "encrypt string plaintext");

const ciphertext = registry.encrypt(pair.recipient, new Uint8Array([1]));
expectTypeError(() => registry.decrypt(), "decrypt missing");
expectTypeError(() => registry.decrypt(pair.identity), "decrypt missing ciphertext");
expectTypeError(() => registry.decrypt(pair.identity, ciphertext, undefined), "decrypt third argument");
expectTypeError(() => registry.decrypt(pair.identity, ciphertext, () => {}), "decrypt callback");
expectTypeError(() => registry.decrypt(1, ciphertext), "decrypt non-string identity");
expectTypeError(() => registry.decrypt(pair.identity, "ciphertext"), "decrypt string ciphertext");

// The documented arities still work.
if (registry.encrypt(pair.recipient, new Uint8Array()).length <= 22) throw Error("encrypt arity");
if (registry.decrypt(pair.identity, ciphertext).length !== 1) throw Error("decrypt arity");
if (registry.identityToRecipient(pair.identity) !== pair.recipient) throw Error("derive arity");
if ("default" in age) throw Error("age must not have a default export");
"#,
    );
}

#[test]
fn age_module_specifiers_share_one_module_instance() {
    run_js(
        r#"
import * as staticAge from "vael:age";

const registry = Vael.getBuiltInModule("vael:age");
const first = await import("vael:age");
const uppercase = await import("VAEL:age");
const again = await import("vael:age");

if (staticAge !== first) throw Error("static and dynamic import differ");
if (first !== uppercase) throw Error("scheme matching is not case-insensitive");
if (first !== again) throw Error("repeated dynamic import was not cached");
if (staticAge.encrypt !== registry.encrypt) throw Error("registry function differs");
if (staticAge.decrypt !== registry.decrypt) throw Error("registry decrypt differs");
if (staticAge.generateIdentity !== registry.generateIdentity) throw Error("registry generator differs");
if (staticAge.identityToRecipient !== registry.identityToRecipient) throw Error("registry derive differs");
if ("default" in staticAge) throw Error("age must not have a default export");
if (import.meta.resolve("vael:age") !== "vael:age") throw Error("age did not resolve canonically");
if (import.meta.resolve("VAEL:age") !== "vael:age") throw Error("scheme is not case-insensitive");
"#,
    );
    // A second importer in a different module must see the same function
    // object, because both modules read the one registry object.
    let dir = tempfile::tempdir().expect("temp dir");
    fs::write(
        dir.path().join("consumer.js"),
        "import { encrypt } from \"vael:age\";\nexport { encrypt };\n",
    )
    .expect("write consumer");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        "import { encrypt as a } from \"vael:age\";\nimport { encrypt as b } from \"./consumer.js\";\nif (a !== b) throw new Error(\"cross-module identity failed\");\n",
    )
    .expect("write main");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("cross-module identity");
}

#[test]
fn age_named_exports_are_the_registry_functions() {
    run_js(
        r#"
import * as age from "vael:age";
const registry = Vael.getBuiltInModule("vael:age");
const exported = Object.keys(age).sort();
const registered = Object.keys(registry).sort();
if (JSON.stringify(exported) !== JSON.stringify(registered)) {
    throw new Error(`export list ${exported} does not match ${registered}`);
}
for (const name of registered) {
    if (typeof age[name] !== "function") throw new Error(`${name} is not a function`);
    if (age[name] !== registry[name]) throw new Error(`${name} is not the registry function`);
}
if ("default" in age) throw new Error("age must not have a default export");
"#,
    );
}

#[test]
fn age_decrypts_an_independent_age_cli_fixture() {
    run_js(
        r#"
import { decrypt, identityToRecipient } from "vael:age";

function fromHex(value) {
    const out = new Uint8Array(value.length / 2);
    for (let i = 0; i < out.length; i++) {
        out[i] = parseInt(value.substring(i * 2, i * 2 + 2), 16);
    }
    return out;
}
function ascii(bytes) {
    let text = "";
    for (const byte of bytes) text += String.fromCharCode(byte);
    return text;
}

// Public test vector published in the age crate's unit tests; never a
// credential. The fixture was produced independently with the age 1.3.2
// command-line tool:
//   printf 'neo:age interop fixture' |
//     age --encrypt --recipient <recipient>
const identity =
    "AGE-SECRET-KEY-1GQ9778VQXMMJVE8SK7J6VT8UJ4HDQAJUVSFCWCM02D8GEWQ72PVQ2Y5J33";
const recipient = "age1t7rxyev2z3rw82stdlrrepyc39nvn86l5078zqkf5uasdy86jp6svpy7pa";
if (identityToRecipient(identity) !== recipient) throw Error("recipient derivation");
const ciphertext = fromHex(
    "6167652d656e6372797074696f6e2e6f72672f76310a2d3e20583235353139203865565a39572b2b7346766b5a2b4f6f52764c3930367346785737726d2f57762b4a79762f4147377067410a30486632774b554d717734646b38466f3479773747354c4b6d67666c336f315a444145426b4a68764c50380a2d2d2d2066343976564d30694d696942456a454972686b6b5237394a72504d656f47744e7730725a305267782b6a730a299f3a639e16f5d4605db5bc12063db20d7dc0b76fce5a856ac3dbfe20e767ae66c874752e9da729e2e610bfd65ef16edcd38ce8f45dfa",
);
const out = decrypt(identity, ciphertext);
if (ascii(out) !== "neo:age interop fixture") throw Error("fixture plaintext");

// Leading and trailing whitespace around key text is trimmed; a middle
// whitespace character is not.
if (identityToRecipient("  " + identity + "\n") !== recipient) throw Error("trimmed identity");
let rejected = false;
try {
    identityToRecipient(identity.slice(0, 10) + " " + identity.slice(10));
} catch (error) {
    rejected = error.code === "VAEL_AGE_IDENTITY";
}
if (!rejected) throw Error("internal whitespace identity was accepted");
"#,
    );
}

#[test]
fn age_roundtrips_empty_binary_unicode_and_large_payloads() {
    run_js(
        r#"
import { decrypt, encrypt, generateIdentity } from "vael:age";
const pair = generateIdentity();

// Empty plaintext still has a header, a payload nonce, and a final tag.
const empty = encrypt(pair.recipient, new Uint8Array());
if (decrypt(pair.identity, empty).length !== 0) throw Error("empty plaintext");

// Every byte value survives, including NUL.
const all = new Uint8Array(256);
for (let i = 0; i < 256; i++) all[i] = i;
const allOut = decrypt(pair.identity, encrypt(pair.recipient, all));
if (allOut.length !== 256) throw Error("binary length");
for (let i = 0; i < 256; i++) {
    if (allOut[i] !== i) throw Error("binary byte " + i);
}

// UTF-8 for 'Grüße, 世界, 🦀'.
const unicode = Uint8Array.from([
    0x47, 0x72, 0xc3, 0xbc, 0xc3, 0x9f, 0x65, 0x2c, 0x20,
    0xe4, 0xb8, 0x96, 0xe7, 0x95, 0x8c, 0x2c, 0x20,
    0xf0, 0x9f, 0xa6, 0x80,
]);
const unicodeOut = decrypt(pair.identity, encrypt(pair.recipient, unicode));
if (unicodeOut.length !== unicode.length) throw Error("unicode length");
for (let i = 0; i < unicode.length; i++) {
    if (unicodeOut[i] !== unicode[i]) throw Error("unicode byte " + i);
}

// More than one 64 KiB STREAM chunk.
const large = new Uint8Array(1024 * 1024 + 3);
for (let i = 0; i < large.length; i++) large[i] = (i * 31 + 7) & 255;
const largeOut = decrypt(pair.identity, encrypt(pair.recipient, large));
if (largeOut.length !== large.length) throw Error("large length");
for (let i = 0; i < large.length; i++) {
    if (largeOut[i] !== large[i]) throw Error("large byte " + i);
}
"#,
    );
}

#[test]
fn age_rejects_wrong_identity_tampering_and_truncation() {
    run_js(
        r#"
import { decrypt, encrypt, generateIdentity } from "vael:age";
const first = generateIdentity();
const second = generateIdentity();
const plaintext = Uint8Array.from([0xde, 0xad, 0xbe, 0xef, 0x00, 0x99]);
const ciphertext = encrypt(second.recipient, plaintext);

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

// Another identity is not an authentication failure; it never matched.
expectCode(() => decrypt(first.identity, ciphertext), "VAEL_AGE_IDENTITY_MISMATCH");

// Every single-bit change is rejected. A header bit can break the MAC or the
// grammar; a payload bit breaks the chunk tag.
for (let i = 0; i < ciphertext.length; i++) {
    for (let bit = 0; bit < 8; bit++) {
        const copy = ciphertext.slice();
        copy[i] ^= 1 << bit;
        let code;
        try {
            decrypt(second.identity, copy);
        } catch (error) {
            code = error.code;
        }
        if (code !== "VAEL_AGE_AUTH" && code !== "VAEL_AGE_FORMAT" &&
            code !== "VAEL_AGE_IDENTITY_MISMATCH") {
            throw Error("tamper at " + i + "/" + bit + " produced " + code);
        }
    }
}

// Every prefix is rejected; a short final chunk fails authentication rather
// than returning partial plaintext.
for (let length = 0; length < ciphertext.length; length++) {
    let code;
    try {
        decrypt(second.identity, ciphertext.slice(0, length));
    } catch (error) {
        code = error.code;
    }
    if (code !== "VAEL_AGE_FORMAT" && code !== "VAEL_AGE_AUTH") {
        throw Error("prefix " + length + " produced " + code);
    }
}

// A wrong key on a structurally valid file must not leak plaintext.
let failure;
try {
    decrypt(first.identity, ciphertext);
} catch (error) {
    failure = error;
}
if (failure.code !== "VAEL_AGE_IDENTITY_MISMATCH") throw Error("wrong identity code");
if (String(failure.message).includes("dead") || String(failure.message).includes("99")) {
    throw Error("error message echoed input");
}

// Not a binary age file, including ASCII armor.
expectCode(() => decrypt(second.identity, new Uint8Array()), "VAEL_AGE_FORMAT");
expectCode(() => decrypt(second.identity, new Uint8Array(64)), "VAEL_AGE_FORMAT");
const armor = "-----BEGIN AGE ENCRYPTED FILE-----\nAAAA\n-----END AGE ENCRYPTED FILE-----\n";
const armorBytes = new Uint8Array(armor.length);
for (let i = 0; i < armor.length; i++) armorBytes[i] = armor.charCodeAt(i);
expectCode(() => decrypt(second.identity, armorBytes), "VAEL_AGE_FORMAT");
expectCode(() => decrypt("not-an-identity", ciphertext), "VAEL_AGE_IDENTITY");
expectCode(() => encrypt("not-a-recipient", plaintext), "VAEL_AGE_RECIPIENT");
expectCode(() => encrypt(second.identity, plaintext), "VAEL_AGE_RECIPIENT");
"#,
    );
}

#[test]
fn age_low_order_recipient_is_a_recipient_error() {
    run_js(
        r#"
import { encrypt } from "vael:age";

// Bech32 for the all-zero X25519 public key. It is syntactically a valid age
// recipient, but every X25519 shared secret with it is all zero, so the
// backend rejects it. That is invalid caller input, not an internal failure.
const LOW_ORDER = "age1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq5cu47z";
let failure;
try {
    encrypt(LOW_ORDER, Uint8Array.from([1, 2, 3]));
} catch (error) {
    failure = error;
}
if (!failure) throw Error("accepted a low-order recipient");
if (failure.code !== "VAEL_AGE_RECIPIENT") {
    throw Error("wrong code for a low-order recipient: " + failure.code);
}
if (String(failure.message).includes(LOW_ORDER)) {
    throw Error("recipient error echoed the recipient");
}
"#,
    );
}

#[test]
fn age_respects_typed_array_views() {
    run_js(
        r#"
import { decrypt, encrypt, generateIdentity } from "vael:age";
const pair = generateIdentity();

// A plaintext view that starts after a sentinel byte and excludes a trailing
// byte.
const backing = Uint8Array.from([0xaa, 0xbb, 0x00, 0x01, 0xfe, 0xcc]);
const plaintext = backing.subarray(2, 5);
const ciphertext = encrypt(pair.recipient, plaintext);
const out = decrypt(pair.identity, ciphertext);
if (out.length !== 3) throw Error("view length");
if (out[0] !== 0x00 || out[1] !== 0x01 || out[2] !== 0xfe) throw Error("view bytes");

// A ciphertext view at a non-zero offset in a larger buffer.
const padded = new Uint8Array(ciphertext.length + 4);
padded.set(ciphertext, 2);
const ciphertextView = padded.subarray(2, 2 + ciphertext.length);
if (decrypt(pair.identity, ciphertextView).length !== 3) throw Error("ciphertext view");

// A SharedArrayBuffer-backed view.
const shared = new SharedArrayBuffer(8);
const sharedView = new Uint8Array(shared, 2, 3);
sharedView.set([7, 8, 9]);
const sharedOut = decrypt(pair.identity, encrypt(pair.recipient, sharedView));
if (sharedOut.length !== 3 || sharedOut[0] !== 7 || sharedOut[2] !== 9) {
    throw Error("shared view roundtrip");
}

// A view over a resizable ArrayBuffer.
const resizable = new ArrayBuffer(4, { maxByteLength: 16 });
const resizableView = new Uint8Array(resizable, 1, 2);
resizableView.set([5, 6]);
resizable.resize(8);
const resizableOut = decrypt(pair.identity, encrypt(pair.recipient, resizableView));
if (resizableOut.length !== 2 || resizableOut[0] !== 5 || resizableOut[1] !== 6) {
    throw Error("resizable roundtrip");
}

// Shrinking until a fixed-length view is out of bounds yields a zero-length
// view; it is encrypted as empty bytes, not stale memory.
resizable.resize(1);
if (resizableView.length !== 0) throw Error("out-of-bounds view is not empty");
if (decrypt(pair.identity, encrypt(pair.recipient, resizableView)).length !== 0) {
    throw Error("out-of-bounds view was not empty");
}

// Detached views are rejected rather than treated as empty.
function detached(n) {
    const array = new Uint8Array(n);
    array.buffer.transfer();
    return array;
}
for (const call of [
    () => encrypt(pair.recipient, detached(0)),
    () => decrypt(pair.identity, detached(0)),
]) {
    try {
        call();
        throw Error("accepted a detached view");
    } catch (error) {
        if (error.code !== "VAEL_AGE_DETACHED") throw error;
    }
}

// These have a Uint8Array-looking shape but are not one. The host's internal
// check rejects each with TypeError.
for (const fake of [
    Object.create(Uint8Array.prototype),
    new Proxy(new Uint8Array(1), {}),
    new Uint8ClampedArray(1),
    new Int8Array(1),
    [1, 2, 3],
]) {
    try {
        encrypt(pair.recipient, fake);
        throw Error("accepted a non-Uint8Array");
    } catch (error) {
        if (!(error instanceof TypeError)) throw error;
    }
}
"#,
    );
}

#[test]
fn age_rejects_inputs_above_the_documented_limits() {
    run_js(
        r#"
import { decrypt, encrypt, generateIdentity } from "vael:age";
const pair = generateIdentity();
const MAX_PLAINTEXT = 8 * 1024 * 1024;
const MAX_CIPHERTEXT = MAX_PLAINTEXT + 1024 * 1024;

function expectLimit(fn) {
    try {
        fn();
    } catch (error) {
        if (error.code !== "VAEL_AGE_LIMIT") throw error;
        return;
    }
    throw Error("expected VAEL_AGE_LIMIT");
}

expectLimit(() => encrypt(pair.recipient, new Uint8Array(MAX_PLAINTEXT + 1)));
expectLimit(() => decrypt(pair.identity, new Uint8Array(MAX_CIPHERTEXT + 1)));

// A key text over 512 bytes is refused before parsing.
let keyError;
try {
    Vael.getBuiltInModule("vael:age").identityToRecipient("A".repeat(513));
} catch (error) {
    keyError = error;
}
if (keyError.code !== "VAEL_AGE_IDENTITY") throw Error("identity length code");
try {
    encrypt("A".repeat(513), new Uint8Array(1));
} catch (error) {
    if (error.code !== "VAEL_AGE_RECIPIENT") throw error;
}

// Exactly at the plaintext bound still roundtrips. This crosses many STREAM
// chunks, so the payload limit and chunk framing are both exercised.
const largest = new Uint8Array(MAX_PLAINTEXT);
for (let i = 0; i < largest.length; i += 4096) largest[i] = i & 255;
const ciphertext = encrypt(pair.recipient, largest);
if (ciphertext.length <= largest.length) throw Error("largest ciphertext length");
const out = decrypt(pair.identity, ciphertext);
if (out.length !== largest.length) throw Error("largest plaintext length");
for (let i = 0; i < largest.length; i += 4096) {
    if (out[i] !== (i & 255)) throw Error("largest byte " + i);
}
"#,
    );
}

#[test]
fn age_error_codes_survive_inherited_code_setters() {
    run_js(
        r#"
import { decrypt } from "vael:age";
const RealError = Error;

function expectStableIdentityError(phase) {
    let failure;
    try {
        decrypt("not-an-identity", new Uint8Array(0));
    } catch (error) {
        failure = error;
    }
    if (!(failure instanceof RealError)) {
        throw new RealError(phase + ": wrong error type: " + failure);
    }
    if (failure.code !== "VAEL_AGE_IDENTITY") {
        throw new RealError(phase + ": stable code lost: " + failure.code);
    }
}

expectStableIdentityError("no mutation");
Object.defineProperty(Object.prototype, "code", {
    __proto__: null,
    set() { throw new RealError("inherited code setter ran"); },
    configurable: true,
});
expectStableIdentityError("throwing setter");
Object.defineProperty(Object.prototype, "code", {
    __proto__: null,
    set() {},
    configurable: true,
});
expectStableIdentityError("no-op setter");
delete Object.prototype.code;
"#,
    );
}

#[test]
fn age_does_not_invoke_javascript_validation_hooks() {
    run_js(
        r#"
import { decrypt, encrypt, generateIdentity } from "vael:age";
const pair = generateIdentity();
const RealUint8Array = Uint8Array;
let calls = 0;
Object.defineProperty(RealUint8Array, Symbol.hasInstance, {
    value() { calls += 1; return true; },
});
const plaintext = new RealUint8Array([42]);
Object.defineProperty(plaintext, "byteLength", { get() { calls += 1; return 1; } });
const ciphertext = encrypt(pair.recipient, plaintext);
Object.defineProperty(ciphertext, "byteLength", { get() { calls += 1; return 34; } });
const result = decrypt(pair.identity, ciphertext);
if (calls) throw Error("observable validation hook was invoked");
if (result.length !== 1 || result[0] !== 42) throw Error("roundtrip");
"#,
    );
}

#[test]
fn age_globals_cannot_intercept_before_or_after_import() {
    run_js(
        r#"
const RealUint8Array = Uint8Array;
const RealError = Error;
const RealTypeError = TypeError;
let generateCalls = 0;
let deriveCalls = 0;
let encryptCalls = 0;
let decryptCalls = 0;
// The built-in registry and its ops are captured before user code runs, so
// replacing either later must not change what the module calls.
Deno.core.ops.op_vael_age_generate_identity = function () {
    generateCalls += 1;
    return { ok: true, identity: "x", recipient: "y" };
};
Deno.core.ops.op_vael_age_identity_to_recipient = function () {
    deriveCalls += 1;
    return { ok: true, value: "y" };
};
Deno.core.ops.op_vael_age_encrypt = function () {
    encryptCalls += 1;
    return { ok: true, value: new RealUint8Array(1) };
};
Deno.core.ops.op_vael_age_decrypt = function () {
    decryptCalls += 1;
    return { ok: true, value: new RealUint8Array(0) };
};
let fakeErrorCalled = false;
let fakeTypeErrorCalled = false;
globalThis.Uint8Array = function FakeUint8Array() {};
globalThis.Error = function FakeError() { fakeErrorCalled = true; };
globalThis.TypeError = function FakeTypeError() { fakeTypeErrorCalled = true; };

const { decrypt, encrypt, generateIdentity, identityToRecipient } = await import("vael:age");
const pair = generateIdentity();
if (!pair.identity.startsWith("AGE-SECRET-KEY-1")) throw new RealError("generator result");
if (!pair.recipient.startsWith("age1")) throw new RealError("recipient result");
if (identityToRecipient(pair.identity) !== pair.recipient) throw new RealError("derivation result");
const plaintext = RealUint8Array.from([1, 2, 3]);
const ciphertext = encrypt(pair.recipient, plaintext);
const out = decrypt(pair.identity, ciphertext);
if (out.length !== 3 || out[0] !== 1 || out[2] !== 3) throw new RealError("decrypt result");
if (generateCalls || deriveCalls || encryptCalls || decryptCalls) {
    throw new RealError("replaced op was called");
}
try {
    decrypt(pair.identity, new RealUint8Array(0));
    throw new RealError("unexpected success");
} catch (error) {
    if (fakeErrorCalled) throw new RealError("fake Error used");
    if (!(error instanceof RealError)) throw new RealError("wrong error type");
    if (error.code !== "VAEL_AGE_FORMAT") throw new RealError("wrong code");
}
try {
    encrypt(1, new RealUint8Array(0));
    throw new RealError("unexpected success");
} catch (error) {
    if (fakeTypeErrorCalled) throw new RealError("fake TypeError used");
    if (!(error instanceof RealTypeError)) throw new RealTypeError("wrong type error");
}
"#,
    );
}

#[test]
fn age_errors_do_not_echo_inputs() {
    run_js(
        r#"
import { decrypt, encrypt, generateIdentity } from "vael:age";
const pair = generateIdentity();
const marker = "SUPER-SECRET-INPUT";
const secret = new Uint8Array(marker.length);
for (let i = 0; i < marker.length; i++) secret[i] = marker.charCodeAt(i);
const ciphertext = encrypt(pair.recipient, secret);
let failure;
try {
    decrypt("not-an-identity-" + marker, ciphertext);
} catch (error) {
    failure = error;
}
if (!failure) throw Error("expected failure");
const text = String(failure.message);
if (text.includes(marker) || text.includes("not-an-identity")) {
    throw Error("identity error message echoed input");
}
let decryptFailure;
try {
    decrypt(pair.identity, ciphertext.slice(0, ciphertext.length - 1));
} catch (error) {
    decryptFailure = error;
}
if (!decryptFailure) throw Error("expected decrypt failure");
if (String(decryptFailure.message).includes(marker)) {
    throw Error("decrypt error message echoed plaintext");
}
"#,
    );
}

#[test]
fn age_is_not_available_under_a_bare_or_misspelled_name() {
    run_js(
        r#"
if (Vael.getBuiltInModule("age") !== undefined) {
    throw new Error("bare registry name must be unknown");
}
if (Vael.getBuiltInModule("Age") !== undefined) {
    throw new Error("registry names are case-sensitive");
}
let bareRejected = false;
try {
    await import("age");
} catch (error) {
    bareRejected = error instanceof TypeError;
}
if (!bareRejected) throw new Error("bare age import was accepted");
"#,
    );
}

#[test]
fn age_missing_export_is_a_link_error() {
    let message = run_js_error("import { nope } from \"vael:age\";\n");
    assert!(
        message.contains("does not provide an export named 'nope'"),
        "{message}"
    );
    assert!(message.contains("vael:age"), "{message}");
}

/// Official C2SP/CCTV `age` test vector suite (`tests/fixtures/age_cctv/testdata/`).
///
/// Accounts for every vector in the 147-file corpus against `vael:age`'s documented
/// binary X25519 v1 contract:
/// - 66 binary X25519 / header / HMAC / stanza / STREAM / version vectors (including
///   `scrypt_and_x25519` and `hybrid_x25519_arg`, which include an X25519 identity) execute
///   directly through `vael:age`'s `decrypt()` (`success` vectors verify SHA-256 against
///   `payload`).
/// - 37 `armored: yes` vectors are asserted to fail with `VAEL_AGE_FORMAT` (`vael:age` is
///   binary-only and does not implement ASCII armor).
/// - 3 oversize STREAM vectors (`stream_257_chunks`, `stream_257_chunks_full`,
///   `stream_258_chunks`, ~16.8 MiB) are asserted to fail with `VAEL_AGE_LIMIT` because
///   `vael:age` enforces `MAX_CIPHERTEXT_BYTES` (9 MiB) and `MAX_PLAINTEXT_BYTES` (8 MiB).
/// - 24 pure binary `scrypt` passphrase vectors are excluded because `vael:age` has no
///   passphrase API (`armor_scrypt` is counted under `armored`, and `scrypt_and_x25519` is
///   executed under `binary_x25519_executed`).
/// - 17 pure binary `mlkem768x25519` hybrid PQ vectors are excluded because `vael:age` is
///   X25519-only (`AGE-SECRET-KEY-PQ-1...` is asserted to fail with `VAEL_AGE_UNSUPPORTED`).
mod cctv {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Read;
    use std::path::Path;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use flate2::read::ZlibDecoder;
    use vaeljs_core::Runtime;

    #[derive(Debug)]
    struct CctvVector {
        name: String,
        expect: String,
        payload_sha256: Option<String>,
        identities: Vec<String>,
        passphrases: Vec<String>,
        armored: bool,
        body: Vec<u8>,
    }

    fn load_cctv_vectors() -> Vec<CctvVector> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/age_cctv/testdata");
        let mut entries: Vec<_> = fs::read_dir(&dir)
            .expect("read age_cctv/testdata")
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.is_file())
            .collect();
        entries.sort();

        assert_eq!(
            entries.len(),
            147,
            "expected all 147 official C2SP/CCTV age test vectors"
        );

        entries
            .into_iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .expect("utf-8 filename")
                    .to_string();
                let raw = fs::read(&path).expect("read vector");
                let sep = raw
                    .windows(2)
                    .position(|w| w == b"\n\n")
                    .expect("vector header/body separator");
                let header = std::str::from_utf8(&raw[..sep]).expect("utf-8 vector header");
                let mut body = raw[sep + 2..].to_vec();

                let mut expect = None;
                let mut payload_sha256 = None;
                let mut identities = Vec::new();
                let mut passphrases = Vec::new();
                let mut armored = false;
                let mut compressed = false;

                for line in header.lines() {
                    let (key, value) = line.split_once(": ").expect("header key: value");
                    match key {
                        "expect" => expect = Some(value.to_string()),
                        "payload" => payload_sha256 = Some(value.to_string()),
                        "identity" => identities.push(value.to_string()),
                        "passphrase" => passphrases.push(value.to_string()),
                        "armored" => armored = value == "yes",
                        "compressed" => {
                            assert_eq!(value, "zlib", "{name}: unsupported compression {value}");
                            compressed = true;
                        }
                        "file key" | "comment" => {}
                        other => panic!("{name}: unknown CCTV header key {other:?}"),
                    }
                }

                if compressed {
                    let mut decoder = ZlibDecoder::new(body.as_slice());
                    let mut decompressed = Vec::new();
                    decoder.read_to_end(&mut decompressed).expect("zlib decode");
                    body = decompressed;
                }

                CctvVector {
                    name,
                    expect: expect.expect("expect header"),
                    payload_sha256,
                    identities,
                    passphrases,
                    armored,
                    body,
                }
            })
            .collect()
    }

    const JS_HELPERS: &str = r#"
import { decrypt } from "vael:age";
import { createHash } from "vael:crypto";

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_LUT = new Int16Array(128).fill(-1);
for (let i = 0; i < B64.length; i++) B64_LUT[B64.charCodeAt(i)] = i;

export function decodeBase64(text) {
    if (text.length === 0) return new Uint8Array(0);
    let pad = 0;
    if (text.endsWith("==")) pad = 2;
    else if (text.endsWith("=")) pad = 1;
    const outLen = (text.length / 4) * 3 - pad;
    const out = new Uint8Array(outLen);
    let o = 0;
    for (let i = 0; i < text.length; i += 4) {
        const a = B64_LUT[text.charCodeAt(i)];
        const b = B64_LUT[text.charCodeAt(i + 1)];
        const c = text[i + 2] === "=" ? 0 : B64_LUT[text.charCodeAt(i + 2)];
        const d = text[i + 3] === "=" ? 0 : B64_LUT[text.charCodeAt(i + 3)];
        const n = (a << 18) | (b << 12) | (c << 6) | d;
        if (o < outLen) out[o++] = (n >> 16) & 255;
        if (o < outLen) out[o++] = (n >> 8) & 255;
        if (o < outLen) out[o++] = n & 255;
    }
    return out;
}

export function sha256Hex(bytes) {
    return createHash("sha256").update(bytes).digest("hex");
}

export { decrypt };
"#;

    const FALLBACK_X25519_IDENTITY: &str =
        "AGE-SECRET-KEY-1EGTZVFFV20835NWYV6270LXYVK2VKNX2MMDKWYKLMGR48UAWX40Q2P2LM0";

    fn run_js_cases(cases: &[(&CctvVector, &[&str])]) {
        let dir = tempfile::tempdir().expect("temp dir");
        fs::write(dir.path().join("helpers.mjs"), JS_HELPERS).expect("write helpers");

        let mut script = String::from(
            "import { decodeBase64, decrypt, sha256Hex } from \"./helpers.mjs\";\nconst failures = [];\n",
        );

        for (vec, expected_codes) in cases.iter() {
            let b64 = STANDARD.encode(&vec.body);
            let x25519_ids: Vec<&str> = vec
                .identities
                .iter()
                .filter(|id| id.starts_with("AGE-SECRET-KEY-1"))
                .map(String::as_str)
                .collect();
            let id_to_use = x25519_ids
                .first()
                .copied()
                .unwrap_or(FALLBACK_X25519_IDENTITY);
            let id_json = serde_json::to_string(id_to_use).unwrap();
            let name_json = serde_json::to_string(&vec.name).unwrap();
            let expect_json = serde_json::to_string(&vec.expect).unwrap();
            let codes_json = serde_json::to_string(expected_codes).unwrap();
            let payload_json = serde_json::to_string(&vec.payload_sha256).unwrap();

            script.push_str(&format!(
                r#"{{
    const name = {name_json};
    const expectOutcome = {expect_json};
    const allowedCodes = {codes_json};
    const expectedHash = {payload_json};
    const bytes = decodeBase64("{b64}");
    try {{
        const pt = decrypt({id_json}, bytes);
        if (!allowedCodes.includes("OK")) {{
            failures.push(`${{name}} (expect ${{expectOutcome}}): unexpectedly decrypted ${{pt.length}} bytes`);
        }} else if (expectedHash !== null) {{
            const actualHash = sha256Hex(pt);
            if (actualHash !== expectedHash) {{
                failures.push(`${{name}}: payload sha256 mismatch: got ${{actualHash}}, expected ${{expectedHash}}`);
            }}
        }}
    }} catch (err) {{
        if (!allowedCodes.includes(err.code)) {{
            failures.push(`${{name}} (expect ${{expectOutcome}}): got ${{err.code}} (${{err.message}}), expected one of ${{JSON.stringify(allowedCodes)}}`);
        }}
    }}
}}
"#,
            ));
        }
        script.push_str("if (failures.length > 0) throw new Error(failures.join(\"\\n\"));\n");

        let main = dir.path().join("main.mjs");
        fs::write(&main, script).expect("write main");
        let mut runtime = Runtime::new().expect("runtime");
        if let Err(err) = runtime.run_file(&main) {
            panic!("CCTV vector assertions failed:\n{err}");
        }
    }

    #[test]
    fn age_cctv_corpus_audit_and_compliance() {
        let vectors = load_cctv_vectors();
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut binary_cases: Vec<(&CctvVector, &[&str])> = Vec::new();
        let mut armor_cases: Vec<(&CctvVector, &[&str])> = Vec::new();
        let mut pq_only_vectors: Vec<&CctvVector> = Vec::new();

        const MAX_VAEL_CIPHERTEXT: usize = 9 * 1024 * 1024;

        for vec in &vectors {
            let has_x25519_id = vec
                .identities
                .iter()
                .any(|id| id.starts_with("AGE-SECRET-KEY-1"));
            let has_pq_id = vec
                .identities
                .iter()
                .any(|id| id.starts_with("AGE-SECRET-KEY-PQ-1"));

            if vec.armored {
                // vael:age is binary-only and rejects ASCII armor before header parsing.
                *counts.entry("armored_rejected_as_format").or_default() += 1;
                armor_cases.push((vec, &["VAEL_AGE_FORMAT"]));
            } else if !vec.passphrases.is_empty() && !has_x25519_id {
                // Excluded: pure scrypt passphrase vectors (vael:age has no passphrase API).
                *counts.entry("excluded_scrypt_passphrase").or_default() += 1;
            } else if has_pq_id && !has_x25519_id {
                // Excluded: ML-KEM-768+X25519 PQ hybrid vectors (vael:age is X25519-only).
                *counts.entry("excluded_pq_hybrid").or_default() += 1;
                pq_only_vectors.push(vec);
            } else if vec.body.len() > MAX_VAEL_CIPHERTEXT {
                // vael:age enforces MAX_CIPHERTEXT_BYTES (9 MiB) before parsing.
                *counts.entry("oversize_rejected_as_limit").or_default() += 1;
                binary_cases.push((vec, &["VAEL_AGE_LIMIT"]));
            } else {
                *counts.entry("binary_x25519_executed").or_default() += 1;
                let expected: &[&str] = match vec.expect.as_str() {
                    "success" => &["OK"],
                    "no match" => &["VAEL_AGE_IDENTITY_MISMATCH"],
                    "HMAC failure" => &["VAEL_AGE_AUTH"],
                    "header failure" => &["VAEL_AGE_FORMAT"],
                    "payload failure" => &["VAEL_AGE_AUTH", "VAEL_AGE_FORMAT"],
                    other => panic!("{}: unexpected expect value {other:?}", vec.name),
                };
                binary_cases.push((vec, expected));
            }
        }

        // Verify exact accounting of all 147 CCTV vectors.
        let total: usize = counts.values().sum();
        assert_eq!(total, 147, "accounting mismatch: {counts:?}");
        assert_eq!(counts.get("binary_x25519_executed").copied(), Some(66));
        assert_eq!(counts.get("armored_rejected_as_format").copied(), Some(37));
        assert_eq!(counts.get("excluded_scrypt_passphrase").copied(), Some(24));
        assert_eq!(counts.get("excluded_pq_hybrid").copied(), Some(17));
        assert_eq!(counts.get("oversize_rejected_as_limit").copied(), Some(3));

        run_js_cases(&binary_cases);
        run_js_cases(&armor_cases);

        // Verify PQ hybrid identities are explicitly rejected with VAEL_AGE_UNSUPPORTED.
        let pq_dir = tempfile::tempdir().expect("temp dir");
        fs::write(pq_dir.path().join("helpers.mjs"), JS_HELPERS).expect("write helpers");
        let pq_id = pq_only_vectors
            .iter()
            .flat_map(|v| v.identities.iter())
            .find(|id| id.starts_with("AGE-SECRET-KEY-PQ-1"))
            .expect("at least one PQ identity");
        let pq_body = STANDARD.encode(&pq_only_vectors[0].body);
        let pq_main = pq_dir.path().join("main.mjs");
        fs::write(
            &pq_main,
            format!(
                r#"import {{ decodeBase64, decrypt }} from "./helpers.mjs";
const bytes = decodeBase64("{pq_body}");
let code;
try {{
    decrypt("{pq_id}", bytes);
}} catch (err) {{
    code = err.code;
}}
if (code !== "VAEL_AGE_UNSUPPORTED") {{
    throw new Error("expected VAEL_AGE_UNSUPPORTED for PQ identity, got " + code);
}}
"#
            ),
        )
        .expect("write pq main");
        Runtime::new()
            .expect("runtime")
            .run_file(&pq_main)
            .expect("PQ identity rejection");
    }
}

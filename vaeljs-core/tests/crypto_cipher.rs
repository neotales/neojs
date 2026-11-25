//! Integration tests for the stateful CBC/GCM cipher surface.
//!
//! The Rust unit tests in `vaeljs-core/src/crypto_cipher.rs` check the primitive
//! and state machine against NIST and Node/OpenSSL fixtures. These tests drive
//! the same paths through the public `Runtime`, the module registry, the host
//! ops, and the JavaScript wrapper.

use std::fs;

use vaeljs_core::Runtime;

fn run_js(source: &str) {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(&file, source).expect("write source");
    let mut runtime = Runtime::new().expect("runtime");
    runtime.run_file(&file).expect("module assertions");
}

const HELPERS: &str = r#"
function fromHex(text) {
    if (text.length % 2 !== 0) throw Error("odd hex");
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
function expectError(code, fn) {
    try {
        fn();
    } catch (error) {
        if (error.code !== code) throw Error(`expected ${code}, got ${error.code}: ${error.message}`);
        return;
    }
    throw Error(`expected ${code}`);
}
function expectTypeError(fn) {
    try {
        fn();
    } catch (error) {
        if (!(error instanceof TypeError) || "code" in error) throw error;
        return;
    }
    throw Error("expected TypeError");
}
"#;

#[test]
fn cipher_known_answer_vectors_roundtrip() {
    run_js(&format!(
        "{HELPERS}\n{}",
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";
const cbcKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const cbcIv = fromHex("101112131415161718191a1b1c1d1e1f");
const cbcPlaintext = fromHex("6e656f6a73206369706865722063626320696e7465726f70");
const cbcCiphertext = "5226084e220371598b38cb20eaa1b85c2ff1b252517261b6f0054232236daa85";

const cbcEncryptor = createEncryptor("aes-256-cbc", cbcKey, cbcIv);
const cbcUpdate = cbcEncryptor.update(cbcPlaintext);
if (!(cbcUpdate instanceof Uint8Array) || cbcUpdate.length !== 0) {
    throw Error("CBC update must return an empty Uint8Array");
}
if (hex(cbcEncryptor.final()) !== cbcCiphertext) throw Error("CBC fixture");
const cbcDecryptor = createDecryptor("AES-256-CBC", cbcKey, cbcIv);
cbcDecryptor.update(fromHex(cbcCiphertext));
if (hex(cbcDecryptor.final()) !== hex(cbcPlaintext)) throw Error("CBC roundtrip");

// NIST SP 800-38A F.2.5, no padding.
const nistKey = fromHex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
const nistIv = fromHex("000102030405060708090a0b0c0d0e0f");
const nistPlaintext = fromHex(
    "6bc1bee22e409f96e93d7e117393172a" +
    "ae2d8a571e03ac9c9eb76fac45af8e51" +
    "30c81c46a35ce411e5fbc1191a0a52ef" +
    "f69f2445df4f9b17ad2b417be66c3710",
);
const nistCiphertext =
    "f58c4c04d6e5f1ba779eabfb5f7bfbd6" +
    "9cfc4e967edb808d679f777bc6702c7d" +
    "39f23369a9d9bacfa530e26304231461" +
    "b2eb05e2c39be9fcda6c19078c6a9d1b";
const nistEncryptor = createEncryptor("aes-256-cbc", nistKey, nistIv, { padding: false });
nistEncryptor.update(nistPlaintext);
if (hex(nistEncryptor.final()) !== nistCiphertext) throw Error("NIST CBC vector");
const nistDecryptor = createDecryptor("aes-256-cbc", nistKey, nistIv, { padding: false });
nistDecryptor.update(fromHex(nistCiphertext));
if (hex(nistDecryptor.final()) !== hex(nistPlaintext)) throw Error("NIST CBC decrypt");

// Node 26 / OpenSSL 3.6 GCM fixture with AAD.
const gcmKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const gcmIv = fromHex("a1b2c3d4e5f60718293a4b5c");
const gcmAad = fromHex("6e656f6a732d6369706865722d636f6e74657874");
const gcmPlaintext = fromHex("6e656f6a73206369706865722067636d20696e7465726f70");
const gcmCiphertext = "6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1";
const gcmTag = "9ea6507bfbc87455b1f24edc000cc662";

const gcmEncryptor = createEncryptor("aes-256-gcm", gcmKey, gcmIv);
gcmEncryptor.setAAD(gcmAad);
gcmEncryptor.update(gcmPlaintext);
if (hex(gcmEncryptor.final()) !== gcmCiphertext) throw Error("GCM ciphertext");
if (hex(gcmEncryptor.getAuthTag()) !== gcmTag) throw Error("GCM tag");

const gcmDecryptor = createDecryptor("aes-256-gcm", gcmKey, gcmIv);
gcmDecryptor.setAAD(gcmAad);
gcmDecryptor.setAuthTag(fromHex(gcmTag));
gcmDecryptor.update(fromHex(gcmCiphertext));
if (hex(gcmDecryptor.final()) !== hex(gcmPlaintext)) throw Error("GCM roundtrip");

// Empty GCM plaintext with AAD still produces a tag and roundtrips.
const emptyEncryptor = createEncryptor("aes-256-gcm", new Uint8Array(32), new Uint8Array(12));
emptyEncryptor.setAAD(fromHex("616164"));
if (emptyEncryptor.update(new Uint8Array(0)).length !== 0) throw Error("empty update");
if (emptyEncryptor.final().length !== 0) throw Error("empty GCM ciphertext");
if (hex(emptyEncryptor.getAuthTag()) !== "1434c31bb6913e5f754c2f7337dcb444") {
    throw Error("empty GCM tag");
}
const emptyDecryptor = createDecryptor("aes-256-gcm", new Uint8Array(32), new Uint8Array(12));
emptyDecryptor.setAAD(fromHex("616164"));
emptyDecryptor.setAuthTag(emptyEncryptor.getAuthTag());
emptyDecryptor.update(new Uint8Array(0));
if (emptyDecryptor.final().length !== 0) throw Error("empty GCM plaintext");

// Every result owns its buffer; no output aliases an input.
if (gcmEncryptor.getAuthTag().buffer === gcmKey.buffer) throw Error("tag aliased key");
"#,
    ));
}

#[test]
fn cipher_accepts_multiple_updates_incrementally() {
    run_js(&format!(
        "{HELPERS}\n{}",
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";

const cbcKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const cbcIv = fromHex("101112131415161718191a1b1c1d1e1f");
const plaintext = fromHex("6e656f6a73206369706865722063626320696e7465726f70");
const expected = "5226084e220371598b38cb20eaa1b85c2ff1b252517261b6f0054232236daa85";

const encryptor = createEncryptor("aes-256-cbc", cbcKey, cbcIv);
for (const chunk of [plaintext.slice(0, 1), plaintext.slice(1, 7), plaintext.slice(7)]) {
    if (encryptor.update(chunk).length !== 0) throw Error("update returned bytes");
}
if (hex(encryptor.final()) !== expected) throw Error("incremental CBC output");

const decryptor = createDecryptor("aes-256-cbc", cbcKey, cbcIv);
const ciphertext = fromHex(expected);
for (const chunk of [ciphertext.slice(0, 1), ciphertext.subarray(1)]) {
    if (decryptor.update(chunk).length !== 0) throw Error("update returned plaintext");
}
if (hex(decryptor.final()) !== hex(plaintext)) throw Error("incremental CBC plaintext");

const gcmKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const gcmIv = fromHex("a1b2c3d4e5f60718293a4b5c");
const gcmAad = fromHex("6e656f6a732d6369706865722d636f6e74657874");
const gcmPlaintext = fromHex("6e656f6a73206369706865722067636d20696e7465726f70");
const gcmCiphertext = fromHex("6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1");
const gcmTag = fromHex("9ea6507bfbc87455b1f24edc000cc662");

const gcmEncryptor = createEncryptor("aes-256-gcm", gcmKey, gcmIv);
gcmEncryptor.setAAD(gcmAad);
for (const chunk of [gcmPlaintext.slice(0, 2), gcmPlaintext.slice(2, 15), gcmPlaintext.slice(15)]) {
    gcmEncryptor.update(chunk);
}
if (hex(gcmEncryptor.final()) !== hex(gcmCiphertext)) throw Error("incremental GCM output");
if (hex(gcmEncryptor.getAuthTag()) !== hex(gcmTag)) throw Error("incremental GCM tag");

const gcmDecryptor = createDecryptor("aes-256-gcm", gcmKey, gcmIv);
gcmDecryptor.setAAD(gcmAad);
gcmDecryptor.setAuthTag(gcmTag);
for (const chunk of [gcmCiphertext.slice(0, 3), gcmCiphertext.slice(3)]) {
    if (gcmDecryptor.update(chunk).length !== 0) throw Error("GCM released plaintext");
}
if (hex(gcmDecryptor.final()) !== hex(gcmPlaintext)) throw Error("incremental GCM plaintext");
"#,
    ));
}

#[test]
fn cipher_gcm_never_releases_plaintext_before_final_and_rejects_bad_tags() {
    run_js(&format!(
        "{HELPERS}\n{}",
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";
const key = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const iv = fromHex("a1b2c3d4e5f60718293a4b5c");
const aad = fromHex("6e656f6a732d6369706865722d636f6e74657874");
const plaintext = fromHex("6e656f6a73206369706865722067636d20696e7465726f70");
const ciphertext = "6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1";
const tag = fromHex("9ea6507bfbc87455b1f24edc000cc662");

const decryptor = createDecryptor("aes-256-gcm", key, iv);
decryptor.setAAD(aad);
decryptor.setAuthTag(tag);
const update = decryptor.update(fromHex(ciphertext));
if (update.length !== 0) throw Error("GCM update returned plaintext before final");
if (hex(decryptor.final()) !== hex(plaintext)) throw Error("GCM plaintext");

for (const mutate of [
    () => {
        const d = createDecryptor("aes-256-gcm", key, iv);
        d.setAAD(aad);
        const bad = tag.slice(); bad[0] ^= 1;
        d.setAuthTag(bad);
        d.update(fromHex(ciphertext));
        d.final();
    },
    () => {
        const d = createDecryptor("aes-256-gcm", key, iv);
        d.setAAD(fromHex("6f74686572"));
        d.setAuthTag(tag);
        d.update(fromHex(ciphertext));
        d.final();
    },
    () => {
        const d = createDecryptor("aes-256-gcm", key, iv);
        d.setAAD(aad);
        d.setAuthTag(tag);
        const bad = fromHex(ciphertext); bad[0] ^= 1;
        d.update(bad);
        d.final();
    },
]) {
    expectError("VAEL_CRYPTO_AUTH", mutate);
}

// A missing tag is rejected before the primitive is asked to decrypt.
const missing = createDecryptor("aes-256-gcm", key, iv);
missing.setAAD(aad);
missing.update(fromHex(ciphertext));
expectError("VAEL_CRYPTO_TAG", () => missing.final());
// The failed object is finalized and cannot be retried.
expectError("VAEL_CRYPTO_STATE", () => missing.final());

// A wrong key and a wrong nonce are authentication failures, not plaintext.
for (const d of [
    (() => {
        const bad = key.slice(); bad[0] ^= 1;
        const x = createDecryptor("aes-256-gcm", bad, iv);
        x.setAAD(aad); x.setAuthTag(tag); x.update(fromHex(ciphertext)); return x;
    })(),
    (() => {
        const bad = iv.slice(); bad[0] ^= 1;
        const x = createDecryptor("aes-256-gcm", key, bad);
        x.setAAD(aad); x.setAuthTag(tag); x.update(fromHex(ciphertext)); return x;
    })(),
]) {
    expectError("VAEL_CRYPTO_AUTH", () => d.final());
}
"#,
    ));
}

#[test]
fn cipher_cbc_padding_and_no_padding_are_strict() {
    run_js(&format!(
        "{HELPERS}\n{}",
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";
const key = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const iv = fromHex("101112131415161718191a1b1c1d1e1f");

// Every length from 0 through 32 roundtrips with padding.
for (let length = 0; length <= 32; length++) {
    const plaintext = new Uint8Array(length);
    for (let i = 0; i < length; i++) plaintext[i] = (i * 7 + 3) & 255;
    const encryptor = createEncryptor("aes-256-cbc", key, iv);
    encryptor.update(plaintext);
    const ciphertext = encryptor.final();
    if (ciphertext.length % 16 !== 0 || ciphertext.length === 0) throw Error("CBC block length " + length);
    const decryptor = createDecryptor("aes-256-cbc", key, iv);
    decryptor.update(ciphertext);
    const out = decryptor.final();
    if (out.length !== length) throw Error("CBC length " + length);
    for (let i = 0; i < length; i++) if (out[i] !== plaintext[i]) throw Error("CBC byte " + length + "/" + i);
    encryptor.close();
    decryptor.close();
}

// No padding accepts only whole blocks.
const noPad = createEncryptor("aes-256-cbc", key, iv);
noPad.setAutoPadding(false);
noPad.update(new Uint8Array(15));
expectError("VAEL_CRYPTO_PADDING", () => noPad.final());

const block = new Uint8Array(16);
for (let i = 0; i < 16; i++) block[i] = i;
const noPadOk = createEncryptor("aes-256-cbc", key, iv, { padding: false });
noPadOk.update(block);
const ciphertext = noPadOk.final();
if (ciphertext.length !== 16) throw Error("no-padding length");
const noPadDecrypt = createDecryptor("aes-256-cbc", key, iv);
noPadDecrypt.setAutoPadding(false);
noPadDecrypt.update(ciphertext);
if (hex(noPadDecrypt.final()) !== hex(block)) throw Error("no-padding roundtrip");

// A non-block-multiple ciphertext is rejected even with padding enabled.
const short = createDecryptor("aes-256-cbc", key, iv);
short.update(new Uint8Array(15));
expectError("VAEL_CRYPTO_PADDING", () => short.final());

// An empty ciphertext has no PKCS#7 padding block.
const empty = createDecryptor("aes-256-cbc", key, iv);
expectError("VAEL_CRYPTO_PADDING", () => empty.final());

// A 16-byte no-padding ciphertext whose plaintext ends in 0x41 is invalid
// padding when decrypted with padding enabled.
const invalidPad = createDecryptor("aes-256-cbc", key, iv);
invalidPad.update(ciphertext);
expectError("VAEL_CRYPTO_PADDING", () => invalidPad.final());

// Tampering with the last ciphertext byte is rejected or yields invalid
// padding; either way no plaintext is returned.
const padded = createEncryptor("aes-256-cbc", key, iv);
padded.update(block);
const paddedCiphertext = padded.final();
const tampered = paddedCiphertext.slice();
tampered[tampered.length - 1] ^= 0x80;
const tamperedDecrypt = createDecryptor("aes-256-cbc", key, iv);
tamperedDecrypt.update(tampered);
expectError("VAEL_CRYPTO_PADDING", () => tamperedDecrypt.final());

// CBC computes no MAC. A wrong key is never reported as VAEL_CRYPTO_AUTH; it
// either yields different plaintext or invalid padding.
const wrongKey = key.slice();
wrongKey[0] ^= 1;
const wrongKeyDecrypt = createDecryptor("aes-256-cbc", wrongKey, iv);
wrongKeyDecrypt.update(paddedCiphertext);
try {
    const out = wrongKeyDecrypt.final();
    if (hex(out) === hex(block)) throw Error("wrong key roundtripped");
} catch (error) {
    if (error.code === "VAEL_CRYPTO_AUTH") throw Error("CBC implied a MAC");
    if (error.code !== "VAEL_CRYPTO_PADDING") throw error;
}
"#,
    ));
}

#[test]
fn cipher_lifecycle_and_argument_validation() {
    run_js(&format!(
        "{HELPERS}\n{}",
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";
const key = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const iv = fromHex("101112131415161718191a1b1c1d1e1f");
const gcmIv = fromHex("a1b2c3d4e5f60718293a4b5c");
const canonical = createEncryptor("aes-256-cbc", key, iv);

const cbc = createEncryptor("aes-256-cbc", key, iv);
cbc.update(new Uint8Array([1, 2, 3]));
cbc.final();
expectError("VAEL_CRYPTO_STATE", () => cbc.final());
expectError("VAEL_CRYPTO_STATE", () => cbc.update(new Uint8Array([4])));
expectError("VAEL_CRYPTO_STATE", () => cbc.setAutoPadding(false));
cbc.close();
cbc.close();
cbc.dispose();
expectError("VAEL_CRYPTO_CLOSED", () => cbc.update(new Uint8Array([4])));
expectError("VAEL_CRYPTO_CLOSED", () => cbc.final());

// GCM metadata ordering.
const gcm = createEncryptor("aes-256-gcm", key, gcmIv);
expectError("VAEL_CRYPTO_STATE", () => gcm.getAuthTag());
gcm.setAAD(new Uint8Array([1]));
expectError("VAEL_CRYPTO_STATE", () => gcm.setAAD(new Uint8Array([2])));
gcm.update(new Uint8Array([3]));
gcm.final();
if (gcm.getAuthTag().length !== 16) throw Error("tag length");

const aadAfterUpdate = createEncryptor("aes-256-gcm", key, gcmIv);
aadAfterUpdate.update(new Uint8Array([1]));
expectError("VAEL_CRYPTO_STATE", () => aadAfterUpdate.setAAD(new Uint8Array([2])));

// Algorithm and direction restrictions.
expectError("VAEL_CRYPTO_UNSUPPORTED", () => {
    const x = createEncryptor("aes-256-cbc", key, iv);
    x.setAAD(new Uint8Array());
});
expectError("VAEL_CRYPTO_UNSUPPORTED", () => {
    const x = createEncryptor("aes-256-gcm", key, gcmIv);
    x.setAutoPadding(false);
});
expectError("VAEL_CRYPTO_UNSUPPORTED", () => {
    const x = createEncryptor("aes-256-cbc", key, iv);
    x.getAuthTag();
});
expectError("VAEL_CRYPTO_UNSUPPORTED", () => {
    const x = createEncryptor("aes-256-gcm", key, gcmIv);
    x.setAuthTag(new Uint8Array(16));
});
expectError("VAEL_CRYPTO_UNSUPPORTED", () => {
    const x = createDecryptor("aes-256-cbc", key, iv);
    x.setAuthTag(new Uint8Array(16));
});

// Input and width validation.
expectError("VAEL_CRYPTO_ALGORITHM", () => createEncryptor("aes-128-cbc", key, iv));
expectError("VAEL_CRYPTO_KEY", () => createEncryptor("aes-256-cbc", new Uint8Array(31), iv));
expectError("VAEL_CRYPTO_IV", () => createEncryptor("aes-256-cbc", key, new Uint8Array(12)));
expectError("VAEL_CRYPTO_IV", () => createEncryptor("aes-256-gcm", key, new Uint8Array(16)));
expectError("VAEL_CRYPTO_TAG", () => {
    const x = createDecryptor("aes-256-gcm", key, gcmIv);
    x.setAuthTag(new Uint8Array(15));
});

expectTypeError(() => createEncryptor("aes-256-cbc", key));
expectTypeError(() => createEncryptor("aes-256-cbc", key, iv, undefined, undefined));
expectTypeError(() => createEncryptor("aes-256-cbc", key, iv, null));
expectTypeError(() => createEncryptor("aes-256-cbc", key, iv, { padding: 1 }));
expectTypeError(() => createEncryptor("aes-256-gcm", key, gcmIv, { padding: false }));
expectTypeError(() => canonical.update());
expectTypeError(() => canonical.update(new Uint8Array([1]), new Uint8Array([2])));
expectTypeError(() => canonical.update(new Uint8Array([1]), "hex"));
expectTypeError(() => canonical.final(undefined));
expectTypeError(() => canonical.setAAD());
expectTypeError(() => canonical.setAuthTag(new Uint8Array(16), undefined));
expectTypeError(() => canonical.getAuthTag(undefined));
expectTypeError(() => canonical.setAutoPadding("yes"));
expectTypeError(() => canonical.close(undefined));

// The class is not directly constructible.
const instance = createEncryptor("aes-256-cbc", key, iv);
if (Object.getPrototypeOf(instance).constructor.name !== "Cipher") throw Error("class name");
"#,
    ));
}

#[test]
fn cipher_string_utf8_and_encoding_rules() {
    run_js(&format!(
        "{HELPERS}\n{}",
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";
const key = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const iv = fromHex("101112131415161718191a1b1c1d1e1f");

const expected = fromHex(
    "4772c3bcc39f652c20e4b896e7958c2c20f09fa680",
);
const encryptor = createEncryptor("aes-256-cbc", key, iv);
if (encryptor.update("Grüße, 世界, 🦀", "utf-8").length !== 0) throw Error("update length");
const ciphertext = encryptor.final();
const decryptor = createDecryptor("aes-256-cbc", key, iv);
decryptor.update(ciphertext);
const out = decryptor.final();
if (hex(out) !== hex(expected)) throw Error("UTF-8 roundtrip");

expectTypeError(() => createEncryptor("aes-256-cbc", key, iv).update("abc", "latin1"));
expectTypeError(() => createEncryptor("aes-256-cbc", key, iv).update(new Uint8Array([1]), "utf8"));
// A lone surrogate becomes U+FFFD, matching Node's UTF-8 conversion.
const lone = createEncryptor("aes-256-cbc", key, iv);
lone.update("\ud800");
const loneCiphertext = lone.final();
const loneDecryptor = createDecryptor("aes-256-cbc", key, iv);
loneDecryptor.update(loneCiphertext);
if (hex(loneDecryptor.final()) !== "efbfbd") throw Error("U+FFFD replacement");
"#,
    ));
}

#[test]
fn cipher_exports_are_registry_functions_and_the_envelope_stays_separate() {
    run_js(
        r#"
import * as crypto from "vael:crypto";
import { createDecryptor, createEncryptor, decrypt, encrypt, generateEnvelopeKey } from "vael:crypto";

const registry = Vael.getBuiltInModule("vael:crypto");
if (createEncryptor !== registry.createEncryptor) throw Error("createEncryptor identity");
if (createDecryptor !== registry.createDecryptor) throw Error("createDecryptor identity");
if (typeof registry.createEncryptor !== "function") throw Error("createEncryptor missing");
if ("createCipheriv" in registry || "createDecipheriv" in registry) {
    throw Error("Node cipher names must not be exported");
}
if (import.meta.resolve("vael:crypto") !== "vael:crypto") throw Error("resolve");

// The NEO1 envelope and the cipher surface are separate: a cipher output is
// not an envelope and the envelope functions do not see cipher objects.
const key = generateEnvelopeKey();
const envelope = encrypt(key, new Uint8Array([1, 2, 3]));
if (envelope[0] !== 0x4e || envelope[1] !== 0x45 || envelope[2] !== 0x4f || envelope[3] !== 0x31) {
    throw Error("envelope magic");
}
const iv = new Uint8Array(16);
const cipher = createEncryptor("aes-256-cbc", key, iv);
cipher.update(envelope);
const ciphertext = cipher.final();
if (ciphertext.length === envelope.length + 34) throw Error("cipher text looked like an envelope");
try {
    decrypt(key, ciphertext);
    throw Error("decrypt accepted a raw ciphertext");
} catch (error) {
    if (error.code !== "VAEL_CRYPTO_FORMAT" && error.code !== "VAEL_CRYPTO_AUTH") throw error;
}
"#,
    );
}

#[test]
fn cipher_resource_caps_and_size_limit() {
    run_js(
        r#"
import { createEncryptor } from "vael:crypto";
const key = new Uint8Array(32);
const iv = new Uint8Array(16);

// One object accepts exactly 8 MiB, and the next byte is rejected before it
// is copied.
const atLimit = createEncryptor("aes-256-cbc", key, iv);
atLimit.update(new Uint8Array(8 * 1024 * 1024));
try {
    atLimit.update(new Uint8Array(1));
    throw Error("over-limit update succeeded");
} catch (error) {
    if (error.code !== "VAEL_CRYPTO_LIMIT") throw error;
}
atLimit.close();

// At most 64 objects may be open. Closing one releases its slot.
const open = [];
for (let i = 0; i < 64; i++) open.push(createEncryptor("aes-256-cbc", key, iv));
let tooMany = null;
try {
    createEncryptor("aes-256-cbc", key, iv);
} catch (error) {
    tooMany = error;
}
if (tooMany === null || tooMany.code !== "VAEL_CRYPTO_TOO_MANY_OBJECTS") {
    throw Error("open-object cap was not enforced");
}
open[0].close();
const replacement = createEncryptor("aes-256-cbc", key, iv);
replacement.close();
for (const cipher of open) cipher.close();
"#,
    );
}

#[test]
fn cipher_rejects_a_detached_input() {
    run_js(
        r#"
import { createEncryptor } from "vael:crypto";
const key = new Uint8Array(32);
const iv = new Uint8Array(16);
const cipher = createEncryptor("aes-256-cbc", key, iv);
const input = new Uint8Array(4);
input.buffer.transfer();
try {
    cipher.update(input);
    throw Error("accepted a detached view");
} catch (error) {
    if (error.code !== "VAEL_CRYPTO_DETACHED") throw error;
}
"#,
    );
}

#[test]
fn cipher_rejects_unknown_options_before_resource_admission() {
    run_js(&format!(
        "{HELPERS}\n{}",
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";

const key = new Uint8Array(32);
const iv = new Uint8Array(16);
const gcmIv = new Uint8Array(12);
const factories = [createEncryptor, createDecryptor];

class Options {
    constructor() {
        this.padding = false;
    }
}

// Unknown keys, both factories and both modes.
for (const factory of factories) {
    expectTypeError(() => factory("aes-256-cbc", key, iv, { autoPadding: false }));
    expectTypeError(() => factory("aes-256-cbc", key, iv, { paddding: false }));
    expectTypeError(() => factory("aes-256-cbc", key, iv, { padding: true, extra: 1 }));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, { autoPadding: false }));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, { authTagLength: 16 }));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, { padding: false }));
}

// Only plain and null-prototype records are accepted.
for (const factory of factories) {
    expectTypeError(() => factory("aes-256-cbc", key, iv, []));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, []));
    expectTypeError(() => factory("aes-256-cbc", key, iv, new Map()));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, new Map()));
    expectTypeError(() => factory("aes-256-cbc", key, iv, () => {}));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, () => {}));
    expectTypeError(() => factory("aes-256-cbc", key, iv, new Options()));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, new Options()));
    expectTypeError(() => factory("aes-256-cbc", key, iv, Object.create({ padding: false })));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, Object.create({ padding: false })));
}

// An inherited getter is never consulted, for either mode or factory.
let inheritedGetter = 0;
const inheritedPrototype = {};
Object.defineProperty(inheritedPrototype, "padding", {
    get() {
        inheritedGetter += 1;
        return false;
    },
});
const inherited = Object.create(inheritedPrototype);
for (const factory of factories) {
    expectTypeError(() => factory("aes-256-cbc", key, iv, inherited));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, inherited));
}
if (inheritedGetter !== 0) throw Error("inherited getter ran");

// An own accessor is rejected without invoking its getter.
let accessorGetter = 0;
const accessor = {};
Object.defineProperty(accessor, "padding", {
    enumerable: true,
    configurable: true,
    get() {
        accessorGetter += 1;
        return false;
    },
});
for (const factory of factories) {
    expectTypeError(() => factory("aes-256-cbc", key, iv, accessor));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, accessor));
}
if (accessorGetter !== 0) throw Error("accessor getter ran");

// Non-enumerable data properties are rejected, including unknown keys.
const nonEnumerable = {};
Object.defineProperty(nonEnumerable, "padding", {
    value: false,
    enumerable: false,
    configurable: true,
});
const nonEnumerableUnknown = {};
Object.defineProperty(nonEnumerableUnknown, "autoPadding", {
    value: false,
    enumerable: false,
    configurable: true,
});
for (const factory of factories) {
    expectTypeError(() => factory("aes-256-cbc", key, iv, nonEnumerable));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, nonEnumerable));
}
expectTypeError(() => createEncryptor("aes-256-cbc", key, iv, nonEnumerableUnknown));

// Symbol keys are rejected.
const symbolOptions = { [Symbol("padding")]: false };
for (const factory of factories) {
    expectTypeError(() => factory("aes-256-cbc", key, iv, symbolOptions));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, symbolOptions));
}

// A Proxy is rejected before any trap runs.
let traps = 0;
const proxy = new Proxy({ padding: false }, {
    get() {
        traps += 1;
        return false;
    },
    getPrototypeOf() {
        traps += 1;
        return Object.prototype;
    },
    ownKeys() {
        traps += 1;
        return ["padding"];
    },
    getOwnPropertyDescriptor() {
        traps += 1;
        return { value: false, enumerable: true, configurable: true };
    },
});
for (const factory of factories) {
    expectTypeError(() => factory("aes-256-cbc", key, iv, proxy));
    expectTypeError(() => factory("aes-256-gcm", key, gcmIv, proxy));
}
if (traps !== 0) throw Error("Proxy traps ran while validating options");

// Valid plain, null-prototype, and empty GCM records work.
const plain = createEncryptor("aes-256-cbc", key, iv, { padding: false });
plain.update(new Uint8Array(16));
if (plain.final().length !== 16) throw Error("plain options");
plain.close();
const nullPrototype = Object.assign(Object.create(null), { padding: false });
const nullPrototypeDecryptor = createDecryptor("aes-256-cbc", key, iv, nullPrototype);
nullPrototypeDecryptor.close();
const emptyOptions = createEncryptor("aes-256-gcm", key, gcmIv, {});
emptyOptions.close();
const nullEmpty = createDecryptor("aes-256-gcm", key, gcmIv, Object.create(null));
nullEmpty.close();

// Leave one object slot. Every invalid option must be a TypeError and must not
// reserve that slot: the next valid create succeeds. Once the cap is full, both
// factories report the cap, invalid options are still TypeErrors, and closing
// an object makes a slot available again.
const open = [];
for (let i = 0; i < 63; i++) {
    open.push(createEncryptor("aes-256-cbc", key, iv));
}
for (const factory of factories) {
    expectTypeError(() => factory("aes-256-cbc", key, iv, { autoPadding: false }));
    expectTypeError(() => factory("aes-256-cbc", key, iv, accessor));
    expectTypeError(() => factory("aes-256-cbc", key, iv, symbolOptions));
    expectTypeError(() => factory("aes-256-cbc", key, iv, proxy));
    expectTypeError(() => factory("aes-256-cbc", key, iv, nonEnumerable));
}
const last = createEncryptor("aes-256-cbc", key, iv);
for (const factory of factories) {
    try {
        factory("aes-256-cbc", key, iv);
        throw Error("the open-object cap was not enforced");
    } catch (error) {
        if (error.code !== "VAEL_CRYPTO_TOO_MANY_OBJECTS") throw error;
    }
    expectTypeError(() => factory("aes-256-cbc", key, iv, { autoPadding: false }));
    expectTypeError(() => factory("aes-256-cbc", key, iv, proxy));
}
last.close();
const replacement = createEncryptor("aes-256-cbc", key, iv);
replacement.close();
const replacementDecryptor = createDecryptor("aes-256-cbc", key, iv);
replacementDecryptor.close();
for (const cipher of open) cipher.close();
"#,
    ));
}

#[test]
fn cipher_readme_try_finally_example_runs() {
    run_js(
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";

const fromHex = (text) => Uint8Array.from(text.match(/../g), (byte) => parseInt(byte, 16));
const toHex = (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");

// NIST SP 800-38A F.2.5 AES-256-CBC test vector, no padding.
const cbcKey = fromHex("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
const cbcIv = fromHex("000102030405060708090a0b0c0d0e0f");
const cbcPlaintext = fromHex(
    "6bc1bee22e409f96e93d7e117393172a" +
    "ae2d8a571e03ac9c9eb76fac45af8e51" +
    "30c81c46a35ce411e5fbc1191a0a52ef" +
    "f69f2445df4f9b17ad2b417be66c3710",
);

let encryptor;
try {
    encryptor = createEncryptor("aes-256-cbc", cbcKey, cbcIv, { padding: false });
    encryptor.update(cbcPlaintext);
    const ciphertext = encryptor.final();
    if (toHex(ciphertext) !==
        "f58c4c04d6e5f1ba779eabfb5f7bfbd6" +
        "9cfc4e967edb808d679f777bc6702c7d" +
        "39f23369a9d9bacfa530e26304231461" +
        "b2eb05e2c39be9fcda6c19078c6a9d1b") {
        throw new Error("CBC vector mismatch");
    }
} finally {
    encryptor?.dispose();
}

// Nonsecret Node/OpenSSL AES-256-GCM fixture with AAD.
const gcmKey = fromHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
const gcmIv = fromHex("a1b2c3d4e5f60718293a4b5c");
const gcmAad = fromHex("6e656f6a732d6369706865722d636f6e74657874");
const gcmCiphertext = fromHex("6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1");
const gcmTag = fromHex("9ea6507bfbc87455b1f24edc000cc662");

function decryptGcm(tag) {
    let decryptor;
    try {
        decryptor = createDecryptor("aes-256-gcm", gcmKey, gcmIv);
        decryptor.setAAD(gcmAad);
        decryptor.setAuthTag(tag);
        decryptor.update(gcmCiphertext);
        return decryptor.final();
    } catch (error) {
        if (error.code === "VAEL_CRYPTO_AUTH") {
            throw new Error("GCM authentication failed; do not use the message");
        }
        throw error;
    } finally {
        decryptor?.dispose();
    }
}

if (toHex(decryptGcm(gcmTag)) !== "6e656f6a73206369706865722067636d20696e7465726f70") {
    throw new Error("GCM vector mismatch");
}

// A tampered tag must fail closed. The error path is part of the example so
// callers copy it; the plaintext was never returned.
const tamperedTag = gcmTag.slice();
tamperedTag[0] ^= 1;
try {
    decryptGcm(tamperedTag);
    throw new Error("tampered GCM tag was accepted");
} catch (error) {
    if (error.message !== "GCM authentication failed; do not use the message") {
        throw error;
    }
}
"#,
    );
}

#[test]
fn cipher_ignores_inherited_intrinsic_prototype_options() {
    run_js(
        r#"
import { createDecryptor, createEncryptor } from "vael:crypto";

const key = new Uint8Array(32);
const iv = new Uint8Array(16);
const gcmIv = new Uint8Array(12);
const originalPadding = Object.getOwnPropertyDescriptor(Object.prototype, "padding");
const originalUnknown =
    Object.getOwnPropertyDescriptor(Object.prototype, "vaelCipherInheritedUnknown");
let getterCalls = 0;

try {
    Object.defineProperty(Object.prototype, "padding", {
        configurable: true,
        get() {
            getterCalls += 1;
            return false;
        },
    });
    Object.defineProperty(Object.prototype, "vaelCipherInheritedUnknown", {
        configurable: true,
        get() {
            getterCalls += 1;
            return false;
        },
    });

    // The inherited padding getter must not run. Default CBC padding is on:
    // empty plaintext produces one full padding block.
    const encryptor = createEncryptor("aes-256-cbc", key, iv, {});
    encryptor.update(new Uint8Array(0));
    const ciphertext = encryptor.final();
    if (ciphertext.length !== 16) {
        throw Error("inherited padding changed the CBC default");
    }
    encryptor.close();

    const decryptor = createDecryptor("aes-256-cbc", key, iv, {});
    decryptor.update(ciphertext);
    if (decryptor.final().length !== 0) {
        throw Error("inherited padding affected CBC decryption");
    }
    decryptor.close();

    // GCM has no own option keys; inherited unknown properties are ignored and
    // must not make the empty record invalid.
    const gcm = createEncryptor("aes-256-gcm", key, gcmIv, {});
    gcm.update(new Uint8Array(0));
    gcm.final();
    gcm.close();
} finally {
    if (originalPadding === undefined) {
        delete Object.prototype.padding;
    } else {
        Object.defineProperty(Object.prototype, "padding", originalPadding);
    }
    if (originalUnknown === undefined) {
        delete Object.prototype.vaelCipherInheritedUnknown;
    } else {
        Object.defineProperty(Object.prototype, "vaelCipherInheritedUnknown", originalUnknown);
    }
}

if (getterCalls !== 0) throw Error("an inherited options getter ran");
"#,
    );
}

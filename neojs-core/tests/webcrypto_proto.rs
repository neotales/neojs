//! Experimental integration tests for the `webcrypto-proto` feature.
//!
//! These run only when the feature is enabled. They check the Web Crypto
//! surface against published SHA-256 and AES-256-GCM vectors, the random
//! globals, `DOMException` error mapping, and the absence of unrelated
//! `deno_web` globals. They do not claim that the surface is secure or
//! complete.

#![cfg(feature = "webcrypto-proto")]

use std::fs;
use std::time::Duration;
use std::time::Instant;

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
fn sha256_matches_published_vectors() {
    run_js(
        r#"
const toHex = (bytes) =>
    Array.from(new Uint8Array(bytes), (b) => b.toString(16).padStart(2, "0")).join("");

const empty = await crypto.subtle.digest("SHA-256", new Uint8Array(0));
if (toHex(empty) !== "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855") {
    throw new Error("empty SHA-256 mismatch: " + toHex(empty));
}

const abc = await crypto.subtle.digest("SHA-256", Uint8Array.from([0x61, 0x62, 0x63]));
if (toHex(abc) !== "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad") {
    throw new Error("abc SHA-256 mismatch: " + toHex(abc));
}

for (const input of [
    new Uint8Array([0, 1, 2, 255]),
    new Uint8Array([0, 1, 2, 255]).buffer,
    new DataView(new Uint8Array([0, 1, 2, 255]).buffer),
]) {
    const digest = await crypto.subtle.digest({ name: "SHA-256" }, input);
    if (!(digest instanceof ArrayBuffer) || digest.byteLength !== 32) {
        throw new Error("digest result is not a 32-byte ArrayBuffer");
    }
}
"#,
    );
}

#[test]
fn aes_gcm_matches_nist_zero_vector_and_rejects_tampering() {
    run_js(
        r#"
const hex = (bytes) =>
    Array.from(new Uint8Array(bytes), (b) => b.toString(16).padStart(2, "0")).join("");
const bytes = (text) => Uint8Array.from(text.match(/../g), (pair) => parseInt(pair, 16));

const keyBytes = new Uint8Array(32);
const iv = new Uint8Array(12);
const plaintext = new Uint8Array(16);
const expectedCiphertext = bytes("cea7403d4d606b6e074ec5d3baf39d18d0d1c8a799996bf0265b98b5d48ab919");

const key = await crypto.subtle.importKey("raw", keyBytes, "AES-GCM", false, ["encrypt", "decrypt"]);
const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, plaintext);
if (hex(sealed) !== hex(expectedCiphertext)) {
    throw new Error("AES-GCM ciphertext/tag mismatch: " + hex(sealed));
}

const opened = await crypto.subtle.decrypt({ name: "AES-GCM", iv }, key, expectedCiphertext);
if (opened.byteLength !== 16 || opened.byteLength !== plaintext.byteLength) {
    throw new Error("AES-GCM plaintext length mismatch");
}
for (const value of new Uint8Array(opened)) {
    if (value !== 0) throw new Error("AES-GCM plaintext is not zero");
}

// A changed tag must fail as an OperationError, not return plaintext.
const tampered = expectedCiphertext.slice();
tampered[tampered.length - 1] ^= 1;
let failure;
try {
    await crypto.subtle.decrypt({ name: "AES-GCM", iv }, key, tampered);
} catch (error) {
    failure = error;
}
if (!(failure instanceof DOMException) || failure.name !== "OperationError") {
    throw new Error(`tampered tag produced ${failure && failure.name}, not OperationError`);
}

// Associated data must be authenticated too.
const aad = new Uint8Array([1, 2, 3]);
const sealedWithAad = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv, additionalData: aad },
    key,
    plaintext,
);
const openedWithAad = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv, additionalData: aad },
    key,
    sealedWithAad,
);
if (new Uint8Array(openedWithAad).some((value) => value !== 0)) {
    throw new Error("AES-GCM associated-data roundtrip mismatch");
}
"#,
    );
}

#[test]
fn random_globals_are_bounded_and_random() {
    run_js(
        r#"
const buffer = new Uint8Array(32);
const returned = crypto.getRandomValues(buffer);
if (returned !== buffer) throw new Error("getRandomValues did not return its argument");
if (!crypto.getRandomValues(new Uint8Array(16)).some((value) => value !== 0)) {
    throw new Error("getRandomValues produced all zeroes");
}

const id = crypto.randomUUID();
if (!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id)) {
    throw new Error("randomUUID is not a v4 UUID: " + id);
}
if (crypto.randomUUID() === id) throw new Error("randomUUID repeated");

let quota;
try {
    crypto.getRandomValues(new Uint8Array(65537));
} catch (error) {
    quota = error;
}
if (!(quota instanceof DOMException) || quota.name !== "QuotaExceededError") {
    throw new Error(`oversized getRandomValues produced ${quota && quota.name}`);
}

let mismatch;
try {
    crypto.getRandomValues(new Float32Array(4));
} catch (error) {
    mismatch = error;
}
if (!(mismatch instanceof DOMException) || mismatch.name !== "TypeMismatchError") {
    throw new Error(`float getRandomValues produced ${mismatch && mismatch.name}`);
}
"#,
    );
}

#[test]
fn crypto_surface_is_present_without_deno_runtime_globals() {
    run_js(
        r#"
if (!(crypto instanceof Crypto)) throw new Error("crypto is not a Crypto");
if (!(crypto.subtle instanceof SubtleCrypto)) throw new Error("subtle is not a SubtleCrypto");
if (CryptoKey.name !== "CryptoKey") throw new Error("CryptoKey global missing");
if (crypto.subtle.digest.length !== 2) throw new Error("digest.length");
if (crypto.subtle.encrypt.length !== 3) throw new Error("encrypt.length");
if (typeof CryptoKey.prototype !== "object") throw new Error("CryptoKey prototype missing");

// The crypto global is a non-writable value, not an accessor.
const descriptor = Object.getOwnPropertyDescriptor(globalThis, "crypto");
if (!descriptor || descriptor.writable !== false || descriptor.get !== undefined) {
    throw new Error("crypto global descriptor");
}
if (typeof isSecureContext !== "undefined") throw new Error("secure-context global appeared");

// Only Deno.core exists: no Deno runtime, no deno_web globals.
if (typeof Deno !== "object" || typeof Deno.core !== "object") throw new Error("Deno.core missing");
for (const name of ["readFile", "writeFile", "env", "args", "exit", "serve"]) {
    if (typeof Deno[name] !== "undefined") throw new Error(`Deno.${name} appeared`);
}
for (const name of ["Event", "EventTarget", "TextEncoder", "TextDecoder", "URL", "Blob", "File", "performance", "setTimeout", "btoa", "atob"]) {
    if (typeof globalThis[name] !== "undefined") throw new Error(`${name} appeared`);
}
"#,
    );
}

#[test]
fn webcrypto_rejections_are_domexceptions() {
    let message = run_js_error(
        r#"
const error = await crypto.subtle.digest("SHA-999", new Uint8Array(1)).catch((error) => error);
if (!(error instanceof DOMException)) throw new Error("unknown algorithm did not reject with DOMException");
if (error.name !== "NotSupportedError") throw new Error("unknown algorithm name: " + error.name);
throw error;
"#,
    );
    assert!(message.contains("NotSupportedError"), "{message}");
}

#[test]
fn subtle_methods_always_return_promises() {
    run_js(
        r#"
// Invalid arguments must reject the returned promise, never throw before it
// is created. This is the contract the `deno_crypto` async forwarders exist to
// preserve.
for (const call of [
    () => crypto.subtle.digest("SHA-256", "not bytes"),
    () => crypto.subtle.digest(42, new Uint8Array(1)),
    () => crypto.subtle.encrypt({ name: "AES-GCM", iv: new Uint8Array(12) }),
]) {
    let promise;
    try {
        promise = call();
    } catch (error) {
        throw new Error("threw synchronously: " + error);
    }
    if (!(promise instanceof Promise)) throw new Error("did not return a Promise");
    const reason = await promise.catch((error) => error);
    if (!(reason instanceof TypeError)) {
        throw new Error("rejection is not a TypeError: " + reason);
    }
}
"#,
    );
}

#[test]
fn digest_rejects_a_non_buffer_source_with_type_error() {
    let message = run_js_error(
        r#"
await crypto.subtle.digest("SHA-256", "not bytes");
"#,
    );
    assert!(
        message.contains("TypeError") || message.to_lowercase().contains("buffer"),
        "{message}"
    );
}

/// Measures whether blocking algorithms run concurrently and compares that
/// with sequential awaits. Run explicitly with `--ignored`; it allocates
/// several 32 MiB inputs.
#[test]
#[ignore = "allocates several 32 MiB inputs and measures wall time"]
fn digest_runs_concurrently_on_the_blocking_pool() {
    run_js(
        r#"
const MiB = 1024 * 1024;
const input = new Uint8Array(32 * MiB);

async function sequential() {
    const start = Date.now();
    for (let i = 0; i < 4; i += 1) {
        await crypto.subtle.digest("SHA-256", input);
    }
    return Date.now() - start;
}

async function concurrent() {
    const start = Date.now();
    await Promise.all(Array.from({ length: 4 }, () => crypto.subtle.digest("SHA-256", input)));
    return Date.now() - start;
}

const serial = await sequential();
const parallel = await concurrent();
console.log(`digest 4x32MiB sequential=${serial}ms concurrent=${parallel}ms`);
// Four blocking tasks should overlap; the ratio is a smoke check, not a
// guarantee about a loaded machine.
if (!(parallel < serial)) {
    throw new Error(`no overlap: sequential=${serial}ms concurrent=${parallel}ms`);
}
"#,
    );
}

/// Shows that an unawaited blocking algorithm keeps the module future pending,
/// and that dropping the runtime does not wait for the blocking job.
///
/// `run_file_async` needs a Tokio runtime on the polling thread, which is
/// supplied here by a current-thread runtime. The PBKDF2 job keeps running on
/// the blocking pool after the timeout and after the `Runtime` is dropped; no
/// JavaScript or Rust API in this surface can cancel it.
#[test]
#[ignore = "runs a multi-second PBKDF2 job to measure cancellation behaviour"]
fn runtime_drop_does_not_wait_for_a_running_blocking_job() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.js");
    fs::write(
        &file,
        r#"
const key = await crypto.subtle.importKey(
    "raw",
    new Uint8Array(16),
    "PBKDF2",
    false,
    ["deriveBits"],
);
// Deliberately not awaited. The runtime still tracks the pending op, so the
// module future does not complete until the derivation finishes.
crypto.subtle.deriveBits(
    { name: "PBKDF2", salt: new Uint8Array(16), iterations: 4_000_000, hash: "SHA-256" },
    key,
    256,
);
"#,
    )
    .expect("write source");

    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("executor");
    let mut runtime = Runtime::new().expect("runtime");
    let started = Instant::now();
    let outcome = executor.block_on(async {
        tokio::time::timeout(Duration::from_millis(300), runtime.run_file_async(&file)).await
    });
    let pending = started.elapsed();
    assert!(
        outcome.is_err(),
        "the unawaited job must keep the future pending"
    );

    let drop_started = Instant::now();
    drop(runtime);
    let dropped = drop_started.elapsed();
    println!("pending after {pending:?}; dropping the runtime took {dropped:?}");
    assert!(
        dropped < Duration::from_millis(100),
        "dropping the runtime waited for blocking work: {dropped:?}"
    );

    // Do not wait for the detached blocking job at the end of the test.
    executor.shutdown_background();
}

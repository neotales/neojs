//! Synchronous AES-256-GCM authenticated encryption for `neo:crypto`.
//!
//! The engine-independent envelope, associated-data construction, size policy,
//! CSPRNG calls, and error categories live in the `neojs-crypto-core` crate,
//! which the V8 and QuickJS runtimes share. This module keeps the V8 `op2` glue,
//! the internal `Uint8Array` validation and copying, and the result-object
//! protocol that the JavaScript wrapper in `neo.js` turns into exceptions.
//!
//! The envelope is specific to neojs. The primitive is standard AES-256-GCM
//! (NIST SP 800-38D), and the shared core's unit tests check it against
//! published vectors, but another tool must reproduce this envelope and
//! associated-data construction to interoperate. Two fixtures were produced
//! independently with Node's OpenSSL-backed AES-256-GCM: the shared core tests
//! supply their nonces to the deterministic path and compare the emitted
//! envelope byte for byte, while the JavaScript integration tests decrypt them
//! and roundtrip envelopes emitted through the public API with a random nonce.
//! The public API does not expose a nonce, so the JavaScript layer does not do
//! a byte-for-byte emission comparison.
//!
//! # Nonces
//!
//! [`encrypt`] draws a fresh 96-bit nonce from the operating system CSPRNG via
//! `getrandom` for every call. The nonce is not derived from a counter, and
//! the caller cannot choose it. A random 96-bit nonce is safe for a limited
//! number of messages under one key: the probability of any collision reaches
//! about 2^-32 at roughly 2^32 encryptions, so a key should be retired well
//! before that. Reusing a key with a repeated nonce destroys AES-GCM's
//! confidentiality and authenticity guarantees; the module avoids the common
//! cause by never exposing a nonce-reuse path, but key lifetime is still the
//! caller's responsibility.
//!
//! # Randomness
//!
//! Every random value in this module comes from the operating system CSPRNG
//! through `getrandom`: [`generate_envelope_key`], the per-message nonce drawn by
//! [`encrypt`], and [`random_bytes`]. There is no `Math.random`, time-based,
//! or other weaker fallback. If the operating system refuses to supply
//! entropy, the call fails with `NEO_CRYPTO_RANDOM`; it never substitutes
//! predictable bytes. [`random_bytes`] is bounded by [`MAX_RANDOM_BYTES`] and
//! allocates fallibly before asking for the bytes.
//!
//! The tests for these functions check length, type, range, and error
//! behaviour. They do not and cannot prove that the output is statistically
//! random or cryptographically secure; that property comes from the operating
//! system CSPRNG, not from the tests.
//!
//! # Keys and memory
//!
//! Keys are exactly 32 bytes. [`generate_envelope_key`] returns 32 CSPRNG bytes; keys
//! can also come from any other vetted CSPRNG. The module does not store,
//! derive, wrap, rotate, or persist keys. Callers must keep the key secret,
//! use a distinct key per purpose, and protect the places it is stored.
//!
//! This module does not promise zeroization. Key bytes are copied into Rust
//! buffers, plaintext and ciphertext are copied into Rust buffers, and the
//! JavaScript heap retains its own copies. Those copies are reclaimed
//! normally, but are not guaranteed to be erased first. Enabling the
//! `zeroize` feature of `aes-gcm` clears one temporary GHASH subkey inside
//! the cipher; it does not clear the AES key schedule, the input buffers, or
//! the JavaScript copies. Callers that require memory erasure must arrange it
//! at a lower layer. No secret is logged or included in an error message.
//!
//! # Limits and allocation
//!
//! The public module accepts at most [`MAX_PLAINTEXT_BYTES`] of plaintext,
//! [`MAX_ASSOCIATED_DATA_BYTES`] of caller associated data, an envelope of at
//! most [`MAX_ENVELOPE_BYTES`], and at most [`MAX_RANDOM_BYTES`] in one
//! `randomBytes` call. The JavaScript-facing checks run before a view is
//! copied or an input buffer is allocated. The core functions use checked
//! arithmetic and fallible reservations; a failed reservation is reported as
//! `NEO_CRYPTO_LIMIT`.
//!
//! That mapping is best effort, not a guarantee. The JavaScript wrapper and
//! V8 allocate too, and an allocation failure inside V8, the Rust allocator,
//! or the JavaScript heap can abort the process or raise an error the module
//! cannot catch, so recovery from out-of-memory is not guaranteed.
//!
//! # JavaScript contract
//!
//! The module object is installed in the realm before user code runs and is
//! exposed as `Neo.getBuiltInModule("crypto")`, `import ... from "crypto"`,
//! and `import ... from "neo:crypto"`. All three paths share one frozen
//! module object and one set of function objects. The module is available in
//! every runtime; there is no secure-context or TLS gate. Every function is
//! synchronous, takes `Uint8Array` values or, for `randomBytes`, one number,
//! and returns a `Uint8Array`.
//!
//! The host ops return a result object rather than throwing. The module
//! wrapper, which captured `Error`, `TypeError`, and `Object.defineProperty`
//! before user code ran, turns a failure into the documented exception and
//! defines `code` as an own property. A non-`Uint8Array` argument throws
//! `TypeError`. Every other failure throws `Error` with one of the stable
//! codes in [`CryptoErrorCode`]. This is not isolation from other code in the
//! same realm, which can still read the arguments and return values it
//! handles.
//!
//! Argument counts are strict, so a callback or other unsupported argument is
//! never silently ignored. `generateEnvelopeKey` takes no arguments,
//! `randomBytes` exactly one, and `encrypt` and `decrypt` two or three; any
//! other count throws `TypeError` before the host op runs. `randomBytes` has
//! no callback form in this slice.
//!
//! # Scope
//!
//! This module implements only [`generate_envelope_key`], [`random_bytes`],
//! [`encrypt`], and [`decrypt`]. It is not Node's `crypto` module and not the
//! Web Crypto API. `crypto.subtle` and a PLAB .NET Crypto-compatible
//! encrypt-then-MAC surface are not implemented here. Hashes (`createHash`),
//! HMAC (`createHmac`), bcrypt, and PBKDF2 are in `crypto_hash`; Shamir secret
//! sharing is in `crypto_shamir`. The stateful
//! `createEncryptor`/`createDecryptor` CBC/GCM surface is in `crypto_cipher`;
//! it does not change this NEO1 envelope. These are additive APIs: the
//! functions above are the synchronous neojs slice and must not be repurposed
//! for a Node-like, WebCrypto-like, or PLAB API contract. Shamir shares use the
//! PLAB GF(256) encoding, but that is a data-format compatibility, not a PLAB
//! API or authentication claim.
//! The envelope generator is `generateEnvelopeKey`. The name `generateKey` is
//! deliberately not exported: it is reserved for a future Node-compatible
//! surface whose `generateKey` has different semantics. WebCrypto's
//! `subtle.generateKey` is separate too.
//!
//! Asynchronous APIs are future work. This slice returns no `Promise` and has
//! no callback form; in particular, it does not wrap blocking work in an
//! already-resolved `Promise` to look nonblocking. A later async surface (for
//! example a WebCrypto-style `crypto.subtle`) must do its work on a real
//! asynchronous path, return a `Promise`, and will be additive.

use deno_core::op2;
use deno_core::v8;

pub(crate) use neojs_crypto_core::{
    CryptoError, CryptoErrorCode, CryptoLimits, KEY_LEN, MAX_ASSOCIATED_DATA_BYTES,
    MAX_ENVELOPE_BYTES, MAX_RANDOM_BYTES, NONCE_LEN, TAG_LEN, decrypt, encrypt,
    generate_envelope_key, random_bytes,
};

/// A validated `Uint8Array` argument, before its bytes are copied.
///
/// Shared with [`crate::crypto_hash`], which applies the same typed-array
/// validation to hash and HMAC inputs.
pub(crate) struct View<'s> {
    pub(crate) view: v8::Local<'s, v8::Uint8Array>,
    pub(crate) length: usize,
}

/// Validate a JavaScript value as a `Uint8Array` and read its length.
///
/// V8's internal type check is used directly, so `Symbol.hasInstance` and
/// `byteLength` getters written in JavaScript are never consulted. The value's
/// view range, not its backing buffer, is what the later copy reads: an
/// offset view copies only its own bytes. A detached view is rejected rather
/// than treated as an empty one. Resizable and shared backing buffers are
/// accepted; no JavaScript runs between reading the length and copying, so a
/// resizable buffer cannot change size in between. V8 exposes a view of a
/// resizable buffer that shrank until the view is out of bounds as a
/// zero-length view, so such a view is encrypted as empty bytes rather than
/// rejected; no stale memory is read.
pub(crate) fn checked_view<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<View<'s>, CryptoError> {
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(CryptoError::new(CryptoErrorCode::Type));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(CryptoError::new(CryptoErrorCode::Detached));
    };
    if buffer.was_detached() {
        return Err(CryptoError::new(CryptoErrorCode::Detached));
    }
    let length = view.byte_length();
    Ok(View { view, length })
}

/// Copy the validated view's bytes into a new buffer.
///
/// The caller checks `length` against the relevant bound before this runs, so
/// an oversized input is rejected without allocating or copying it.
pub(crate) fn copy_view(view: &View<'_>) -> Result<Vec<u8>, CryptoError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(view.length)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    bytes.resize(view.length, 0);
    let written = view.view.copy_contents(&mut bytes);
    if written != view.length {
        // No JavaScript runs during this op, so the view cannot have changed.
        // A short copy would mean V8 considers the view detached or out of
        // bounds after `byte_length`; fail instead of using a partial buffer.
        return Err(CryptoError::new(CryptoErrorCode::Detached));
    }
    Ok(bytes)
}

fn key_from_view(view: &View<'_>) -> Result<Vec<u8>, CryptoError> {
    if view.length != KEY_LEN {
        Err(CryptoError::new(CryptoErrorCode::Key))
    } else {
        copy_view(view)
    }
}

/// Validate a `randomBytes` length without consulting JavaScript hooks.
///
/// The value must already be a primitive number (`Value::isNumber`), so V8's
/// `NumberValue` performs no user-visible conversion. Integers from 0 through
/// [`MAX_RANDOM_BYTES`] are accepted; everything else is a `RangeError`.
fn random_length<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<usize, CryptoError> {
    if !value.is_number() {
        return Err(CryptoError::new(CryptoErrorCode::Type));
    }
    let Some(number) = value.number_value(scope) else {
        return Err(CryptoError::new(CryptoErrorCode::Type));
    };
    if !number.is_finite()
        || number.trunc() != number
        || number < 0.0
        || number > MAX_RANDOM_BYTES as f64
    {
        return Err(CryptoError::new(CryptoErrorCode::Range));
    }
    Ok(number as usize)
}

/// Read an optional trailing associated-data argument.
///
/// `undefined` means no associated data. `null` and every other non-view value
/// are a `TypeError`, which is checked before any byte is copied.
fn associated_data_from<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<u8>, CryptoError> {
    if value.is_undefined() {
        return Ok(Vec::new());
    }
    let view = checked_view(scope, value)?;
    if view.length > MAX_ASSOCIATED_DATA_BYTES {
        return Err(CryptoError::new(CryptoErrorCode::Limit));
    }
    copy_view(&view)
}

/// Create the null-prototype result object the JavaScript wrapper reads.
///
/// A null prototype means a script cannot intercept these property writes or
/// reads through `Object.prototype`.
pub(crate) fn response<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = v8::null(scope);
    v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[])
}

pub(crate) fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(key) = v8::String::new(scope, name) else {
        return;
    };
    object.create_data_property(scope, key.into(), value);
}

pub(crate) fn set_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) {
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    set(scope, object, name, value.into());
}

/// Wrap a Rust byte vector as a `Uint8Array` that owns its backing store.
pub(crate) fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let length = bytes.len();
    if length == 0 {
        let buffer = v8::ArrayBuffer::new(scope, 0);
        return v8::Uint8Array::new(scope, buffer, 0, 0);
    }
    let store = v8::ArrayBuffer::new_backing_store_from_bytes(bytes.into_boxed_slice());
    let store = store.make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length)
}

pub(crate) fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: CryptoError,
) {
    set_failure_parts(
        scope,
        object,
        error.code().js_class(),
        error.code().as_str(),
        error.message(),
    );
}

/// Fill a result object's failure fields from a JavaScript error class, an
/// optional stable code, and a fixed message.
///
/// The message is supplied by the caller and must never contain input bytes.
pub(crate) fn set_failure_parts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    code: Option<&str>,
    message: &str,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    set_string(scope, object, "name", name);
    if let Some(code) = code {
        set_string(scope, object, "code", code);
    }
    set_string(scope, object, "message", message);
}

/// Turn a Rust result into `{ ok: true, value }` or `{ ok: false, ... }`.
fn outcome<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<Vec<u8>, CryptoError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(bytes) => match bytes_to_v8(scope, bytes) {
            Some(value) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", value.into());
            }
            None => set_failure(scope, object, CryptoError::new(CryptoErrorCode::Limit)),
        },
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

#[op2]
fn op_neo_crypto_generate_envelope_key<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
) -> v8::Local<'s, v8::Value> {
    outcome(scope, generate_envelope_key().map(Vec::from))
}

#[op2]
fn op_neo_crypto_random_bytes<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    length: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = random_length(scope, length).and_then(random_bytes);
    outcome(scope, result)
}

#[op2]
fn op_neo_crypto_encrypt<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    key: v8::Local<'s, v8::Value>,
    plaintext: v8::Local<'s, v8::Value>,
    associated_data: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let limits = CryptoLimits::default();
        let key = key_from_view(&checked_view(scope, key)?)?;
        let plaintext = checked_view(scope, plaintext)?;
        limits.check_plaintext(plaintext.length)?;
        let plaintext = copy_view(&plaintext)?;
        let associated_data = associated_data_from(scope, associated_data)?;
        encrypt(&key, plaintext, &associated_data, limits)
    })();
    outcome(scope, result)
}

#[op2]
fn op_neo_crypto_decrypt<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    key: v8::Local<'s, v8::Value>,
    envelope: v8::Local<'s, v8::Value>,
    associated_data: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let limits = CryptoLimits::default();
        let key = key_from_view(&checked_view(scope, key)?)?;
        let envelope = checked_view(scope, envelope)?;
        if envelope.length > MAX_ENVELOPE_BYTES {
            return Err(CryptoError::new(CryptoErrorCode::Limit));
        }
        let envelope = copy_view(&envelope)?;
        let associated_data = associated_data_from(scope, associated_data)?;
        decrypt(&key, envelope, &associated_data, limits)
    })();
    outcome(scope, result)
}

deno_core::extension!(
    neojs_crypto,
    ops = [
        op_neo_crypto_generate_envelope_key,
        op_neo_crypto_random_bytes,
        op_neo_crypto_encrypt,
        op_neo_crypto_decrypt
    ],
    docs = "Synchronous AES-256-GCM ops behind `Neo.getBuiltInModule(\"crypto\")`."
);

//! Synchronous AES-256-GCM authenticated encryption for `neo:crypto`.
//!
//! The cryptographic primitive is the RustCrypto `aes-gcm` crate. This module
//! defines the byte envelope, the associated-data construction, the size
//! bounds, and the JavaScript-facing error/code mapping. It is not a sandbox,
//! and it does not manage keys for the caller.
//!
//! The envelope layout, associated-data construction, size policy, and core
//! seal/open logic are adapted from the `neo:crypto` module in the NeoLiteJS
//! project (`neojs-core/src/crypto.rs`). That source is MIT licensed,
//! Copyright (c) 2026 NeoTales contributors; the full upstream MIT copyright
//! and permission notice is reproduced in `neojs-core/THIRD_PARTY_NOTICES.md`.
//! The project licence remains `LICENSE.md` (Copyright (c) 2026 NeoTales and
//! x0craft). The `deno_core` ops, JavaScript view validation, result-object
//! protocol, and argument-count checks are new; the test cases are adapted
//! from the reference test suite and rewritten for this runtime. No PLAB code
//! or format is used here.
//!
//! # Envelope
//!
//! Every value returned by [`encrypt`] has this layout:
//!
//! ```text
//! offset  size  field
//! 0       4     magic, ASCII "NEO1"
//! 4       1     envelope version, 1
//! 5       1     algorithm, 1 = AES-256-GCM
//! 6       12    nonce
//! 18      n     ciphertext (same length as the plaintext)
//! 18+n    16    authentication tag
//! ```
//!
//! The first 18 bytes (header and nonce) followed by the caller's associated
//! data are passed to AES-GCM as associated data. A decryptor that supplies a
//! different header, nonce, or associated-data value gets an authentication
//! failure, never plaintext. The magic, version, and algorithm bytes are
//! checked before the primitive runs, so an unknown format is reported as
//! malformed rather than as an authentication failure.
//!
//! The envelope is specific to neojs. The primitive is standard AES-256-GCM
//! (NIST SP 800-38D), and the unit tests below check it against published
//! vectors, but another tool must reproduce this envelope and
//! associated-data construction to interoperate. Two fixtures were produced
//! independently with Node's OpenSSL-backed AES-256-GCM: the unit tests supply
//! their nonces to [`encrypt_with_nonce`] and compare the emitted envelope
//! byte for byte, while the JavaScript integration tests decrypt them and
//! roundtrip envelopes emitted through the public API with a random nonce.
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
//! Web Crypto API. `crypto.subtle`, hashes (`createHash`), HMAC
//! (`createHmac`), bcrypt, PBKDF2, Shamir secret sharing, and a PLAB .NET
//! Crypto-compatible encrypt-then-MAC surface are not implemented here. The
//! stateful `createEncryptor`/`createDecryptor` CBC/GCM surface is a separate
//! module, `crypto_cipher`; it does not change this NEO1 envelope. These are
//! additive APIs: the functions above are the synchronous neojs slice and must
//! not be repurposed for a Node-like, WebCrypto-like, or PLAB-like contract.
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

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce, Tag};
use deno_core::op2;
use deno_core::v8;

/// Length of an AES-256 key in bytes.
pub(crate) const KEY_LEN: usize = 32;
/// Length of the AES-GCM nonce in bytes.
pub(crate) const NONCE_LEN: usize = 12;
/// Length of the AES-GCM authentication tag in bytes.
pub(crate) const TAG_LEN: usize = 16;

const MAGIC: &[u8; 4] = b"NEO1";
const VERSION: u8 = 1;
const ALGORITHM_AES_256_GCM: u8 = 1;
/// Magic, version, and algorithm.
const HEADER_LEN: usize = 6;
/// Header, nonce, and tag prepended/appended around the ciphertext.
pub(crate) const OVERHEAD: usize = HEADER_LEN + NONCE_LEN + TAG_LEN;

/// Largest plaintext accepted by the public module.
pub(crate) const MAX_PLAINTEXT_BYTES: usize = 8 * 1024 * 1024;
/// Largest caller associated-data value accepted by the public module.
pub(crate) const MAX_ASSOCIATED_DATA_BYTES: usize = 8 * 1024 * 1024;
/// Largest complete envelope accepted by the public module.
pub(crate) const MAX_ENVELOPE_BYTES: usize = MAX_PLAINTEXT_BYTES + OVERHEAD;
/// Largest byte count accepted by one `randomBytes` call.
pub(crate) const MAX_RANDOM_BYTES: usize = 8 * 1024 * 1024;

/// Size bounds used by the core and overridable in unit tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CryptoLimits {
    pub(crate) max_plaintext: usize,
    pub(crate) max_associated_data: usize,
}

impl Default for CryptoLimits {
    fn default() -> Self {
        Self {
            max_plaintext: MAX_PLAINTEXT_BYTES,
            max_associated_data: MAX_ASSOCIATED_DATA_BYTES,
        }
    }
}

impl CryptoLimits {
    fn check_plaintext(self, length: usize) -> Result<(), CryptoError> {
        if length > self.max_plaintext {
            Err(CryptoError::new(CryptoErrorCode::Limit))
        } else {
            Ok(())
        }
    }

    fn check_associated_data(self, length: usize) -> Result<(), CryptoError> {
        if length > self.max_associated_data {
            Err(CryptoError::new(CryptoErrorCode::Limit))
        } else {
            Ok(())
        }
    }

    fn max_envelope(self) -> Result<usize, CryptoError> {
        self.max_plaintext
            .checked_add(OVERHEAD)
            .ok_or_else(|| CryptoError::new(CryptoErrorCode::Limit))
    }
}

/// Stable machine-readable failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CryptoErrorCode {
    /// The key is not exactly 32 bytes.
    Key,
    /// The envelope is truncated or names an unknown format.
    Format,
    /// The tag did not verify under the supplied key and associated data.
    Authentication,
    /// The operating system CSPRNG failed.
    Random,
    /// An input exceeded a documented bound or an allocation was refused.
    Limit,
    /// The view's buffer was detached.
    Detached,
    /// An argument is not a `Uint8Array` or a number.
    Type,
    /// A numeric argument is not an integer or is outside its documented range.
    Range,
}

impl CryptoErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    ///
    /// Argument errors ([`CryptoErrorCode::Type`] and
    /// [`CryptoErrorCode::Range`]) have no code: the wrapper throws
    /// `TypeError` or `RangeError` instead, matching the documented contract.
    pub(crate) fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Key => "NEO_CRYPTO_KEY",
            Self::Format => "NEO_CRYPTO_FORMAT",
            Self::Authentication => "NEO_CRYPTO_AUTH",
            Self::Random => "NEO_CRYPTO_RANDOM",
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::Detached => "NEO_CRYPTO_DETACHED",
            Self::Type | Self::Range => return None,
        })
    }

    /// The JavaScript error class used for this failure.
    pub(crate) fn js_class(self) -> &'static str {
        match self {
            Self::Type => "TypeError",
            Self::Range => "RangeError",
            _ => "Error",
        }
    }
}

/// A failure safe to describe without echoing key, plaintext, or ciphertext.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CryptoError {
    code: CryptoErrorCode,
}

impl CryptoError {
    fn new(code: CryptoErrorCode) -> Self {
        Self { code }
    }

    pub(crate) fn code(self) -> CryptoErrorCode {
        self.code
    }

    /// A fixed message. It never contains input bytes.
    pub(crate) fn message(self) -> &'static str {
        match self.code {
            CryptoErrorCode::Key => "key must be exactly 32 bytes",
            CryptoErrorCode::Format => "malformed or unsupported crypto envelope",
            CryptoErrorCode::Authentication => "authentication failed",
            CryptoErrorCode::Random => "operating system random number generator failed",
            CryptoErrorCode::Limit => "input exceeds the crypto size limit",
            CryptoErrorCode::Detached => "input is detached or out of bounds",
            CryptoErrorCode::Type => "expected a Uint8Array or a number",
            CryptoErrorCode::Range => "length must be a non-negative safe integer within the limit",
        }
    }
}

/// Length of the associated data this module will authenticate: the header,
/// the nonce, and the caller's bytes.
fn aad_capacity(associated_data_len: usize) -> Result<usize, CryptoError> {
    HEADER_LEN
        .checked_add(NONCE_LEN)
        .and_then(|fixed| fixed.checked_add(associated_data_len))
        .ok_or_else(|| CryptoError::new(CryptoErrorCode::Limit))
}

fn cipher(key: &[u8]) -> Result<Aes256Gcm, CryptoError> {
    if key.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    // The only failure `new_from_slice` reports is a wrong key length, which
    // the check above already handled.
    Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::new(CryptoErrorCode::Key))
}

/// Encrypt with a caller-supplied nonce and associated data.
///
/// This is the primitive used by the known-answer tests. The public envelope
/// API never lets a caller choose a nonce.
#[cfg(test)]
fn seal_detached(
    key: &[u8],
    nonce: &[u8],
    associated_data: &[u8],
    plaintext: &[u8],
) -> Result<(Vec<u8>, [u8; TAG_LEN]), CryptoError> {
    if nonce.len() != NONCE_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    let cipher = cipher(key)?;
    let mut buffer = plaintext.to_vec();
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(nonce), associated_data, &mut buffer)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    let mut tag_bytes = [0u8; TAG_LEN];
    tag_bytes.copy_from_slice(tag.as_slice());
    Ok((buffer, tag_bytes))
}

/// Decrypt a detached ciphertext and tag with a caller-supplied nonce.
#[cfg(test)]
fn open_detached(
    key: &[u8],
    nonce: &[u8],
    associated_data: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    if nonce.len() != NONCE_LEN || tag.len() != TAG_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    let cipher = cipher(key)?;
    let mut buffer = ciphertext.to_vec();
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(nonce),
            associated_data,
            &mut buffer,
            Tag::from_slice(tag),
        )
        .map_err(|_| CryptoError::new(CryptoErrorCode::Authentication))?;
    Ok(buffer)
}

/// Encrypt `plaintext` into a fresh versioned envelope.
///
/// The caller must have checked the input sizes; this function checks them
/// again before allocating. `plaintext` is taken by value so its buffer can be
/// encrypted in place without another plaintext copy.
///
/// Limits and key lifetime: the public API allows 8 MiB of plaintext, 8 MiB of
/// associated data, and an 8 MiB + 34-byte envelope. The 96-bit nonce is
/// random, and the probability of a nonce collision reaches about 2^-32 after
/// roughly 2^32 encryptions under one key. Retire a key well before that; this
/// function does not track key use.
pub(crate) fn encrypt(
    key: &[u8],
    plaintext: Vec<u8>,
    associated_data: &[u8],
    limits: CryptoLimits,
) -> Result<Vec<u8>, CryptoError> {
    if key.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    limits.check_plaintext(plaintext.len())?;
    limits.check_associated_data(associated_data.len())?;

    let mut nonce = [0u8; NONCE_LEN];
    fill_random(&mut nonce)?;
    encrypt_with_nonce(key, plaintext, associated_data, limits, nonce)
}

/// Encrypt with a supplied nonce, after validation.
///
/// Production code reaches this through [`encrypt`] after drawing a nonce from
/// the operating system CSPRNG. The deterministic unit test reaches it
/// directly with the nonce from an independent fixture, so it exercises the
/// same envelope construction, associated-data binding, and AES-GCM call as
/// the public path. The nonce is never exposed to JavaScript.
fn encrypt_with_nonce(
    key: &[u8],
    plaintext: Vec<u8>,
    associated_data: &[u8],
    limits: CryptoLimits,
    nonce: [u8; NONCE_LEN],
) -> Result<Vec<u8>, CryptoError> {
    if key.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    limits.check_plaintext(plaintext.len())?;
    limits.check_associated_data(associated_data.len())?;

    let cipher = cipher(key)?;
    let envelope_capacity = OVERHEAD
        .checked_add(plaintext.len())
        .ok_or_else(|| CryptoError::new(CryptoErrorCode::Limit))?;
    let mut envelope = Vec::new();
    envelope
        .try_reserve_exact(envelope_capacity)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    envelope.extend_from_slice(MAGIC);
    envelope.push(VERSION);
    envelope.push(ALGORITHM_AES_256_GCM);
    envelope.extend_from_slice(&nonce);

    // Bind the format bytes, version, algorithm, and nonce to the tag. A
    // decryptor that sees a different header or nonce fails authentication.
    let mut aad = Vec::new();
    aad.try_reserve_exact(aad_capacity(associated_data.len())?)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    aad.extend_from_slice(&envelope);
    aad.extend_from_slice(associated_data);

    let mut buffer = plaintext;
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut buffer)
        // Our explicit bounds are far below AES-GCM's size limits, so a
        // primitive error here is a size/bound failure.
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;

    envelope.extend_from_slice(&buffer);
    envelope.extend_from_slice(tag.as_slice());
    Ok(envelope)
}

/// Decrypt one versioned envelope.
///
/// The caller must have checked the input sizes; this function checks them
/// again before allocating. On success the envelope buffer becomes the
/// plaintext buffer, avoiding a separate plaintext allocation. On any failure
/// no plaintext is returned.
///
/// Limits and key lifetime: the public API allows an 8 MiB + 34-byte envelope
/// and 8 MiB of associated data. AES-GCM requires unique nonces under one key;
/// retire a key well before about 2^32 messages under it. This function does
/// not track or enforce key use, and a failed tag check returns no plaintext.
pub(crate) fn decrypt(
    key: &[u8],
    mut envelope: Vec<u8>,
    associated_data: &[u8],
    limits: CryptoLimits,
) -> Result<Vec<u8>, CryptoError> {
    if key.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    if envelope.len() < OVERHEAD {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    if envelope.len() > limits.max_envelope()? {
        return Err(CryptoError::new(CryptoErrorCode::Limit));
    }
    limits.check_associated_data(associated_data.len())?;
    if &envelope[..MAGIC.len()] != MAGIC {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    if envelope[MAGIC.len()] != VERSION {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }
    if envelope[MAGIC.len() + 1] != ALGORITHM_AES_256_GCM {
        return Err(CryptoError::new(CryptoErrorCode::Format));
    }

    let ciphertext_len = envelope.len() - OVERHEAD;
    if ciphertext_len > limits.max_plaintext {
        return Err(CryptoError::new(CryptoErrorCode::Limit));
    }
    let cipher = cipher(key)?;
    let mut aad = Vec::new();
    aad.try_reserve_exact(aad_capacity(associated_data.len())?)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    aad.extend_from_slice(&envelope[..HEADER_LEN + NONCE_LEN]);
    aad.extend_from_slice(associated_data);

    let tag_start = envelope.len() - TAG_LEN;
    let (body, tag) = envelope.split_at_mut(tag_start);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&body[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
    let ciphertext = &mut body[HEADER_LEN + NONCE_LEN..];
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(&nonce),
            &aad,
            ciphertext,
            Tag::from_slice(tag),
        )
        // The explicit bounds above are below AES-GCM's size limits, so this
        // error is a tag failure. Size failures were already mapped to Limit.
        .map_err(|_| CryptoError::new(CryptoErrorCode::Authentication))?;

    // Discard the header/nonce prefix and the trailing tag. The remaining
    // bytes are the plaintext that was decrypted in place.
    envelope.drain(..HEADER_LEN + NONCE_LEN);
    envelope.truncate(ciphertext_len);
    Ok(envelope)
}

/// Fill `bytes` from the operating system CSPRNG.
///
/// This is the only randomness source in the module. A failure is reported as
/// [`CryptoErrorCode::Random`]; there is no fallback.
fn fill_random(bytes: &mut [u8]) -> Result<(), CryptoError> {
    getrandom::fill(bytes).map_err(|_| CryptoError::new(CryptoErrorCode::Random))
}

/// Return 32 random bytes from the operating system CSPRNG.
pub(crate) fn generate_envelope_key() -> Result<[u8; KEY_LEN], CryptoError> {
    let mut key = [0u8; KEY_LEN];
    fill_random(&mut key)?;
    Ok(key)
}

/// Return `length` random bytes from the operating system CSPRNG.
///
/// The caller must have validated the public bound; this function enforces it
/// again before allocating. The allocation is fallible, so a huge request is
/// rejected instead of aborting.
pub(crate) fn random_bytes(length: usize) -> Result<Vec<u8>, CryptoError> {
    if length > MAX_RANDOM_BYTES {
        return Err(CryptoError::new(CryptoErrorCode::Limit));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    bytes.resize(length, 0);
    fill_random(&mut bytes)?;
    Ok(bytes)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0, "hex length");
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    fn encode_hex(bytes: &[u8]) -> String {
        let mut text = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            text.push_str(&format!("{byte:02x}"));
        }
        text
    }

    fn default_limits() -> CryptoLimits {
        CryptoLimits::default()
    }

    // Test cases 13-16 from "The Galois/Counter Mode of Operation (GCM)",
    // McGrew and Viega, Appendix B, also published in BoringSSL's
    // aes_256_gcm_tests.txt. These exercise the standard AES-256-GCM
    // primitive, not the neojs envelope, so they use the vectors' own
    // associated data.
    struct Vector {
        name: &'static str,
        key: &'static str,
        nonce: &'static str,
        aad: &'static str,
        plaintext: &'static str,
        ciphertext: &'static str,
        tag: &'static str,
    }

    const VECTORS: [Vector; 4] = [
        Vector {
            name: "case 13 (empty)",
            key: "0000000000000000000000000000000000000000000000000000000000000000",
            nonce: "000000000000000000000000",
            aad: "",
            plaintext: "",
            ciphertext: "",
            tag: "530f8afbc74536b9a963b4f1c4cb738b",
        },
        Vector {
            name: "case 14 (one zero block)",
            key: "0000000000000000000000000000000000000000000000000000000000000000",
            nonce: "000000000000000000000000",
            aad: "",
            plaintext: "00000000000000000000000000000000",
            ciphertext: "cea7403d4d606b6e074ec5d3baf39d18",
            tag: "d0d1c8a799996bf0265b98b5d48ab919",
        },
        Vector {
            name: "case 15 (four blocks)",
            key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
            nonce: "cafebabefacedbaddecaf888",
            aad: "",
            plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
            ciphertext: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662898015ad",
            tag: "b094dac5d93471bdec1a502270e3cc6c",
        },
        Vector {
            name: "case 16 (four blocks with AAD)",
            key: "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308",
            nonce: "cafebabefacedbaddecaf888",
            aad: "feedfacedeadbeeffeedfacedeadbeefabaddad2",
            plaintext: "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b39",
            ciphertext: "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662",
            tag: "76fc6ece0f4e1768cddf8853bb2d551b",
        },
    ];

    #[test]
    fn published_aes_256_gcm_vectors_match() {
        for vector in &VECTORS {
            let key = decode_hex(vector.key);
            let nonce = decode_hex(vector.nonce);
            let aad = decode_hex(vector.aad);
            let plaintext = decode_hex(vector.plaintext);
            let (ciphertext, tag) = seal_detached(&key, &nonce, &aad, &plaintext).unwrap();
            assert_eq!(
                encode_hex(&ciphertext),
                vector.ciphertext,
                "{}",
                vector.name
            );
            assert_eq!(encode_hex(&tag), vector.tag, "{}", vector.name);
            assert_eq!(
                open_detached(&key, &nonce, &aad, &ciphertext, &tag).unwrap(),
                plaintext,
                "{}",
                vector.name
            );
        }
    }

    /// Envelope fixture produced independently with Node/OpenSSL
    /// (`node:crypto` `aes-256-gcm`) using this module's documented header,
    /// nonce, and associated-data layout. The key is a public test vector, not
    /// a credential.
    const INTEROP_KEY: &str = "8f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";
    const INTEROP_AAD: &str = "00010203040506070809";
    const INTEROP_PLAINTEXT: &str =
        "6e656f6a732063727970746f2063726f73732d696d706c656d656e746174696f6e2066697874757265";
    const INTEROP_ENVELOPE: &str = "4e454f310101a1b2c3d4e5f60718293a4b5c6da183558bece799386fca37bf3f217b09605d495c551d38c28e68bd5cbdd804db19b8ed0a913ffff9b52d67946d74dd47c5b93bdec06d68df";

    /// The neolitejs `neo:crypto` fixture, produced independently with Node's
    /// OpenSSL-backed AES-256-GCM. Matching it confirms the envelope remains
    /// compatible with the reference implementation.
    const REFERENCE_ENVELOPE: &str = "4e454f310101000102030405060708090a0b2967b921a697bb6bf92eb7e2df9d1d1feca6a75299032b094a0230ed39f87f14b520fd7a392dc47ab999";
    const REFERENCE_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const REFERENCE_AAD: &str = "636f6e746578742d6279746573";

    #[test]
    fn independent_envelope_fixtures_decrypt() {
        let key = decode_hex(REFERENCE_KEY);
        let envelope = decode_hex(REFERENCE_ENVELOPE);
        let aad = decode_hex(REFERENCE_AAD);
        assert_eq!(
            decrypt(&key, envelope, &aad, default_limits()).unwrap(),
            b"neo:crypto interop fixture"
        );

        let key = decode_hex(INTEROP_KEY);
        let aad = decode_hex(INTEROP_AAD);
        let envelope = decode_hex(INTEROP_ENVELOPE);
        assert_eq!(
            decrypt(&key, envelope, &aad, default_limits()).unwrap(),
            decode_hex(INTEROP_PLAINTEXT)
        );
    }

    fn nonce_from_envelope(envelope: &[u8]) -> [u8; NONCE_LEN] {
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&envelope[HEADER_LEN..HEADER_LEN + NONCE_LEN]);
        nonce
    }

    /// Compare the envelope path used by the public `encrypt` core against
    /// independently produced fixtures, byte for byte.
    ///
    /// The public JavaScript API never exposes a nonce, so it cannot be asked
    /// to reproduce a fixed fixture. This test supplies the fixture's nonce to
    /// the same `encrypt_with_nonce` implementation that `encrypt` calls after
    /// drawing a random nonce. The public JS tests separately decrypt these
    /// fixtures and roundtrip envelopes emitted with a random nonce.
    #[test]
    fn envelope_encryption_matches_independent_fixtures_byte_for_byte() {
        // Node 26 / OpenSSL 3.6 fixture generated for this repository.
        let key = decode_hex(INTEROP_KEY);
        let aad = decode_hex(INTEROP_AAD);
        let plaintext = decode_hex(INTEROP_PLAINTEXT);
        let expected = decode_hex(INTEROP_ENVELOPE);
        let emitted = encrypt_with_nonce(
            &key,
            plaintext,
            &aad,
            default_limits(),
            nonce_from_envelope(&expected),
        )
        .unwrap();
        assert_eq!(encode_hex(&emitted), INTEROP_ENVELOPE);

        // The neolitejs reference fixture, independently produced with Node's
        // OpenSSL-backed AES-256-GCM.
        let key = decode_hex(REFERENCE_KEY);
        let aad = decode_hex(REFERENCE_AAD);
        let expected = decode_hex(REFERENCE_ENVELOPE);
        let emitted = encrypt_with_nonce(
            &key,
            b"neo:crypto interop fixture".to_vec(),
            &aad,
            default_limits(),
            nonce_from_envelope(&expected),
        )
        .unwrap();
        assert_eq!(emitted, expected);
    }

    #[test]
    fn envelope_has_the_documented_layout() {
        let key = [7u8; KEY_LEN];
        let plaintext = b"layout".to_vec();
        let envelope = encrypt(&key, plaintext.clone(), b"", default_limits()).unwrap();
        assert_eq!(&envelope[..4], b"NEO1");
        assert_eq!(envelope[4], VERSION);
        assert_eq!(envelope[5], ALGORITHM_AES_256_GCM);
        assert_eq!(envelope.len(), OVERHEAD + plaintext.len());
        let ciphertext = &envelope[HEADER_LEN + NONCE_LEN..envelope.len() - TAG_LEN];
        assert_eq!(ciphertext.len(), plaintext.len());
        assert_ne!(ciphertext, plaintext.as_slice());
        assert_ne!(
            &envelope[HEADER_LEN..HEADER_LEN + NONCE_LEN],
            &[0u8; NONCE_LEN]
        );
        assert_eq!(
            decrypt(&key, envelope, b"", default_limits()).unwrap(),
            plaintext
        );
    }

    #[test]
    fn roundtrips_empty_binary_and_unicode_payloads() {
        let key = [0x5au8; KEY_LEN];
        let unicode = "Grüße, 世界, 🦀".as_bytes();
        let all_bytes: Vec<u8> = (0..=255u8).collect();
        for (plaintext, aad) in [
            (&b""[..], &b""[..]),
            (unicode, "context: ünïcode".as_bytes()),
            (&all_bytes, &[0u8, 1, 2, 0xff]),
        ] {
            let envelope = encrypt(&key, plaintext.to_vec(), aad, default_limits()).unwrap();
            assert_eq!(
                decrypt(&key, envelope, aad, default_limits()).unwrap(),
                plaintext
            );
        }
    }

    #[test]
    fn every_single_bit_flip_is_rejected() {
        let key = [0x11u8; KEY_LEN];
        let envelope = encrypt(
            &key,
            b"\x00\xffbinary".to_vec(),
            b"associated",
            default_limits(),
        )
        .unwrap();
        for byte in 0..envelope.len() {
            for bit in 0..8 {
                let mut tampered = envelope.clone();
                tampered[byte] ^= 1 << bit;
                let error = decrypt(&key, tampered, b"associated", default_limits())
                    .expect_err("a changed envelope must fail");
                assert!(
                    matches!(
                        error.code(),
                        CryptoErrorCode::Authentication | CryptoErrorCode::Format
                    ),
                    "byte {byte} bit {bit} produced {:?}",
                    error.code()
                );
            }
        }
    }

    #[test]
    fn wrong_key_and_wrong_associated_data_are_rejected() {
        let key = [0x22u8; KEY_LEN];
        let envelope = encrypt(&key, b"secret".to_vec(), b"context", default_limits()).unwrap();
        assert_eq!(
            decrypt(
                &[0x23u8; KEY_LEN],
                envelope.clone(),
                b"context",
                default_limits()
            )
            .unwrap_err()
            .code(),
            CryptoErrorCode::Authentication
        );
        assert_eq!(
            decrypt(&key, envelope.clone(), b"other", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Authentication
        );
        assert_eq!(
            decrypt(&key, envelope.clone(), b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Authentication
        );
        assert_eq!(
            decrypt(&key, envelope, b"context", default_limits()).unwrap(),
            b"secret"
        );
    }

    #[test]
    fn key_length_is_enforced() {
        for length in [0usize, 1, 16, 31, 33, 64] {
            let key = vec![0x33u8; length];
            assert_eq!(
                encrypt(&key, b"x".to_vec(), b"", default_limits())
                    .unwrap_err()
                    .code(),
                CryptoErrorCode::Key
            );
            assert_eq!(
                decrypt(&key, vec![0u8; OVERHEAD], b"", default_limits())
                    .unwrap_err()
                    .code(),
                CryptoErrorCode::Key
            );
        }
        assert!(encrypt(&[0x33u8; KEY_LEN], b"x".to_vec(), b"", default_limits()).is_ok());
    }

    #[test]
    fn malformed_envelopes_are_rejected() {
        let key = [0x44u8; KEY_LEN];
        assert_eq!(
            decrypt(&key, Vec::new(), b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );
        assert_eq!(
            decrypt(&key, vec![0u8; OVERHEAD - 1], b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );

        let envelope = encrypt(&key, b"payload".to_vec(), b"", default_limits()).unwrap();
        let mut bad_magic = envelope.clone();
        bad_magic[0] = b'X';
        assert_eq!(
            decrypt(&key, bad_magic, b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );

        let mut bad_version = envelope.clone();
        bad_version[4] = VERSION + 1;
        assert_eq!(
            decrypt(&key, bad_version, b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );

        let mut bad_algorithm = envelope.clone();
        bad_algorithm[5] = 2;
        assert_eq!(
            decrypt(&key, bad_algorithm, b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Format
        );

        let mut bad_tag = envelope;
        let last = bad_tag.len() - 1;
        bad_tag[last] ^= 1;
        assert_eq!(
            decrypt(&key, bad_tag, b"", default_limits())
                .unwrap_err()
                .code(),
            CryptoErrorCode::Authentication
        );
    }

    #[test]
    fn small_limits_are_checked_at_the_boundary() {
        let limits = CryptoLimits {
            max_plaintext: 8,
            max_associated_data: 5,
        };
        let key = [0x77u8; KEY_LEN];

        // Exactly at the limit is allowed.
        let envelope = encrypt(&key, vec![0u8; 8], b"12345", limits).unwrap();
        assert_eq!(envelope.len(), OVERHEAD + 8);
        assert!(decrypt(&key, envelope.clone(), b"12345", limits).is_ok());

        // One byte over is rejected before the core allocates.
        assert_eq!(
            encrypt(&key, vec![0u8; 9], b"", limits).unwrap_err().code(),
            CryptoErrorCode::Limit
        );
        assert_eq!(
            encrypt(&key, vec![0u8; 1], b"123456", limits)
                .unwrap_err()
                .code(),
            CryptoErrorCode::Limit
        );
        assert_eq!(
            decrypt(&key, envelope.clone(), b"123456", limits)
                .unwrap_err()
                .code(),
            CryptoErrorCode::Limit
        );

        // An envelope one byte over the plaintext bound is a limit failure,
        // not a format failure, because its header is still valid.
        let mut oversized = envelope;
        oversized.push(0);
        assert_eq!(
            decrypt(&key, oversized, b"12345", limits)
                .unwrap_err()
                .code(),
            CryptoErrorCode::Limit
        );
    }

    #[test]
    fn random_bytes_is_bounded_and_filled() {
        assert!(random_bytes(0).unwrap().is_empty());

        let first = random_bytes(32).unwrap();
        let second = random_bytes(32).unwrap();
        assert_eq!(first.len(), 32);
        assert_eq!(second.len(), 32);
        // This only shows the two calls did not return the same buffer. It is
        // not a statistical test and cannot prove cryptographic security.
        assert_ne!(first, second);

        assert_eq!(
            random_bytes(MAX_RANDOM_BYTES + 1).unwrap_err().code(),
            CryptoErrorCode::Limit
        );
        assert_eq!(
            random_bytes(MAX_RANDOM_BYTES).unwrap().len(),
            MAX_RANDOM_BYTES
        );

        let key = generate_envelope_key().unwrap();
        assert_eq!(key.len(), KEY_LEN);
        assert_ne!(key, [0u8; KEY_LEN]);
    }

    #[test]
    fn errors_have_stable_codes_and_no_input_bytes() {
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Key).code().as_str(),
            Some("NEO_CRYPTO_KEY")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Format).code().as_str(),
            Some("NEO_CRYPTO_FORMAT")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Authentication)
                .code()
                .as_str(),
            Some("NEO_CRYPTO_AUTH")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Random).code().as_str(),
            Some("NEO_CRYPTO_RANDOM")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Limit).code().as_str(),
            Some("NEO_CRYPTO_LIMIT")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Detached).code().as_str(),
            Some("NEO_CRYPTO_DETACHED")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Type).code().as_str(),
            None
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Range).code().as_str(),
            None
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Type).code().js_class(),
            "TypeError"
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Range).code().js_class(),
            "RangeError"
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Random).code().js_class(),
            "Error"
        );
        for code in [
            CryptoErrorCode::Key,
            CryptoErrorCode::Format,
            CryptoErrorCode::Authentication,
            CryptoErrorCode::Random,
            CryptoErrorCode::Limit,
            CryptoErrorCode::Detached,
            CryptoErrorCode::Type,
            CryptoErrorCode::Range,
        ] {
            let message = CryptoError::new(code).message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
        }
    }
}

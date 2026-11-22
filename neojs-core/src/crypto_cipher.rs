//! Bounded, stateful AES-256-CBC and AES-256-GCM ciphers for `neo:crypto`.
//!
//! This is a Node `createCipheriv`-inspired surface, not a Node-compatible one.
//! The factories are `createEncryptor` and `createDecryptor` in JavaScript;
//! there is deliberately no `createCipheriv`/`createDecipheriv` export and no
//! claim that a script can replace Node's module.
//!
//! # Algorithms and inputs
//!
//! The accepted names are `aes-256-cbc` and `aes-256-gcm`, compared
//! case-insensitively. The key is a `Uint8Array` of exactly 32 bytes. The IV is
//! a `Uint8Array` of exactly 16 bytes for CBC and 12 bytes for GCM (the
//! standard 96-bit nonce). Other lengths are rejected rather than stretched,
//! truncated, or hashed. The primitives are the RustCrypto `cbc` 0.2.1 (with
//! `aes` 0.9.4) and `aes-gcm` 0.10.3 crates; this module does not implement a
//! cipher or an authentication construction itself.
//!
//! The caller supplies the IV/nonce; the module does not generate, count, or
//! track it. For GCM, never reuse a 12-byte nonce under one key: a repeat
//! destroys both confidentiality and authenticity, and this API cannot detect
//! it. For CBC, the 16-byte IV must be unpredictable and must not repeat for a
//! key. The separate `encrypt`/`decrypt` NEO1 envelope draws a fresh random
//! nonce for every call and is the better choice when the caller does not need
//! to reproduce a Node-style streaming shape.
//!
//! `update(data[, inputEncoding])` accepts a `Uint8Array` of exact bytes, or a
//! string encoded as UTF-8. A string may name only `utf8`/`utf-8`; a
//! `Uint8Array` may not name an encoding at all. Output is always a
//! `Uint8Array`; Node's output encodings and callback forms are not supported.
//! The factory options policy is strict. `options` is either `undefined` or a
//! non-Proxy object whose immediate prototype is `null` or this realm's
//! intrinsic `Object.prototype`. Validation is own-only: every own property
//! must be an enumerable data property with a string key. Own symbols,
//! accessors, and non-enumerable properties are rejected, and a custom
//! prototype is rejected. Inherited properties on the intrinsic
//! `Object.prototype` or a null prototype are not read at all, so a polluted
//! `Object.prototype.padding` or unknown property has no effect. Arrays, class
//! instances, functions, and Proxies are rejected with `TypeError` before a
//! native resource slot is reserved. CBC permits only `padding` with a boolean
//! or `undefined` value; GCM permits no own properties. The value is read from
//! the validated own-property descriptor, not by a later `options.padding`
//! access, and no Proxy trap or getter is invoked during validation. Unknown
//! own keys such as `autoPadding` or `paddding` are rejected rather than
//! ignored.
//!
//! # Buffering and output timing
//!
//! `update` copies its input into a bounded module-owned buffer and returns an
//! empty `Uint8Array`. `final()` performs the cipher operation over the whole
//! buffered message and returns the output. The concatenation of all `update`
//! returns and `final` is the same byte stream a streaming implementation would
//! produce, but this slice does not release chunks early. Buffering is the only
//! correct choice for GCM here: the RustCrypto one-shot API exposes no
//! incremental GHASH state, and releasing GCM plaintext before the tag is
//! checked would be unauthenticated plaintext. CBC could be streamed, but is
//! buffered too so both algorithms have one documented lifecycle.
//!
//! # CBC
//!
//! CBC is confidential but **not authenticated**. This module computes no MAC
//! and implies no MAC. Prefer an established AEAD such as AES-256-GCM for new
//! designs. If CBC interoperability is required, use encrypt-then-MAC with an
//! independent MAC key: authenticate the IV and ciphertext, verify the MAC
//! before decrypting, and reject the message on any MAC failure. Do not use
//! authenticate-then-encrypt. Be careful with padding oracles: exposing whether
//! CBC padding is valid, including through distinct `NEO_CRYPTO_PADDING`
//! behavior, can let an attacker decrypt or forge data. This API does not add a
//! MAC, and `final()` succeeding does not mean the ciphertext was authentic.
//! Padding defaults to PKCS#7 and is verified strictly at `final()`: every
//! padding byte must have the padding length, and an empty ciphertext, a
//! non-block-multiple ciphertext, or an invalid final block is
//! `NEO_CRYPTO_PADDING`. `setAutoPadding(false)` disables padding; then the
//! plaintext or ciphertext must already be a multiple of 16 bytes.
//!
//! # GCM
//!
//! GCM is authenticated. The caller must supply a fresh, unique 12-byte nonce
//! under each key; this module neither generates nor tracks it. `setAAD(aad)`
//! may be called at most once, before the first `update` or `final`; the
//! associated data is authenticated and not encrypted. The encryption object's
//! tag is available from `getAuthTag()` only after `final()`; it is 16 bytes. A
//! decryption object must call
//! `setAuthTag(tag)` with exactly 16 bytes before `final()`. The tag is checked
//! over the collected ciphertext and associated data, and `final()` returns no
//! plaintext when the check fails: `NEO_CRYPTO_AUTH`. Because `update` never
//! returns plaintext for GCM, no unauthenticated plaintext leaves the object.
//!
//! # Lifecycle and limits
//!
//! `final()` consumes the message but leaves the object usable only for
//! `getAuthTag()` (GCM encryption). A second `final()`, an `update` after
//! `final()`, or a metadata change after `final()` is `NEO_CRYPTO_STATE`.
//! `close()` (alias `dispose()`) releases the native state and is idempotent; a
//! later call is `NEO_CRYPTO_CLOSED`. Call `close()`/`dispose()` in a `finally`
//! block even after a successful or failed `final()`: finalization releases the
//! buffered message, but the object keeps its native slot until it is closed or
//! the runtime is dropped. There is no finalizer, so an abandoned object keeps
//! its slot and any remaining buffer until `close()`/`dispose()` or runtime
//! teardown.
//!
//! One object accepts at most 8 MiB of cipher input and 8 MiB of GCM associated
//! data. At most 64 cipher objects may be open per runtime, and their combined
//! buffered input and associated data may not exceed 64 MiB. Creating past the
//! object cap is `NEO_CRYPTO_TOO_MANY_OBJECTS`; exceeding the aggregate buffer
//! cap is `NEO_CRYPTO_BUSY`. Rust allocations are fallible and a refused
//! reservation is `NEO_CRYPTO_LIMIT`; V8 and the JavaScript heap allocate too,
//! so recovery from out-of-memory is not guaranteed.
//!
//! # Not Node, not async
//!
//! This is not Node's `crypto` module and does not provide full Node
//! compatibility: there is no `createCipheriv`, no `autoPadding` option name,
//! no output encoding, no `getAuthTag` before `final`, no streaming chunk
//! timing, and no callback or Promise form. The existing `encrypt`/`decrypt`
//! NEO1 envelope is a separate API and keeps its own format; a cipher object
//! does not emit or parse NEO1. Asynchronous ciphers are outstanding work: a
//! genuine Promise form would need a bounded worker design for a stateful
//! object, and this slice did not add a `Promise.resolve` wrapper that would
//! only look asynchronous. The synchronous API works on every executor.
//!
//! No zeroization is promised. Keys, IVs, plaintext, ciphertext, and associated
//! data are copied into Rust and JavaScript buffers and reclaimed normally, not
//! guaranteed to be erased first. No key, plaintext, or ciphertext is logged or
//! included in an error message.
//!
//! # Licensing
//!
//! `cbc`, `aes`, and `cipher` are RustCrypto crates dual-licensed MIT OR
//! Apache-2.0; `aes-gcm` is MIT OR Apache-2.0. `neojs-core` uses the MIT
//! option. The notices are reproduced in `neojs-core/THIRD_PARTY_NOTICES.md`.

use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;

use aes_gcm::Aes256Gcm;
use aes_gcm::Nonce;
use aes_gcm::Tag;
use aes_gcm::aead::AeadInPlace;
use aes_gcm::aead::KeyInit as AeadKeyInit;
use cbc::cipher::BlockModeDecrypt;
use cbc::cipher::BlockModeEncrypt;
use cbc::cipher::KeyIvInit;
use cbc::cipher::block_padding::NoPadding;
use cbc::cipher::block_padding::Pkcs7;
use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_core::v8;

use crate::crypto::CryptoError;
use crate::crypto::CryptoErrorCode;
use crate::crypto::KEY_LEN;
use crate::crypto::NONCE_LEN;
use crate::crypto::TAG_LEN;
use crate::crypto::bytes_to_v8;
use crate::crypto::checked_view;
use crate::crypto::copy_view;
use crate::crypto::response;
use crate::crypto::set;
use crate::crypto::set_failure_parts;

/// CBC block length in bytes.
const CBC_IV_LEN: usize = 16;
/// Largest cipher input accepted by one object.
pub(crate) const MAX_CIPHER_INPUT_BYTES: usize = 8 * 1024 * 1024;
/// Largest GCM associated-data value accepted by one object.
pub(crate) const MAX_CIPHER_AAD_BYTES: usize = 8 * 1024 * 1024;
/// Largest number of cipher objects that may be open at once.
pub(crate) const MAX_OPEN_CIPHERS: usize = 64;
/// Largest combined buffered input and associated data across open objects.
pub(crate) const MAX_OPEN_CIPHER_BYTES: usize = 64 * 1024 * 1024;

/// One supported cipher algorithm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Algorithm {
    Aes256Cbc,
    Aes256Gcm,
}

impl Algorithm {
    fn parse(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("aes-256-cbc") {
            Some(Self::Aes256Cbc)
        } else if name.eq_ignore_ascii_case("aes-256-gcm") {
            Some(Self::Aes256Gcm)
        } else {
            None
        }
    }

    fn iv_len(self) -> usize {
        match self {
            Self::Aes256Cbc => CBC_IV_LEN,
            Self::Aes256Gcm => NONCE_LEN,
        }
    }
}

/// Whether an object encrypts or decrypts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Direction {
    Encrypt,
    Decrypt,
}

/// Whether the message has been finalized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Buffering,
    Finalized,
}

/// Per-object size bounds, overridable in unit tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CipherLimits {
    max_input: usize,
    max_aad: usize,
}

impl Default for CipherLimits {
    fn default() -> Self {
        Self {
            max_input: MAX_CIPHER_INPUT_BYTES,
            max_aad: MAX_CIPHER_AAD_BYTES,
        }
    }
}

/// Stable machine-readable cipher failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CipherErrorCode {
    /// The algorithm name is not one of the two supported names.
    Algorithm,
    /// The key is not exactly 32 bytes.
    Key,
    /// The IV or nonce is not the algorithm's required length.
    Iv,
    /// The tag is not exactly 16 bytes.
    Tag,
    /// Decryption was finalized without a tag.
    MissingTag,
    /// The GCM tag did not verify.
    Authentication,
    /// CBC padding or a no-padding block length is invalid.
    Padding,
    /// The operation is not valid in the object's current state.
    State,
    /// The object was closed.
    Closed,
    /// The operation is not supported for this algorithm or direction.
    Unsupported,
    /// An input or allocation exceeded a documented bound.
    Limit,
    /// Too many cipher objects are open.
    TooManyObjects,
    /// The aggregate cipher buffer budget is full.
    Busy,
    /// The view's buffer was detached.
    Detached,
    /// A worker or internal invariant failed.
    Internal,
    /// An argument has the wrong kind.
    Type,
    /// The options record violates the option policy.
    Options,
    /// A numeric argument is outside its documented range.
    Range,
}

impl CipherErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Algorithm => "NEO_CRYPTO_ALGORITHM",
            Self::Key => "NEO_CRYPTO_KEY",
            Self::Iv => "NEO_CRYPTO_IV",
            Self::Tag | Self::MissingTag => "NEO_CRYPTO_TAG",
            Self::Authentication => "NEO_CRYPTO_AUTH",
            Self::Padding => "NEO_CRYPTO_PADDING",
            Self::State => "NEO_CRYPTO_STATE",
            Self::Closed => "NEO_CRYPTO_CLOSED",
            Self::Unsupported => "NEO_CRYPTO_UNSUPPORTED",
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::TooManyObjects => "NEO_CRYPTO_TOO_MANY_OBJECTS",
            Self::Busy => "NEO_CRYPTO_BUSY",
            Self::Detached => "NEO_CRYPTO_DETACHED",
            Self::Internal => "NEO_CRYPTO_INTERNAL",
            Self::Type | Self::Options | Self::Range => return None,
        })
    }

    fn js_class(self) -> &'static str {
        match self {
            Self::Type | Self::Options => "TypeError",
            Self::Range => "RangeError",
            _ => "Error",
        }
    }
}

/// A failure safe to describe without echoing key, plaintext, or ciphertext.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CipherError {
    code: CipherErrorCode,
}

impl CipherError {
    fn new(code: CipherErrorCode) -> Self {
        Self { code }
    }

    fn code(self) -> CipherErrorCode {
        self.code
    }

    /// A fixed message. It never contains input bytes.
    fn message(self) -> &'static str {
        match self.code {
            CipherErrorCode::Algorithm => "unsupported cipher algorithm",
            CipherErrorCode::Key => "key must be exactly 32 bytes",
            CipherErrorCode::Iv => "iv has the wrong length for the cipher algorithm",
            CipherErrorCode::Tag => "authentication tag must be exactly 16 bytes",
            CipherErrorCode::MissingTag => "authentication tag is required before final",
            CipherErrorCode::Authentication => "authentication failed",
            CipherErrorCode::Padding => "invalid cipher padding or block length",
            CipherErrorCode::State => "cipher operation is not valid in the current state",
            CipherErrorCode::Closed => "the cipher object is closed",
            CipherErrorCode::Unsupported => "operation is not supported for this cipher",
            CipherErrorCode::Limit => "input exceeds the cipher size limit",
            CipherErrorCode::TooManyObjects => "too many cipher objects are open",
            CipherErrorCode::Busy => "too much cipher buffering is pending",
            CipherErrorCode::Detached => "input is detached or out of bounds",
            CipherErrorCode::Internal => "the cipher operation failed internally",
            CipherErrorCode::Type => "expected a Uint8Array, a string, or a boolean",
            CipherErrorCode::Options => {
                "options must be a plain or null-prototype object with own enumerable data properties; only CBC accepts padding"
            }
            CipherErrorCode::Range => "numeric argument is outside its documented range",
        }
    }
}

impl From<CryptoError> for CipherError {
    fn from(error: CryptoError) -> Self {
        let code = match error.code() {
            CryptoErrorCode::Key => CipherErrorCode::Key,
            CryptoErrorCode::Limit => CipherErrorCode::Limit,
            CryptoErrorCode::Detached => CipherErrorCode::Detached,
            CryptoErrorCode::Type => CipherErrorCode::Type,
            CryptoErrorCode::Range => CipherErrorCode::Range,
            CryptoErrorCode::Authentication => CipherErrorCode::Authentication,
            CryptoErrorCode::Random | CryptoErrorCode::Format => CipherErrorCode::Internal,
        };
        Self::new(code)
    }
}

#[derive(Default)]
struct CipherBudgetState {
    open: usize,
    bytes: usize,
}

/// The per-runtime object and buffer budgets.
struct CipherBudget {
    max_open: usize,
    max_bytes: usize,
    state: Mutex<CipherBudgetState>,
}

impl Default for CipherBudget {
    fn default() -> Self {
        Self::with_limits(MAX_OPEN_CIPHERS, MAX_OPEN_CIPHER_BYTES)
    }
}

impl CipherBudget {
    fn with_limits(max_open: usize, max_bytes: usize) -> Self {
        Self {
            max_open,
            max_bytes,
            state: Mutex::new(CipherBudgetState::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, CipherBudgetState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn reserve_open(self: &Arc<Self>) -> Result<OpenReservation, CipherError> {
        let mut state = self.lock();
        if state.open >= self.max_open {
            return Err(CipherError::new(CipherErrorCode::TooManyObjects));
        }
        state.open += 1;
        Ok(OpenReservation {
            budget: Arc::clone(self),
        })
    }

    fn charge(self: &Arc<Self>, bytes: usize) -> Result<(), CipherError> {
        if bytes == 0 {
            return Ok(());
        }
        let mut state = self.lock();
        let next = state
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| CipherError::new(CipherErrorCode::Busy))?;
        if next > self.max_bytes {
            return Err(CipherError::new(CipherErrorCode::Busy));
        }
        state.bytes = next;
        Ok(())
    }

    fn release(&self, bytes: usize) {
        if bytes == 0 {
            return;
        }
        let mut state = self.lock();
        state.bytes = state.bytes.saturating_sub(bytes);
    }
}

/// An open-object slot released when its resource is dropped.
struct OpenReservation {
    budget: Arc<CipherBudget>,
}

impl Drop for OpenReservation {
    fn drop(&mut self) {
        let mut state = self.budget.lock();
        state.open = state.open.saturating_sub(1);
    }
}

/// A buffer whose bytes are charged against the per-runtime aggregate budget.
struct Buffered {
    budget: Arc<CipherBudget>,
    max: usize,
    bytes: Vec<u8>,
    charged: usize,
}

impl Buffered {
    fn new(budget: Arc<CipherBudget>, max: usize) -> Self {
        Self {
            budget,
            max,
            bytes: Vec::new(),
            charged: 0,
        }
    }

    fn from_vec(
        budget: Arc<CipherBudget>,
        max: usize,
        bytes: Vec<u8>,
    ) -> Result<Self, CipherError> {
        if bytes.len() > max {
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        budget.charge(bytes.len())?;
        let charged = bytes.len();
        Ok(Self {
            budget,
            max,
            bytes,
            charged,
        })
    }

    fn append(&mut self, data: &[u8]) -> Result<(), CipherError> {
        let next = self
            .bytes
            .len()
            .checked_add(data.len())
            .ok_or_else(|| CipherError::new(CipherErrorCode::Limit))?;
        if next > self.max {
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        if data.is_empty() {
            return Ok(());
        }
        self.budget.charge(data.len())?;
        if self.bytes.try_reserve(data.len()).is_err() {
            self.budget.release(data.len());
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        self.bytes.extend_from_slice(data);
        self.charged += data.len();
        Ok(())
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn into_bytes(mut self) -> Vec<u8> {
        let bytes = std::mem::take(&mut self.bytes);
        self.budget.release(self.charged);
        self.charged = 0;
        bytes
    }
}

impl Drop for Buffered {
    fn drop(&mut self) {
        if self.charged > 0 {
            self.budget.release(self.charged);
            self.charged = 0;
        }
    }
}

/// One cipher object's state.
struct CipherState {
    algorithm: Algorithm,
    direction: Direction,
    key: [u8; KEY_LEN],
    iv: Vec<u8>,
    padding: bool,
    stage: Stage,
    saw_update: bool,
    input: Option<Buffered>,
    aad: Option<Buffered>,
    tag: Option<[u8; TAG_LEN]>,
    limits: CipherLimits,
    budget: Arc<CipherBudget>,
}

impl CipherState {
    fn new(
        direction: Direction,
        algorithm: Algorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
        limits: CipherLimits,
        budget: Arc<CipherBudget>,
    ) -> Result<Self, CipherError> {
        if key.len() != KEY_LEN {
            return Err(CipherError::new(CipherErrorCode::Key));
        }
        if iv.len() != algorithm.iv_len() {
            return Err(CipherError::new(CipherErrorCode::Iv));
        }
        let mut key_bytes = [0u8; KEY_LEN];
        key_bytes.copy_from_slice(key);
        Ok(Self {
            algorithm,
            direction,
            key: key_bytes,
            iv: iv.to_vec(),
            padding,
            stage: Stage::Buffering,
            saw_update: false,
            input: Some(Buffered::new(Arc::clone(&budget), limits.max_input)),
            aad: None,
            tag: None,
            limits,
            budget,
        })
    }

    /// Bytes this object may still accept through `update`.
    fn remaining_input(&self) -> usize {
        self.input
            .as_ref()
            .map(|input| self.limits.max_input.saturating_sub(input.bytes.len()))
            .unwrap_or(0)
    }

    fn update(&mut self, data: &[u8]) -> Result<(), CipherError> {
        if self.stage != Stage::Buffering {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| CipherError::new(CipherErrorCode::State))?;
        input.append(data)?;
        self.saw_update = true;
        Ok(())
    }

    fn set_aad(&mut self, aad: Vec<u8>) -> Result<(), CipherError> {
        if self.algorithm != Algorithm::Aes256Gcm {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        if self.stage != Stage::Buffering || self.saw_update || self.aad.is_some() {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        if aad.len() > self.limits.max_aad {
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        self.aad = Some(Buffered::from_vec(
            Arc::clone(&self.budget),
            self.limits.max_aad,
            aad,
        )?);
        Ok(())
    }

    fn set_tag(&mut self, tag: &[u8]) -> Result<(), CipherError> {
        if self.algorithm != Algorithm::Aes256Gcm || self.direction != Direction::Decrypt {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        if self.stage != Stage::Buffering {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        if tag.len() != TAG_LEN {
            return Err(CipherError::new(CipherErrorCode::Tag));
        }
        if self.tag.is_some() {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        let mut bytes = [0u8; TAG_LEN];
        bytes.copy_from_slice(tag);
        self.tag = Some(bytes);
        Ok(())
    }

    fn set_padding(&mut self, padding: bool) -> Result<(), CipherError> {
        if self.algorithm != Algorithm::Aes256Cbc {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        if self.stage != Stage::Buffering {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        self.padding = padding;
        Ok(())
    }

    fn auth_tag(&self) -> Result<[u8; TAG_LEN], CipherError> {
        if self.algorithm != Algorithm::Aes256Gcm || self.direction != Direction::Encrypt {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        if self.stage != Stage::Finalized {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        self.tag
            .ok_or_else(|| CipherError::new(CipherErrorCode::State))
    }

    fn finalize(&mut self) -> Result<Vec<u8>, CipherError> {
        if self.stage != Stage::Buffering {
            return Err(CipherError::new(CipherErrorCode::State));
        }
        // Finalization consumes the buffered message even when the tag is
        // missing or the padding is invalid, so a failed object cannot be
        // retried with a different tag or padding setting. The JS wrapper
        // applies the same rule by marking its object finalized first.
        self.stage = Stage::Finalized;
        let input = self
            .input
            .take()
            .ok_or_else(|| CipherError::new(CipherErrorCode::State))?;
        let aad = self.aad.take();
        if self.algorithm == Algorithm::Aes256Gcm
            && self.direction == Direction::Decrypt
            && self.tag.is_none()
        {
            drop(input);
            return Err(CipherError::new(CipherErrorCode::MissingTag));
        }
        let aad_bytes = aad.as_ref().map(Buffered::as_slice).unwrap_or(&[]);
        match (self.algorithm, self.direction) {
            (Algorithm::Aes256Cbc, Direction::Encrypt) => {
                cbc_encrypt(&self.key, &self.iv, self.padding, &input.into_bytes())
            }
            (Algorithm::Aes256Cbc, Direction::Decrypt) => {
                cbc_decrypt(&self.key, &self.iv, self.padding, &input.into_bytes())
            }
            (Algorithm::Aes256Gcm, Direction::Encrypt) => {
                let (output, tag) =
                    gcm_encrypt(&self.key, &self.iv, aad_bytes, input.into_bytes())?;
                self.tag = Some(tag);
                Ok(output)
            }
            (Algorithm::Aes256Gcm, Direction::Decrypt) => {
                let tag = self
                    .tag
                    .take()
                    .ok_or_else(|| CipherError::new(CipherErrorCode::MissingTag))?;
                gcm_decrypt(&self.key, &self.iv, aad_bytes, input.into_bytes(), &tag)
            }
        }
    }
}

/// A cipher object held in the resource table.
struct CipherResource {
    state: RefCell<Option<CipherState>>,
    _open: OpenReservation,
}

impl CipherResource {
    fn new(state: CipherState, open: OpenReservation) -> Self {
        Self {
            state: RefCell::new(Some(state)),
            _open: open,
        }
    }
}

impl Resource for CipherResource {
    fn name(&self) -> Cow<'_, str> {
        "neo::crypto::Cipher".into()
    }
}

/// CBC encryption with PKCS#7 padding or a strict no-padding mode.
fn cbc_encrypt(
    key: &[u8],
    iv: &[u8],
    padding: bool,
    plaintext: &[u8],
) -> Result<Vec<u8>, CipherError> {
    let cipher = cbc::Encryptor::<aes::Aes256>::new_from_slices(key, iv)
        .map_err(|_| CipherError::new(CipherErrorCode::Key))?;
    if !padding && !plaintext.len().is_multiple_of(CBC_IV_LEN) {
        return Err(CipherError::new(CipherErrorCode::Padding));
    }
    let output_len = if padding {
        ((plaintext.len() / CBC_IV_LEN) + 1)
            .checked_mul(CBC_IV_LEN)
            .ok_or_else(|| CipherError::new(CipherErrorCode::Limit))?
    } else {
        plaintext.len()
    };
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_len)
        .map_err(|_| CipherError::new(CipherErrorCode::Limit))?;
    output.resize(output_len, 0);
    let produced = if padding {
        cipher.encrypt_padded_b2b::<Pkcs7>(plaintext, &mut output)
    } else {
        cipher.encrypt_padded_b2b::<NoPadding>(plaintext, &mut output)
    }
    .map_err(|_| CipherError::new(CipherErrorCode::Padding))?;
    let length = produced.len();
    output.truncate(length);
    Ok(output)
}

/// CBC decryption with strict PKCS#7 removal or no padding.
fn cbc_decrypt(
    key: &[u8],
    iv: &[u8],
    padding: bool,
    ciphertext: &[u8],
) -> Result<Vec<u8>, CipherError> {
    if !ciphertext.len().is_multiple_of(CBC_IV_LEN) {
        return Err(CipherError::new(CipherErrorCode::Padding));
    }
    if ciphertext.is_empty() {
        return if padding {
            Err(CipherError::new(CipherErrorCode::Padding))
        } else {
            Ok(Vec::new())
        };
    }
    let cipher = cbc::Decryptor::<aes::Aes256>::new_from_slices(key, iv)
        .map_err(|_| CipherError::new(CipherErrorCode::Key))?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(ciphertext.len())
        .map_err(|_| CipherError::new(CipherErrorCode::Limit))?;
    output.resize(ciphertext.len(), 0);
    let produced = if padding {
        cipher.decrypt_padded_b2b::<Pkcs7>(ciphertext, &mut output)
    } else {
        cipher.decrypt_padded_b2b::<NoPadding>(ciphertext, &mut output)
    }
    .map_err(|_| CipherError::new(CipherErrorCode::Padding))?;
    let length = produced.len();
    output.truncate(length);
    Ok(output)
}

fn gcm_cipher(key: &[u8]) -> Result<Aes256Gcm, CipherError> {
    Aes256Gcm::new_from_slice(key).map_err(|_| CipherError::new(CipherErrorCode::Key))
}

fn gcm_encrypt(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    plaintext: Vec<u8>,
) -> Result<(Vec<u8>, [u8; TAG_LEN]), CipherError> {
    if nonce.len() != NONCE_LEN {
        return Err(CipherError::new(CipherErrorCode::Iv));
    }
    let cipher = gcm_cipher(key)?;
    let mut buffer = plaintext;
    let tag = cipher
        .encrypt_in_place_detached(Nonce::from_slice(nonce), aad, &mut buffer)
        .map_err(|_| CipherError::new(CipherErrorCode::Limit))?;
    let mut tag_bytes = [0u8; TAG_LEN];
    tag_bytes.copy_from_slice(tag.as_slice());
    Ok((buffer, tag_bytes))
}

fn gcm_decrypt(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    ciphertext: Vec<u8>,
    tag: &[u8; TAG_LEN],
) -> Result<Vec<u8>, CipherError> {
    if nonce.len() != NONCE_LEN {
        return Err(CipherError::new(CipherErrorCode::Iv));
    }
    let cipher = gcm_cipher(key)?;
    let mut buffer = ciphertext;
    cipher
        .decrypt_in_place_detached(
            Nonce::from_slice(nonce),
            aad,
            &mut buffer,
            Tag::from_slice(tag),
        )
        .map_err(|_| CipherError::new(CipherErrorCode::Authentication))?;
    Ok(buffer)
}

/// The per-runtime budget handle placed in `OpState`.
#[derive(Default)]
struct CipherBudgetHandle {
    budget: Arc<CipherBudget>,
}

impl CipherBudgetHandle {
    fn budget(&self) -> Arc<CipherBudget> {
        Arc::clone(&self.budget)
    }
}

/// Read a primitive string argument without invoking JavaScript hooks.
fn checked_string<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::String>, CipherError> {
    if !value.is_string() {
        return Err(CipherError::new(CipherErrorCode::Type));
    }
    v8::Local::<v8::String>::try_from(value).map_err(|_| CipherError::new(CipherErrorCode::Type))
}

fn algorithm_from<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Algorithm, CipherError> {
    let string = checked_string(value)?;
    if string.length() > 32 {
        return Err(CipherError::new(CipherErrorCode::Algorithm));
    }
    let name = string.to_rust_string_lossy(scope);
    Algorithm::parse(&name).ok_or_else(|| CipherError::new(CipherErrorCode::Algorithm))
}

fn direction_from(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Direction, CipherError> {
    if !value.is_number() {
        return Err(CipherError::new(CipherErrorCode::Type));
    }
    let Some(number) = value.number_value(scope) else {
        return Err(CipherError::new(CipherErrorCode::Type));
    };
    if number == 0.0 {
        Ok(Direction::Encrypt)
    } else if number == 1.0 {
        Ok(Direction::Decrypt)
    } else {
        Err(CipherError::new(CipherErrorCode::Range))
    }
}

fn padding_from(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<bool, CipherError> {
    if !value.is_boolean() {
        return Err(CipherError::new(CipherErrorCode::Type));
    }
    Ok(value.boolean_value(scope))
}

fn options_error() -> CipherError {
    CipherError::new(CipherErrorCode::Options)
}

/// The intrinsic `Object.prototype` for the current realm.
///
/// `v8::Object::new` creates an ordinary object using the intrinsic prototype,
/// so this identity is stable even if user code later replaces the `Object`
/// constructor or `Object.prototype` properties.
fn plain_object_prototype<'s>(
    scope: &v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Value>, CipherError> {
    let object = v8::Object::new(scope);
    object.get_prototype(scope).ok_or_else(options_error)
}

/// Read one own data property from an internal descriptor object.
///
/// The descriptor object is created by V8, not by user code. The own-property
/// check runs before `get`, so an absent `value`/`get`/`set` never consults a
/// possibly polluted prototype.
fn own_descriptor_field<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, CipherError> {
    let Some(key) = v8::String::new(scope, name) else {
        return Err(options_error());
    };
    let key: v8::Local<'s, v8::Name> = key.into();
    match object.has_own_property(scope, key) {
        Some(true) => object
            .get(scope, key.into())
            .map(Some)
            .ok_or_else(options_error),
        Some(false) => Ok(None),
        None => Err(options_error()),
    }
}

fn own_descriptor_boolean(
    scope: &v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
) -> Result<bool, CipherError> {
    let Some(value) = own_descriptor_field(scope, object, name)? else {
        return Err(options_error());
    };
    if !value.is_boolean() {
        return Err(options_error());
    }
    Ok(value.boolean_value(scope))
}

/// Validate the factory options record and return CBC padding.
///
/// Policy: `undefined` or a non-proxy object whose immediate prototype is
/// `null` or the current realm's intrinsic `Object.prototype`. Validation is
/// own-only: every own property must be an enumerable data property with a
/// string key. Own symbols, accessors, and non-enumerable properties are
/// rejected, and a non-plain prototype is rejected. Inherited properties on the
/// allowed intrinsic prototype (or on a null prototype) are never read, so
/// polluting `Object.prototype.padding` cannot change the result. CBC permits
/// only `padding` with a boolean or `undefined` value; GCM permits no own
/// properties. The value is read from the validated descriptor, not by
/// re-reading the record, so a getter or a Proxy trap cannot supply a different
/// value after validation. The check runs before the object slot is reserved.
fn options_padding<'s>(
    scope: &v8::PinScope<'s, '_>,
    algorithm: Algorithm,
    value: v8::Local<'s, v8::Value>,
) -> Result<bool, CipherError> {
    if value.is_undefined() {
        return Ok(true);
    }
    if !value.is_object() || value.is_proxy() {
        return Err(options_error());
    }
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| options_error())?;
    let Some(prototype) = object.get_prototype(scope) else {
        return Err(options_error());
    };
    if !prototype.is_null() && prototype != plain_object_prototype(scope)? {
        return Err(options_error());
    }

    let args = v8::GetPropertyNamesArgs {
        mode: v8::KeyCollectionMode::OwnOnly,
        property_filter: v8::PropertyFilter::ALL_PROPERTIES,
        index_filter: v8::IndexFilter::IncludeIndices,
        key_conversion: v8::KeyConversionMode::ConvertToString,
    };
    let Some(names) = object.get_own_property_names(scope, args) else {
        return Err(options_error());
    };
    if algorithm == Algorithm::Aes256Gcm {
        return if names.length() == 0 {
            Ok(true)
        } else {
            Err(options_error())
        };
    }
    // CBC has at most one accepted key, so reject a record with several own
    // properties before reading any descriptor.
    if names.length() > 1 {
        return Err(options_error());
    }

    let mut padding = true;
    for index in 0..names.length() {
        let Some(name_value) = names.get_index(scope, index) else {
            return Err(options_error());
        };
        if name_value.is_symbol() {
            return Err(options_error());
        }
        let Ok(name) = v8::Local::<v8::Name>::try_from(name_value) else {
            return Err(options_error());
        };
        let Some(descriptor_value) = object.get_own_property_descriptor(scope, name) else {
            return Err(options_error());
        };
        let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor_value) else {
            return Err(options_error());
        };
        if !own_descriptor_boolean(scope, descriptor, "enumerable")? {
            return Err(options_error());
        }
        if own_descriptor_field(scope, descriptor, "get")?.is_some()
            || own_descriptor_field(scope, descriptor, "set")?.is_some()
        {
            return Err(options_error());
        }
        let Ok(key) = v8::Local::<v8::String>::try_from(name_value) else {
            return Err(options_error());
        };
        if key.to_rust_string_lossy(scope) != "padding" {
            return Err(options_error());
        }
        let Some(value) = own_descriptor_field(scope, descriptor, "value")? else {
            return Err(options_error());
        };
        if value.is_undefined() {
            continue;
        }
        if !value.is_boolean() {
            return Err(options_error());
        }
        padding = value.boolean_value(scope);
    }
    Ok(padding)
}

/// Read the strings-only input encoding code. Only 0 (UTF-8) is accepted.
fn input_encoding_code(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<u32, CipherError> {
    if value.is_undefined() {
        return Ok(0);
    }
    if !value.is_number() {
        return Err(CipherError::new(CipherErrorCode::Type));
    }
    let Some(number) = value.number_value(scope) else {
        return Err(CipherError::new(CipherErrorCode::Type));
    };
    if !number.is_finite() || number.trunc() != number || number < 0.0 || number > 0.0 {
        return Err(CipherError::new(CipherErrorCode::Range));
    }
    Ok(0)
}

/// Convert a checked string to UTF-8 bytes, enforcing `limit`.
fn string_bytes(
    scope: &v8::PinScope<'_, '_>,
    string: v8::Local<'_, v8::String>,
    limit: usize,
) -> Result<Vec<u8>, CipherError> {
    let length = string.utf8_length(scope);
    if length > limit {
        return Err(CipherError::new(CipherErrorCode::Limit));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| CipherError::new(CipherErrorCode::Limit))?;
    bytes.resize(length, 0);
    let written =
        string.write_utf8_v2(scope, &mut bytes, v8::WriteFlags::kReplaceInvalidUtf8, None);
    if written != length {
        return Err(CipherError::new(CipherErrorCode::Internal));
    }
    Ok(bytes)
}

/// Read an update input: `Uint8Array` bytes or a UTF-8 string.
fn input_bytes(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    encoding: v8::Local<'_, v8::Value>,
    limit: usize,
) -> Result<Vec<u8>, CipherError> {
    if value.is_string() {
        let code = input_encoding_code(scope, encoding)?;
        if code != 0 {
            return Err(CipherError::new(CipherErrorCode::Type));
        }
        let string = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| CipherError::new(CipherErrorCode::Type))?;
        return string_bytes(scope, string, limit);
    }
    if !encoding.is_undefined() {
        return Err(CipherError::new(CipherErrorCode::Type));
    }
    let view = checked_view(scope, value)?;
    if view.length > limit {
        return Err(CipherError::new(CipherErrorCode::Limit));
    }
    Ok(copy_view(&view)?)
}

fn set_cipher_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: CipherError,
) {
    set_failure_parts(
        scope,
        object,
        error.code().js_class(),
        error.code().as_str(),
        error.message(),
    );
}

fn outcome_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<ResourceId, CipherError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(id) => {
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(
                scope,
                object,
                "id",
                v8::Integer::new_from_unsigned(scope, id).into(),
            );
        }
        Err(error) => set_cipher_failure(scope, object, error),
    }
    object.into()
}

fn outcome_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<(), CipherError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(()) => set(scope, object, "ok", v8::Boolean::new(scope, true).into()),
        Err(error) => set_cipher_failure(scope, object, error),
    }
    object.into()
}

fn outcome_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<Vec<u8>, CipherError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(bytes) => match bytes_to_v8(scope, bytes) {
            Some(value) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", value.into());
            }
            None => set_cipher_failure(scope, object, CipherError::new(CipherErrorCode::Limit)),
        },
        Err(error) => set_cipher_failure(scope, object, error),
    }
    object.into()
}

#[op2]
fn op_neo_crypto_cipher_create<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    direction: v8::Local<'s, v8::Value>,
    algorithm: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
    iv: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let direction = direction_from(scope, direction)?;
        let algorithm = algorithm_from(scope, algorithm)?;
        let padding = options_padding(scope, algorithm, options)?;
        let key_view = checked_view(scope, key)?;
        if key_view.length != KEY_LEN {
            return Err(CipherError::new(CipherErrorCode::Key));
        }
        let key = copy_view(&key_view)?;
        let iv_view = checked_view(scope, iv)?;
        if iv_view.length != algorithm.iv_len() {
            return Err(CipherError::new(CipherErrorCode::Iv));
        }
        let iv = copy_view(&iv_view)?;
        let budget = state.borrow::<CipherBudgetHandle>().budget();
        let open = budget.reserve_open()?;
        let cipher = CipherState::new(
            direction,
            algorithm,
            &key,
            &iv,
            padding,
            CipherLimits::default(),
            budget,
        )?;
        Ok(state.resource_table.add(CipherResource::new(cipher, open)))
    })();
    outcome_id(scope, result)
}

#[op2]
fn op_neo_crypto_cipher_update<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    data: v8::Local<'s, v8::Value>,
    encoding: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<CipherResource>(id)
            .map_err(|_| CipherError::new(CipherErrorCode::Closed))?;
        let mut guard = resource.state.borrow_mut();
        let cipher = guard
            .as_mut()
            .ok_or_else(|| CipherError::new(CipherErrorCode::Closed))?;
        let limit = cipher.remaining_input();
        let bytes = input_bytes(scope, data, encoding, limit)?;
        cipher.update(&bytes)
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_neo_crypto_cipher_final<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<CipherResource>(id)
            .map_err(|_| CipherError::new(CipherErrorCode::Closed))?;
        let mut guard = resource.state.borrow_mut();
        let cipher = guard
            .as_mut()
            .ok_or_else(|| CipherError::new(CipherErrorCode::Closed))?;
        cipher.finalize()
    })();
    outcome_bytes(scope, result)
}

#[op2]
fn op_neo_crypto_cipher_set_aad<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    aad: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<CipherResource>(id)
            .map_err(|_| CipherError::new(CipherErrorCode::Closed))?;
        let mut guard = resource.state.borrow_mut();
        let cipher = guard
            .as_mut()
            .ok_or_else(|| CipherError::new(CipherErrorCode::Closed))?;
        if cipher.algorithm != Algorithm::Aes256Gcm {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        let view = checked_view(scope, aad)?;
        if view.length > cipher.limits.max_aad {
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        let bytes = copy_view(&view)?;
        cipher.set_aad(bytes)
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_neo_crypto_cipher_set_tag<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    tag: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<CipherResource>(id)
            .map_err(|_| CipherError::new(CipherErrorCode::Closed))?;
        let mut guard = resource.state.borrow_mut();
        let cipher = guard
            .as_mut()
            .ok_or_else(|| CipherError::new(CipherErrorCode::Closed))?;
        let view = checked_view(scope, tag)?;
        if view.length != TAG_LEN {
            return Err(CipherError::new(CipherErrorCode::Tag));
        }
        let bytes = copy_view(&view)?;
        cipher.set_tag(&bytes)
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_neo_crypto_cipher_get_tag<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<CipherResource>(id)
            .map_err(|_| CipherError::new(CipherErrorCode::Closed))?;
        let guard = resource.state.borrow();
        let cipher = guard
            .as_ref()
            .ok_or_else(|| CipherError::new(CipherErrorCode::Closed))?;
        Ok(cipher.auth_tag()?.to_vec())
    })();
    outcome_bytes(scope, result)
}

#[op2]
fn op_neo_crypto_cipher_set_padding<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    padding: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let enabled = padding_from(scope, padding)?;
        let resource = state
            .resource_table
            .get::<CipherResource>(id)
            .map_err(|_| CipherError::new(CipherErrorCode::Closed))?;
        let mut guard = resource.state.borrow_mut();
        let cipher = guard
            .as_mut()
            .ok_or_else(|| CipherError::new(CipherErrorCode::Closed))?;
        cipher.set_padding(enabled)
    })();
    outcome_unit(scope, result)
}

/// Drop one cipher object's native state explicitly.
#[op2(fast)]
fn op_neo_crypto_cipher_release(state: &mut OpState, #[smi] id: ResourceId) {
    let _ = state.resource_table.take::<CipherResource>(id);
}

deno_core::extension!(
    neojs_crypto_cipher,
    ops = [
        op_neo_crypto_cipher_create,
        op_neo_crypto_cipher_update,
        op_neo_crypto_cipher_final,
        op_neo_crypto_cipher_set_aad,
        op_neo_crypto_cipher_set_tag,
        op_neo_crypto_cipher_get_tag,
        op_neo_crypto_cipher_set_padding,
        op_neo_crypto_cipher_release
    ],
    state = |state| {
        state.put(CipherBudgetHandle::default());
    },
    docs =
        "Bounded AES-256-CBC and AES-256-GCM cipher ops behind `Neo.getBuiltInModule(\"crypto\")`."
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

    fn budget() -> Arc<CipherBudget> {
        Arc::new(CipherBudget::default())
    }

    fn state(
        direction: Direction,
        algorithm: Algorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
    ) -> CipherState {
        CipherState::new(
            direction,
            algorithm,
            key,
            iv,
            padding,
            CipherLimits::default(),
            budget(),
        )
        .expect("valid state")
    }

    fn state_with_budget(
        direction: Direction,
        algorithm: Algorithm,
        key: &[u8],
        iv: &[u8],
        padding: bool,
        limits: CipherLimits,
        budget: Arc<CipherBudget>,
    ) -> CipherState {
        CipherState::new(direction, algorithm, key, iv, padding, limits, budget)
            .expect("valid state")
    }

    // NIST SP 800-38A F.2.5 / F.2.6, AES-256-CBC.
    const NIST_CBC_KEY: &str = "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4";
    const NIST_CBC_IV: &str = "000102030405060708090a0b0c0d0e0f";
    const NIST_CBC_PLAINTEXT: &str = concat!(
        "6bc1bee22e409f96e93d7e117393172a",
        "ae2d8a571e03ac9c9eb76fac45af8e51",
        "30c81c46a35ce411e5fbc1191a0a52ef",
        "f69f2445df4f9b17ad2b417be66c3710",
    );
    const NIST_CBC_CIPHERTEXT: &str = concat!(
        "f58c4c04d6e5f1ba779eabfb5f7bfbd6",
        "9cfc4e967edb808d679f777bc6702c7d",
        "39f23369a9d9bacfa530e26304231461",
        "b2eb05e2c39be9fcda6c19078c6a9d1b",
    );

    // NIST/McGrew-Viega GCM test case 16 (four blocks with AAD), also used by
    // the one-shot envelope tests in `crypto.rs`.
    const NIST_GCM_KEY: &str = "feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308";
    const NIST_GCM_NONCE: &str = "cafebabefacedbaddecaf888";
    const NIST_GCM_AAD: &str = "feedfacedeadbeeffeedfacedeadbeefabaddad2";
    const NIST_GCM_PLAINTEXT: &str = concat!(
        "d9313225f88406e5a55909c5aff5269a",
        "86a7a9531534f7da2e4c303d8a318a72",
        "1c3c0c95956809532fcf0e2449a6b525",
        "b16aedf5aa0de657ba637b39",
    );
    const NIST_GCM_CIPHERTEXT: &str = concat!(
        "522dc1f099567d07f47f37a32a84427d",
        "643a8cdcbfe5c0c97598a2bd2555d1aa",
        "8cb08e48590dbb3da7b08b1056828838",
        "c5f61e6393ba7a0abcc9f662",
    );
    const NIST_GCM_TAG: &str = "76fc6ece0f4e1768cddf8853bb2d551b";

    // Node 26 / OpenSSL 3.6 fixture for the public AES-256-CBC surface.
    const NODE_CBC_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const NODE_CBC_IV: &str = "101112131415161718191a1b1c1d1e1f";
    const NODE_CBC_PLAINTEXT: &str = "6e656f6a73206369706865722063626320696e7465726f70";
    const NODE_CBC_CIPHERTEXT: &str =
        "5226084e220371598b38cb20eaa1b85c2ff1b252517261b6f0054232236daa85";

    // Node 26 / OpenSSL 3.6 fixture for the public AES-256-GCM surface.
    const NODE_GCM_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const NODE_GCM_IV: &str = "a1b2c3d4e5f60718293a4b5c";
    const NODE_GCM_AAD: &str = "6e656f6a732d6369706865722d636f6e74657874";
    const NODE_GCM_PLAINTEXT: &str = "6e656f6a73206369706865722067636d20696e7465726f70";
    const NODE_GCM_CIPHERTEXT: &str = "6c430e93ec1204a1c46d5cf416133f6de7b2cdaa473a24a1";
    const NODE_GCM_TAG: &str = "9ea6507bfbc87455b1f24edc000cc662";

    #[test]
    fn published_nist_cbc_vector_matches_both_directions() {
        let key = decode_hex(NIST_CBC_KEY);
        let iv = decode_hex(NIST_CBC_IV);
        let plaintext = decode_hex(NIST_CBC_PLAINTEXT);
        let expected = decode_hex(NIST_CBC_CIPHERTEXT);

        let ciphertext = cbc_encrypt(&key, &iv, false, &plaintext).unwrap();
        assert_eq!(encode_hex(&ciphertext), NIST_CBC_CIPHERTEXT);
        let recovered = cbc_decrypt(&key, &iv, false, &expected).unwrap();
        assert_eq!(recovered, plaintext);
    }

    #[test]
    fn published_nist_gcm_vector_matches_both_directions() {
        let key = decode_hex(NIST_GCM_KEY);
        let nonce = decode_hex(NIST_GCM_NONCE);
        let aad = decode_hex(NIST_GCM_AAD);
        let plaintext = decode_hex(NIST_GCM_PLAINTEXT);
        let expected = decode_hex(NIST_GCM_CIPHERTEXT);
        let expected_tag = tag_array(&decode_hex(NIST_GCM_TAG));

        let (ciphertext, tag) = gcm_encrypt(&key, &nonce, &aad, plaintext.clone()).unwrap();
        assert_eq!(encode_hex(&ciphertext), NIST_GCM_CIPHERTEXT);
        assert_eq!(encode_hex(&tag), NIST_GCM_TAG);
        assert_eq!(
            gcm_decrypt(&key, &nonce, &aad, expected, &expected_tag).unwrap(),
            plaintext
        );
    }

    #[test]
    fn node_cbc_fixture_matches_through_the_state() {
        let key = decode_hex(NODE_CBC_KEY);
        let iv = decode_hex(NODE_CBC_IV);
        let plaintext = decode_hex(NODE_CBC_PLAINTEXT);

        let mut encrypt = state(Direction::Encrypt, Algorithm::Aes256Cbc, &key, &iv, true);
        encrypt.update(&plaintext).unwrap();
        let ciphertext = encrypt.finalize().unwrap();
        assert_eq!(encode_hex(&ciphertext), NODE_CBC_CIPHERTEXT);

        let mut decrypt = state(Direction::Decrypt, Algorithm::Aes256Cbc, &key, &iv, true);
        decrypt.update(&ciphertext).unwrap();
        assert_eq!(decrypt.finalize().unwrap(), plaintext);
    }

    #[test]
    fn node_gcm_fixture_matches_through_the_state() {
        let key = decode_hex(NODE_GCM_KEY);
        let iv = decode_hex(NODE_GCM_IV);
        let aad = decode_hex(NODE_GCM_AAD);
        let plaintext = decode_hex(NODE_GCM_PLAINTEXT);
        let expected_tag = decode_hex(NODE_GCM_TAG);

        let mut encrypt = state(Direction::Encrypt, Algorithm::Aes256Gcm, &key, &iv, false);
        encrypt.set_aad(aad.clone()).unwrap();
        encrypt.update(&plaintext).unwrap();
        let ciphertext = encrypt.finalize().unwrap();
        assert_eq!(encode_hex(&ciphertext), NODE_GCM_CIPHERTEXT);
        assert_eq!(encode_hex(&encrypt.auth_tag().unwrap()), NODE_GCM_TAG);

        let mut decrypt = state(Direction::Decrypt, Algorithm::Aes256Gcm, &key, &iv, false);
        decrypt.set_aad(aad).unwrap();
        decrypt.set_tag(&expected_tag).unwrap();
        decrypt.update(&ciphertext).unwrap();
        assert_eq!(decrypt.finalize().unwrap(), plaintext);
    }

    #[test]
    fn cbc_padding_is_strict_and_padding_can_be_disabled() {
        let key = [0x31u8; KEY_LEN];
        let iv = [0x32u8; CBC_IV_LEN];

        // Empty input still gets a full padding block.
        let ciphertext = cbc_encrypt(&key, &iv, true, &[]).unwrap();
        assert_eq!(ciphertext.len(), CBC_IV_LEN);
        assert!(
            cbc_decrypt(&key, &iv, true, &ciphertext)
                .unwrap()
                .is_empty()
        );
        // With padding off, empty is already a whole number of blocks.
        assert!(cbc_encrypt(&key, &iv, false, &[]).unwrap().is_empty());
        assert!(cbc_decrypt(&key, &iv, false, &[]).unwrap().is_empty());

        // A non-block-multiple plaintext is rejected when padding is off.
        assert_eq!(
            cbc_encrypt(&key, &iv, false, &[0u8; 15])
                .unwrap_err()
                .code(),
            CipherErrorCode::Padding
        );
        assert_eq!(
            cbc_decrypt(&key, &iv, false, &[0u8; 15])
                .unwrap_err()
                .code(),
            CipherErrorCode::Padding
        );

        // An empty ciphertext with padding enabled has no padding block.
        assert_eq!(
            cbc_decrypt(&key, &iv, true, &[]).unwrap_err().code(),
            CipherErrorCode::Padding
        );

        // A valid no-padding block decrypted with padding enabled exposes the
        // final plaintext byte 0x41 as an invalid padding length.
        let block = [0x41u8; CBC_IV_LEN];
        let ciphertext = cbc_encrypt(&key, &iv, false, &block).unwrap();
        assert_eq!(
            cbc_decrypt(&key, &iv, true, &ciphertext)
                .unwrap_err()
                .code(),
            CipherErrorCode::Padding
        );

        // A tampered last block must not be returned as plaintext.
        let ciphertext = cbc_encrypt(&key, &iv, true, &block).unwrap();
        let mut tampered = ciphertext;
        let last = tampered.len() - 1;
        tampered[last] ^= 0x80;
        assert_eq!(
            cbc_decrypt(&key, &iv, true, &tampered).unwrap_err().code(),
            CipherErrorCode::Padding
        );
    }

    fn tag_array(value: &[u8]) -> [u8; TAG_LEN] {
        value.try_into().expect("16-byte tag")
    }

    #[test]
    fn gcm_rejects_bad_tags_keys_nonces_and_aad() {
        let key = decode_hex(NODE_GCM_KEY);
        let iv = decode_hex(NODE_GCM_IV);
        let aad = decode_hex(NODE_GCM_AAD);
        let ciphertext = decode_hex(NODE_GCM_CIPHERTEXT);
        let tag = tag_array(&decode_hex(NODE_GCM_TAG));

        let mut bad_tag = tag;
        bad_tag[0] ^= 1;
        assert_eq!(
            gcm_decrypt(&key, &iv, &aad, ciphertext.clone(), &bad_tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
        assert_eq!(
            gcm_decrypt(&key, &iv, b"other", ciphertext.clone(), &tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
        let mut wrong_nonce = iv.clone();
        wrong_nonce[0] ^= 1;
        assert_eq!(
            gcm_decrypt(&key, &wrong_nonce, &aad, ciphertext.clone(), &tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
        let mut wrong_key = key.clone();
        wrong_key[0] ^= 1;
        assert_eq!(
            gcm_decrypt(&wrong_key, &iv, &aad, ciphertext.clone(), &tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
        let mut tampered = ciphertext;
        tampered[0] ^= 1;
        assert_eq!(
            gcm_decrypt(&key, &iv, &aad, tampered, &tag)
                .unwrap_err()
                .code(),
            CipherErrorCode::Authentication
        );
    }

    #[test]
    fn gcm_requires_a_tag_before_final_and_never_returns_plaintext_from_update() {
        let key = [0x41u8; KEY_LEN];
        let iv = [0x42u8; NONCE_LEN];
        let mut decrypt = state(Direction::Decrypt, Algorithm::Aes256Gcm, &key, &iv, false);
        decrypt.update(&[0u8; 16]).unwrap();
        assert_eq!(
            decrypt.finalize().unwrap_err().code(),
            CipherErrorCode::MissingTag
        );

        let mut encrypt = state(Direction::Encrypt, Algorithm::Aes256Gcm, &key, &iv, false);
        encrypt.update(&[0u8; 16]).unwrap();
        assert_eq!(
            encrypt.auth_tag().unwrap_err().code(),
            CipherErrorCode::State
        );
        encrypt.finalize().unwrap();
        assert!(encrypt.auth_tag().is_ok());
    }

    #[test]
    fn lifecycle_rejects_out_of_order_operations() {
        let key = [0x51u8; KEY_LEN];
        let iv = [0x52u8; NONCE_LEN];
        let mut encrypt = state(Direction::Encrypt, Algorithm::Aes256Gcm, &key, &iv, false);
        encrypt.update(b"payload").unwrap();
        encrypt.finalize().unwrap();
        assert_eq!(
            encrypt.update(b"later").unwrap_err().code(),
            CipherErrorCode::State
        );
        assert_eq!(
            encrypt.finalize().unwrap_err().code(),
            CipherErrorCode::State
        );
        assert_eq!(
            encrypt.set_aad(Vec::new()).unwrap_err().code(),
            CipherErrorCode::State
        );

        let mut decrypt = state(Direction::Decrypt, Algorithm::Aes256Gcm, &key, &iv, false);
        decrypt.set_aad(Vec::new()).unwrap();
        assert_eq!(
            decrypt.set_aad(Vec::new()).unwrap_err().code(),
            CipherErrorCode::State
        );
        decrypt.update(b"ciphertext").unwrap();
        assert_eq!(
            decrypt.set_aad(Vec::new()).unwrap_err().code(),
            CipherErrorCode::State
        );
        let mut tag = [0u8; TAG_LEN];
        tag[0] = 1;
        decrypt.set_tag(&tag).unwrap();
        assert_eq!(
            decrypt.set_tag(&tag).unwrap_err().code(),
            CipherErrorCode::State
        );
    }

    #[test]
    fn unsupported_operations_are_reported_for_the_algorithm() {
        let key = [0x61u8; KEY_LEN];
        let cbc_iv = [0x62u8; CBC_IV_LEN];
        let gcm_iv = [0x63u8; NONCE_LEN];

        let mut cbc = state(
            Direction::Encrypt,
            Algorithm::Aes256Cbc,
            &key,
            &cbc_iv,
            true,
        );
        assert_eq!(
            cbc.set_aad(Vec::new()).unwrap_err().code(),
            CipherErrorCode::Unsupported
        );

        let mut gcm = state(
            Direction::Encrypt,
            Algorithm::Aes256Gcm,
            &key,
            &gcm_iv,
            false,
        );
        assert_eq!(
            gcm.set_padding(false).unwrap_err().code(),
            CipherErrorCode::Unsupported
        );

        let decrypt = state(
            Direction::Decrypt,
            Algorithm::Aes256Gcm,
            &key,
            &gcm_iv,
            false,
        );
        assert_eq!(
            decrypt.auth_tag().unwrap_err().code(),
            CipherErrorCode::Unsupported
        );
    }

    #[test]
    fn per_object_limits_are_checked_before_buffering() {
        let limits = CipherLimits {
            max_input: 4,
            max_aad: 2,
        };
        let key = [0x71u8; KEY_LEN];
        let iv = [0x72u8; NONCE_LEN];
        let mut cipher = state_with_budget(
            Direction::Encrypt,
            Algorithm::Aes256Gcm,
            &key,
            &iv,
            false,
            limits,
            budget(),
        );
        cipher.set_aad(vec![1, 2]).unwrap();
        cipher.update(&[1, 2, 3, 4]).unwrap();
        assert_eq!(
            cipher.update(&[5]).unwrap_err().code(),
            CipherErrorCode::Limit
        );
        assert_eq!(cipher.remaining_input(), 0);

        let mut other = state_with_budget(
            Direction::Encrypt,
            Algorithm::Aes256Gcm,
            &key,
            &iv,
            false,
            limits,
            budget(),
        );
        assert_eq!(
            other.set_aad(vec![1, 2, 3]).unwrap_err().code(),
            CipherErrorCode::Limit
        );
    }

    #[test]
    fn aggregate_and_open_budgets_are_enforced() {
        let shared = Arc::new(CipherBudget::with_limits(2, 4));
        let key = [0x81u8; KEY_LEN];
        let iv = [0x82u8; NONCE_LEN];
        let limits = CipherLimits {
            max_input: 64,
            max_aad: 64,
        };
        let mut first = state_with_budget(
            Direction::Encrypt,
            Algorithm::Aes256Gcm,
            &key,
            &iv,
            false,
            limits,
            Arc::clone(&shared),
        );
        first.update(&[1, 2, 3, 4]).unwrap();
        let mut second = state_with_budget(
            Direction::Encrypt,
            Algorithm::Aes256Gcm,
            &key,
            &iv,
            false,
            limits,
            Arc::clone(&shared),
        );
        assert_eq!(
            second.update(&[5]).unwrap_err().code(),
            CipherErrorCode::Busy
        );
        drop(first);
        second.update(&[5]).unwrap();

        let small = Arc::new(CipherBudget::with_limits(1, 16));
        let first = small.reserve_open().unwrap();
        assert_eq!(
            small.reserve_open().err().unwrap().code(),
            CipherErrorCode::TooManyObjects
        );
        drop(first);
        assert!(small.reserve_open().is_ok());
    }

    #[test]
    fn key_and_iv_lengths_are_enforced() {
        let key = [0u8; KEY_LEN];
        let iv = [0u8; NONCE_LEN];
        for length in [0usize, 1, 15, 31, 33] {
            assert_eq!(
                CipherState::new(
                    Direction::Encrypt,
                    Algorithm::Aes256Gcm,
                    &vec![0u8; length],
                    &iv,
                    false,
                    CipherLimits::default(),
                    budget(),
                )
                .err()
                .unwrap()
                .code(),
                CipherErrorCode::Key
            );
        }
        for length in [0usize, 1, 11, 13, 16] {
            assert_eq!(
                CipherState::new(
                    Direction::Encrypt,
                    Algorithm::Aes256Gcm,
                    &key,
                    &vec![0u8; length],
                    false,
                    CipherLimits::default(),
                    budget(),
                )
                .err()
                .unwrap()
                .code(),
                CipherErrorCode::Iv
            );
        }
        for length in [0usize, 12, 15, 17] {
            assert_eq!(
                CipherState::new(
                    Direction::Encrypt,
                    Algorithm::Aes256Cbc,
                    &key,
                    &vec![0u8; length],
                    true,
                    CipherLimits::default(),
                    budget(),
                )
                .err()
                .unwrap()
                .code(),
                CipherErrorCode::Iv
            );
        }
    }

    #[test]
    fn errors_have_stable_codes_and_safe_messages() {
        assert_eq!(
            CipherError::new(CipherErrorCode::Algorithm).code().as_str(),
            Some("NEO_CRYPTO_ALGORITHM")
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::MissingTag)
                .code()
                .as_str(),
            Some("NEO_CRYPTO_TAG")
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::Padding).code().as_str(),
            Some("NEO_CRYPTO_PADDING")
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::State).code().as_str(),
            Some("NEO_CRYPTO_STATE")
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::Type).code().as_str(),
            None
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::Options).code().as_str(),
            None
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::Options).code().js_class(),
            "TypeError"
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::Range).code().js_class(),
            "RangeError"
        );
        for code in [
            CipherErrorCode::Algorithm,
            CipherErrorCode::Key,
            CipherErrorCode::Iv,
            CipherErrorCode::Tag,
            CipherErrorCode::MissingTag,
            CipherErrorCode::Authentication,
            CipherErrorCode::Padding,
            CipherErrorCode::State,
            CipherErrorCode::Closed,
            CipherErrorCode::Unsupported,
            CipherErrorCode::Limit,
            CipherErrorCode::TooManyObjects,
            CipherErrorCode::Busy,
            CipherErrorCode::Detached,
            CipherErrorCode::Internal,
            CipherErrorCode::Type,
            CipherErrorCode::Options,
            CipherErrorCode::Range,
        ] {
            let message = CipherError::new(code).message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
        }
    }
}

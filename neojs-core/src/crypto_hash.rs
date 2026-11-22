//! Incremental SHA-2 hashing and HMAC for `neo:crypto`, plus bounded
//! asynchronous one-shot variants.
//!
//! The cryptographic primitives are the RustCrypto `sha2` and `hmac` crates.
//! This module defines the JavaScript-facing lifecycle, the accepted algorithm
//! and encoding names, the input bounds, the typed-array validation, and the
//! error mapping. It is not a sandbox and does not manage secrets for the
//! caller.
//!
//! # Algorithms
//!
//! The accepted names are `sha256`, `sha384`, and `sha512`, compared
//! case-insensitively. No other name is accepted, and no OpenSSL alias such as
//! `sha2-256` is guessed. The digest lengths are 32, 48, and 64 bytes.
//!
//! # Synchronous API
//!
//! `create_hash` returns an object with `update(data[, encoding])` and
//! `digest([encoding])`. `update` accepts a `Uint8Array` of exact bytes or a
//! string encoded as UTF-8; a string may name only `utf8`/`utf-8`, and a
//! `Uint8Array` may not name an encoding at all. `digest()` returns a
//! `Uint8Array`; `digest("hex")`, `digest("base64")`, `digest("base64url")`,
//! and `digest("latin1")` (alias `binary`) return a string. `update` returns
//! the hash object, so calls chain.
//!
//! `create_hmac` takes the same algorithm names and a key that is a
//! `Uint8Array` or a UTF-8 string. HMAC accepts a key of any length, including
//! zero; the key is copied when the object is created, so later mutation of
//! the caller's view does not change it.
//!
//! # Lifecycle
//!
//! `digest` consumes the state. A second `digest`, and any `update` after a
//! successful `digest`, fail with `NEO_CRYPTO_DIGESTED`. An invalid digest
//! encoding is rejected before the state is consumed, so the object remains
//! usable. There is no `copy()` in this slice. `close()` (and its alias
//! `dispose()`) releases the state without producing a digest and is
//! idempotent; a later `update` or `digest` fails with `NEO_CRYPTO_CLOSED`.
//! The object's Rust state lives in the runtime's resource table until `digest`,
//! `close()`, or the runtime is dropped. There is no finalizer, so an object
//! that is created and abandoned keeps its small state until the runtime is
//! dropped. A per-runtime hard cap, [`MAX_OPEN_DIGESTS`], bounds those abandoned
//! objects; creation past it fails with `NEO_CRYPTO_TOO_MANY_OBJECTS`.
//!
//! # Asynchronous API
//!
//! `hash_async` and `hmac_async` return a `Promise` and are genuinely
//! nonblocking: argument validation, the budget admission, and a single bounded
//! copy of the input run on the isolate thread. All hashing runs on the
//! runtime's bounded Tokio blocking pool through a `deno_core` async op: the
//! worker builds the hash state, hashes the HMAC key, hashes the data, and
//! encodes the digest. They are not `Promise.resolve` wrappers around a blocking
//! call. The input is copied before the promise is returned, so mutating the
//! caller's buffer afterwards does not change the result.
//!
//! The pool has four blocking threads, configured by the runtime. At most
//! [`MAX_PENDING_ASYNC_JOBS`] jobs may be pending at once, even if every job is
//! empty, and at most [`MAX_PENDING_ASYNC_BYTES`] of copied data and HMAC key
//! bytes may be pending. A submission over either bound fails with
//! `NEO_CRYPTO_BUSY` rather than queueing unbounded work. The reservation is
//! released when the worker finishes, panics, or fails to start; a job aborted
//! before it starts also releases it, while an already-running job stays charged
//! until it returns. deno_core hosts async ops on a Tokio current-thread runtime on the isolate
//! thread. `run_file` provides one; so does `run_file_async` when the caller's
//! executor is a Tokio current-thread runtime. Any other executor, including a
//! multi-thread Tokio runtime, gets `NEO_CRYPTO_NO_EXECUTOR` before any input is
//! copied, and the synchronous API still works. A runtime with no captured
//! blocking-pool handle reports `NEO_CRYPTO_INTERNAL`. The JavaScript wrapper
//! calls the join op as soon as it receives the job id, so the job's
//! resource-table entry is removed then. If the returned promise is never
//! awaited, its completed digest is retained by that promise until the promise
//! is collected; the worker and its reservation still finish and release
//! first.
//!
//! Cancellation is best effort, not a guarantee that work stops, and it is not
//! triggered by abandoning the returned JavaScript `Promise`. The wrapper
//! starts the join op immediately, so an unawaited promise still drives the job
//! to completion; its result is retained by the promise until it is collected.
//! Dropping the runtime drops the `JsRuntime`, its pending join-op futures, and
//! any unjoined job resources. That aborts a job that has not started on a
//! blocking thread; Tokio then drops its owned input and releases its
//! reservation. Dropping only the `run_file_async` entry future stops polling
//! but does not drop the runtime's op futures or resources, so it is not
//! cancellation. A job that has already started on a blocking thread cannot be
//! stopped: abort is a no-op, the input stays owned until the worker returns,
//! and the job and byte reservations remain charged until it finishes. Its
//! result is discarded if the join future is gone. The four-thread pool and the
//! 64-job/64-MiB admission bounds still apply to every admitted job, so
//! cancellation cannot be used to exceed them.
//!
//! # Limits and allocation
//!
//! One synchronous hash object accepts at most [`MAX_HASH_INPUT_BYTES`] of
//! input across all `update` calls. One HMAC key may be at most
//! [`MAX_HMAC_KEY_BYTES`], and one asynchronous call accepts at most
//! [`MAX_HASH_INPUT_BYTES`] of data. A string is measured by its UTF-8 length
//! before it is copied. [`MAX_OPEN_DIGESTS`] bounds open incremental objects per
//! runtime. The Rust allocations are fallible; a refused reservation is
//! reported as `NEO_CRYPTO_LIMIT`. V8 and the JavaScript heap allocate too, so
//! recovery from out-of-memory is not guaranteed.
//!
//! # Typed arrays
//!
//! Input views are validated with V8's internal `Uint8Array` check, so a
//! `Proxy`, a subclass-shaped object, and another typed-array kind are
//! rejected with `TypeError`; `Symbol.hasInstance` and `byteLength` getters are
//! never consulted. A detached view is rejected with `NEO_CRYPTO_DETACHED`.
//! Offset views copy only their own range. `SharedArrayBuffer` views and
//! resizable-buffer views are accepted, and the bytes are copied before any
//! later JavaScript can run.
//!
//! # Password hashing
//!
//! The [`password`] child module adds synchronous and asynchronous PBKDF2 and
//! bcrypt: `pbkdf2`/`pbkdf2Async`, `bcryptHash`/`bcryptHashAsync`, and
//! `bcryptVerify`/`bcryptVerifyAsync`. It accepts the same `sha256`, `sha384`,
//! and `sha512` names for PBKDF2, uses exact bytes or UTF-8 strings, and shares
//! this module's executor, pending-job and pending-byte budget, cancellation
//! gate, and typed-array validation. See the child module documentation for the
//! bcrypt version, cost, 72-byte, and malicious-hash policy.
//!
//! # Scope
//!
//! This is not Node's `crypto` module. It does not implement algorithms outside
//! the three names above, `createCipheriv`, `subtle`, callbacks, streaming
//! readers, or OpenSSL's algorithm aliases. It makes no zeroization promise:
//! key bytes, input bytes, and digest state may remain in Rust buffers and the
//! JavaScript heap until reclaimed.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
#[cfg(feature = "test-support")]
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::MutexGuard;
#[cfg(feature = "test-support")]
use std::sync::atomic::AtomicU32;
#[cfg(feature = "test-support")]
use std::sync::atomic::Ordering;
#[cfg(feature = "test-support")]
use std::thread::ThreadId;
#[cfg(feature = "test-support")]
use std::time::Duration;
#[cfg(feature = "test-support")]
use std::time::Instant;

use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_core::v8;
use hmac::Hmac;
use hmac::KeyInit;
use hmac::Mac;
use sha2::Digest;
use sha2::Sha256;
use sha2::Sha384;
use sha2::Sha512;
use tokio::runtime::Handle;
use tokio::runtime::RuntimeFlavor;
use tokio::task::AbortHandle;
use tokio::task::JoinHandle;

use crate::crypto::CryptoError;
use crate::crypto::CryptoErrorCode;
use crate::crypto::View;
use crate::crypto::bytes_to_v8;
use crate::crypto::checked_view;
use crate::crypto::copy_view;
use crate::crypto::response;
use crate::crypto::set;
use crate::crypto::set_failure_parts;

mod password;

/// Test-only hook to make the next bcrypt salt draw fail. Compiled only with
/// the off-by-default `test-support` feature; a normal build has no hook.
#[cfg(feature = "test-support")]
pub(crate) fn fail_next_bcrypt_random() {
    password::fail_next_bcrypt_random();
}

/// Largest input accepted by one synchronous hash object and by one
/// asynchronous call.
pub(crate) const MAX_HASH_INPUT_BYTES: usize = 16 * 1024 * 1024;

/// Largest HMAC key accepted, in bytes.
pub(crate) const MAX_HMAC_KEY_BYTES: usize = 1024 * 1024;

/// Largest amount of copied input allowed to be pending across asynchronous
/// calls.
pub(crate) const MAX_PENDING_ASYNC_BYTES: usize = 64 * 1024 * 1024;

/// Largest number of asynchronous jobs allowed to be pending at once,
/// regardless of their size. This bounds the queue even when every job hashes
/// an empty input.
pub(crate) const MAX_PENDING_ASYNC_JOBS: usize = 64;

/// Largest number of incremental hash and HMAC objects that may be open at
/// once. This bounds state that a caller creates and abandons without calling
/// `close()` or `digest()`.
pub(crate) const MAX_OPEN_DIGESTS: usize = 1024;

/// A supported SHA-2 algorithm.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Algorithm {
    Sha256,
    Sha384,
    Sha512,
}

impl Algorithm {
    /// Parse one of the three accepted, case-insensitive names.
    fn parse(name: &str) -> Option<Self> {
        if name.eq_ignore_ascii_case("sha256") {
            Some(Self::Sha256)
        } else if name.eq_ignore_ascii_case("sha384") {
            Some(Self::Sha384)
        } else if name.eq_ignore_ascii_case("sha512") {
            Some(Self::Sha512)
        } else {
            None
        }
    }
}

/// A digest output encoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputEncoding {
    /// Raw digest bytes, returned as a `Uint8Array`.
    Binary,
    /// Lowercase hexadecimal text.
    Hex,
    /// Standard Base64 with padding.
    Base64,
    /// URL-safe Base64 without padding.
    Base64Url,
    /// One JavaScript code unit per digest byte.
    Latin1,
}

impl OutputEncoding {
    /// Decode the numeric code the JavaScript wrapper passes.
    fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => Self::Binary,
            1 => Self::Hex,
            2 => Self::Base64,
            3 => Self::Base64Url,
            4 => Self::Latin1,
            _ => return None,
        })
    }
}

/// Stable machine-readable hash failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HashErrorCode {
    /// The algorithm name is not one of the three supported names.
    Algorithm,
    /// The string input encoding is not `utf8`.
    Encoding,
    /// An input or allocation exceeded a documented bound.
    Limit,
    /// `digest` was already called on this object.
    Digested,
    /// Too much asynchronous input is already pending.
    Busy,
    /// The event loop cannot host deno_core async ops.
    NoExecutor,
    /// Too many incremental hash objects are open.
    TooManyObjects,
    /// The view's buffer was detached.
    Detached,
    /// A worker or the runtime executor failed.
    Internal,
    /// An argument is neither a `Uint8Array` nor a string.
    Type,
    /// A numeric argument is not an integer in its documented range.
    Range,
}

impl HashErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    ///
    /// Argument errors ([`HashErrorCode::Type`] and [`HashErrorCode::Range`])
    /// have no code and become `TypeError` or `RangeError`.
    fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Algorithm => "NEO_CRYPTO_ALGORITHM",
            Self::Encoding => "NEO_CRYPTO_ENCODING",
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::Digested => "NEO_CRYPTO_DIGESTED",
            Self::Busy => "NEO_CRYPTO_BUSY",
            Self::NoExecutor => "NEO_CRYPTO_NO_EXECUTOR",
            Self::TooManyObjects => "NEO_CRYPTO_TOO_MANY_OBJECTS",
            Self::Detached => "NEO_CRYPTO_DETACHED",
            Self::Internal => "NEO_CRYPTO_INTERNAL",
            Self::Type | Self::Range => return None,
        })
    }

    /// The JavaScript error class used for this failure.
    fn js_class(self) -> &'static str {
        match self {
            Self::Type => "TypeError",
            Self::Range => "RangeError",
            _ => "Error",
        }
    }
}

/// A failure safe to describe without echoing input bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HashError {
    code: HashErrorCode,
}

impl HashError {
    fn new(code: HashErrorCode) -> Self {
        Self { code }
    }

    fn code(self) -> HashErrorCode {
        self.code
    }

    /// A fixed message. It never contains input bytes or a key.
    fn message(self) -> &'static str {
        match self.code {
            HashErrorCode::Algorithm => "unsupported hash algorithm",
            HashErrorCode::Encoding => "unsupported input encoding",
            HashErrorCode::Limit => "input exceeds the hash size limit",
            HashErrorCode::Digested => "digest has already been called",
            HashErrorCode::Busy => "too much asynchronous hash work is pending",
            HashErrorCode::NoExecutor => {
                "asynchronous hashing needs a Tokio current-thread event loop"
            }
            HashErrorCode::TooManyObjects => "too many incremental hash objects are open",
            HashErrorCode::Detached => "input is detached or out of bounds",
            HashErrorCode::Internal => "the hash worker failed",
            HashErrorCode::Type => "expected a Uint8Array or a string",
            HashErrorCode::Range => "numeric argument is outside its documented range",
        }
    }
}

impl From<CryptoError> for HashError {
    fn from(error: CryptoError) -> Self {
        match error.code() {
            CryptoErrorCode::Type => Self::new(HashErrorCode::Type),
            CryptoErrorCode::Detached => Self::new(HashErrorCode::Detached),
            CryptoErrorCode::Limit => Self::new(HashErrorCode::Limit),
            _ => Self::new(HashErrorCode::Internal),
        }
    }
}

/// One incremental hash state.
enum HashState {
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
}

impl HashState {
    fn new(algorithm: Algorithm) -> Self {
        match algorithm {
            Algorithm::Sha256 => Self::Sha256(Sha256::new()),
            Algorithm::Sha384 => Self::Sha384(Sha384::new()),
            Algorithm::Sha512 => Self::Sha512(Sha512::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Sha256(hasher) => hasher.update(data),
            Self::Sha384(hasher) => hasher.update(data),
            Self::Sha512(hasher) => hasher.update(data),
        }
    }

    fn finalize(self) -> Vec<u8> {
        match self {
            Self::Sha256(hasher) => hasher.finalize().to_vec(),
            Self::Sha384(hasher) => hasher.finalize().to_vec(),
            Self::Sha512(hasher) => hasher.finalize().to_vec(),
        }
    }
}

/// One incremental HMAC state.
enum HmacState {
    Sha256(Hmac<Sha256>),
    Sha384(Hmac<Sha384>),
    Sha512(Hmac<Sha512>),
}

impl HmacState {
    fn new(algorithm: Algorithm, key: &[u8]) -> Result<Self, HashError> {
        #[cfg(feature = "test-support")]
        record_hmac_key_placement();
        // `new_from_slice` only rejects a key length the primitive cannot
        // represent; HMAC accepts every length this module admits.
        let failed = || HashError::new(HashErrorCode::Internal);
        Ok(match algorithm {
            Algorithm::Sha256 => {
                Self::Sha256(Hmac::<Sha256>::new_from_slice(key).map_err(|_| failed())?)
            }
            Algorithm::Sha384 => {
                Self::Sha384(Hmac::<Sha384>::new_from_slice(key).map_err(|_| failed())?)
            }
            Algorithm::Sha512 => {
                Self::Sha512(Hmac::<Sha512>::new_from_slice(key).map_err(|_| failed())?)
            }
        })
    }

    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Sha256(mac) => mac.update(data),
            Self::Sha384(mac) => mac.update(data),
            Self::Sha512(mac) => mac.update(data),
        }
    }

    fn finalize(self) -> Vec<u8> {
        match self {
            Self::Sha256(mac) => mac.finalize().into_bytes().to_vec(),
            Self::Sha384(mac) => mac.finalize().into_bytes().to_vec(),
            Self::Sha512(mac) => mac.finalize().into_bytes().to_vec(),
        }
    }
}

enum Inner {
    Hash(HashState),
    Hmac(HmacState),
}

impl Inner {
    fn update(&mut self, data: &[u8]) {
        match self {
            Self::Hash(hasher) => hasher.update(data),
            Self::Hmac(mac) => mac.update(data),
        }
    }

    fn finalize(self) -> Vec<u8> {
        match self {
            Self::Hash(hasher) => hasher.finalize(),
            Self::Hmac(mac) => mac.finalize(),
        }
    }
}

/// One hash or HMAC object's state.
struct DigestState {
    total: usize,
    inner: Inner,
}

impl DigestState {
    fn hash(algorithm: Algorithm) -> Self {
        Self {
            total: 0,
            inner: Inner::Hash(HashState::new(algorithm)),
        }
    }

    fn hmac(algorithm: Algorithm, key: &[u8]) -> Result<Self, HashError> {
        if key.len() > MAX_HMAC_KEY_BYTES {
            return Err(HashError::new(HashErrorCode::Limit));
        }
        Ok(Self {
            total: 0,
            inner: Inner::Hmac(HmacState::new(algorithm, key)?),
        })
    }

    /// Bytes this object may still accept.
    fn remaining(&self) -> usize {
        MAX_HASH_INPUT_BYTES.saturating_sub(self.total)
    }

    /// Add `data` to the digest, enforcing the aggregate input bound.
    fn update(&mut self, data: &[u8]) -> Result<(), HashError> {
        #[cfg(feature = "test-support")]
        record_update_placement();
        let next = self
            .total
            .checked_add(data.len())
            .ok_or_else(|| HashError::new(HashErrorCode::Limit))?;
        if next > MAX_HASH_INPUT_BYTES {
            return Err(HashError::new(HashErrorCode::Limit));
        }
        self.inner.update(data);
        self.total = next;
        Ok(())
    }

    fn finalize(self) -> Vec<u8> {
        self.inner.finalize()
    }
}

/// A synchronous hash or HMAC object held in the resource table.
///
/// The open-object reservation is part of the resource, so dropping the
/// resource through `digest`, `close`, or the runtime's resource table releases
/// the hard-cap slot exactly once.
struct DigestResource {
    state: RefCell<Option<DigestState>>,
    _open: OpenReservation,
}

impl DigestResource {
    fn new(state: DigestState, open: OpenReservation) -> Self {
        Self {
            state: RefCell::new(Some(state)),
            _open: open,
        }
    }
}

impl Resource for DigestResource {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "neo::crypto::Digest".into()
    }
}

/// One pending asynchronous job. The join op takes the handle once.
struct AsyncDigestJob {
    handle: RefCell<Option<JoinHandle<Vec<u8>>>>,
}

impl Drop for AsyncDigestJob {
    fn drop(&mut self) {
        // A resource dropped before its join op takes the handle (for example
        // when the runtime is torn down) aborts a job that has not started.
        // Aborting a running `spawn_blocking` task is a no-op.
        if let Some(handle) = self.handle.get_mut().take() {
            handle.abort();
        }
    }
}

impl Resource for AsyncDigestJob {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "neo::crypto::AsyncDigest".into()
    }
}

/// Aborts a queued blocking task if the join future is dropped.
///
/// A running `spawn_blocking` task cannot be stopped; abort is a no-op for it,
/// and its reservation remains charged until it finishes.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The data, key, and budget reservation owned by one blocking worker.
///
/// Field order is deliberate: Rust drops struct fields in declaration order, so
/// `data` and `key` are released before `_reservation`. The reservation
/// therefore stays charged until both allocations are gone, including when the
/// closure is dropped before it starts, when `run` returns early or panics, and
/// when a running worker is abandoned by runtime teardown dropping its join
/// future. `run` only borrows, so the struct can be dropped explicitly after
/// the result is produced.
struct WorkerOwned {
    data: Vec<u8>,
    key: Vec<u8>,
    _reservation: PendingReservation,
}

impl WorkerOwned {
    /// Build the state and hash on the calling thread.
    ///
    /// The caller must invoke this from the blocking worker, after the probe (in
    /// test builds) is installed. It borrows the owned data and key so the
    /// reservation stays held until `self` drops.
    fn run(&self, algorithm: Algorithm, kind: u32, encoding: OutputEncoding) -> Vec<u8> {
        let mut digest = if kind == 0 {
            DigestState::hash(algorithm)
        } else {
            match DigestState::hmac(algorithm, &self.key) {
                Ok(digest) => digest,
                Err(_) => return Vec::new(),
            }
        };
        if digest.update(&self.data).is_err() {
            return Vec::new();
        }
        encode_digest(&digest.finalize(), encoding)
    }
}

/// The per-runtime budgets for asynchronous jobs and open incremental objects.
#[derive(Default)]
struct BudgetState {
    pending_jobs: usize,
    pending_bytes: usize,
    open_digests: usize,
}

/// Shared budget counters. One mutex keeps the job and byte admission atomic,
/// and it is safe to release from a blocking worker thread, a dropped op
/// future, or a panic unwind.
#[derive(Default)]
pub(crate) struct Budget {
    state: Mutex<BudgetState>,
}

impl Budget {
    fn lock(&self) -> MutexGuard<'_, BudgetState> {
        // A panic while the lock is held cannot corrupt the counters; recover
        // the data instead of poisoning every later reservation.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reserve one pending job and `bytes` of owned input.
    ///
    /// The job count is checked and reserved even when `bytes` is zero, so an
    /// unbounded number of empty jobs cannot queue. The byte count includes the
    /// HMAC key, which the worker owns until it has built the keyed state.
    fn reserve_pending(self: &Arc<Self>, bytes: usize) -> Result<PendingReservation, HashError> {
        let mut state = self.lock();
        if state.pending_jobs >= MAX_PENDING_ASYNC_JOBS {
            return Err(HashError::new(HashErrorCode::Busy));
        }
        let next_bytes = state
            .pending_bytes
            .checked_add(bytes)
            .ok_or_else(|| HashError::new(HashErrorCode::Busy))?;
        if next_bytes > MAX_PENDING_ASYNC_BYTES {
            return Err(HashError::new(HashErrorCode::Busy));
        }
        state.pending_jobs += 1;
        state.pending_bytes = next_bytes;
        Ok(PendingReservation {
            budget: Arc::clone(self),
            bytes,
        })
    }

    /// Reserve one pending job for a module that shares this budget but not
    /// this module's private error type.
    pub(crate) fn reserve_pending_busy(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Option<PendingReservation> {
        self.reserve_pending(bytes).ok()
    }

    /// Reserve one open incremental hash object.
    fn reserve_open(self: &Arc<Self>) -> Result<OpenReservation, HashError> {
        let mut state = self.lock();
        if state.open_digests >= MAX_OPEN_DIGESTS {
            return Err(HashError::new(HashErrorCode::TooManyObjects));
        }
        state.open_digests += 1;
        Ok(OpenReservation {
            budget: Arc::clone(self),
        })
    }
}

/// A reservation released when the worker finishes, panics, or is aborted
/// before it starts. The guard is not `Clone`, so release happens exactly once.
/// A running job cannot be aborted and keeps this charge until it returns.
pub(crate) struct PendingReservation {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Drop for PendingReservation {
    fn drop(&mut self) {
        let mut state = self.budget.lock();
        state.pending_jobs = state.pending_jobs.saturating_sub(1);
        state.pending_bytes = state.pending_bytes.saturating_sub(self.bytes);
    }
}

/// An open-object reservation released when its resource is dropped.
struct OpenReservation {
    budget: Arc<Budget>,
}

impl Drop for OpenReservation {
    fn drop(&mut self) {
        let mut state = self.budget.lock();
        state.open_digests = state.open_digests.saturating_sub(1);
    }
}

/// The per-runtime executor and budgets for hashing.
///
/// The runtime is created inside a Tokio driver, so the extension's state
/// initializer sees that driver's handle. An embedding that installs this
/// extension without a Tokio context gets synchronous hashing and synchronous
/// Shamir secret sharing only.
pub(crate) struct CryptoExecutor {
    handle: Option<Handle>,
    budget: Arc<Budget>,
    /// Whether the worker closure ran on the isolate thread (1) or another
    /// thread (2). Recorded only with the off-by-default `test-support`
    /// feature.
    #[cfg(feature = "test-support")]
    worker_placement: Arc<AtomicU32>,
    /// Where `DigestState::update` actually ran for the last asynchronous job.
    #[cfg(feature = "test-support")]
    update_placement: Arc<AtomicU32>,
    /// Where `HmacState::new` (HMAC key construction) actually ran for the last
    /// asynchronous job.
    #[cfg(feature = "test-support")]
    hmac_key_placement: Arc<AtomicU32>,
    /// Where PBKDF2 derivation actually ran for the last asynchronous job.
    #[cfg(feature = "test-support")]
    pbkdf2_placement: Arc<AtomicU32>,
    /// Where bcrypt hashing or verification actually ran for the last
    /// asynchronous job.
    #[cfg(feature = "test-support")]
    bcrypt_placement: Arc<AtomicU32>,
    /// Where Shamir splitting or combination actually ran for the last
    /// asynchronous job.
    #[cfg(feature = "test-support")]
    shamir_placement: Arc<AtomicU32>,
    #[cfg(feature = "test-support")]
    isolate_thread: ThreadId,
    /// Test-only gate captured when the runtime is created.
    #[cfg(feature = "test-support")]
    cancellation_gate: Option<Arc<CancellationGate>>,
}

impl CryptoExecutor {
    fn capture() -> Self {
        Self {
            handle: Handle::try_current().ok(),
            budget: Arc::new(Budget::default()),
            #[cfg(feature = "test-support")]
            worker_placement: Arc::new(AtomicU32::new(0)),
            #[cfg(feature = "test-support")]
            update_placement: Arc::new(AtomicU32::new(0)),
            #[cfg(feature = "test-support")]
            hmac_key_placement: Arc::new(AtomicU32::new(0)),
            #[cfg(feature = "test-support")]
            pbkdf2_placement: Arc::new(AtomicU32::new(0)),
            #[cfg(feature = "test-support")]
            bcrypt_placement: Arc::new(AtomicU32::new(0)),
            #[cfg(feature = "test-support")]
            shamir_placement: Arc::new(AtomicU32::new(0)),
            #[cfg(feature = "test-support")]
            isolate_thread: std::thread::current().id(),
            #[cfg(feature = "test-support")]
            cancellation_gate: CANCELLATION_GATE.with(|slot| slot.borrow().clone()),
        }
    }

    pub(crate) fn budget(&self) -> Arc<Budget> {
        Arc::clone(&self.budget)
    }

    /// Whether this runtime captured a blocking-pool handle.
    pub(crate) fn has_handle(&self) -> bool {
        self.handle.is_some()
    }

    /// Build a placement probe for one asynchronous job.
    ///
    /// The caller installs it on the isolate thread before copying input. The
    /// worker installs its own clone, so a change that moves the real work back
    /// onto the isolate thread is observable.
    #[cfg(feature = "test-support")]
    fn probe(&self) -> Arc<AsyncProbe> {
        Arc::new(AsyncProbe {
            isolate_thread: self.isolate_thread,
            update_placement: Arc::clone(&self.update_placement),
            hmac_key_placement: Arc::clone(&self.hmac_key_placement),
            pbkdf2_placement: Arc::clone(&self.pbkdf2_placement),
            bcrypt_placement: Arc::clone(&self.bcrypt_placement),
            shamir_placement: Arc::clone(&self.shamir_placement),
        })
    }

    /// Submit one asynchronous crypto job to the bounded blocking pool.
    ///
    /// The caller must have checked that [`Self::handle`] is present and must
    /// already hold a [`PendingReservation`] for the bytes the closure owns.
    /// With `test-support`, the worker records its placement, installs the
    /// probe, and enters the cancellation gate before running `work`. The
    /// closure returns its raw bytes; an empty vector is the internal-failure
    /// sentinel used by the join ops.
    pub(crate) fn spawn_blocking<F, R>(&self, work: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let handle = self
            .handle
            .clone()
            .expect("the caller checked the executor handle");
        #[cfg(feature = "test-support")]
        let probe = self.probe();
        #[cfg(feature = "test-support")]
        let worker_placement = Arc::clone(&self.worker_placement);
        #[cfg(feature = "test-support")]
        let cancellation_gate = self.cancellation_gate.clone();
        #[cfg(feature = "test-support")]
        let gate_budget = Arc::clone(&self.budget);
        handle.spawn_blocking(move || {
            #[cfg(feature = "test-support")]
            {
                let on_isolate = std::thread::current().id() == probe.isolate_thread;
                worker_placement.store(if on_isolate { 1 } else { 2 }, Ordering::Release);
            }
            #[cfg(feature = "test-support")]
            let _worker_probe = ProbeGuard::install(Some(probe));
            #[cfg(feature = "test-support")]
            let _gate_guard = cancellation_gate.as_ref().map(|gate| {
                gate.enter_worker(gate_budget);
                GateLeaveGuard(Arc::clone(gate))
            });
            work()
        })
    }
}

/// Per-job placement probe installed on the isolate thread and on the worker
/// thread while an asynchronous operation runs.
///
/// The instrumentation lives in [`DigestState::update`], [`HmacState::new`],
/// and the Shamir split/combine core, not just in the worker closure, so a
/// change that moves the actual work back onto the isolate thread is
/// observable.
#[cfg(feature = "test-support")]
struct AsyncProbe {
    isolate_thread: ThreadId,
    update_placement: Arc<AtomicU32>,
    hmac_key_placement: Arc<AtomicU32>,
    pbkdf2_placement: Arc<AtomicU32>,
    bcrypt_placement: Arc<AtomicU32>,
    shamir_placement: Arc<AtomicU32>,
}

#[cfg(feature = "test-support")]
thread_local! {
    static ASYNC_PROBE: RefCell<Option<Arc<AsyncProbe>>> = const { RefCell::new(None) };
}

#[cfg(feature = "test-support")]
fn install_probe(probe: Option<Arc<AsyncProbe>>) -> Option<Arc<AsyncProbe>> {
    ASYNC_PROBE.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), probe))
}

#[cfg(feature = "test-support")]
fn record_update_placement() {
    ASYNC_PROBE.with(|slot| {
        if let Some(probe) = slot.borrow().as_ref() {
            let on_isolate = std::thread::current().id() == probe.isolate_thread;
            probe
                .update_placement
                .store(if on_isolate { 1 } else { 2 }, Ordering::Release);
        }
    });
}

#[cfg(feature = "test-support")]
fn record_hmac_key_placement() {
    ASYNC_PROBE.with(|slot| {
        if let Some(probe) = slot.borrow().as_ref() {
            let on_isolate = std::thread::current().id() == probe.isolate_thread;
            probe
                .hmac_key_placement
                .store(if on_isolate { 1 } else { 2 }, Ordering::Release);
        }
    });
}

#[cfg(feature = "test-support")]
fn record_pbkdf2_placement() {
    ASYNC_PROBE.with(|slot| {
        if let Some(probe) = slot.borrow().as_ref() {
            let on_isolate = std::thread::current().id() == probe.isolate_thread;
            probe
                .pbkdf2_placement
                .store(if on_isolate { 1 } else { 2 }, Ordering::Release);
        }
    });
}

#[cfg(feature = "test-support")]
fn record_bcrypt_placement() {
    ASYNC_PROBE.with(|slot| {
        if let Some(probe) = slot.borrow().as_ref() {
            let on_isolate = std::thread::current().id() == probe.isolate_thread;
            probe
                .bcrypt_placement
                .store(if on_isolate { 1 } else { 2 }, Ordering::Release);
        }
    });
}

/// Record where the last Shamir split or combine ran.
#[cfg(feature = "test-support")]
pub(crate) fn record_shamir_placement() {
    ASYNC_PROBE.with(|slot| {
        if let Some(probe) = slot.borrow().as_ref() {
            let on_isolate = std::thread::current().id() == probe.isolate_thread;
            probe
                .shamir_placement
                .store(if on_isolate { 1 } else { 2 }, Ordering::Release);
        }
    });
}

/// Installs a probe for the current thread and restores the previous one.
#[cfg(feature = "test-support")]
struct ProbeGuard(Option<Arc<AsyncProbe>>);

#[cfg(feature = "test-support")]
impl ProbeGuard {
    fn install(probe: Option<Arc<AsyncProbe>>) -> Self {
        Self(install_probe(probe))
    }
}

#[cfg(feature = "test-support")]
impl Drop for ProbeGuard {
    fn drop(&mut self) {
        install_probe(self.0.take());
    }
}

/// Test-only gate that holds a real blocking hash worker after it starts.
///
/// The gate is installed into the runtime by [`with_cancellation_gate`] before
/// [`Runtime::new`](crate::Runtime::new). A worker that reaches the gate
/// publishes the runtime's budget and blocks until `release()`. A test can
/// inspect the live budget while the worker is held, drop the async join
/// future/runtime, inspect it again, then release the worker and confirm the
/// reservation is gone.
#[cfg(feature = "test-support")]
#[derive(Default)]
struct CancellationState {
    started: bool,
    started_count: usize,
    join_started: bool,
    join_started_count: usize,
    finished: bool,
    finished_count: usize,
    released: bool,
    budget: Option<Arc<Budget>>,
}

#[cfg(feature = "test-support")]
pub struct CancellationGate {
    state: Mutex<CancellationState>,
    release: Condvar,
    finished_cv: Condvar,
}

#[cfg(feature = "test-support")]
impl CancellationGate {
    /// Create an uninstalled gate.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(CancellationState::default()),
            release: Condvar::new(),
            finished_cv: Condvar::new(),
        })
    }

    fn enter_worker(&self, budget: Arc<Budget>) {
        {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            state.budget = Some(budget);
            state.started = true;
            state.started_count += 1;
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while !state.released {
            state = self.release.wait(state).unwrap_or_else(|p| p.into_inner());
        }
    }

    fn leave_worker(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.finished = true;
        state.finished_count += 1;
        self.finished_cv.notify_all();
    }

    /// Whether a worker has reached the gate.
    pub fn is_started(&self) -> bool {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).started
    }

    /// How many workers have reached the gate. The runtime's blocking pool has
    /// four threads, so a queued job is one that never increments this count.
    pub fn started_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .started_count
    }

    /// How many gated workers have left.
    pub fn finished_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .finished_count
    }

    /// Signal that the real join op has taken the handle and is about to await.
    fn join_started(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.join_started = true;
        state.join_started_count += 1;
    }

    /// Whether the real join op has taken the handle and is about to await.
    pub fn is_join_started(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .join_started
    }

    /// How many real join ops have taken their handles and are about to await.
    pub fn join_started_count(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .join_started_count
    }

    /// Live pending job and byte counters for the gated worker's runtime.
    pub fn budget(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let budget = state.budget.as_ref().expect("the worker has not started");
        let budget = budget.lock();
        (budget.pending_jobs, budget.pending_bytes)
    }

    /// Let the gated worker continue.
    pub fn release(&self) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.released = true;
        self.release.notify_all();
    }

    /// Wait until the gated worker has left, up to `timeout`.
    ///
    /// Returns `false` if the deadline expires first. The caller must still
    /// release the gate; the test's release guard does that on panic as well.
    pub fn wait_finished_timeout(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while !state.finished {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next, result) = self
                .finished_cv
                .wait_timeout(state, remaining)
                .unwrap_or_else(|p| p.into_inner());
            state = next;
            if result.timed_out() && !state.finished {
                return false;
            }
        }
        true
    }

    /// Wait until at least `count` gated workers have left, up to `timeout`.
    ///
    /// Returns `false` if the deadline expires first. The caller must still
    /// release the gate; the test's release guard does that on panic as well.
    pub fn wait_finished_count_timeout(&self, count: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while state.finished_count < count {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next, result) = self
                .finished_cv
                .wait_timeout(state, remaining)
                .unwrap_or_else(|p| p.into_inner());
            state = next;
            if result.timed_out() && state.finished_count < count {
                return false;
            }
        }
        true
    }
}

#[cfg(feature = "test-support")]
struct GateLeaveGuard(Arc<CancellationGate>);

#[cfg(feature = "test-support")]
impl Drop for GateLeaveGuard {
    fn drop(&mut self) {
        self.0.leave_worker();
    }
}

#[cfg(feature = "test-support")]
thread_local! {
    static CANCELLATION_GATE: RefCell<Option<Arc<CancellationGate>>> = const { RefCell::new(None) };
}

/// Install a cancellation gate for runtimes created by `f`.
///
/// The gate is read when [`CryptoExecutor::capture`] runs during
/// `Runtime::new`; the previous thread-local is restored when `f` returns.
#[cfg(feature = "test-support")]
pub fn with_cancellation_gate<R>(gate: Arc<CancellationGate>, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<Arc<CancellationGate>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CANCELLATION_GATE.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let previous = CANCELLATION_GATE.with(|slot| slot.replace(Some(gate)));
    let _restore = Restore(previous);
    f()
}

/// Read a JavaScript value that must already be a primitive string.
fn checked_string<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::String>, HashError> {
    if !value.is_string() {
        return Err(HashError::new(HashErrorCode::Type));
    }
    v8::Local::<v8::String>::try_from(value).map_err(|_| HashError::new(HashErrorCode::Type))
}

/// Convert a checked string to UTF-8 bytes, enforcing `limit`.
///
/// The UTF-8 length is read before allocating. Lone surrogates are replaced
/// with U+FFFD by V8's lossy conversion, matching the replacement behavior of
/// Node's UTF-8 string encoding.
fn string_bytes(
    scope: &v8::PinScope<'_, '_>,
    string: v8::Local<'_, v8::String>,
    limit: usize,
) -> Result<Vec<u8>, HashError> {
    let length = string.utf8_length(scope);
    if length > limit {
        return Err(HashError::new(HashErrorCode::Limit));
    }
    // Write directly into the single module-owned destination. The
    // `REPLACE_INVALID_UTF8` flag preserves JavaScript's U+FFFD replacement
    // for an unpaired surrogate, matching the previous lossy conversion
    // without an intermediate Rust `String`.
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| HashError::new(HashErrorCode::Limit))?;
    bytes.resize(length, 0);
    let written =
        string.write_utf8_v2(scope, &mut bytes, v8::WriteFlags::kReplaceInvalidUtf8, None);
    if written != length {
        return Err(HashError::new(HashErrorCode::Internal));
    }
    Ok(bytes)
}

/// Measure a hash input without copying it.
///
/// This is used to admit an asynchronous job to the byte budget before the
/// input is copied or a worker is started. No JavaScript runs between this
/// measurement and the later copy, so a resizable view cannot change in
/// between.
fn input_length(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    limit: usize,
) -> Result<usize, HashError> {
    if value.is_string() {
        let string = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| HashError::new(HashErrorCode::Type))?;
        let length = string.utf8_length(scope);
        if length > limit {
            return Err(HashError::new(HashErrorCode::Limit));
        }
        return Ok(length);
    }
    let view: View<'_> = checked_view(scope, value)?;
    if view.length > limit {
        return Err(HashError::new(HashErrorCode::Limit));
    }
    Ok(view.length)
}

/// Read a hash input: `Uint8Array` bytes or a UTF-8 string.
fn input_bytes(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    limit: usize,
) -> Result<Vec<u8>, HashError> {
    if value.is_string() {
        let string = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| HashError::new(HashErrorCode::Type))?;
        return string_bytes(scope, string, limit);
    }
    let view: View<'_> = checked_view(scope, value)?;
    if view.length > limit {
        return Err(HashError::new(HashErrorCode::Limit));
    }
    Ok(copy_view(&view)?)
}

/// Read an algorithm argument without invoking JavaScript conversion hooks.
fn algorithm_from(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Algorithm, HashError> {
    let string = checked_string(value)?;
    // The accepted names are at most six ASCII characters. Reject a longer
    // string before copying it, so an unrelated huge string cannot make this
    // path allocate it.
    if string.length() > 16 {
        return Err(HashError::new(HashErrorCode::Algorithm));
    }
    let name = string.to_rust_string_lossy(scope);
    Algorithm::parse(&name).ok_or_else(|| HashError::new(HashErrorCode::Algorithm))
}

/// Read an integer numeric argument in `0..=maximum`.
fn number_code(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    maximum: u32,
) -> Result<u32, HashError> {
    if !value.is_number() {
        return Err(HashError::new(HashErrorCode::Type));
    }
    let Some(number) = value.number_value(scope) else {
        return Err(HashError::new(HashErrorCode::Type));
    };
    if !number.is_finite() || number.trunc() != number || number < 0.0 || number > maximum as f64 {
        return Err(HashError::new(HashErrorCode::Range));
    }
    Ok(number as u32)
}

/// Read the strings-only input encoding code. Only 0 (UTF-8) is accepted.
fn input_encoding_code(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<u32, HashError> {
    if value.is_undefined() {
        return Ok(0);
    }
    let code = number_code(scope, value, 0)?;
    if code == 0 {
        Ok(code)
    } else {
        Err(HashError::new(HashErrorCode::Encoding))
    }
}

/// Encode a digest as the requested output.
///
/// `Latin1` returns the raw digest bytes; the JavaScript wrapper maps each byte
/// to one code unit. `Hex`, `Base64`, and `Base64Url` return ASCII bytes.
fn encode_digest(digest: &[u8], encoding: OutputEncoding) -> Vec<u8> {
    match encoding {
        OutputEncoding::Binary | OutputEncoding::Latin1 => digest.to_vec(),
        OutputEncoding::Hex => {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut text = Vec::with_capacity(digest.len() * 2);
            for byte in digest {
                text.push(HEX[(byte >> 4) as usize]);
                text.push(HEX[(byte & 0x0f) as usize]);
            }
            text
        }
        OutputEncoding::Base64 => base64_encode(digest, false),
        OutputEncoding::Base64Url => base64_encode(digest, true),
    }
}

/// Standard or URL-safe Base64. URL-safe output omits padding.
fn base64_encode(input: &[u8], url_safe: bool) -> Vec<u8> {
    const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let alphabet = if url_safe { URL_SAFE } else { STANDARD };
    let mut output = Vec::with_capacity(input.len().div_ceil(3) * 4);
    let (chunks, remainder) = input.as_chunks::<3>();
    for chunk in chunks {
        let value = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | chunk[2] as u32;
        output.push(alphabet[((value >> 18) & 63) as usize]);
        output.push(alphabet[((value >> 12) & 63) as usize]);
        output.push(alphabet[((value >> 6) & 63) as usize]);
        output.push(alphabet[(value & 63) as usize]);
    }
    match remainder {
        [first] => {
            let value = (*first as u32) << 16;
            output.push(alphabet[((value >> 18) & 63) as usize]);
            output.push(alphabet[((value >> 12) & 63) as usize]);
            if !url_safe {
                output.push(b'=');
                output.push(b'=');
            }
        }
        [first, second] => {
            let value = ((*first as u32) << 16) | ((*second as u32) << 8);
            output.push(alphabet[((value >> 18) & 63) as usize]);
            output.push(alphabet[((value >> 12) & 63) as usize]);
            output.push(alphabet[((value >> 6) & 63) as usize]);
            if !url_safe {
                output.push(b'=');
            }
        }
        [] => {}
        _ => unreachable!("a three-byte chunk has at most a two-byte remainder"),
    }
    output
}

/// Create the null-prototype result object and record a resource id or failure.
fn outcome_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<ResourceId, HashError>,
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
        Err(error) => set_failure_parts(
            scope,
            object,
            error.code().js_class(),
            error.code().as_str(),
            error.message(),
        ),
    }
    object.into()
}

/// Record success with no value, or the documented failure.
fn outcome_unit<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<(), HashError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(()) => set(scope, object, "ok", v8::Boolean::new(scope, true).into()),
        Err(error) => set_failure_parts(
            scope,
            object,
            error.code().js_class(),
            error.code().as_str(),
            error.message(),
        ),
    }
    object.into()
}

/// Record a digest as raw bytes or as a string, or the documented failure.
fn outcome_digest<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<(OutputEncoding, Vec<u8>), HashError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok((OutputEncoding::Binary, bytes)) => match bytes_to_v8(scope, bytes) {
            Some(value) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", value.into());
            }
            None => set_failure_parts(
                scope,
                object,
                "Error",
                Some("NEO_CRYPTO_LIMIT"),
                HashError::new(HashErrorCode::Limit).message(),
            ),
        },
        Ok((_, bytes)) => {
            match v8::String::new_from_one_byte(scope, &bytes, v8::NewStringType::Normal) {
                Some(value) => {
                    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                    set(scope, object, "value", value.into());
                }
                None => set_failure_parts(
                    scope,
                    object,
                    "Error",
                    Some("NEO_CRYPTO_LIMIT"),
                    HashError::new(HashErrorCode::Limit).message(),
                ),
            }
        }
        Err(error) => set_failure_parts(
            scope,
            object,
            error.code().js_class(),
            error.code().as_str(),
            error.message(),
        ),
    }
    object.into()
}

#[op2]
fn op_neo_crypto_hash_create<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    algorithm: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let budget = state.borrow::<CryptoExecutor>().budget();
        let open = budget.reserve_open()?;
        let algorithm = algorithm_from(scope, algorithm)?;
        Ok(state
            .resource_table
            .add(DigestResource::new(DigestState::hash(algorithm), open)))
    })();
    outcome_id(scope, result)
}

/// Drop one incremental hash object's state explicitly.
///
/// The JavaScript wrapper calls this from `close()`/`dispose()`. The open-object
/// reservation is released when the resource is removed, whether the object is
/// closed, digested, or dropped with the runtime.
#[op2(fast)]
fn op_neo_crypto_hash_release(state: &mut OpState, #[smi] id: ResourceId) {
    let _ = state.resource_table.take::<DigestResource>(id);
}

#[op2]
fn op_neo_crypto_hmac_create<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    algorithm: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let budget = state.borrow::<CryptoExecutor>().budget();
        let open = budget.reserve_open()?;
        let algorithm = algorithm_from(scope, algorithm)?;
        let key = input_bytes(scope, key, MAX_HMAC_KEY_BYTES)?;
        let digest = DigestState::hmac(algorithm, &key)?;
        Ok(state.resource_table.add(DigestResource::new(digest, open)))
    })();
    outcome_id(scope, result)
}

#[op2]
fn op_neo_crypto_hash_update<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    data: v8::Local<'s, v8::Value>,
    encoding: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let resource = state
            .resource_table
            .get::<DigestResource>(id)
            .map_err(|_| HashError::new(HashErrorCode::Digested))?;
        let mut guard = resource.state.borrow_mut();
        let Some(digest) = guard.as_mut() else {
            return Err(HashError::new(HashErrorCode::Digested));
        };
        let bytes = if data.is_string() {
            let code = input_encoding_code(scope, encoding)?;
            if code != 0 {
                return Err(HashError::new(HashErrorCode::Encoding));
            }
            let string = v8::Local::<v8::String>::try_from(data)
                .map_err(|_| HashError::new(HashErrorCode::Type))?;
            string_bytes(scope, string, digest.remaining())?
        } else {
            if !encoding.is_undefined() {
                return Err(HashError::new(HashErrorCode::Type));
            }
            let view: View<'_> = checked_view(scope, data)?;
            if view.length > digest.remaining() {
                return Err(HashError::new(HashErrorCode::Limit));
            }
            copy_view(&view)?
        };
        digest.update(&bytes)
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_neo_crypto_hash_digest<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    encoding: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let code = number_code(scope, encoding, 4)?;
        let encoding =
            OutputEncoding::from_code(code).ok_or_else(|| HashError::new(HashErrorCode::Range))?;
        let resource = state
            .resource_table
            .get::<DigestResource>(id)
            .map_err(|_| HashError::new(HashErrorCode::Digested))?;
        let digest = resource
            .state
            .borrow_mut()
            .take()
            .ok_or_else(|| HashError::new(HashErrorCode::Digested))?;
        // Drop the table's reference now that the state is consumed. The
        // JavaScript object cannot be used again, and the resource is freed
        // without waiting for the runtime to be dropped.
        let _ = state.resource_table.take::<DigestResource>(id);
        let bytes = encode_digest(&digest.finalize(), encoding);
        Ok((encoding, bytes))
    })();
    outcome_digest(scope, result)
}

/// Validate and copy an asynchronous call, then submit it to the blocking pool.
///
/// All user-caused failures are reported here, before a worker starts, so they
/// keep their stable `code`. The join op only reports an internal worker
/// failure, as an empty result.
#[allow(clippy::too_many_arguments)]
#[op2]
fn op_neo_crypto_async_submit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    algorithm: v8::Local<'s, v8::Value>,
    kind: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
    encoding: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        // deno_core spawns the async-op driver with `deno_unsync::spawn`, which
        // requires a Tokio current-thread runtime on the isolate thread. Without
        // one it panics, and a multi-thread runtime could poll a non-`Send` op
        // future on another thread. Reject before the input is copied or a
        // pending op is created, so the failure is a clean stable error.
        if !matches!(
            Handle::try_current().map(|handle| handle.runtime_flavor()),
            Ok(RuntimeFlavor::CurrentThread)
        ) {
            return Err(HashError::new(HashErrorCode::NoExecutor));
        }
        let algorithm = algorithm_from(scope, algorithm)?;
        let kind = number_code(scope, kind, 1)?;
        let encoding_code = number_code(scope, encoding, 4)?;
        let encoding = OutputEncoding::from_code(encoding_code)
            .ok_or_else(|| HashError::new(HashErrorCode::Range))?;
        let budget = {
            let executor = state.borrow::<CryptoExecutor>();
            if executor.handle.is_none() {
                return Err(HashError::new(HashErrorCode::Internal));
            }
            executor.budget()
        };
        // The isolate-thread probe is installed before any possible mutation
        // that moves key construction or `update` back onto this thread.
        #[cfg(feature = "test-support")]
        let _isolate_probe = ProbeGuard::install(Some(state.borrow::<CryptoExecutor>().probe()));

        // Measure before copying, admit the job to both budgets, then copy.
        // The worker owns the data and, for HMAC, the key until it has built
        // the keyed state, so both lengths count against the byte budget.
        let data_length = input_length(scope, data, MAX_HASH_INPUT_BYTES)?;
        let key_length = if kind == 1 {
            input_length(scope, key, MAX_HMAC_KEY_BYTES)?
        } else {
            0
        };
        let pending_bytes = data_length
            .checked_add(key_length)
            .ok_or_else(|| HashError::new(HashErrorCode::Busy))?;
        let reservation = budget.reserve_pending(pending_bytes)?;
        let data = input_bytes(scope, data, MAX_HASH_INPUT_BYTES)?;
        let key = if kind == 1 {
            input_bytes(scope, key, MAX_HMAC_KEY_BYTES)?
        } else {
            Vec::new()
        };
        debug_assert_eq!(data.len(), data_length);
        debug_assert_eq!(key.len(), key_length);

        // No hashing happens here. The worker owns the data, key, and
        // reservation in that field order, so the reservation outlives both
        // allocations on every exit path.
        let job = WorkerOwned {
            data,
            key,
            _reservation: reservation,
        };
        let join = state.borrow::<CryptoExecutor>().spawn_blocking(move || {
            // The job is declared after the worker's probe and gate guards so
            // it drops first on unwind; the explicit drop below also runs
            // before the gate reports finished.
            let job = job;
            let result = job.run(algorithm, kind, encoding);
            // Drop the owned allocations and then the reservation before the
            // result leaves the worker.
            drop(job);
            result
        });
        Ok(state.resource_table.add(AsyncDigestJob {
            handle: RefCell::new(Some(join)),
        }))
    })();
    outcome_id(scope, result)
}

/// Wait for one asynchronous hash. An empty result means the worker failed.
#[op2]
#[buffer]
pub(crate) async fn op_neo_crypto_async_join(
    state: Rc<RefCell<OpState>>,
    #[smi] id: ResourceId,
) -> Vec<u8> {
    let handle = {
        let mut state = state.borrow_mut();
        let Ok(job) = state.resource_table.take::<AsyncDigestJob>(id) else {
            return Vec::new();
        };
        job.handle.borrow_mut().take()
    };
    let Some(handle) = handle else {
        return Vec::new();
    };
    #[cfg(feature = "test-support")]
    if let Some(gate) = state
        .borrow()
        .borrow::<CryptoExecutor>()
        .cancellation_gate
        .clone()
    {
        gate.join_started();
    }
    // If this Rust join-op future is dropped with the runtime, abort a job that
    // has not started. Abandoning the JavaScript Promise does not drop this
    // future; the wrapper starts the join immediately. Dropping only the entry
    // future leaves the runtime's op futures alive. A running `spawn_blocking`
    // task cannot be stopped and keeps its reservation until it finishes.
    let _abort = AbortOnDrop(handle.abort_handle());
    handle.await.unwrap_or_default()
}

deno_core::extension!(
    neojs_crypto_hash,
    ops = [
        op_neo_crypto_hash_create,
        op_neo_crypto_hmac_create,
        op_neo_crypto_hash_release,
        op_neo_crypto_hash_update,
        op_neo_crypto_hash_digest,
        op_neo_crypto_async_submit,
        op_neo_crypto_async_join,
        password::op_neo_crypto_pbkdf2,
        password::op_neo_crypto_pbkdf2_async_submit,
        password::op_neo_crypto_password_async_join,
        password::op_neo_crypto_bcrypt_hash,
        password::op_neo_crypto_bcrypt_hash_async_submit,
        password::op_neo_crypto_bcrypt_verify,
        password::op_neo_crypto_bcrypt_verify_async_submit
    ],
    state = |state| {
        state.put(CryptoExecutor::capture());
    },
    docs = "SHA-2, HMAC, PBKDF2, and bcrypt ops behind `Neo.getBuiltInModule(\"crypto\")`."
);

/// The off-by-default test-support extension, used to prove that asynchronous
/// hashing runs on a blocking worker and not on the isolate thread.
#[cfg(feature = "test-support")]
#[op2]
#[string]
fn op_neo_crypto_test_worker_placement(state: &mut OpState) -> String {
    let executor = state.borrow::<CryptoExecutor>();
    format!(
        "{},{},{}",
        executor.worker_placement.load(Ordering::Acquire),
        executor.update_placement.load(Ordering::Acquire),
        executor.hmac_key_placement.load(Ordering::Acquire),
    )
}

#[cfg(feature = "test-support")]
#[op2]
#[string]
fn op_neo_crypto_test_shamir_placement(state: &mut OpState) -> String {
    let executor = state.borrow::<CryptoExecutor>();
    format!(
        "{},{}",
        executor.worker_placement.load(Ordering::Acquire),
        executor.shamir_placement.load(Ordering::Acquire),
    )
}

#[cfg(feature = "test-support")]
#[op2]
#[string]
fn op_neo_crypto_test_password_placement(state: &mut OpState) -> String {
    let executor = state.borrow::<CryptoExecutor>();
    format!(
        "{},{},{}",
        executor.worker_placement.load(Ordering::Acquire),
        executor.pbkdf2_placement.load(Ordering::Acquire),
        executor.bcrypt_placement.load(Ordering::Acquire),
    )
}

#[cfg(feature = "test-support")]
deno_core::extension!(
    neojs_crypto_hash_test,
    ops = [
        op_neo_crypto_test_worker_placement,
        op_neo_crypto_test_password_placement,
        op_neo_crypto_test_shamir_placement
    ],
    docs = "Test-only worker-placement probes for `neo:crypto`."
);

/// Return the test-support extension. Compiled only with the off-by-default
/// `test-support` feature, which a normal build never enables.
#[cfg(feature = "test-support")]
pub(crate) fn test_support_extension() -> deno_core::Extension {
    neojs_crypto_hash_test::init()
}

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
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// FIPS 180-4 / NIST CAVP vectors, independently reproduced with Python's
    /// `hashlib` (OpenSSL backend) for this repository.
    const HASH_VECTORS: [(&str, Algorithm, &str); 9] = [
        (
            "",
            Algorithm::Sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "abc",
            Algorithm::Sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            Algorithm::Sha256,
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
        (
            "abc",
            Algorithm::Sha384,
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7",
        ),
        (
            "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            Algorithm::Sha384,
            "09330c33f71147e83d192fc782cd1b4753111b173b3b05d22fa08086e3b0f712fcc7c71a557e2db966c3e9fa91746039",
        ),
        (
            "",
            Algorithm::Sha384,
            "38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b",
        ),
        (
            "abc",
            Algorithm::Sha512,
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        ),
        (
            "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu",
            Algorithm::Sha512,
            "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909",
        ),
        (
            "",
            Algorithm::Sha512,
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
        ),
    ];

    #[test]
    fn published_hash_vectors_match_incremental_and_one_shot() {
        for (message, algorithm, expected) in HASH_VECTORS {
            let mut state = DigestState::hash(algorithm);
            // Feed the vector in uneven chunks so buffering is exercised.
            let bytes = message.as_bytes();
            let split = bytes.len() / 2;
            state.update(&bytes[..split]).unwrap();
            state.update(&bytes[split..]).unwrap();
            assert_eq!(encode_hex(&state.finalize()), expected, "{message:?}");
        }
    }

    #[test]
    fn algorithm_names_are_small_and_case_insensitive() {
        assert_eq!(Algorithm::parse("sha256"), Some(Algorithm::Sha256));
        assert_eq!(Algorithm::parse("SHA384"), Some(Algorithm::Sha384));
        assert_eq!(Algorithm::parse("Sha512"), Some(Algorithm::Sha512));
        for name in [
            "", "sha1", "sha224", "sha2-256", "sha-256", "md5", "SHA3-256",
        ] {
            assert_eq!(Algorithm::parse(name), None, "{name:?}");
        }
    }

    /// RFC 4231 test cases 1-3, independently reproduced with Python's `hmac`
    /// module for this repository.
    #[test]
    fn published_hmac_vectors_match() {
        let cases: [(Vec<u8>, &[u8], Algorithm, &str); 9] = [
            (
                vec![0x0b; 20],
                b"Hi There",
                Algorithm::Sha256,
                "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?",
                Algorithm::Sha256,
                "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
            ),
            (
                vec![0xaa; 20],
                &[0xdd; 50],
                Algorithm::Sha256,
                "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe",
            ),
            (
                vec![0x0b; 20],
                b"Hi There",
                Algorithm::Sha384,
                "afd03944d84895626b0825f4ab46907f15f9dadbe4101ec682aa034c7cebc59cfaea9ea9076ede7f4af152e8b2fa9cb6",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?",
                Algorithm::Sha384,
                "af45d2e376484031617f78d2b58a6b1b9c7ef464f5a01b47e42ec3736322445e8e2240ca5e69e2c78b3239ecfab21649",
            ),
            (
                vec![0xaa; 20],
                &[0xdd; 50],
                Algorithm::Sha384,
                "88062608d3e6ad8a0aa2ace014c8a86f0aa635d947ac9febe83ef4e55966144b2a5ab39dc13814b94e3ab6e101a34f27",
            ),
            (
                vec![0x0b; 20],
                b"Hi There",
                Algorithm::Sha512,
                "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854",
            ),
            (
                b"Jefe".to_vec(),
                b"what do ya want for nothing?",
                Algorithm::Sha512,
                "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737",
            ),
            (
                vec![0xaa; 20],
                &[0xdd; 50],
                Algorithm::Sha512,
                "fa73b0089d56a284efb0f0756c890be9b1b5dbdd8ee81a3655f83e33b2279d39bf3e848279a722c806b485a47e67c807b946a337bee8942674278859e13292fb",
            ),
        ];
        for (key, data, algorithm, expected) in cases {
            let mut state = DigestState::hmac(algorithm, &key).unwrap();
            state.update(data).unwrap();
            assert_eq!(encode_hex(&state.finalize()), expected);
        }
    }

    #[test]
    fn empty_hmac_key_is_accepted() {
        let mut state = DigestState::hmac(Algorithm::Sha256, b"").unwrap();
        state.update(b"").unwrap();
        assert_eq!(state.finalize().len(), 32);
    }

    #[test]
    fn digest_encodings_are_stable() {
        let bytes = decode_hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            encode_digest(&bytes, OutputEncoding::Hex),
            b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            std::str::from_utf8(&encode_digest(&bytes, OutputEncoding::Base64)).unwrap(),
            "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0="
        );
        assert_eq!(
            std::str::from_utf8(&encode_digest(&bytes, OutputEncoding::Base64Url)).unwrap(),
            "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"
        );
        assert_eq!(
            encode_digest(&bytes, OutputEncoding::Binary),
            bytes,
            "binary is the raw digest"
        );
        assert_eq!(
            encode_digest(&bytes, OutputEncoding::Latin1),
            bytes,
            "latin1 bytes are the raw digest"
        );
    }

    #[test]
    fn base64_padding_is_correct_for_every_remainder() {
        for (input, standard, url_safe) in [
            (&b""[..], "", ""),
            (b"f", "Zg==", "Zg"),
            (b"fo", "Zm8=", "Zm8"),
            (b"foo", "Zm9v", "Zm9v"),
            (b"foob", "Zm9vYg==", "Zm9vYg"),
            (b"fooba", "Zm9vYmE=", "Zm9vYmE"),
            (b"foobar", "Zm9vYmFy", "Zm9vYmFy"),
        ] {
            assert_eq!(
                std::str::from_utf8(&base64_encode(input, false)).unwrap(),
                standard
            );
            assert_eq!(
                std::str::from_utf8(&base64_encode(input, true)).unwrap(),
                url_safe
            );
        }
    }

    #[test]
    fn aggregate_input_bound_is_enforced() {
        let mut state = DigestState::hash(Algorithm::Sha256);
        state.update(&vec![0u8; MAX_HASH_INPUT_BYTES]).unwrap();
        assert_eq!(state.remaining(), 0);
        assert_eq!(
            state.update(&[0u8]).unwrap_err().code(),
            HashErrorCode::Limit
        );
    }

    #[test]
    fn hmac_key_bound_is_enforced() {
        let key = vec![0u8; MAX_HMAC_KEY_BYTES + 1];
        let error = match DigestState::hmac(Algorithm::Sha256, &key) {
            Ok(_) => panic!("an oversized key must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), HashErrorCode::Limit);
        assert!(DigestState::hmac(Algorithm::Sha256, &key[..MAX_HMAC_KEY_BYTES]).is_ok());
    }

    #[test]
    fn pending_budget_counts_bytes_and_jobs_and_releases() {
        let budget = Arc::new(Budget::default());
        let chunk = MAX_HASH_INPUT_BYTES;
        let first = budget.reserve_pending(chunk).unwrap();
        let second = budget.reserve_pending(chunk).unwrap();
        let third = budget.reserve_pending(chunk).unwrap();
        let fourth = budget.reserve_pending(chunk).unwrap();
        let error = match budget.reserve_pending(1) {
            Ok(_) => panic!("an over-limit reservation must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), HashErrorCode::Busy);
        {
            let state = budget.lock();
            assert_eq!(state.pending_jobs, 4);
            assert_eq!(state.pending_bytes, MAX_PENDING_ASYNC_BYTES);
        }
        drop(fourth);
        let fifth = budget.reserve_pending(chunk).unwrap();
        drop(first);
        drop(second);
        drop(third);
        drop(fifth);
        let state = budget.lock();
        assert_eq!(state.pending_jobs, 0);
        assert_eq!(state.pending_bytes, 0);
    }

    #[test]
    fn pending_job_cap_applies_to_empty_jobs() {
        let budget = Arc::new(Budget::default());
        let mut reservations = Vec::new();
        for _ in 0..MAX_PENDING_ASYNC_JOBS {
            reservations.push(budget.reserve_pending(0).unwrap());
        }
        let error = match budget.reserve_pending(0) {
            Ok(_) => panic!("an over-cap empty job must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), HashErrorCode::Busy);
        assert_eq!(budget.lock().pending_jobs, MAX_PENDING_ASYNC_JOBS);
        assert_eq!(budget.lock().pending_bytes, 0);
        drop(reservations.pop());
        assert!(budget.reserve_pending(0).is_ok());
        drop(reservations);
        let state = budget.lock();
        assert_eq!(state.pending_jobs, 0);
        assert_eq!(state.pending_bytes, 0);
    }

    #[test]
    fn pending_reservation_releases_on_error_and_panic() {
        let budget = Arc::new(Budget::default());
        let error = match budget.reserve_pending(MAX_PENDING_ASYNC_BYTES + 1) {
            Ok(_) => panic!("an over-limit reservation must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), HashErrorCode::Busy);
        {
            let state = budget.lock();
            assert_eq!(state.pending_jobs, 0);
            assert_eq!(state.pending_bytes, 0);
        }

        // Dropping a live reservation simulates cancellation before the worker
        // consumes it.
        {
            let _reservation = budget.reserve_pending(4).unwrap();
            assert_eq!(budget.lock().pending_jobs, 1);
        }
        {
            let state = budget.lock();
            assert_eq!(state.pending_jobs, 0);
            assert_eq!(state.pending_bytes, 0);
        }

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let budget = Arc::clone(&budget);
            move || {
                let _reservation = budget.reserve_pending(8).unwrap();
                panic!("simulated worker panic");
            }
        }));
        assert!(unwind.is_err());
        let state = budget.lock();
        assert_eq!(state.pending_jobs, 0);
        assert_eq!(state.pending_bytes, 0);
    }

    #[test]
    fn open_object_cap_is_enforced_and_releasable() {
        let budget = Arc::new(Budget::default());
        let mut open = Vec::new();
        for _ in 0..MAX_OPEN_DIGESTS {
            open.push(budget.reserve_open().unwrap());
        }
        let error = match budget.reserve_open() {
            Ok(_) => panic!("an over-cap open object must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), HashErrorCode::TooManyObjects);
        assert_eq!(budget.lock().open_digests, MAX_OPEN_DIGESTS);
        drop(open.pop());
        let replacement = budget.reserve_open().unwrap();
        drop(open);
        drop(replacement);
        assert_eq!(budget.lock().open_digests, 0);
    }

    #[test]
    fn worker_owned_holds_reservation_until_it_drops() {
        let budget = Arc::new(Budget::default());
        let reservation = budget.reserve_pending(8).unwrap();
        let job = WorkerOwned {
            data: vec![0u8; 8],
            key: Vec::new(),
            _reservation: reservation,
        };
        assert_eq!(budget.lock().pending_jobs, 1);
        assert_eq!(budget.lock().pending_bytes, 8);
        let result = job.run(Algorithm::Sha256, 0, OutputEncoding::Hex);
        assert_eq!(result.len(), 64);
        // `run` borrows the job; the reservation is still held after it returns.
        assert_eq!(budget.lock().pending_jobs, 1);
        assert_eq!(budget.lock().pending_bytes, 8);
        drop(job);
        let state = budget.lock();
        assert_eq!(state.pending_jobs, 0);
        assert_eq!(state.pending_bytes, 0);
        drop(state);

        // A job dropped before it starts (cancellation) releases its
        // reservation through the same field-order drop path.
        {
            let reservation = budget.reserve_pending(12).unwrap();
            let _job = WorkerOwned {
                data: vec![1u8; 8],
                key: vec![2u8; 4],
                _reservation: reservation,
            };
            assert_eq!(budget.lock().pending_jobs, 1);
        }
        let state = budget.lock();
        assert_eq!(state.pending_jobs, 0);
        assert_eq!(state.pending_bytes, 0);
    }

    #[test]
    fn errors_have_stable_codes_and_no_input_bytes() {
        assert_eq!(
            HashError::new(HashErrorCode::Algorithm).code().as_str(),
            Some("NEO_CRYPTO_ALGORITHM")
        );
        assert_eq!(
            HashError::new(HashErrorCode::Encoding).code().as_str(),
            Some("NEO_CRYPTO_ENCODING")
        );
        assert_eq!(
            HashError::new(HashErrorCode::Digested).code().as_str(),
            Some("NEO_CRYPTO_DIGESTED")
        );
        assert_eq!(
            HashError::new(HashErrorCode::Busy).code().as_str(),
            Some("NEO_CRYPTO_BUSY")
        );
        assert_eq!(
            HashError::new(HashErrorCode::NoExecutor).code().as_str(),
            Some("NEO_CRYPTO_NO_EXECUTOR")
        );
        assert_eq!(
            HashError::new(HashErrorCode::TooManyObjects)
                .code()
                .as_str(),
            Some("NEO_CRYPTO_TOO_MANY_OBJECTS")
        );
        assert_eq!(
            HashError::new(HashErrorCode::Internal).code().as_str(),
            Some("NEO_CRYPTO_INTERNAL")
        );
        assert_eq!(HashError::new(HashErrorCode::Type).code().as_str(), None);
        assert_eq!(HashError::new(HashErrorCode::Range).code().as_str(), None);
        assert_eq!(
            HashError::new(HashErrorCode::Type).code().js_class(),
            "TypeError"
        );
        assert_eq!(
            HashError::new(HashErrorCode::Range).code().js_class(),
            "RangeError"
        );
        for code in [
            HashErrorCode::Algorithm,
            HashErrorCode::Encoding,
            HashErrorCode::Limit,
            HashErrorCode::Digested,
            HashErrorCode::Busy,
            HashErrorCode::NoExecutor,
            HashErrorCode::TooManyObjects,
            HashErrorCode::Detached,
            HashErrorCode::Internal,
            HashErrorCode::Type,
            HashErrorCode::Range,
        ] {
            let message = HashError::new(code).message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
        }
    }
}

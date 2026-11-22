//! PBKDF2 and bcrypt password hashing for `neo:crypto`.
//!
//! This is a child of [`crate::crypto_hash`], so it shares that module's
//! bounded executor, pending-job budget, cancellation gate, placement probes,
//! and V8 argument readers. The primitives are the RustCrypto `pbkdf2` crate
//! and the `bcrypt` crate; the wire formats, the accepted algorithm names, the
//! work-factor and input bounds, and the JavaScript error mapping are defined
//! here.
//!
//! # PBKDF2
//!
//! `pbkdf2(algorithm, password, salt, iterations, length)` is synchronous and
//! returns `length` raw bytes. The accepted names are `sha256`, `sha384`, and
//! `sha512`, compared case-insensitively. `password` and `salt` are exact
//! `Uint8Array` bytes or strings encoded as UTF-8. `iterations` is an integer
//! from 1 through [`MAX_PBKDF2_ITERATIONS`]; `length` is an integer from 1
//! through [`MAX_PBKDF2_OUTPUT_BYTES`]. One password or salt is at most
//! [`MAX_PBKDF2_INPUT_BYTES`] bytes. A string is measured by its UTF-8 length
//! before it is copied.
//!
//! `pbkdf2Async` has the same arguments and returns a `Promise` that resolves
//! to the derived bytes. Validation, budget admission, and one bounded copy
//! into a module-owned zeroizing buffer run on the isolate thread; the
//! derivation runs on the shared blocking pool. It is not a resolved-`Promise`
//! wrapper around a blocking call.
//!
//! # bcrypt
//!
//! `bcryptHash(password[, cost])` returns a 60-character modular-crypt string
//! in the `$2b$` format. `password` is exact bytes or a UTF-8 string. At most
//! [`MAX_BCRYPT_PASSWORD_BYTES`] bytes are accepted; a longer input is
//! rejected with `NEO_CRYPTO_LIMIT`. Exactly 72 bytes are accepted and hashed
//! as 72 bytes with no added NUL. `cost` is an integer from
//! [`MIN_BCRYPT_COST`] through [`MAX_BCRYPT_COST`], defaulting to
//! [`DEFAULT_BCRYPT_COST`]. The salt is 16 bytes from the operating system
//! CSPRNG; an entropy failure is `NEO_CRYPTO_RANDOM` and never falls back to
//! predictable bytes.
//!
//! `bcryptVerify(password, hash)` accepts `$2a$`, `$2b$`, and `$2y$` hashes
//! only. It returns `false` for a malformed string, an unsupported version
//! (`$2x$` included), a cost outside 4 through 16, or a password longer than
//! 72 bytes. It does not throw for those cases and does not compute a hash for
//! an over-cost or malformed input, so a malicious hash cannot force an
//! unbounded work factor. A password longer than 72 bytes is also `false`, not
//! an error. The comparison inside the bcrypt crate is constant-time.
//!
//! `bcryptHashAsync` and `bcryptVerifyAsync` are the asynchronous forms. A
//! well-formed verification runs on the shared blocking pool. An oversized
//! password, a malformed hash, or an over-cost hash resolves to `false`
//! without starting a worker, because there is no expensive work to do. An
//! invalid argument type still rejects.
//!
//! # Executor and limits
//!
//! PBKDF2 and bcrypt jobs share the same per-runtime budget as `hashAsync` and
//! `hmacAsync`: at most [`MAX_PENDING_ASYNC_JOBS`] jobs and
//! [`MAX_PENDING_ASYNC_BYTES`] of copied input may be pending. A submission
//! over either bound rejects with `NEO_CRYPTO_BUSY`. The reservation is
//! released when the worker finishes, panics, or is aborted before it starts;
//! an already-running job stays charged until it returns. As with hashing,
//! an asynchronous call needs a Tokio current-thread runtime on the isolate
//! thread to host deno_core's async-op driver; any other executor rejects with
//! `NEO_CRYPTO_NO_EXECUTOR` before copying input or starting a worker.
//!
//! Cancellation is best effort and is not triggered by abandoning the returned
//! JavaScript `Promise`. The wrapper starts the join op immediately, so an
//! unawaited promise still drives the job to completion; its result is retained
//! by the promise until it is collected. Dropping the runtime drops the
//! `JsRuntime`, its pending join-op futures, and unjoined job resources; that
//! aborts a job that has not started on a blocking thread and releases its
//! reservation. Dropping only the `run_file_async` entry future stops polling
//! but does not drop those op futures or resources, so it is not cancellation. A
//! job that has already started cannot be stopped: abort is a no-op, its
//! zeroizing password/salt copies and reservation stay owned until it returns,
//! and its result is discarded. A worker's stable failure category
//! (random-source, limit, or internal) is carried through the join buffer rather
//! than flattened into the empty internal sentinel.
//!
//! # Zeroization
//!
//! Password and salt destination buffers owned by this module are wrapped in
//! `zeroize::Zeroizing` and cleared when the operation finishes. A string is
//! converted directly into that destination with V8's UTF-8 writer, using
//! U+FFFD for an unpaired surrogate, so this module creates no intermediate
//! Rust string. V8's internal string storage and flattening, conversion storage
//! inside V8 or its dependencies, the `bcrypt` crate's internals, PBKDF2/HMAC
//! state, and the JavaScript heap copies are outside this guarantee. This is
//! best-effort clearing, not a promise of complete memory erasure. No error
//! message contains a password, salt, derived key, bcrypt hash, or other input
//! bytes.
//!
//! # Scope
//!
//! This is not Node's `crypto`, its `pbkdf2`/`pbkdf2Sync`, or its `bcrypt`
//! surface. There is no callback form and no PHC, MCF, or `crypt(3)` parser.
//! PBKDF2 supports only the three SHA-2 names above; SHA-1 is not accepted.

use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;
#[cfg(feature = "test-support")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "test-support")]
use std::sync::atomic::Ordering;

use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_core::v8;
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use sha2::Sha384;
use sha2::Sha512;
use tokio::runtime::Handle;
use tokio::runtime::RuntimeFlavor;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

use super::AbortOnDrop;
use super::Algorithm;
use super::CryptoExecutor;
use super::HashError;
use super::HashErrorCode;
use super::PendingReservation;
#[cfg(feature = "test-support")]
use super::ProbeGuard;
use super::algorithm_from;
use super::checked_string;
use super::input_length;
use super::number_code;
use crate::crypto::CryptoError;
use crate::crypto::CryptoErrorCode;
use crate::crypto::bytes_to_v8;
use crate::crypto::checked_view;
use crate::crypto::response;
use crate::crypto::set;
use crate::crypto::set_failure_parts;

/// Largest PBKDF2 iteration count accepted in one call. This is a neojs bound,
/// not a PBKDF2 limit.
pub(crate) const MAX_PBKDF2_ITERATIONS: u32 = 1_000_000;

/// Largest PBKDF2 output length, in bytes.
pub(crate) const MAX_PBKDF2_OUTPUT_BYTES: usize = 1024;

/// Largest PBKDF2 password or salt, in bytes.
pub(crate) const MAX_PBKDF2_INPUT_BYTES: usize = 1024 * 1024;

/// Smallest bcrypt cost accepted when hashing or verifying.
pub(crate) const MIN_BCRYPT_COST: u32 = 4;

/// Largest bcrypt cost accepted when hashing or verifying.
///
/// Verification never computes a hash whose encoded cost is above this bound,
/// so a malicious modular-crypt string cannot force an unbounded work factor.
pub(crate) const MAX_BCRYPT_COST: u32 = 16;

/// Default bcrypt cost for `bcryptHash`.
pub(crate) const DEFAULT_BCRYPT_COST: u32 = 10;

/// Largest password accepted by bcrypt, in bytes. Exactly 72 bytes is allowed;
/// 73 or more is rejected by hashing and returns `false` from verification.
pub(crate) const MAX_BCRYPT_PASSWORD_BYTES: usize = 72;

/// A complete bcrypt modular-crypt string is always 60 ASCII bytes.
const BCRYPT_HASH_BYTES: usize = 60;

/// Test-only, deterministically consumed injection for the next bcrypt salt
/// draw. It is compiled only with the off-by-default `test-support` feature, so
/// a normal build has no production hook. The async worker consumes it from a
/// blocking thread, which is why it is an atomic rather than thread-local.
#[cfg(feature = "test-support")]
static FAIL_NEXT_BCRYPT_RANDOM: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "test-support")]
pub(super) fn fail_next_bcrypt_random() {
    FAIL_NEXT_BCRYPT_RANDOM.store(true, Ordering::SeqCst);
}

#[cfg(feature = "test-support")]
fn take_bcrypt_random_failure() -> bool {
    FAIL_NEXT_BCRYPT_RANDOM.swap(false, Ordering::SeqCst)
}

/// A supported PBKDF2 PRF. Only the SHA-2 names shared with hashing are
/// accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Prf {
    Sha256,
    Sha384,
    Sha512,
}

impl Prf {
    fn from_algorithm(algorithm: Algorithm) -> Self {
        match algorithm {
            Algorithm::Sha256 => Self::Sha256,
            Algorithm::Sha384 => Self::Sha384,
            Algorithm::Sha512 => Self::Sha512,
        }
    }
}

/// Stable machine-readable password-hashing failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PasswordErrorCode {
    /// The algorithm name is not `sha256`, `sha384`, or `sha512`.
    Algorithm,
    /// An input or allocation exceeded a documented bound.
    Limit,
    /// Too much asynchronous crypto work is already pending.
    Busy,
    /// The event loop cannot host deno_core async ops.
    NoExecutor,
    /// The operating system CSPRNG failed.
    Random,
    /// The view's buffer was detached.
    Detached,
    /// A worker or the runtime executor failed.
    Internal,
    /// An argument is neither a `Uint8Array` nor a string.
    Type,
    /// A numeric argument is not an integer in its documented range.
    Range,
}

impl PasswordErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    ///
    /// Argument errors ([`Self::Type`] and [`Self::Range`]) have no code and
    /// become `TypeError` or `RangeError`.
    fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Algorithm => "NEO_CRYPTO_ALGORITHM",
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::Busy => "NEO_CRYPTO_BUSY",
            Self::NoExecutor => "NEO_CRYPTO_NO_EXECUTOR",
            Self::Random => "NEO_CRYPTO_RANDOM",
            Self::Detached => "NEO_CRYPTO_DETACHED",
            Self::Internal => "NEO_CRYPTO_INTERNAL",
            Self::Type | Self::Range => return None,
        })
    }

    fn js_class(self) -> &'static str {
        match self {
            Self::Type => "TypeError",
            Self::Range => "RangeError",
            _ => "Error",
        }
    }
}

/// A failure safe to describe without echoing a password, salt, derived key, or
/// bcrypt hash.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PasswordError {
    code: PasswordErrorCode,
}

impl PasswordError {
    fn new(code: PasswordErrorCode) -> Self {
        Self { code }
    }

    fn code(self) -> PasswordErrorCode {
        self.code
    }

    /// A fixed message. It never contains input or output bytes.
    fn message(self) -> &'static str {
        match self.code {
            PasswordErrorCode::Algorithm => "unsupported password hashing algorithm",
            PasswordErrorCode::Limit => "input exceeds the password hashing size limit",
            PasswordErrorCode::Busy => "too much asynchronous crypto work is pending",
            PasswordErrorCode::NoExecutor => {
                "asynchronous password hashing needs a Tokio current-thread event loop"
            }
            PasswordErrorCode::Random => "operating system random number generator failed",
            PasswordErrorCode::Detached => "input is detached or out of bounds",
            PasswordErrorCode::Internal => "the password hashing worker failed",
            PasswordErrorCode::Type => "expected a Uint8Array or a string",
            PasswordErrorCode::Range => "numeric argument is outside its documented range",
        }
    }
}

impl From<HashError> for PasswordError {
    fn from(error: HashError) -> Self {
        let code = match error.code() {
            HashErrorCode::Algorithm => PasswordErrorCode::Algorithm,
            HashErrorCode::Limit => PasswordErrorCode::Limit,
            HashErrorCode::Busy => PasswordErrorCode::Busy,
            HashErrorCode::NoExecutor => PasswordErrorCode::NoExecutor,
            HashErrorCode::Detached => PasswordErrorCode::Detached,
            HashErrorCode::Type => PasswordErrorCode::Type,
            HashErrorCode::Range => PasswordErrorCode::Range,
            // The remaining hash errors cannot arise from these argument
            // readers or the shared budget; keep the mapping closed and fail
            // safely if one is ever added.
            _ => PasswordErrorCode::Internal,
        };
        Self { code }
    }
}

impl From<CryptoError> for PasswordError {
    fn from(error: CryptoError) -> Self {
        let code = match error.code() {
            CryptoErrorCode::Type => PasswordErrorCode::Type,
            CryptoErrorCode::Detached => PasswordErrorCode::Detached,
            CryptoErrorCode::Limit => PasswordErrorCode::Limit,
            _ => PasswordErrorCode::Internal,
        };
        Self { code }
    }
}

/// Worker-to-join result tags. The join op returns raw bytes, so a worker
/// failure needs a tag that survives the buffer instead of becoming the empty
/// internal-failure sentinel.
const WORKER_OK: u8 = 0;
const WORKER_RANDOM: u8 = 1;
const WORKER_LIMIT: u8 = 2;
const WORKER_INTERNAL: u8 = 3;

/// Prefix a successful worker payload with [`WORKER_OK`].
fn worker_success(payload: Vec<u8>) -> Vec<u8> {
    let mut output = Vec::with_capacity(payload.len() + 1);
    output.push(WORKER_OK);
    output.extend_from_slice(&payload);
    output
}

/// Encode a worker failure with its stable category.
///
/// Range and argument errors were already handled before a worker starts; any
/// such error here is a bug and is reported as internal. The categories that
/// can arise in a worker are random-source failure, an allocation limit, and
/// the generic internal failure.
fn worker_error(error: PasswordError) -> Vec<u8> {
    let tag = match error.code() {
        PasswordErrorCode::Random => WORKER_RANDOM,
        PasswordErrorCode::Limit => WORKER_LIMIT,
        _ => WORKER_INTERNAL,
    };
    vec![tag]
}

/// Allocate a zeroizing destination and copy into it.
///
/// The allocation is already wrapped in `Zeroizing` before `copy` runs, so a
/// short copy, a panic in `copy`, or a later error drops a cleared buffer
/// rather than a plain `Vec`. The caller supplies the exact length and the
/// error category for a short or stretched copy, which is reported as
/// `short_error`.
fn copy_zeroizing<F>(
    length: usize,
    limit: usize,
    short_error: PasswordErrorCode,
    copy: F,
) -> Result<Zeroizing<Vec<u8>>, PasswordError>
where
    F: FnOnce(&mut [u8]) -> usize,
{
    if length > limit {
        return Err(PasswordError::new(PasswordErrorCode::Limit));
    }
    let mut bytes = Zeroizing::new(Vec::new());
    bytes
        .try_reserve_exact(length)
        .map_err(|_| PasswordError::new(PasswordErrorCode::Limit))?;
    bytes.resize(length, 0);
    if copy(bytes.as_mut_slice()) != length {
        return Err(PasswordError::new(short_error));
    }
    Ok(bytes)
}

/// Read a password or salt into a zeroizing buffer.
///
/// A string is measured by its UTF-8 length and written directly into the
/// zeroizing destination with V8's UTF-8 writer, using U+FFFD for an unpaired
/// surrogate; there is no intermediate Rust string. A view is length-checked,
/// allocated as `Zeroizing` first, then copied with `copy_contents`; a short
/// copy is a detached-input failure and the allocation is cleared on drop.
fn read_secret_input(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>, PasswordError> {
    if value.is_string() {
        let string = super::checked_string(value)?;
        let length = string.utf8_length(scope);
        // Convert directly into the zeroizing destination with V8's UTF-8
        // writer. `REPLACE_INVALID_UTF8` gives JavaScript's U+FFFD replacement
        // for an unpaired surrogate. This avoids `to_rust_string_lossy`, whose
        // WTF-16 fallback can allocate a temporary Rust string before this
        // module can wrap it.
        return copy_zeroizing(length, limit, PasswordErrorCode::Internal, |destination| {
            string.write_utf8_v2(
                scope,
                destination,
                v8::WriteFlags::kReplaceInvalidUtf8,
                None,
            )
        });
    }
    let view = checked_view(scope, value)?;
    copy_zeroizing(
        view.length,
        limit,
        PasswordErrorCode::Detached,
        |destination| view.view.copy_contents(destination),
    )
}

/// Derive `length` bytes with PBKDF2. The caller has validated every bound.
fn derive(
    algorithm: Algorithm,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> Result<Vec<u8>, PasswordError> {
    if iterations == 0 || iterations > MAX_PBKDF2_ITERATIONS {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }
    if length == 0 || length > MAX_PBKDF2_OUTPUT_BYTES {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }
    if password.len() > MAX_PBKDF2_INPUT_BYTES || salt.len() > MAX_PBKDF2_INPUT_BYTES {
        return Err(PasswordError::new(PasswordErrorCode::Limit));
    }
    #[cfg(feature = "test-support")]
    super::record_pbkdf2_placement();

    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| PasswordError::new(PasswordErrorCode::Limit))?;
    output.resize(length, 0);
    match Prf::from_algorithm(algorithm) {
        Prf::Sha256 => pbkdf2_hmac::<Sha256>(password, salt, iterations, &mut output),
        Prf::Sha384 => pbkdf2_hmac::<Sha384>(password, salt, iterations, &mut output),
        Prf::Sha512 => pbkdf2_hmac::<Sha512>(password, salt, iterations, &mut output),
    }
    Ok(output)
}

/// Hash a password with a fresh CSPRNG salt. The caller has validated `cost`.
fn hash(password: &[u8], cost: u32) -> Result<String, PasswordError> {
    if password.len() > MAX_BCRYPT_PASSWORD_BYTES {
        return Err(PasswordError::new(PasswordErrorCode::Limit));
    }
    if !(MIN_BCRYPT_COST..=MAX_BCRYPT_COST).contains(&cost) {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }
    #[cfg(feature = "test-support")]
    if take_bcrypt_random_failure() {
        return Err(PasswordError::new(PasswordErrorCode::Random));
    }
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|_| PasswordError::new(PasswordErrorCode::Random))?;
    hash_with_salt(password, cost, salt)
}

/// Hash with a caller-supplied salt. The unit tests use this to reproduce the
/// published Go bcrypt fixture byte for byte; the public API always draws a
/// fresh salt.
fn hash_with_salt(password: &[u8], cost: u32, salt: [u8; 16]) -> Result<String, PasswordError> {
    if password.len() > MAX_BCRYPT_PASSWORD_BYTES {
        return Err(PasswordError::new(PasswordErrorCode::Limit));
    }
    if !(MIN_BCRYPT_COST..=MAX_BCRYPT_COST).contains(&cost) {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }
    #[cfg(feature = "test-support")]
    super::record_bcrypt_placement();

    // Zeroize our copy when the operation returns. The bcrypt crate zeroizes
    // its own internal password buffer because its `zeroize` feature is on.
    let password = Zeroizing::new(password.to_vec());
    let salt = Zeroizing::new(salt);
    bcrypt::hash_with_salt(password.as_slice(), cost, *salt)
        .map(|parts| parts.format_for_version(bcrypt::Version::TwoB))
        .map_err(|error| match error {
            bcrypt::BcryptError::Rand(_) => PasswordError::new(PasswordErrorCode::Random),
            _ => PasswordError::new(PasswordErrorCode::Internal),
        })
}

/// Verify a password against a bcrypt modular-crypt string.
///
/// Malformed strings, unsupported versions, and costs outside
/// `MIN_BCRYPT_COST..=MAX_BCRYPT_COST` return `false` without computing a hash.
/// An input longer than 72 bytes also returns `false`. The bcrypt crate's
/// comparison is constant-time.
fn verify(password: &[u8], hash: &str) -> bool {
    if password.len() > MAX_BCRYPT_PASSWORD_BYTES {
        return false;
    }
    let Some(()) = valid_bcrypt_hash(hash) else {
        return false;
    };
    #[cfg(feature = "test-support")]
    super::record_bcrypt_placement();

    let password = Zeroizing::new(password.to_vec());
    bcrypt::verify(password.as_slice(), hash).unwrap_or(false)
}

/// Strictly validate the modular-crypt envelope before any expensive work.
///
/// The accepted versions are `2a`, `2b`, and `2y`; `2x` is rejected. The cost
/// must be two ASCII digits in `MIN_BCRYPT_COST..=MAX_BCRYPT_COST`. The body is
/// validated by the bcrypt crate's parser, which also checks the bcrypt base64
/// alphabet and the exact salt and hash lengths.
fn valid_bcrypt_hash(hash: &str) -> Option<()> {
    let bytes = hash.as_bytes();
    if bytes.len() != BCRYPT_HASH_BYTES || !hash.is_ascii() {
        return None;
    }
    if bytes[0] != b'$' || bytes[3] != b'$' || bytes[6] != b'$' {
        return None;
    }
    let version = &bytes[1..3];
    if version != b"2a" && version != b"2b" && version != b"2y" {
        return None;
    }
    if !bytes[4].is_ascii_digit() || !bytes[5].is_ascii_digit() {
        return None;
    }
    let cost = u32::from(bytes[4] - b'0') * 10 + u32::from(bytes[5] - b'0');
    if !(MIN_BCRYPT_COST..=MAX_BCRYPT_COST).contains(&cost) {
        return None;
    }
    bcrypt::HashParts::from_str(hash).ok()?;
    Some(())
}

/// Create the null-prototype result object used by the host ops.
fn outcome_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<ResourceId, PasswordError>,
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
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

fn outcome_bytes<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<Vec<u8>, PasswordError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(bytes) => match bytes_to_v8(scope, bytes) {
            Some(value) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", value.into());
            }
            None => set_failure(scope, object, PasswordError::new(PasswordErrorCode::Limit)),
        },
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

fn outcome_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<String, PasswordError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(text) => match v8::String::new(scope, &text) {
            Some(value) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", value.into());
            }
            None => set_failure(scope, object, PasswordError::new(PasswordErrorCode::Limit)),
        },
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

fn outcome_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<bool, PasswordError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(value) => {
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(
                scope,
                object,
                "value",
                v8::Boolean::new(scope, value).into(),
            );
        }
        Err(error) => set_failure(scope, object, error),
    }
    object.into()
}

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: PasswordError,
) {
    set_failure_parts(
        scope,
        object,
        error.code().js_class(),
        error.code().as_str(),
        error.message(),
    );
}

/// Reject a call that cannot host deno_core's async-op driver.
///
/// This mirrors the hash path: `deno_unsync::spawn` needs a Tokio
/// current-thread runtime on the isolate thread, and a multi-thread runtime
/// could poll a non-`Send` op future on another thread. The check runs before
/// input is copied or a job is reserved.
fn check_executor() -> Result<(), PasswordError> {
    if matches!(
        Handle::try_current().map(|handle| handle.runtime_flavor()),
        Ok(RuntimeFlavor::CurrentThread)
    ) {
        Ok(())
    } else {
        Err(PasswordError::new(PasswordErrorCode::NoExecutor))
    }
}

/// Read an iteration count or output length from its numeric argument.
fn read_count(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    minimum: u32,
    maximum: u32,
) -> Result<u32, PasswordError> {
    let value = number_code(scope, value, maximum)?;
    if value < minimum {
        return Err(PasswordError::new(PasswordErrorCode::Range));
    }
    Ok(value)
}

/// Read a bcrypt cost, defaulting only when the argument is `undefined`.
fn read_cost(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<u32, PasswordError> {
    if value.is_undefined() {
        return Ok(DEFAULT_BCRYPT_COST);
    }
    read_count(scope, value, MIN_BCRYPT_COST, MAX_BCRYPT_COST)
}

/// One PBKDF2 worker's owned state.
///
/// Field order is deliberate: the zeroizing copies are dropped before the
/// budget reservation, including when the closure is cancelled, when `run`
/// returns an error, and on unwind.
struct Pbkdf2Job {
    password: Zeroizing<Vec<u8>>,
    salt: Zeroizing<Vec<u8>>,
    _reservation: PendingReservation,
}

impl Pbkdf2Job {
    fn run(
        self,
        algorithm: Algorithm,
        iterations: u32,
        length: usize,
    ) -> Result<Vec<u8>, PasswordError> {
        derive(
            algorithm,
            self.password.as_slice(),
            self.salt.as_slice(),
            iterations,
            length,
        )
    }
}

/// One bcrypt hash worker's owned state. The password is zeroized before the
/// reservation is released.
struct BcryptHashJob {
    password: Zeroizing<Vec<u8>>,
    _reservation: PendingReservation,
}

impl BcryptHashJob {
    fn run(self, cost: u32) -> Result<String, PasswordError> {
        hash(self.password.as_slice(), cost)
    }
}

/// One bcrypt verification worker's owned state.
struct BcryptVerifyJob {
    password: Zeroizing<Vec<u8>>,
    hash: String,
    _reservation: PendingReservation,
}

impl BcryptVerifyJob {
    fn run(self) -> bool {
        verify(self.password.as_slice(), &self.hash)
    }
}

/// One pending PBKDF2 or bcrypt job. The join op takes the handle once.
struct AsyncPasswordJob {
    handle: RefCell<Option<JoinHandle<Vec<u8>>>>,
}

impl Drop for AsyncPasswordJob {
    fn drop(&mut self) {
        // A resource dropped before its join op takes the handle (for example
        // when the runtime is torn down) aborts a job that has not started.
        // Aborting a running `spawn_blocking` task is a no-op.
        if let Some(handle) = self.handle.get_mut().take() {
            handle.abort();
        }
    }
}

impl Resource for AsyncPasswordJob {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "neo::crypto::AsyncPassword".into()
    }
}

#[op2]
pub(super) fn op_neo_crypto_pbkdf2<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    algorithm: v8::Local<'s, v8::Value>,
    password: v8::Local<'s, v8::Value>,
    salt: v8::Local<'s, v8::Value>,
    iterations: v8::Local<'s, v8::Value>,
    length: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let algorithm = algorithm_from(scope, algorithm)?;
        let iterations = read_count(scope, iterations, 1, MAX_PBKDF2_ITERATIONS)?;
        let length = read_count(scope, length, 1, MAX_PBKDF2_OUTPUT_BYTES as u32)?;
        let password = read_secret_input(scope, password, MAX_PBKDF2_INPUT_BYTES)?;
        let salt = read_secret_input(scope, salt, MAX_PBKDF2_INPUT_BYTES)?;
        derive(
            algorithm,
            password.as_slice(),
            salt.as_slice(),
            iterations,
            length as usize,
        )
    })();
    outcome_bytes(scope, result)
}

#[op2]
pub(super) fn op_neo_crypto_pbkdf2_async_submit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    algorithm: v8::Local<'s, v8::Value>,
    password: v8::Local<'s, v8::Value>,
    salt: v8::Local<'s, v8::Value>,
    iterations: v8::Local<'s, v8::Value>,
    length: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        check_executor()?;
        let algorithm = algorithm_from(scope, algorithm)?;
        let iterations = read_count(scope, iterations, 1, MAX_PBKDF2_ITERATIONS)?;
        let length = read_count(scope, length, 1, MAX_PBKDF2_OUTPUT_BYTES as u32)?;
        let budget = {
            let executor = state.borrow::<CryptoExecutor>();
            if executor.handle.is_none() {
                return Err(PasswordError::new(PasswordErrorCode::Internal));
            }
            executor.budget()
        };
        // Install the isolate-thread probe before any copy, so a regression
        // that derives on the isolate thread is observable.
        #[cfg(feature = "test-support")]
        let _isolate_probe = ProbeGuard::install(Some(state.borrow::<CryptoExecutor>().probe()));

        let password_length = input_length(scope, password, MAX_PBKDF2_INPUT_BYTES)?;
        let salt_length = input_length(scope, salt, MAX_PBKDF2_INPUT_BYTES)?;
        let pending_bytes = password_length
            .checked_add(salt_length)
            .ok_or_else(|| PasswordError::new(PasswordErrorCode::Busy))?;
        let reservation = budget.reserve_pending(pending_bytes)?;
        // Wrap as soon as the copy exists, before the next fallible copy or any
        // queueing, so a later failure or cancellation zeroizes it.
        let password = read_secret_input(scope, password, MAX_PBKDF2_INPUT_BYTES)?;
        let salt = read_secret_input(scope, salt, MAX_PBKDF2_INPUT_BYTES)?;
        debug_assert_eq!(password.len(), password_length);
        debug_assert_eq!(salt.len(), salt_length);
        let job = Pbkdf2Job {
            password,
            salt,
            _reservation: reservation,
        };

        let join = state.borrow::<CryptoExecutor>().spawn_blocking(move || {
            match job.run(algorithm, iterations, length as usize) {
                Ok(result) => worker_success(result),
                Err(error) => worker_error(error),
            }
        });
        Ok(state.resource_table.add(AsyncPasswordJob {
            handle: RefCell::new(Some(join)),
        }))
    })();
    outcome_id(scope, result)
}

#[op2]
pub(super) fn op_neo_crypto_bcrypt_hash<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    password: v8::Local<'s, v8::Value>,
    cost: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let cost = read_cost(scope, cost)?;
        let password = read_secret_input(scope, password, MAX_BCRYPT_PASSWORD_BYTES)?;
        hash(password.as_slice(), cost)
    })();
    outcome_string(scope, result)
}

#[op2]
pub(super) fn op_neo_crypto_bcrypt_hash_async_submit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    password: v8::Local<'s, v8::Value>,
    cost: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        check_executor()?;
        let cost = read_cost(scope, cost)?;
        let budget = {
            let executor = state.borrow::<CryptoExecutor>();
            if executor.handle.is_none() {
                return Err(PasswordError::new(PasswordErrorCode::Internal));
            }
            executor.budget()
        };
        #[cfg(feature = "test-support")]
        let _isolate_probe = ProbeGuard::install(Some(state.borrow::<CryptoExecutor>().probe()));

        let password_length = input_length(scope, password, MAX_BCRYPT_PASSWORD_BYTES)?;
        let reservation = budget.reserve_pending(password_length)?;
        let password = read_secret_input(scope, password, MAX_BCRYPT_PASSWORD_BYTES)?;
        debug_assert_eq!(password.len(), password_length);
        let job = BcryptHashJob {
            password,
            _reservation: reservation,
        };

        let join = state
            .borrow::<CryptoExecutor>()
            .spawn_blocking(move || match job.run(cost) {
                Ok(hash) => worker_success(hash.into_bytes()),
                Err(error) => worker_error(error),
            });
        Ok(state.resource_table.add(AsyncPasswordJob {
            handle: RefCell::new(Some(join)),
        }))
    })();
    outcome_id(scope, result)
}

#[op2]
pub(super) fn op_neo_crypto_bcrypt_verify<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    password: v8::Local<'s, v8::Value>,
    hash: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let hash = checked_string(hash)?;
        // A password over 72 bytes is a false result, not an error, matching
        // bcrypt's fixed-size password input. Measure without a byte cap so an
        // oversized password is distinguished from a type error, then reject
        // it below without copying.
        let password_length = input_length(scope, password, usize::MAX)?;
        if hash.length() != BCRYPT_HASH_BYTES || password_length > MAX_BCRYPT_PASSWORD_BYTES {
            return Ok(false);
        }
        let hash = hash.to_rust_string_lossy(scope);
        let password = read_secret_input(scope, password, MAX_BCRYPT_PASSWORD_BYTES)?;
        Ok(verify(password.as_slice(), &hash))
    })();
    outcome_bool(scope, result)
}

#[op2]
pub(super) fn op_neo_crypto_bcrypt_verify_async_submit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    password: v8::Local<'s, v8::Value>,
    hash: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        check_executor()?;
        let hash = checked_string(hash)?;
        // Match the synchronous precedence: the hash must be a string, then
        // the password is validated (including detachment), and only then is a
        // malformed or over-cost hash a false result rather than an argument
        // error.
        let password_length = input_length(scope, password, usize::MAX)?;
        if hash.length() != BCRYPT_HASH_BYTES || password_length > MAX_BCRYPT_PASSWORD_BYTES {
            return Ok(VerifyAsync::Immediate(false));
        }
        let hash = hash.to_rust_string_lossy(scope);
        if valid_bcrypt_hash(&hash).is_none() {
            // An unsupported version, an over-cost hash, or an invalid body is
            // false without a worker. The worker path would only repeat this
            // validation.
            return Ok(VerifyAsync::Immediate(false));
        }
        let budget = {
            let executor = state.borrow::<CryptoExecutor>();
            if executor.handle.is_none() {
                return Err(PasswordError::new(PasswordErrorCode::Internal));
            }
            executor.budget()
        };
        #[cfg(feature = "test-support")]
        let _isolate_probe = ProbeGuard::install(Some(state.borrow::<CryptoExecutor>().probe()));

        let reservation = budget.reserve_pending(password_length)?;
        let password = read_secret_input(scope, password, MAX_BCRYPT_PASSWORD_BYTES)?;
        debug_assert_eq!(password.len(), password_length);
        let job = BcryptVerifyJob {
            password,
            hash,
            _reservation: reservation,
        };

        let join = state
            .borrow::<CryptoExecutor>()
            .spawn_blocking(move || worker_success(vec![u8::from(job.run())]));
        Ok(VerifyAsync::Job(state.resource_table.add(
            AsyncPasswordJob {
                handle: RefCell::new(Some(join)),
            },
        )))
    })();
    match result {
        Ok(VerifyAsync::Immediate(value)) => outcome_bool(scope, Ok(value)),
        Ok(VerifyAsync::Job(id)) => outcome_id(scope, Ok(id)),
        Err(error) => outcome_id(scope, Err(error)),
    }
}

enum VerifyAsync {
    Immediate(bool),
    Job(ResourceId),
}

/// Wait for one asynchronous PBKDF2 or bcrypt job.
///
/// An empty result is the internal-failure sentinel. A real PBKDF2 output has
/// at least one byte, a bcrypt hash has 60 bytes, and a verification has one
/// byte (`0` or `1`).
#[op2]
#[buffer]
pub(super) async fn op_neo_crypto_password_async_join(
    state: Rc<RefCell<OpState>>,
    #[smi] id: ResourceId,
) -> Vec<u8> {
    let handle = {
        let mut state = state.borrow_mut();
        let Ok(job) = state.resource_table.take::<AsyncPasswordJob>(id) else {
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

#[cfg(test)]
mod tests {
    use super::super::MAX_PENDING_ASYNC_BYTES;
    use super::super::MAX_PENDING_ASYNC_JOBS;
    use super::*;

    fn encode_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// RFC 7914 section 11, PBKDF2-HMAC-SHA256. These are published vectors,
    /// not values produced by this implementation.
    #[test]
    fn published_pbkdf2_sha256_vectors_match() {
        type Pbkdf2Case = (&'static [u8], &'static [u8], u32, usize, &'static str);
        let cases: [Pbkdf2Case; 2] = [
            (
                b"passwd",
                b"salt",
                1,
                64,
                "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783",
            ),
            (
                b"Password",
                b"NaCl",
                80_000,
                64,
                "4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56a1d425a1225833549adb841b51c9b3176a272bdebba1d078478f62b397f33c8d",
            ),
        ];
        for (password, salt, iterations, length, expected) in cases {
            let output = derive(Algorithm::Sha256, password, salt, iterations, length).unwrap();
            assert_eq!(encode_hex(&output), expected);
        }
    }

    /// Independently reproduced with Python's `hashlib` (OpenSSL backend) for
    /// this repository. The SHA-256 row is also the RFC 7914 vector above.
    #[test]
    fn sha384_and_sha512_vectors_match() {
        let cases: [(Algorithm, &str); 3] = [
            (
                Algorithm::Sha384,
                "cd3443723a41cf1460cca9efeede428a8898a82d2ad4d1fc5cca08ed3f4d3cb47a62a70b3cb9ce65dcbfb9fb9d425027a8be69b53e2a22674b0939e5e0a682f7",
            ),
            (
                Algorithm::Sha512,
                "c74319d99499fc3e9013acff597c23c5baf0a0bec5634c46b8352b793e324723d55caa76b2b25c43402dcfdc06cdcf66f95b7d0429420b39520006749c51a04e",
            ),
            (
                Algorithm::Sha256,
                "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783",
            ),
        ];
        for (algorithm, expected) in cases {
            let output = derive(algorithm, b"passwd", b"salt", 1, 64).unwrap();
            assert_eq!(encode_hex(&output), expected);
        }
    }

    #[test]
    fn nul_and_unicode_inputs_are_exact_bytes() {
        // Python: hashlib.pbkdf2_hmac("sha256", "pässwörd\0".encode(),
        // "sält\0".encode(), 2048, 48).hex()
        let expected = "e6635e1a12f44a9bfe0ea9fbf1ca3b46197ab653a51824dd04ae991232b28d9155ab1b190a9cbba124272bee01a6d70b";
        let output = derive(
            Algorithm::Sha256,
            "pässwörd\0".as_bytes(),
            "sält\0".as_bytes(),
            2048,
            48,
        )
        .unwrap();
        assert_eq!(encode_hex(&output), expected);

        // Raw bytes, including NUL and 0xff.
        let expected = "60d38d8523b876fa767fb807eec4c919d8c75512bb89ad7954bc0328812eb4cc";
        let output = derive(Algorithm::Sha256, b"\x00\xffpass", b"\x00salt\xff", 2, 32).unwrap();
        assert_eq!(encode_hex(&output), expected);
    }

    #[test]
    fn zeroizing_copy_rejects_a_short_copy_and_an_over_limit_length() {
        // Controlled short copy: the destination is a `Zeroizing` allocation
        // before `copy` runs, so this failure drops a cleared buffer. This test
        // observes the error category; it does not inspect freed memory.
        let error = copy_zeroizing(8, 8, PasswordErrorCode::Detached, |_| 7).unwrap_err();
        assert_eq!(error.code(), PasswordErrorCode::Detached);

        // Over-limit inputs are rejected before any allocation or copy.
        let error = copy_zeroizing(9, 8, PasswordErrorCode::Detached, |_| 9).unwrap_err();
        assert_eq!(error.code(), PasswordErrorCode::Limit);

        // A full copy still produces the expected bytes.
        let bytes = copy_zeroizing(4, 4, PasswordErrorCode::Detached, |destination| {
            destination.copy_from_slice(b"test");
            4
        })
        .unwrap();
        assert_eq!(bytes.as_slice(), b"test");
    }

    #[test]
    fn pbkdf2_bounds_are_enforced() {
        assert_eq!(
            derive(Algorithm::Sha256, b"p", b"s", 0, 32)
                .unwrap_err()
                .code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            derive(Algorithm::Sha256, b"p", b"s", MAX_PBKDF2_ITERATIONS + 1, 32)
                .unwrap_err()
                .code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            derive(Algorithm::Sha256, b"p", b"s", 1, 0)
                .unwrap_err()
                .code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            derive(
                Algorithm::Sha256,
                b"p",
                b"s",
                1,
                MAX_PBKDF2_OUTPUT_BYTES + 1
            )
            .unwrap_err()
            .code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            derive(
                Algorithm::Sha256,
                &vec![0u8; MAX_PBKDF2_INPUT_BYTES + 1],
                b"s",
                1,
                16,
            )
            .unwrap_err()
            .code(),
            PasswordErrorCode::Limit
        );
        assert_eq!(
            derive(
                Algorithm::Sha256,
                b"p",
                &vec![0u8; MAX_PBKDF2_INPUT_BYTES + 1],
                1,
                16,
            )
            .unwrap_err()
            .code(),
            PasswordErrorCode::Limit
        );
    }

    /// The Go bcrypt test vector, also used by `rust-bcrypt`. The salt is a
    /// fixed public test value, not a credential.
    const GO_SALT: [u8; 16] = [
        101, 201, 101, 75, 19, 227, 199, 20, 239, 236, 133, 32, 30, 109, 243, 30,
    ];
    const GO_HASH: &str = "$2b$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga";

    #[test]
    fn bcrypt_hash_matches_the_go_fixture_byte_for_byte() {
        let hash = hash_with_salt(b"allmine", 10, GO_SALT).unwrap();
        assert_eq!(hash, GO_HASH);
    }

    #[test]
    fn bcrypt_verifies_2a_2b_and_2y_fixtures() {
        // Go bcrypt fixture, written as both 2a and 2b. For an ASCII password
        // the two prefixes use the same core.
        assert!(verify(b"allmine", GO_HASH));
        assert!(verify(
            b"allmine",
            "$2a$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(verify(
            b"allmine",
            "$2y$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        // jBCrypt empty-password fixture.
        assert!(verify(
            b"",
            "$2a$06$DCq7YPn5Rq63x1Lad4cll.TV4S6ytwfsfvkgY8jIucDrjc8deX1s."
        ));
        // Apache htpasswd fixture.
        assert!(verify(
            b"password",
            "$2y$04$10NU4r76Q4nBoXH7Sw4RKu9LjsXUslmRavZFzvlPUCYI3Hkl4xX/W"
        ));
        assert!(!verify(
            b"wrong password",
            "$2y$04$10NU4r76Q4nBoXH7Sw4RKu9LjsXUslmRavZFzvlPUCYI3Hkl4xX/W"
        ));
    }

    #[test]
    fn bcrypt_rejects_malformed_and_overcost_hashes_without_hashing() {
        let valid = GO_HASH;
        assert!(!verify(b"allmine", ""));
        assert!(!verify(b"allmine", &valid[..59]));
        assert!(!verify(b"allmine", &format!("{valid}0")));
        assert!(!verify(
            b"allmine",
            "$2x$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(!verify(
            b"allmine",
            "$2b$03$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        // A syntactically valid hash with a cost above the verification cap.
        assert!(!verify(
            b"allmine",
            "$2b$17$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        assert!(!verify(
            b"allmine",
            "$2b$31$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga"
        ));
        // Invalid bcrypt base64, including a non-ASCII character.
        assert!(!verify(
            b"allmine",
            "$2b$10$XajjQvNhvvRt5GSeFk1xFeyqRrsxkhBkUiQeg0dt.wU1qD4aFDcga!"
        ));
        let non_ascii = valid.replace('X', "\u{012f}");
        assert!(!verify(b"allmine", &non_ascii));
    }

    #[test]
    fn bcrypt_password_bound_is_72_bytes() {
        let seventy_two = vec![b'a'; MAX_BCRYPT_PASSWORD_BYTES];
        let encoded = hash_with_salt(&seventy_two, 4, GO_SALT).unwrap();
        assert!(verify(&seventy_two, &encoded));
        assert!(!verify(&[b'a'; MAX_BCRYPT_PASSWORD_BYTES + 1], &encoded));
        assert_eq!(
            hash(&[b'a'; MAX_BCRYPT_PASSWORD_BYTES + 1], 4)
                .unwrap_err()
                .code(),
            PasswordErrorCode::Limit
        );
        assert_eq!(
            hash(b"p", MIN_BCRYPT_COST - 1).unwrap_err().code(),
            PasswordErrorCode::Range
        );
        assert_eq!(
            hash(b"p", MAX_BCRYPT_COST + 1).unwrap_err().code(),
            PasswordErrorCode::Range
        );
    }

    #[test]
    fn bcrypt_nul_and_unicode_passwords_roundtrip() {
        for password in [
            &b"a\0b"[..],
            "pässwörd".as_bytes(),
            &[0u8; 16],
            &[0xffu8; 16],
        ] {
            let hash = hash_with_salt(password, 4, GO_SALT).unwrap();
            assert!(verify(password, &hash), "{password:?}");
            assert!(!verify(b"other", &hash), "{password:?}");
        }
    }

    #[test]
    fn bcrypt_generated_hashes_use_2b_and_verify() {
        let hash = hash(b"correct horse battery staple", 4).unwrap();
        assert_eq!(hash.len(), BCRYPT_HASH_BYTES);
        assert!(hash.starts_with("$2b$04$"));
        assert!(verify(b"correct horse battery staple", &hash));
        assert!(!verify(b"correct horse battery stapl", &hash));
    }

    #[test]
    fn error_codes_are_stable_and_never_echo_input() {
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Algorithm)
                .code()
                .as_str(),
            Some("NEO_CRYPTO_ALGORITHM")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Limit).code().as_str(),
            Some("NEO_CRYPTO_LIMIT")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Busy).code().as_str(),
            Some("NEO_CRYPTO_BUSY")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::NoExecutor)
                .code()
                .as_str(),
            Some("NEO_CRYPTO_NO_EXECUTOR")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Random)
                .code()
                .as_str(),
            Some("NEO_CRYPTO_RANDOM")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Detached)
                .code()
                .as_str(),
            Some("NEO_CRYPTO_DETACHED")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Internal)
                .code()
                .as_str(),
            Some("NEO_CRYPTO_INTERNAL")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Type).code().as_str(),
            None
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Range).code().as_str(),
            None
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Type)
                .code()
                .js_class(),
            "TypeError"
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Range)
                .code()
                .js_class(),
            "RangeError"
        );
        for code in [
            PasswordErrorCode::Algorithm,
            PasswordErrorCode::Limit,
            PasswordErrorCode::Busy,
            PasswordErrorCode::NoExecutor,
            PasswordErrorCode::Random,
            PasswordErrorCode::Detached,
            PasswordErrorCode::Internal,
            PasswordErrorCode::Type,
            PasswordErrorCode::Range,
        ] {
            let message = PasswordError::new(code).message();
            assert!(!message.is_empty());
            assert!(!message.contains('\u{0}'));
        }
    }

    #[test]
    fn pending_budget_counts_password_jobs_to_the_shared_cap_and_releases() {
        let budget = std::sync::Arc::new(super::super::Budget::default());
        let mut reservations = Vec::new();
        for _ in 0..MAX_PENDING_ASYNC_JOBS {
            reservations.push(budget.reserve_pending(1).unwrap());
        }
        {
            let state = budget.lock();
            assert_eq!(state.pending_jobs, MAX_PENDING_ASYNC_JOBS);
            assert_eq!(state.pending_bytes, MAX_PENDING_ASYNC_JOBS);
        }
        let error = match budget.reserve_pending(0) {
            Ok(_) => panic!("an over-cap reservation must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), HashErrorCode::Busy);
        drop(reservations);
        let state = budget.lock();
        assert_eq!(state.pending_jobs, 0);
        assert_eq!(state.pending_bytes, 0);
    }

    #[test]
    fn shared_budget_constants_are_the_documented_values() {
        assert_eq!(MAX_PENDING_ASYNC_JOBS, 64);
        assert_eq!(MAX_PENDING_ASYNC_BYTES, 64 * 1024 * 1024);
        assert_eq!(MIN_BCRYPT_COST, 4);
        assert_eq!(MAX_BCRYPT_COST, 16);
        assert_eq!(DEFAULT_BCRYPT_COST, 10);
        assert_eq!(MAX_BCRYPT_PASSWORD_BYTES, 72);
        assert_eq!(MAX_PBKDF2_ITERATIONS, 1_000_000);
        assert_eq!(MAX_PBKDF2_OUTPUT_BYTES, 1024);
    }
}

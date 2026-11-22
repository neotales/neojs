//! Shamir secret sharing over GF(256) for `neo:crypto`.
//!
//! This module defines the bytewise share format, the field arithmetic, the
//! JavaScript-facing validation, the size bounds, and the asynchronous
//! executor condition. It is not a sandbox, does not manage keys for the
//! caller, and is not an authenticated secret-sharing scheme.
//!
//! The field and share encoding follow the Neotales Shamir implementation in
//! the PLAB .NET library (`Crypto/src/ShamirSecretSharing.cs`, revision
//! `79faf48ab85d23fcdbf64e29924ac0338841cb58`) and the `neotales-crypto`
//! `shamir` module in the rust-libraries source tree. Both use reduction
//! polynomial `x^8 + x^4 + x^3 + x + 1` (`0x11b`) with generator `0x03` and
//! one-based share indexes. This module is independently written in Rust; it
//! does not link, call, or vendor either implementation. The upstream MIT
//! notices are reproduced in `neojs-core/THIRD_PARTY_NOTICES.md`.
//!
//! # Share format and interoperability
//!
//! `shamirSplit(secret, threshold, shareCount)` treats each secret byte as the
//! constant term of its own degree-`threshold - 1` polynomial over GF(256).
//! The other coefficients are drawn independently from the operating system
//! CSPRNG. Share `i` is the polynomial evaluated at the field element `i`, for
//! `i` from 1 through `shareCount`. `shamirCombine` interpolates the shares at
//! `x = 0`.
//!
//! A share is a plain object with an integer `index` in `1..=255` and a
//! `Uint8Array` `value` of the same length as the secret. Indexes, coefficients,
//! and evaluation are bytewise: a set of shares produced by the PLAB .NET
//! implementation, the `neotales-crypto` Rust implementation, or this module
//! can be combined by any of the others, and this module's shares can be
//! combined by them. The JavaScript object shape is not a PLAB wire format:
//! callers that serialize shares must carry the index and value bytes
//! themselves.
//!
//! # Threshold and authenticity limitations
//!
//! Shamir secret sharing provides confidentiality, not integrity. A share is
//! not authenticated: this module has no MAC, commitment, checksum, or
//! redundant share. Any `threshold` shares reconstruct a value, but that value
//! is the original secret only if the shares are genuine, untampered, and from
//! the same split. Tampered shares, shares mixed from different splits, and a
//! wrong share index all reconstruct a different byte string silently. Fewer
//! than `threshold` shares also produce a byte string, not an error; it is
//! simply not the original secret. `shamirCombine` rejects fewer than two
//! shares and, when the optional `threshold` argument is supplied, fewer than
//! that many, but it cannot tell whether a set of shares is genuine.
//!
//! Callers who need to detect a wrong reconstruction must protect the secret
//! separately, for example by placing the recovered bytes inside an
//! authenticated envelope or comparing them with a digest or MAC stored out of
//! band. Such a check reveals only whether the reconstruction was correct; it
//! does not make the shares themselves authenticated. This module does not
//! implement any such check.
//!
//! The secret length is public because every share value has that length.
//! Indexes are public. The polynomial coefficients and the secret bytes are not
//! logged, and no error message contains input bytes.
//!
//! # Randomness
//!
//! Every coefficient other than the constant term comes from the operating
//! system CSPRNG through `getrandom`. There is no `Math.random`, counter,
//! time-based, or other fallback. If the operating system refuses to supply
//! entropy, a split fails with `NEO_CRYPTO_RANDOM`; it never returns shares made
//! from predictable coefficients. Combine uses no randomness.
//!
//! # Bounds and allocation
//!
//! A secret is at most [`MAX_SHAMIR_SECRET_BYTES`], a threshold and share count
//! are each between 2 and 255, and the total share bytes
//! (`secret length * shareCount`) are at most [`MAX_SHAMIR_TOTAL_BYTES`]. Every
//! share value passed to combine must also be at most
//! [`MAX_SHAMIR_SECRET_BYTES`]; this bound is checked before any value is
//! copied and applies identically to the synchronous and asynchronous APIs. The
//! total bound caps combine input further. Rust allocations are fallible; a
//! refused reservation is `NEO_CRYPTO_LIMIT`. V8 and the JavaScript heap
//! allocate too, so recovery from out-of-memory is not guaranteed.
//!
//! Inputs backed by a `SharedArrayBuffer` are rejected with
//! `NEO_CRYPTO_SHAMIR_SHARED`. Copying such a view is not one atomic snapshot:
//! another agent can change bytes during the copy, so a split or combine would
//! silently use an incoherent value. A caller that needs shared memory must
//! first copy a synchronized snapshot into a non-shared buffer.
//!
//! # Asynchronous API
//!
//! `shamirSplitAsync` and `shamirCombineAsync` return a `Promise`. Validation,
//! budget admission, and one bounded copy of the input run on the isolate
//! thread; the polynomial split or interpolation runs on the runtime's existing
//! bounded Tokio blocking pool through a `deno_core` async op. They are not
//! `Promise.resolve` wrappers around a blocking call. Copying the finished
//! share values or secret into JavaScript happens when the promise resolves.
//!
//! The asynchronous split draws its random coefficients on the worker, so the
//! isolate is not blocked by entropy reads or polynomial arithmetic. The
//! pending-job and pending-byte budgets are the same ones used by
//! `hashAsync`, `pbkdf2Async`, and bcrypt: at most
//! [`MAX_PENDING_ASYNC_JOBS`] jobs and [`MAX_PENDING_ASYNC_BYTES`] of the
//! worker's exact owned peak may be pending. A split reserves the secret copy,
//! one packed output, and the temporary noise buffer; it writes the packed
//! representation directly and never builds a second full share set. A combine
//! reserves the copied shares, the recovered secret, and the public
//! interpolation basis. A submission over either bound rejects with
//! `NEO_CRYPTO_BUSY`. The reservation is released when the worker finishes,
//! panics, or is aborted before it starts; an already-running job stays charged
//! until it returns. A split whose total share bytes are close to the 64 MiB
//! limit can be accepted synchronously but rejected asynchronously because the
//! pending budget also includes the secret copy, header, index bytes, and
//! noise.
//!
//! As with the other asynchronous crypto calls, deno_core hosts async ops on a
//! Tokio current-thread runtime on the isolate thread. `run_file` provides one;
//! so does `run_file_async` when the caller's executor is a Tokio
//! current-thread runtime. Any other executor gets `NEO_CRYPTO_NO_EXECUTOR`
//! before input is copied, and the synchronous API still works. A runtime with
//! no captured blocking-pool handle reports `NEO_CRYPTO_INTERNAL`.
//!
//! Cancellation is best effort, not a guarantee that work stops, and it is not
//! triggered by abandoning the returned JavaScript `Promise`. The wrapper
//! starts the join op immediately, so an unawaited promise still drives the job
//! to completion. The worker returns a `Zeroizing` result; the join stores it in
//! a resource until the synchronous take op transfers it to a zeroizing V8
//! backing store. Dropping the runtime before that transfer aborts a job that
//! has not started on a blocking thread and clears any completed result. A job
//! that has already started cannot be stopped, keeps its owned buffers and
//! reservation until it returns, and its result is still cleared on drop.
//!
//! # Memory and side channels
//!
//! Rust buffers holding the secret, random coefficients, share values, and the
//! packed worker result are wrapped in `zeroize::Zeroizing` or cleared with the
//! vetted `zeroize::Zeroize` implementation for `Vec`, which volatile-writes
//! the elements and full spare capacity and then applies its optimization
//! barrier. Every share value and recovered secret returned to JavaScript is
//! placed in a V8 backing store whose deleter clears the allocation before V8
//! frees it. The join result stays in a `Zeroizing` resource until that
//! transfer, so a discarded or abandoned result is cleared on runtime teardown.
//! This is still best-effort clearing: JavaScript can copy the bytes into
//! ordinary V8 memory (for example with `slice` or `structuredClone`), and the
//! JavaScript heap, V8 string storage, and caller-held copies are outside this
//! guarantee. The tests prove that the clear path is reached and that live
//! capacity is zero before the allocation is freed; they are not a
//! compiler-level proof that every copy is erased. The module does not promise
//! complete memory erasure.
//!
//! GF(256) multiplication is a fixed eight-round branchless mask loop, and
//! inversion uses a fixed seven-step exponentiation. Secret bytes are not used
//! as table indexes, and the arithmetic has no secret-dependent loop count.
//! JavaScript, V8, the allocator, and the operating system are not constant
//! time, so this is a reduction of obvious data-dependent branches, not a proof
//! that no timing or cache side channel exists. The module makes no such proof.
//!
//! # Scope
//!
//! This is not Node's `crypto`, the Web Crypto API, or the PLAB .NET API. It
//! implements only GF(256) Shamir splitting and combining with one-based
//! indexes. It has no share authentication, no verifiable secret sharing, no
//! proactive resharing, no group or weighted thresholds, and no serialization
//! format. The names `shamirSplit`, `shamirCombine`, `shamirSplitAsync`, and
//! `shamirCombineAsync` are additive to `neo:crypto` and must not be inferred
//! to mean any of those schemes.

use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_core::v8;
use tokio::runtime::Handle;
use tokio::runtime::RuntimeFlavor;
use tokio::task::AbortHandle;
use tokio::task::JoinHandle;
use zeroize::Zeroize;
use zeroize::Zeroizing;

use crate::crypto::CryptoError;
use crate::crypto::CryptoErrorCode;
use crate::crypto::View;
use crate::crypto::bytes_to_v8;
use crate::crypto::checked_view;
use crate::crypto::copy_view;
use crate::crypto::response;
use crate::crypto::set;
use crate::crypto::set_failure_parts;
use crate::crypto_hash::CryptoExecutor;
use crate::crypto_hash::PendingReservation;
#[cfg(feature = "test-support")]
use crate::crypto_hash::record_shamir_placement;

/// Largest secret accepted by a split.
pub(crate) const MAX_SHAMIR_SECRET_BYTES: usize = 8 * 1024 * 1024;

/// Largest number of shares in one split or combine.
pub(crate) const MAX_SHAMIR_SHARES: usize = 255;

/// Largest `secret length * share count` product accepted by a split, and the
/// largest total share-value bytes accepted by a combine.
pub(crate) const MAX_SHAMIR_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// Random noise is drawn in chunks of about this many bytes. This bounds the
/// temporary random buffer independently of the secret length.
const NOISE_BUFFER_BYTES: usize = 64 * 1024;

/// Largest packed worker result accepted by the unpack ops: the share bytes,
/// one index byte per share, and the nine-byte split header.
const MAX_SHAMIR_PACKED_BYTES: usize = MAX_SHAMIR_TOTAL_BYTES + MAX_SHAMIR_SHARES + 8;

/// Stable machine-readable Shamir failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShamirErrorCode {
    /// An input or allocation exceeded a documented bound.
    Limit,
    /// The operating system CSPRNG failed.
    Random,
    /// Two shares used the same index.
    DuplicateIndex,
    /// Share values differed in length.
    LengthMismatch,
    /// The view's buffer was detached.
    Detached,
    /// The view is backed by a `SharedArrayBuffer`.
    Shared,
    /// An argument has the wrong JavaScript type.
    Type,
    /// A numeric argument is not an integer in its documented range.
    Range,
    /// Too much asynchronous work is already pending.
    Busy,
    /// The event loop cannot host deno_core async ops.
    NoExecutor,
    /// A worker, field operation, or the runtime executor failed.
    Internal,
}

impl ShamirErrorCode {
    /// The value exposed as the JavaScript error's `code` property.
    ///
    /// Argument errors ([`ShamirErrorCode::Type`] and
    /// [`ShamirErrorCode::Range`]) have no code and become `TypeError` or
    /// `RangeError`.
    fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Limit => "NEO_CRYPTO_LIMIT",
            Self::Random => "NEO_CRYPTO_RANDOM",
            Self::DuplicateIndex => "NEO_CRYPTO_SHAMIR_DUPLICATE",
            Self::LengthMismatch => "NEO_CRYPTO_SHAMIR_LENGTH",
            Self::Detached => "NEO_CRYPTO_DETACHED",
            Self::Shared => "NEO_CRYPTO_SHAMIR_SHARED",
            Self::Busy => "NEO_CRYPTO_BUSY",
            Self::NoExecutor => "NEO_CRYPTO_NO_EXECUTOR",
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

    /// The one-byte tag carried from a worker through the join buffer.
    fn tag(self) -> u8 {
        match self {
            Self::Random => 1,
            Self::Limit => 2,
            Self::DuplicateIndex => 3,
            Self::LengthMismatch => 4,
            Self::Detached => 5,
            Self::Type => 6,
            Self::Range => 7,
            Self::Busy | Self::NoExecutor | Self::Internal => 8,
            Self::Shared => 9,
        }
    }

    /// Decode a worker tag. Unknown tags are internal failures.
    fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Random,
            2 => Self::Limit,
            3 => Self::DuplicateIndex,
            4 => Self::LengthMismatch,
            5 => Self::Detached,
            6 => Self::Type,
            7 => Self::Range,
            9 => Self::Shared,
            _ => Self::Internal,
        }
    }
}

/// A failure safe to describe without echoing a secret or share value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShamirError {
    code: ShamirErrorCode,
}

impl ShamirError {
    fn new(code: ShamirErrorCode) -> Self {
        Self { code }
    }

    fn code(self) -> ShamirErrorCode {
        self.code
    }

    /// A fixed message. It never contains input bytes.
    fn message(self) -> &'static str {
        match self.code {
            ShamirErrorCode::Limit => "input exceeds the Shamir size limit",
            ShamirErrorCode::Random => "operating system random number generator failed",
            ShamirErrorCode::DuplicateIndex => "duplicate share index",
            ShamirErrorCode::LengthMismatch => "all shares must have the same length",
            ShamirErrorCode::Detached => "input is detached or out of bounds",
            ShamirErrorCode::Shared => "SharedArrayBuffer-backed input cannot be copied coherently",
            ShamirErrorCode::Type => "expected a Uint8Array, number, or share object",
            ShamirErrorCode::Range => "numeric argument is outside its documented range",
            ShamirErrorCode::Busy => "too much asynchronous crypto work is pending",
            ShamirErrorCode::NoExecutor => {
                "asynchronous Shamir needs a Tokio current-thread event loop"
            }
            ShamirErrorCode::Internal => "the Shamir worker failed",
        }
    }
}

impl From<CryptoError> for ShamirError {
    fn from(error: CryptoError) -> Self {
        let code = match error.code() {
            CryptoErrorCode::Type => ShamirErrorCode::Type,
            CryptoErrorCode::Range => ShamirErrorCode::Range,
            CryptoErrorCode::Detached => ShamirErrorCode::Detached,
            CryptoErrorCode::Limit => ShamirErrorCode::Limit,
            CryptoErrorCode::Random => ShamirErrorCode::Random,
            _ => ShamirErrorCode::Internal,
        };
        Self::new(code)
    }
}

/// One share of a split secret.
///
/// The value is zeroized when the share is dropped. The public JavaScript API
/// returns plain `{ index, value }` objects whose `value` bytes are owned by
/// V8; this Rust type is the module's temporary working representation.
struct ShamirShare {
    index: u8,
    value: Vec<u8>,
}

impl std::fmt::Debug for ShamirShare {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShamirShare")
            .field("index", &self.index)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

impl Drop for ShamirShare {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

/// A parsed share before its bytes are copied.
///
/// Reading every property and view once gives validation a stable snapshot: a
/// getter or a later mutation cannot make the checked length differ from the
/// copied length. Proxies and accessor properties are rejected by
/// [`read_share_plan`].
struct SharePlan<'s> {
    index: u8,
    view: View<'s>,
}

/// Test-only, deterministically consumed injection for the next split's noise
/// draw. It is compiled only with the off-by-default `test-support` feature, so
/// a normal build has no production hook. The async worker consumes it from a
/// blocking thread, which is why it is an atomic rather than thread-local.
#[cfg(feature = "test-support")]
static FAIL_NEXT_SHAMIR_RANDOM: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "test-support")]
pub(crate) fn fail_next_shamir_random() {
    FAIL_NEXT_SHAMIR_RANDOM.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(feature = "test-support")]
fn take_shamir_random_failure() -> bool {
    FAIL_NEXT_SHAMIR_RANDOM.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// Fill `bytes` from the operating system CSPRNG, with no fallback.
fn fill_random(bytes: &mut [u8]) -> Result<(), ShamirError> {
    #[cfg(feature = "test-support")]
    if take_shamir_random_failure() {
        return Err(ShamirError::new(ShamirErrorCode::Random));
    }
    getrandom::fill(bytes).map_err(|_| ShamirError::new(ShamirErrorCode::Random))
}

/// Allocate a zero-filled byte vector without aborting on a refused
/// reservation.
fn zeroed_vec(length: usize) -> Result<Vec<u8>, ShamirError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    bytes.resize(length, 0);
    Ok(bytes)
}

/// Copy a byte slice with a fallible reservation.
fn copied_slice(source: &[u8]) -> Result<Vec<u8>, ShamirError> {
    let mut bytes = zeroed_vec(source.len())?;
    bytes.copy_from_slice(source);
    Ok(bytes)
}

/// Validate a `Uint8Array` and reject a `SharedArrayBuffer` backing store.
///
/// Copying a view backed by shared memory is not an atomic snapshot: another
/// agent can change bytes during the copy, so the split or combine would use an
/// incoherent value. The caller must copy the bytes into a non-shared buffer
/// first if it needs its own synchronization.
fn checked_shamir_view<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<View<'s>, ShamirError> {
    let view = checked_view(scope, value)?;
    let Some(buffer) = view.view.buffer(scope) else {
        return Err(ShamirError::new(ShamirErrorCode::Detached));
    };
    if buffer.get_backing_store().is_shared() {
        return Err(ShamirError::new(ShamirErrorCode::Shared));
    }
    Ok(view)
}

/// Multiply a value length by a share count and enforce the total bound.
fn check_total(length: usize, count: usize) -> Result<usize, ShamirError> {
    let total = length
        .checked_mul(count)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
    if total > MAX_SHAMIR_TOTAL_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    Ok(total)
}

/// Bytes of random noise one split needs for its temporary buffer.
///
/// The chunk width also returned is how many secret bytes are processed per
/// CSPRNG fill.
fn noise_size(secret_length: usize, threshold: usize) -> Result<(usize, usize), ShamirError> {
    let stride = threshold - 1;
    let chunk_bytes = (NOISE_BUFFER_BYTES / stride).max(1).min(secret_length);
    let noise_length = chunk_bytes
        .checked_mul(stride)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
    Ok((chunk_bytes, noise_length))
}

/// Exact bytes an asynchronous split worker will own at one time.
///
/// This counts the secret copy, the single packed output (header, index bytes,
/// and share values), and the temporary noise buffer. It does not count a
/// second full share set, because the worker writes the packed representation
/// directly.
fn split_pending_bytes(
    secret_length: usize,
    threshold: usize,
    share_count: usize,
) -> Result<usize, ShamirError> {
    let total = secret_length
        .checked_mul(share_count)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Busy))?;
    let (_, noise_length) = noise_size(secret_length, threshold)?;
    let packed = 8usize
        .checked_add(share_count)
        .and_then(|bytes| bytes.checked_add(total))
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Busy))?;
    secret_length
        .checked_add(packed)
        .and_then(|bytes| bytes.checked_add(noise_length))
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Busy))
}

/// Exact bytes an asynchronous combine worker will own at one time.
///
/// This counts the copied share values, the recovered secret output, and the
/// public interpolation basis. The worker no longer builds a second packed copy
/// of the secret.
fn combine_pending_bytes(
    total_share_bytes: usize,
    secret_length: usize,
    share_count: usize,
) -> Result<usize, ShamirError> {
    total_share_bytes
        .checked_add(secret_length)
        .and_then(|bytes| bytes.checked_add(share_count))
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Busy))
}

/// Validate split parameters and return the total share bytes.
fn validate_split(
    secret_length: usize,
    threshold: usize,
    share_count: usize,
) -> Result<usize, ShamirError> {
    if secret_length == 0
        || !(2..=MAX_SHAMIR_SHARES).contains(&threshold)
        || !(threshold..=MAX_SHAMIR_SHARES).contains(&share_count)
    {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    if secret_length > MAX_SHAMIR_SECRET_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    check_total(secret_length, share_count)
}

/// Validate a combine share set and return the secret length.
fn validate_combine(shares: &[ShamirShare]) -> Result<usize, ShamirError> {
    if shares.len() < 2 || shares.len() > MAX_SHAMIR_SHARES {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    let Some(first) = shares.first() else {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    };
    let length = first.value.len();
    if length == 0 {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    if length > MAX_SHAMIR_SECRET_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    check_total(length, shares.len())?;
    let mut seen = [false; 256];
    for share in shares {
        if share.index == 0 {
            return Err(ShamirError::new(ShamirErrorCode::Range));
        }
        let slot = usize::from(share.index);
        if seen[slot] {
            return Err(ShamirError::new(ShamirErrorCode::DuplicateIndex));
        }
        seen[slot] = true;
        if share.value.is_empty() {
            return Err(ShamirError::new(ShamirErrorCode::Range));
        }
        if share.value.len() != length {
            return Err(ShamirError::new(ShamirErrorCode::LengthMismatch));
        }
    }
    Ok(length)
}

/// Validate the combine shape from a share plan and return its value length.
fn validate_plan(plans: &[SharePlan<'_>]) -> Result<usize, ShamirError> {
    if plans.len() < 2 || plans.len() > MAX_SHAMIR_SHARES {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    let Some(first) = plans.first() else {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    };
    let length = first.view.length;
    if length == 0 {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    if length > MAX_SHAMIR_SECRET_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    check_total(length, plans.len())?;
    let mut seen = [false; 256];
    for plan in plans {
        if plan.index == 0 {
            return Err(ShamirError::new(ShamirErrorCode::Range));
        }
        let slot = usize::from(plan.index);
        if seen[slot] {
            return Err(ShamirError::new(ShamirErrorCode::DuplicateIndex));
        }
        seen[slot] = true;
        if plan.view.length == 0 {
            return Err(ShamirError::new(ShamirErrorCode::Range));
        }
        if plan.view.length != length {
            return Err(ShamirError::new(ShamirErrorCode::LengthMismatch));
        }
    }
    Ok(length)
}

/// Split `secret` into `share_count` shares so any `threshold` recover it.
///
/// The secret is measured and validated before any share is allocated. Random
/// coefficients are drawn in bounded chunks, so the temporary random buffer is
/// at most about [`NOISE_BUFFER_BYTES`], not proportional to the secret length.
fn split(
    secret: &[u8],
    threshold: usize,
    share_count: usize,
) -> Result<Vec<ShamirShare>, ShamirError> {
    split_with(secret, threshold, share_count, fill_random)
}

/// Split with a caller-supplied noise source.
///
/// Production reaches this through [`split`] with [`fill_random`]. The
/// deterministic unit test supplies fixed bytes so the emitted share values can
/// be checked against an independent implementation.
fn split_with<F>(
    secret: &[u8],
    threshold: usize,
    share_count: usize,
    mut fill: F,
) -> Result<Vec<ShamirShare>, ShamirError>
where
    F: FnMut(&mut [u8]) -> Result<(), ShamirError>,
{
    validate_split(secret.len(), threshold, share_count)?;
    #[cfg(feature = "test-support")]
    record_shamir_placement();

    let mut shares = Vec::new();
    shares
        .try_reserve_exact(share_count)
        .map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    for position in 0..share_count {
        let index =
            u8::try_from(position + 1).map_err(|_| ShamirError::new(ShamirErrorCode::Range))?;
        shares.push(ShamirShare {
            index,
            value: zeroed_vec(secret.len())?,
        });
    }

    let stride = threshold - 1;
    let (chunk_bytes, noise_length) = noise_size(secret.len(), threshold)?;
    let mut noise = Zeroizing::new(zeroed_vec(noise_length)?);

    for start in (0..secret.len()).step_by(chunk_bytes) {
        let width = chunk_bytes.min(secret.len() - start);
        let used = width * stride;
        fill(&mut noise[..used])?;
        for offset in 0..width {
            let constant = secret[start + offset];
            let coefficients = &noise[offset * stride..(offset + 1) * stride];
            for share in &mut shares {
                share.value[start + offset] = evaluate(constant, coefficients, share.index);
            }
        }
    }
    Ok(shares)
}

/// Recover the secret from `shares` with Lagrange interpolation at `x = 0`.
///
/// Interpolation coefficients depend only on the public share indexes, so they
/// are computed once per share set rather than once per secret byte.
fn combine(shares: &[ShamirShare]) -> Result<Vec<u8>, ShamirError> {
    let length = validate_combine(shares)?;
    #[cfg(feature = "test-support")]
    record_shamir_placement();

    let basis = compute_basis(shares)?;
    let mut secret = Zeroizing::new(zeroed_vec(length)?);
    for (byte_index, slot) in secret.as_mut_slice().iter_mut().enumerate() {
        let mut value = 0u8;
        for (share, coefficient) in shares.iter().zip(basis.iter()) {
            value ^= mul(share.value[byte_index], *coefficient);
        }
        *slot = value;
    }
    Ok(std::mem::take(&mut *secret))
}

/// Compute one Lagrange basis coefficient per share at `x = 0`.
fn compute_basis(shares: &[ShamirShare]) -> Result<Vec<u8>, ShamirError> {
    let mut basis = zeroed_vec(shares.len())?;
    for (outer, share) in shares.iter().enumerate() {
        let xi = share.index;
        let mut coefficient = 1u8;
        for (inner, other) in shares.iter().enumerate() {
            if outer == inner {
                continue;
            }
            let denominator = xi ^ other.index;
            coefficient = mul(coefficient, div(other.index, denominator)?);
        }
        basis[outer] = coefficient;
    }
    Ok(basis)
}

/// Evaluate the polynomial with `constant` as coefficient 0 and `coefficients`
/// as coefficients 1 through `threshold - 1` at `point`.
fn evaluate(constant: u8, coefficients: &[u8], point: u8) -> u8 {
    let mut result = 0u8;
    for coefficient in coefficients.iter().rev() {
        result = mul(result, point) ^ coefficient;
    }
    mul(result, point) ^ constant
}

/// `left / right` in GF(256), or an internal failure for a zero divisor.
///
/// Share indexes are validated to be distinct, so a zero denominator cannot
/// arise from the public combine path.
fn div(left: u8, right: u8) -> Result<u8, ShamirError> {
    if right == 0 {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    Ok(mul(left, inverse(right)))
}

/// `value^-1` in GF(256) by `value^254`, with a fixed seven-step trip count.
///
/// The caller must pass a non-zero element. A zero input returns zero; the
/// public paths reject zero indexes before this function is reached.
fn inverse(value: u8) -> u8 {
    let mut result = 1u8;
    let mut factor = mul(value, value);
    for _ in 0..7 {
        result = mul(result, factor);
        factor = mul(factor, factor);
    }
    result
}

/// `left * right` in GF(256), reduction polynomial `0x11b`.
///
/// This is a fixed eight-round Russian-peasant loop. Conditional XORs are mask
/// arithmetic, so the round count and control flow do not depend on either
/// operand. There is no secret-indexed table.
fn mul(left: u8, right: u8) -> u8 {
    let mut a = left;
    let mut b = right;
    let mut product = 0u8;
    for _ in 0..8 {
        let bit = b & 1;
        product ^= a & 0u8.wrapping_sub(bit);
        let carry = a >> 7;
        a = a.wrapping_shl(1);
        a ^= 0x1b & 0u8.wrapping_sub(carry);
        b >>= 1;
    }
    product
}

/// Read an integer parameter in `minimum..=maximum`.
fn read_integer(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    minimum: u32,
    maximum: u32,
) -> Result<usize, ShamirError> {
    if !value.is_number() {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    }
    let Some(number) = value.number_value(scope) else {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    };
    if !number.is_finite()
        || number.trunc() != number
        || number < f64::from(minimum)
        || number > f64::from(maximum)
    {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    Ok(number as usize)
}

/// Read an optional combine threshold.
fn read_optional_threshold(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Option<usize>, ShamirError> {
    if value.is_undefined() {
        return Ok(None);
    }
    read_integer(scope, value, 2, MAX_SHAMIR_SHARES as u32).map(Some)
}

/// Read one own data property from a share object.
///
/// Accessor properties are rejected, so reading this property cannot run a
/// getter and cannot produce a different value between validation and copying.
fn descriptor_data_value<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: v8::Local<'s, v8::Name>,
    field: &str,
) -> Result<v8::Local<'s, v8::Value>, ShamirError> {
    let Some(descriptor_value) = object.get_own_property_descriptor(scope, key) else {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    };
    let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor_value) else {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    };
    let Some(get_key) = v8::String::new(scope, "get") else {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    };
    let get_key: v8::Local<'s, v8::Name> = get_key.into();
    if descriptor.has_own_property(scope, get_key) != Some(false) {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    }
    let Some(field_key) = v8::String::new(scope, field) else {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    };
    let field_key: v8::Local<'s, v8::Name> = field_key.into();
    descriptor
        .get(scope, field_key.into())
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Type))
}

/// Read one own data property by name.
fn own_data_property<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    field: &str,
) -> Result<v8::Local<'s, v8::Value>, ShamirError> {
    let Some(key) = v8::String::new(scope, name) else {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    };
    descriptor_data_value(scope, object, key.into(), field)
}

/// Read one array element through its own data descriptor.
///
/// `Array.prototype` element access can invoke an accessor. Reading the
/// descriptor first rejects an accessor element without running it and also
/// rejects a hole.
fn own_array_element<'s>(
    scope: &v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    position: u32,
) -> Result<v8::Local<'s, v8::Value>, ShamirError> {
    let number = v8::Integer::new_from_unsigned(scope, position);
    let Some(key) = number.to_string(scope) else {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    };
    descriptor_data_value(scope, array.cast::<v8::Object>(), key.into(), "value")
}

/// Validate and capture every share's index and view without copying values.
fn read_share_plan<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<SharePlan<'s>>, ShamirError> {
    if !value.is_array() || value.is_proxy() {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    }
    let array = v8::Local::<v8::Array>::try_from(value)
        .map_err(|_| ShamirError::new(ShamirErrorCode::Type))?;
    let count = array.length() as usize;
    if !(2..=MAX_SHAMIR_SHARES).contains(&count) {
        return Err(ShamirError::new(ShamirErrorCode::Range));
    }
    let mut plans = Vec::new();
    plans
        .try_reserve_exact(count)
        .map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    for position in 0..count {
        let element = own_array_element(scope, array, position as u32)?;
        if !element.is_object() || element.is_proxy() {
            return Err(ShamirError::new(ShamirErrorCode::Type));
        }
        let object = v8::Local::<v8::Object>::try_from(element)
            .map_err(|_| ShamirError::new(ShamirErrorCode::Type))?;
        let index_value = own_data_property(scope, object, "index", "value")?;
        let index = read_integer(scope, index_value, 1, MAX_SHAMIR_SHARES as u32)?;
        let index = u8::try_from(index).map_err(|_| ShamirError::new(ShamirErrorCode::Range))?;
        let value_property = own_data_property(scope, object, "value", "value")?;
        let view = checked_shamir_view(scope, value_property)?;
        plans.push(SharePlan { index, view });
    }
    validate_plan(&plans)?;
    Ok(plans)
}

/// Copy a validated share plan into owned, zeroizing share values.
fn copy_shares(plans: &[SharePlan<'_>]) -> Result<Vec<ShamirShare>, ShamirError> {
    let mut shares = Vec::new();
    shares
        .try_reserve_exact(plans.len())
        .map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    for plan in plans {
        shares.push(ShamirShare {
            index: plan.index,
            value: copy_view(&plan.view)?,
        });
    }
    Ok(shares)
}

/// A parsed async split payload.
struct SplitPayload<'a> {
    count: usize,
    length: usize,
    body: &'a [u8],
}

/// Split directly into the packed worker representation.
///
/// This writes one packed allocation and never builds a second full share set.
/// The header is `count` as a little-endian `u32`, `value length` as a
/// little-endian `u32`, then one index byte and one value per share. The
/// caller's `Zeroizing` wrapper owns the result until it is stored in a
/// [`WorkerResult`].
fn split_packed(
    secret: &[u8],
    threshold: usize,
    share_count: usize,
) -> Result<SensitiveVec, ShamirError> {
    let total = validate_split(secret.len(), threshold, share_count)?;
    #[cfg(feature = "test-support")]
    record_shamir_placement();

    let stride = secret
        .len()
        .checked_add(1)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
    let capacity = 8usize
        .checked_add(share_count)
        .and_then(|bytes| bytes.checked_add(total))
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
    let mut packed = Zeroizing::new(zeroed_vec(capacity)?);

    let count = u32::try_from(share_count).map_err(|_| ShamirError::new(ShamirErrorCode::Range))?;
    let length =
        u32::try_from(secret.len()).map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    packed[0..4].copy_from_slice(&count.to_le_bytes());
    packed[4..8].copy_from_slice(&length.to_le_bytes());
    for position in 0..share_count {
        let index =
            u8::try_from(position + 1).map_err(|_| ShamirError::new(ShamirErrorCode::Range))?;
        packed[8 + position * stride] = index;
    }

    let stride = threshold - 1;
    let (chunk_bytes, noise_length) = noise_size(secret.len(), threshold)?;
    let mut noise = Zeroizing::new(zeroed_vec(noise_length)?);
    for start in (0..secret.len()).step_by(chunk_bytes) {
        let width = chunk_bytes.min(secret.len() - start);
        let used = width * stride;
        fill_random(&mut noise[..used])?;
        for offset in 0..width {
            let constant = secret[start + offset];
            let coefficients = &noise[offset * stride..(offset + 1) * stride];
            for position in 0..share_count {
                let base = 8 + position * (secret.len() + 1);
                let index = packed[base];
                let value = evaluate(constant, coefficients, index);
                packed[base + 1 + start + offset] = value;
            }
        }
    }
    Ok(SensitiveVec::new(std::mem::take(&mut *packed)))
}

/// Parse the header and body of a packed split payload.
fn parse_split_payload(bytes: &[u8]) -> Result<SplitPayload<'_>, ShamirError> {
    if bytes.len() < 8 {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    let count = u32::from_le_bytes(
        bytes[0..4]
            .try_into()
            .map_err(|_| ShamirError::new(ShamirErrorCode::Internal))?,
    ) as usize;
    let length = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| ShamirError::new(ShamirErrorCode::Internal))?,
    ) as usize;
    if !(2..=MAX_SHAMIR_SHARES).contains(&count) || length == 0 {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    let total = count
        .checked_mul(length)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
    if total > MAX_SHAMIR_TOTAL_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    let expected = count
        .checked_mul(length)
        .and_then(|body| body.checked_add(count))
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
    if bytes.len() != 8 + expected {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    Ok(SplitPayload {
        count,
        length,
        body: &bytes[8..],
    })
}

#[cfg(feature = "test-support")]
static ZEROIZED_V8_BUFFERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "test-support")]
static ZEROIZED_WORKER_RESULTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Test-only count of Shamir V8 backing stores released through the zeroizing
/// deleter. Compiled only with the off-by-default `test-support` feature.
#[cfg(feature = "test-support")]
pub(crate) fn zeroized_v8_buffers() -> usize {
    ZEROIZED_V8_BUFFERS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Test-only count of sensitive worker buffers cleared at the actual wipe
/// boundary.
#[cfg(feature = "test-support")]
pub(crate) fn zeroized_worker_results() -> usize {
    ZEROIZED_WORKER_RESULTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// V8 backing-store deleter for the original `Vec` allocation.
///
/// `deleter_data` is a `Box<Vec<u8>>` created by [`zeroizing_bytes_to_v8`].
/// The vector still owns its original allocation and capacity, so clearing the
/// full capacity covers bytes past the logical length without reallocating.
/// Dropping the vector frees that same allocation.
unsafe extern "C" fn wipe_vec_backing_store(
    _data: *mut c_void,
    _byte_length: usize,
    deleter_data: *mut c_void,
) {
    if deleter_data.is_null() {
        return;
    }
    // SAFETY: `deleter_data` is the `Box<Vec<u8>>` handed to V8, and V8 calls
    // this deleter exactly once.
    let mut owner = unsafe { Box::from_raw(deleter_data.cast::<Vec<u8>>()) };
    // `Zeroize for Vec` uses volatile writes over the elements and then over
    // the full spare capacity, followed by its optimization barrier. This
    // covers the original allocation including capacity without reallocating.
    owner.zeroize();
    #[cfg(any(debug_assertions, feature = "test-support"))]
    {
        // SAFETY: `Zeroize for Vec` has just written zero to every capacity
        // byte, so the memory is initialized and can be inspected safely. This
        // fails a test if the wipe is removed.
        let all = unsafe { std::slice::from_raw_parts(owner.as_ptr(), owner.capacity()) };
        assert!(
            all.iter().all(|byte| *byte == 0),
            "the full vector allocation was not zeroized"
        );
    }
    #[cfg(feature = "test-support")]
    ZEROIZED_V8_BUFFERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // `owner` drops here and frees the original vector allocation.
}

/// Move bytes into a V8 `Uint8Array` whose backing store is zeroized when V8
/// releases it.
///
/// The exact original `Vec` (pointer, length, and capacity) is retained in a
/// boxed deleter context. The function never shrinks the vector or converts it
/// to a boxed slice, so no secret-bearing allocation is reallocated and freed
/// unwiped. If transfer fails, the backing store's context drop clears it. A JS
/// caller can still copy the bytes into ordinary JavaScript memory, which this
/// module cannot clear.
fn zeroizing_bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut bytes: Zeroizing<Vec<u8>>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let length = bytes.len();
    if length == 0 {
        // A zero-length vector can still have capacity holding old bytes.
        let mut empty = std::mem::take(&mut *bytes);
        // `Zeroize for Vec` clears the logical length and then volatile-writes
        // zero over the full spare capacity, with its optimization barrier.
        empty.zeroize();
        drop(empty);
        return bytes_to_v8(scope, Vec::new());
    }
    let vec = std::mem::take(&mut *bytes);
    let original_pointer = vec.as_ptr();
    let mut owner = Box::new(vec);
    debug_assert_eq!(owner.len(), length);
    debug_assert_eq!(owner.as_ptr(), original_pointer);
    let data = owner.as_mut_ptr().cast::<c_void>();
    let context = Box::into_raw(owner).cast::<c_void>();
    // SAFETY: `data` points into the vector allocation owned by `context`.
    // V8 takes both until it calls `wipe_vec_backing_store`, which reconstructs
    // the box and frees the original vector allocation.
    let store = unsafe {
        v8::ArrayBuffer::new_backing_store_from_ptr(data, length, wipe_vec_backing_store, context)
    };
    let store = store.make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length)
}

/// Define an own array index without consulting a prototype setter.
fn define_array_index<'s>(
    scope: &v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    position: u32,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), ShamirError> {
    let number = v8::Integer::new_from_unsigned(scope, position);
    let Some(key) = number.to_string(scope) else {
        return Err(ShamirError::new(ShamirErrorCode::Type));
    };
    if array
        .create_data_property(scope, key.into(), value)
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?
    {
        Ok(())
    } else {
        Err(ShamirError::new(ShamirErrorCode::Internal))
    }
}

/// Build a JavaScript array of frozen `{ index, value }` share objects.
///
/// Each value is transferred to a zeroizing V8 backing store.
fn shares_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    shares: Vec<ShamirShare>,
) -> Result<v8::Local<'s, v8::Array>, ShamirError> {
    let count =
        i32::try_from(shares.len()).map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    let array = v8::Array::new(scope, count);
    for (position, mut share) in shares.into_iter().enumerate() {
        let value = zeroizing_bytes_to_v8(scope, Zeroizing::new(std::mem::take(&mut share.value)))
            .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
        let object = v8::Object::new(scope);
        set(
            scope,
            object,
            "index",
            v8::Integer::new_from_unsigned(scope, u32::from(share.index)).into(),
        );
        set(scope, object, "value", value.into());
        define_array_index(scope, array, position as u32, object.into())?;
    }
    Ok(array)
}

/// Build a JavaScript share array directly from a packed split payload.
fn packed_shares_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    packed: &[u8],
) -> Result<v8::Local<'s, v8::Array>, ShamirError> {
    let payload = parse_split_payload(packed)?;
    let count =
        i32::try_from(payload.count).map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    let array = v8::Array::new(scope, count);
    let mut offset = 0usize;
    for position in 0..payload.count {
        let index = *payload
            .body
            .get(offset)
            .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
        if index == 0 {
            return Err(ShamirError::new(ShamirErrorCode::Internal));
        }
        let end = offset + 1 + payload.length;
        let value = payload
            .body
            .get(offset + 1..end)
            .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
        let value = zeroizing_bytes_to_v8(scope, Zeroizing::new(copied_slice(value)?))
            .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))?;
        let object = v8::Object::new(scope);
        set(
            scope,
            object,
            "index",
            v8::Integer::new_from_unsigned(scope, u32::from(index)).into(),
        );
        set(scope, object, "value", value.into());
        define_array_index(scope, array, position as u32, object.into())?;
        offset = end;
    }
    if offset != payload.body.len() {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    Ok(array)
}

/// Unpack a split worker result into a JavaScript share array.
fn unpack_split_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    packed: Vec<u8>,
) -> Result<v8::Local<'s, v8::Array>, ShamirError> {
    let packed = SensitiveVec::new(packed);
    if packed.len() > MAX_SHAMIR_PACKED_BYTES {
        return Err(ShamirError::new(ShamirErrorCode::Limit));
    }
    packed_shares_to_v8(scope, packed.as_slice())
}

/// Unpack a combine worker result into a zeroizing JavaScript secret.
fn unpack_combine_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    secret: Vec<u8>,
) -> Result<v8::Local<'s, v8::Uint8Array>, ShamirError> {
    let mut secret = SensitiveVec::new(secret);
    if secret.is_empty() {
        return Err(ShamirError::new(ShamirErrorCode::Internal));
    }
    let bytes = secret
        .take()
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
    zeroizing_bytes_to_v8(scope, Zeroizing::new(bytes))
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))
}

/// Build a failure response object.
fn failure<'s>(scope: &mut v8::PinScope<'s, '_>, error: ShamirError) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    set_failure_parts(
        scope,
        object,
        error.code().js_class(),
        error.code().as_str(),
        error.message(),
    );
    object.into()
}

/// Build a successful response object around one value.
fn success<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
    set(scope, object, "value", value);
    object.into()
}

/// Turn a Rust split result into a JavaScript response.
fn outcome_shares<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<Vec<ShamirShare>, ShamirError>,
) -> v8::Local<'s, v8::Value> {
    match result {
        Ok(shares) => match shares_to_v8(scope, shares) {
            Ok(value) => success(scope, value.into()),
            Err(error) => failure(scope, error),
        },
        Err(error) => failure(scope, error),
    }
}

/// Turn a Rust combine result into a zeroizing JavaScript response.
fn outcome_secret<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<Vec<u8>, ShamirError>,
) -> v8::Local<'s, v8::Value> {
    match result {
        Ok(bytes) => match zeroizing_bytes_to_v8(scope, Zeroizing::new(bytes)) {
            Some(value) => success(scope, value.into()),
            None => failure(scope, ShamirError::new(ShamirErrorCode::Limit)),
        },
        Err(error) => failure(scope, error),
    }
}

/// Turn a resource id result into a JavaScript response.
fn outcome_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<ResourceId, ShamirError>,
) -> v8::Local<'s, v8::Value> {
    match result {
        Ok(id) => {
            let object = response(scope);
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(
                scope,
                object,
                "id",
                v8::Integer::new_from_unsigned(scope, id).into(),
            );
            object.into()
        }
        Err(error) => failure(scope, error),
    }
}

/// Split a secret synchronously.
#[op2]
fn op_neo_crypto_shamir_split<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    secret: v8::Local<'s, v8::Value>,
    threshold: v8::Local<'s, v8::Value>,
    share_count: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let threshold = read_integer(scope, threshold, 2, MAX_SHAMIR_SHARES as u32)?;
        let share_count = read_integer(scope, share_count, 2, MAX_SHAMIR_SHARES as u32)?;
        let view = checked_shamir_view(scope, secret)?;
        validate_split(view.length, threshold, share_count)?;
        let secret = Zeroizing::new(copy_view(&view)?);
        split(&secret, threshold, share_count)
    })();
    outcome_shares(scope, result)
}

/// Split a secret on the blocking pool.
#[op2]
fn op_neo_crypto_shamir_split_async_submit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    secret: v8::Local<'s, v8::Value>,
    threshold: v8::Local<'s, v8::Value>,
    share_count: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        if !matches!(
            Handle::try_current().map(|handle| handle.runtime_flavor()),
            Ok(RuntimeFlavor::CurrentThread)
        ) {
            return Err(ShamirError::new(ShamirErrorCode::NoExecutor));
        }
        let threshold = read_integer(scope, threshold, 2, MAX_SHAMIR_SHARES as u32)?;
        let share_count = read_integer(scope, share_count, 2, MAX_SHAMIR_SHARES as u32)?;
        let view = checked_shamir_view(scope, secret)?;
        validate_split(view.length, threshold, share_count)?;
        let pending = split_pending_bytes(view.length, threshold, share_count)?;
        let executor = state
            .try_borrow::<CryptoExecutor>()
            .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
        if !executor.has_handle() {
            return Err(ShamirError::new(ShamirErrorCode::Internal));
        }
        let Some(reservation) = executor.budget().reserve_pending_busy(pending) else {
            return Err(ShamirError::new(ShamirErrorCode::Busy));
        };
        let secret = Zeroizing::new(copy_view(&view)?);
        let job = SplitJob {
            secret,
            threshold,
            share_count,
            _reservation: reservation,
        };
        let join = state
            .borrow::<CryptoExecutor>()
            .spawn_blocking(move || job.run());
        Ok(state.resource_table.add(AsyncShamirJob {
            handle: RefCell::new(Some(join)),
            result: RefCell::new(None),
        }))
    })();
    outcome_id(scope, result)
}

/// Combine shares synchronously.
#[op2]
fn op_neo_crypto_shamir_combine<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    shares: v8::Local<'s, v8::Value>,
    threshold: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let threshold = read_optional_threshold(scope, threshold)?;
        let plans = read_share_plan(scope, shares)?;
        if let Some(threshold) = threshold
            && plans.len() < threshold
        {
            return Err(ShamirError::new(ShamirErrorCode::Range));
        }
        let shares = copy_shares(&plans)?;
        combine(&shares)
    })();
    outcome_secret(scope, result)
}

/// Combine shares on the blocking pool.
#[op2]
fn op_neo_crypto_shamir_combine_async_submit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    shares: v8::Local<'s, v8::Value>,
    threshold: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        if !matches!(
            Handle::try_current().map(|handle| handle.runtime_flavor()),
            Ok(RuntimeFlavor::CurrentThread)
        ) {
            return Err(ShamirError::new(ShamirErrorCode::NoExecutor));
        }
        let threshold = read_optional_threshold(scope, threshold)?;
        let plans = read_share_plan(scope, shares)?;
        if let Some(threshold) = threshold
            && plans.len() < threshold
        {
            return Err(ShamirError::new(ShamirErrorCode::Range));
        }
        let total = validate_plan_total(&plans)?;
        let secret_length = plans
            .first()
            .map(|plan| plan.view.length)
            .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
        let pending = combine_pending_bytes(total, secret_length, plans.len())?;
        let executor = state
            .try_borrow::<CryptoExecutor>()
            .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
        if !executor.has_handle() {
            return Err(ShamirError::new(ShamirErrorCode::Internal));
        }
        let Some(reservation) = executor.budget().reserve_pending_busy(pending) else {
            return Err(ShamirError::new(ShamirErrorCode::Busy));
        };
        let shares = copy_shares(&plans)?;
        let job = CombineJob {
            shares,
            _reservation: reservation,
        };
        let join = state
            .borrow::<CryptoExecutor>()
            .spawn_blocking(move || job.run());
        Ok(state.resource_table.add(AsyncShamirJob {
            handle: RefCell::new(Some(join)),
            result: RefCell::new(None),
        }))
    })();
    outcome_id(scope, result)
}

/// Total bytes a validated plan will copy.
fn validate_plan_total(plans: &[SharePlan<'_>]) -> Result<usize, ShamirError> {
    let length = validate_plan(plans)?;
    length
        .checked_mul(plans.len())
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))
}

/// Bytes owned by a worker or take path that are cleared when dropped.
///
/// `None` means the bytes were handed to V8 or there were none. A `Some` vector
/// is cleared with the vetted [`Zeroize for Vec`] implementation, which
/// volatile-writes the elements and full spare capacity and then applies its
/// optimization barrier. The drop instrumentation runs only after that clear.
struct SensitiveVec(Option<Vec<u8>>);

impl SensitiveVec {
    fn new(bytes: Vec<u8>) -> Self {
        Self(Some(bytes))
    }

    fn empty() -> Self {
        Self(None)
    }

    fn take(&mut self) -> Option<Vec<u8>> {
        self.0.take()
    }

    fn as_slice(&self) -> &[u8] {
        self.0.as_deref().unwrap_or(&[])
    }

    fn len(&self) -> usize {
        self.0.as_ref().map_or(0, Vec::len)
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for SensitiveVec {
    fn drop(&mut self) {
        if let Some(mut bytes) = self.0.take() {
            bytes.zeroize();
            #[cfg(debug_assertions)]
            {
                // SAFETY: `Zeroize for Vec` has just written zero to every
                // capacity byte. Reading it is initialized and catches a
                // removed wipe in tests without reading freed memory.
                let all = unsafe { std::slice::from_raw_parts(bytes.as_ptr(), bytes.capacity()) };
                debug_assert!(
                    all.iter().all(|byte| *byte == 0),
                    "sensitive bytes were not cleared before drop"
                );
            }
            #[cfg(feature = "test-support")]
            ZEROIZED_WORKER_RESULTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// The result a blocking worker hands back.
struct WorkerResult {
    status: u8,
    data: SensitiveVec,
}

impl WorkerResult {
    fn success(data: SensitiveVec) -> Self {
        Self { status: 0, data }
    }

    fn error(error: ShamirError) -> Self {
        Self {
            status: error.code().tag(),
            data: SensitiveVec::empty(),
        }
    }
}

/// The worker-owned state for one asynchronous split.
struct SplitJob {
    secret: Zeroizing<Vec<u8>>,
    threshold: usize,
    share_count: usize,
    _reservation: PendingReservation,
}

impl SplitJob {
    fn run(self) -> WorkerResult {
        match split_packed(&self.secret, self.threshold, self.share_count) {
            Ok(packed) => WorkerResult::success(packed),
            Err(error) => WorkerResult::error(error),
        }
    }
}

/// The worker-owned state for one asynchronous combine.
struct CombineJob {
    shares: Vec<ShamirShare>,
    _reservation: PendingReservation,
}

impl CombineJob {
    fn run(self) -> WorkerResult {
        match combine(&self.shares) {
            Ok(secret) => WorkerResult::success(SensitiveVec::new(secret)),
            Err(error) => WorkerResult::error(error),
        }
    }
}

/// One pending split or combine job. The join op takes the handle once and
/// stores the zeroizing result here; the take op removes it.
struct AsyncShamirJob {
    handle: RefCell<Option<JoinHandle<WorkerResult>>>,
    result: RefCell<Option<WorkerResult>>,
}

impl Drop for AsyncShamirJob {
    fn drop(&mut self) {
        // A resource dropped before its join op takes the handle (for example
        // when the runtime is torn down) aborts a job that has not started.
        // Aborting a running `spawn_blocking` task is a no-op.
        if let Some(handle) = self.handle.get_mut().take() {
            handle.abort();
        }
    }
}

impl Resource for AsyncShamirJob {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "neo::crypto::AsyncShamir".into()
    }
}

/// Aborts a queued blocking task if the join future is dropped.
///
/// A running `spawn_blocking` task cannot be stopped; abort is a no-op for it.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Wait for one asynchronous split or combine and store its zeroizing result.
///
/// The returned bytes are only a status tag, never a secret or share value.
/// The result stays in the resource until `op_neo_crypto_shamir_take_result`
/// transfers it to a zeroizing V8 backing store. If the runtime is torn down
/// first, dropping the resource drops the `WorkerResult` and clears its data.
#[op2]
#[buffer]
async fn op_neo_crypto_shamir_async_join(
    state: Rc<RefCell<OpState>>,
    #[smi] id: ResourceId,
) -> Vec<u8> {
    let handle = {
        let state = state.borrow_mut();
        let Ok(job) = state.resource_table.get::<AsyncShamirJob>(id) else {
            return vec![ShamirErrorCode::Internal.tag()];
        };
        job.handle.borrow_mut().take()
    };
    let Some(handle) = handle else {
        return vec![ShamirErrorCode::Internal.tag()];
    };
    #[cfg(feature = "test-support")]
    if let Some(executor) = state.borrow().try_borrow::<CryptoExecutor>() {
        executor.note_join_started();
    }
    let _abort = AbortOnDrop(handle.abort_handle());
    let output = match handle.await {
        Ok(output) => output,
        Err(_) => WorkerResult::error(ShamirError::new(ShamirErrorCode::Internal)),
    };
    let status = output.status;
    {
        let state = state.borrow_mut();
        if let Ok(job) = state.resource_table.get::<AsyncShamirJob>(id) {
            *job.result.borrow_mut() = Some(output);
        }
        // If the resource vanished, `output` drops here and its `Zeroizing`
        // data is cleared.
    }
    if status == 0 {
        Vec::new()
    } else {
        vec![status]
    }
}

/// Take one finished worker result and transfer it to JavaScript.
///
/// `kind` is 1 for a split (share array) or 2 for a combine (secret bytes).
/// The resource is removed before transfer. Any error drops the
/// `WorkerResult`, which clears its `Zeroizing` data.
#[op2]
fn op_neo_crypto_shamir_take_result<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    #[smi] id: ResourceId,
    kind: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let kind = read_integer(scope, kind, 1, 2)?;
        let Ok(job) = state.resource_table.take::<AsyncShamirJob>(id) else {
            return Err(ShamirError::new(ShamirErrorCode::Internal));
        };
        let Some(result) = job.result.borrow_mut().take() else {
            return Err(ShamirError::new(ShamirErrorCode::Internal));
        };
        let mut result = result;
        let status = result.status;
        if status != 0 {
            // Dropping `WorkerResult` clears any sensitive data it still holds.
            return Err(ShamirError::new(ShamirErrorCode::from_tag(status)));
        }
        let data = result
            .data
            .take()
            .ok_or_else(|| ShamirError::new(ShamirErrorCode::Internal))?;
        match kind {
            1 => unpack_split_result(scope, data).map(Into::into),
            2 => unpack_combine_result(scope, data).map(Into::into),
            _ => Err(ShamirError::new(ShamirErrorCode::Range)),
        }
    })();
    match result {
        Ok(value) => success(scope, value),
        Err(error) => failure(scope, error),
    }
}

deno_core::extension!(
    neojs_crypto_shamir,
    ops = [
        op_neo_crypto_shamir_split,
        op_neo_crypto_shamir_split_async_submit,
        op_neo_crypto_shamir_combine,
        op_neo_crypto_shamir_combine_async_submit,
        op_neo_crypto_shamir_async_join,
        op_neo_crypto_shamir_take_result
    ],
    docs = "GF(256) Shamir secret sharing ops behind `Neo.getBuiltInModule(\"crypto\")`."
);

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        (0..value.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn gf256_matches_known_products_and_inverses() {
        assert_eq!(mul(0, 5), 0);
        assert_eq!(mul(5, 0), 0);
        assert_eq!(mul(3, 3), 5);
        assert_eq!(mul(0x57, 0x83), 0xc1);
        assert_eq!(div(5, 3).unwrap(), 3);
        assert_eq!(div(0, 1).unwrap(), 0);
        assert!(div(1, 0).is_err());
        for value in 1..=255u8 {
            assert_eq!(mul(value, inverse(value)), 1, "inverse of {value}");
        }
    }

    #[test]
    fn independent_combine_vector_matches_the_plab_encoding() {
        let shares = [
            ShamirShare {
                index: 1,
                value: vec![0x51],
            },
            ShamirShare {
                index: 2,
                value: vec![0x64],
            },
        ];
        assert_eq!(combine(&shares).unwrap(), vec![0x42]);
    }

    #[test]
    fn independent_three_of_five_vector_matches() {
        // Generated independently with a Python implementation of GF(256)
        // reduction polynomial 0x11b for this repository. The noise input was
        // 00 01 02 ... 1b, one coefficient per secret byte per non-constant
        // term.
        let secret = b"shamir-interop";
        let noise = decode_hex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b");
        let expected = [
            (1u8, "7269606c68732c686f7564736e71"),
            (2, "77607d7d5d4a01490a1c19023b28"),
            (3, "76617c7c5c4b00480b1d18033a29"),
            (4, "63502105d9eacda125177e4184b3"),
            (5, "62512004d8ebcca024167f4085b2"),
        ];
        let mut position = 0usize;
        let shares = split_with(secret, 3, 5, |bytes| {
            let end = position + bytes.len();
            bytes.copy_from_slice(&noise[position..end]);
            position = end;
            Ok(())
        })
        .unwrap();
        for (share, (index, value)) in shares.iter().zip(expected) {
            assert_eq!(share.index, index);
            assert_eq!(encode_hex(&share.value), value);
        }
        assert_eq!(combine(&shares).unwrap(), secret);
    }

    #[test]
    fn every_threshold_subset_reconstructs() {
        let secret = b"shamir-interop";
        let shares = split(secret, 3, 5).unwrap();
        for first in 0..shares.len() {
            for second in first + 1..shares.len() {
                for third in second + 1..shares.len() {
                    let subset = [
                        ShamirShare {
                            index: shares[first].index,
                            value: shares[first].value.clone(),
                        },
                        ShamirShare {
                            index: shares[second].index,
                            value: shares[second].value.clone(),
                        },
                        ShamirShare {
                            index: shares[third].index,
                            value: shares[third].value.clone(),
                        },
                    ];
                    assert_eq!(combine(&subset).unwrap(), secret);
                }
            }
        }
    }

    #[test]
    fn sensitive_vec_drop_clears_full_capacity_at_the_boundary() {
        let mut vec = Vec::with_capacity(32);
        vec.extend_from_slice(b"secret");
        let capacity = vec.capacity();
        assert!(capacity > vec.len());
        unsafe {
            std::ptr::write_bytes(vec.as_mut_ptr().add(vec.len()), 0xaa, capacity - vec.len());
        }
        #[cfg(feature = "test-support")]
        let before = zeroized_worker_results();
        // The Drop implementation's debug assertion fails if the full-capacity
        // wipe is removed. It inspects the live allocation before it is freed.
        drop(SensitiveVec::new(vec));
        #[cfg(feature = "test-support")]
        assert_eq!(
            zeroized_worker_results(),
            before + 1,
            "the clear-boundary counter did not advance after a wiped drop"
        );
    }

    #[test]
    fn deleter_wipes_excess_capacity_from_the_original_vec_context() {
        let mut vec = Vec::with_capacity(32);
        vec.extend_from_slice(b"secret");
        let length = vec.len();
        let capacity = vec.capacity();
        assert!(capacity > length);
        // Initialize the unused capacity so the debug assertion in the deleter
        // can inspect it after zeroization.
        unsafe {
            std::ptr::write_bytes(vec.as_mut_ptr().add(length), 0xaa, capacity - length);
        }
        let data = vec.as_mut_ptr().cast::<c_void>();
        let context = Box::into_raw(Box::new(vec)).cast::<c_void>();
        // SAFETY: `data`/`context` match the layout the deleter expects; this is
        // the same ownership transfer V8 performs exactly once.
        unsafe {
            wipe_vec_backing_store(data, length, context);
        }
    }

    #[test]
    fn vec_owner_preserves_and_wipes_the_original_capacity() {
        let mut vec = Vec::with_capacity(32);
        vec.extend_from_slice(b"secret");
        assert!(vec.capacity() > vec.len());
        let pointer = vec.as_ptr();
        let capacity = vec.capacity();
        // Initialize the unused capacity so the test can inspect it after the
        // wipe without reading uninitialized memory.
        unsafe {
            std::ptr::write_bytes(vec.as_mut_ptr().add(vec.len()), 0xaa, capacity - vec.len());
        }
        let mut owner = Box::new(vec);
        assert_eq!(owner.as_ptr(), pointer);
        assert_eq!(owner.capacity(), capacity);
        owner.zeroize();
        // SAFETY: all `capacity` bytes were initialized and are now zeroed.
        let all = unsafe { std::slice::from_raw_parts(owner.as_ptr(), owner.capacity()) };
        assert!(all.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn async_pending_bytes_match_worker_peak() {
        // 1000 secret bytes, 3-of-5: 5000 share bytes, 2000 noise bytes, and a
        // 5013-byte packed output, plus the 1000-byte secret copy.
        assert_eq!(split_pending_bytes(1000, 3, 5).unwrap(), 8013);
        // 100 secret bytes, 255-of-255: 25400 noise bytes, 25763 packed bytes.
        assert_eq!(split_pending_bytes(100, 255, 255).unwrap(), 51263);
        // 5000 share bytes, 1000-byte secret, five basis bytes.
        assert_eq!(combine_pending_bytes(5000, 1000, 5).unwrap(), 6005);
    }

    #[test]
    fn total_bound_is_checked_without_allocating() {
        assert_eq!(
            check_total(MAX_SHAMIR_TOTAL_BYTES / 2 + 1, 2)
                .unwrap_err()
                .code(),
            ShamirErrorCode::Limit
        );
        assert_eq!(check_total(1, 255).unwrap(), 255);
        assert_eq!(
            check_total(MAX_SHAMIR_SECRET_BYTES, 2).unwrap(),
            MAX_SHAMIR_SECRET_BYTES * 2
        );
    }

    #[test]
    fn split_rejects_invalid_shapes_without_allocating_output() {
        assert_eq!(split(b"", 2, 3).unwrap_err().code(), ShamirErrorCode::Range);
        assert_eq!(
            split(b"secret", 1, 3).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            split(b"secret", 3, 2).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            split(b"secret", 2, 256).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            split(&vec![0u8; MAX_SHAMIR_SECRET_BYTES + 1], 2, 2)
                .unwrap_err()
                .code(),
            ShamirErrorCode::Limit
        );
    }

    #[test]
    fn combine_rejects_malformed_sets() {
        let share = |index, value: &[u8]| ShamirShare {
            index,
            value: value.to_vec(),
        };
        assert_eq!(
            combine(&[share(1, b"x")]).unwrap_err().code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            combine(&[share(1, b"x"), share(1, b"y")])
                .unwrap_err()
                .code(),
            ShamirErrorCode::DuplicateIndex
        );
        assert_eq!(
            combine(&[share(1, b"x"), share(3, b"yy")])
                .unwrap_err()
                .code(),
            ShamirErrorCode::LengthMismatch
        );
        assert_eq!(
            combine(&[share(0, b"x"), share(2, b"y")])
                .unwrap_err()
                .code(),
            ShamirErrorCode::Range
        );
        assert_eq!(
            combine(&[share(1, b""), share(2, b"")]).unwrap_err().code(),
            ShamirErrorCode::Range
        );
    }
}

//! Incremental SHA-2 hashing and HMAC ops for `vael:crypto`, plus bounded
//! asynchronous one-shot variants.
//!
//! Pure SHA-2 hashing, HMAC, output encodings, and bounds live in
//! [`vaeljs_crypto_core::hash`]. This module retains the V8 resource table,
//! Tokio blocking-pool worker dispatch, per-runtime pending-job/byte budget,
//! test-only placement/cancellation hooks, and `deno_core` ops.

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
use tokio::runtime::Handle;
use tokio::runtime::RuntimeFlavor;
use tokio::task::AbortHandle;
use tokio::task::JoinHandle;
use vaeljs_crypto_core::hash::{
    DigestState as CoreDigestState, HashAlgorithm as Algorithm, HashError as CoreHashError,
    HashErrorCode as CoreHashErrorCode, MAX_HASH_INPUT_BYTES, MAX_HMAC_KEY_BYTES, MAX_OPEN_DIGESTS,
    OutputEncoding, encode_digest,
};

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

/// Test-only hook to make the next bcrypt salt draw fail.
#[cfg(feature = "test-support")]
pub(crate) fn fail_next_bcrypt_random() {
    vaeljs_crypto_core::test_support::fail_next_bcrypt_random();
}

/// Largest amount of copied input allowed to be pending across asynchronous
/// calls.
pub(crate) const MAX_PENDING_ASYNC_BYTES: usize = 64 * 1024 * 1024;

/// Largest number of asynchronous jobs allowed to be pending at once.
pub(crate) const MAX_PENDING_ASYNC_JOBS: usize = 64;

/// Stable machine-readable hash failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HashErrorCode {
    Algorithm,
    Encoding,
    Limit,
    Digested,
    Busy,
    NoExecutor,
    TooManyObjects,
    Detached,
    Internal,
    Type,
    Range,
}

impl HashErrorCode {
    fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Algorithm => "VAEL_CRYPTO_ALGORITHM",
            Self::Encoding => "VAEL_CRYPTO_ENCODING",
            Self::Limit => "VAEL_CRYPTO_LIMIT",
            Self::Digested => "VAEL_CRYPTO_DIGESTED",
            Self::Busy => "VAEL_CRYPTO_BUSY",
            Self::NoExecutor => "VAEL_CRYPTO_NO_EXECUTOR",
            Self::TooManyObjects => "VAEL_CRYPTO_TOO_MANY_OBJECTS",
            Self::Detached => "VAEL_CRYPTO_DETACHED",
            Self::Internal => "VAEL_CRYPTO_INTERNAL",
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

impl From<CoreHashError> for HashError {
    fn from(error: CoreHashError) -> Self {
        let code = match error.code() {
            CoreHashErrorCode::Algorithm => HashErrorCode::Algorithm,
            CoreHashErrorCode::Encoding => HashErrorCode::Encoding,
            CoreHashErrorCode::Limit => HashErrorCode::Limit,
            CoreHashErrorCode::Digested | CoreHashErrorCode::Closed => HashErrorCode::Digested,
            CoreHashErrorCode::TooManyObjects => HashErrorCode::TooManyObjects,
            CoreHashErrorCode::Internal | _ => HashErrorCode::Internal,
        };
        Self::new(code)
    }
}

/// Thin wrapper around [`CoreDigestState`] that records test-only worker
/// placement probes on `hmac` key construction and `update`.
struct DigestState(CoreDigestState);

impl DigestState {
    fn hash(algorithm: Algorithm) -> Self {
        Self(CoreDigestState::hash(algorithm))
    }

    fn hmac(algorithm: Algorithm, key: &[u8]) -> Result<Self, HashError> {
        #[cfg(feature = "test-support")]
        record_hmac_key_placement();
        Ok(Self(CoreDigestState::hmac(algorithm, key)?))
    }

    fn remaining(&self) -> usize {
        self.0.remaining()
    }

    fn update(&mut self, data: &[u8]) -> Result<(), HashError> {
        #[cfg(feature = "test-support")]
        record_update_placement();
        Ok(self.0.update(data)?)
    }

    fn finalize(self) -> Vec<u8> {
        self.0.finalize()
    }
}

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
        "vael::crypto::Digest".into()
    }
}

struct AsyncDigestJob {
    handle: RefCell<Option<JoinHandle<Vec<u8>>>>,
}

impl Drop for AsyncDigestJob {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.get_mut().take() {
            handle.abort();
        }
    }
}

impl Resource for AsyncDigestJob {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "vael::crypto::AsyncDigest".into()
    }
}

struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct WorkerOwned {
    data: Vec<u8>,
    key: Vec<u8>,
    _reservation: PendingReservation,
}

impl WorkerOwned {
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

#[derive(Default)]
struct BudgetState {
    pending_jobs: usize,
    pending_bytes: usize,
    open_digests: usize,
}

#[derive(Default)]
pub(crate) struct Budget {
    state: Mutex<BudgetState>,
}

impl Budget {
    fn lock(&self) -> MutexGuard<'_, BudgetState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

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

    pub(crate) fn reserve_pending_busy(
        self: &Arc<Self>,
        bytes: usize,
    ) -> Option<PendingReservation> {
        self.reserve_pending(bytes).ok()
    }

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

struct OpenReservation {
    budget: Arc<Budget>,
}

impl Drop for OpenReservation {
    fn drop(&mut self) {
        let mut state = self.budget.lock();
        state.open_digests = state.open_digests.saturating_sub(1);
    }
}

pub(crate) struct CryptoExecutor {
    handle: Option<Handle>,
    budget: Arc<Budget>,
    #[cfg(feature = "test-support")]
    worker_placement: Arc<AtomicU32>,
    #[cfg(feature = "test-support")]
    update_placement: Arc<AtomicU32>,
    #[cfg(feature = "test-support")]
    hmac_key_placement: Arc<AtomicU32>,
    #[cfg(feature = "test-support")]
    pbkdf2_placement: Arc<AtomicU32>,
    #[cfg(feature = "test-support")]
    bcrypt_placement: Arc<AtomicU32>,
    #[cfg(feature = "test-support")]
    shamir_placement: Arc<AtomicU32>,
    #[cfg(feature = "test-support")]
    isolate_thread: ThreadId,
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

    pub(crate) fn has_handle(&self) -> bool {
        self.handle.is_some()
    }

    #[cfg(feature = "test-support")]
    pub(crate) fn note_join_started(&self) {
        if let Some(gate) = &self.cancellation_gate {
            gate.join_started();
        }
    }

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

/// Test-only gate that holds a real blocking hash worker after it starts.
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

    /// How many workers have reached the gate.
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

fn checked_string<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::String>, HashError> {
    if !value.is_string() {
        return Err(HashError::new(HashErrorCode::Type));
    }
    v8::Local::<v8::String>::try_from(value).map_err(|_| HashError::new(HashErrorCode::Type))
}

fn string_bytes(
    scope: &v8::PinScope<'_, '_>,
    string: v8::Local<'_, v8::String>,
    limit: usize,
) -> Result<Vec<u8>, HashError> {
    let length = string.utf8_length(scope);
    if length > limit {
        return Err(HashError::new(HashErrorCode::Limit));
    }
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

fn algorithm_from(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Algorithm, HashError> {
    let string = checked_string(value)?;
    if string.length() > 16 {
        return Err(HashError::new(HashErrorCode::Algorithm));
    }
    let name = string.to_rust_string_lossy(scope);
    Algorithm::parse(&name).ok_or_else(|| HashError::new(HashErrorCode::Algorithm))
}

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
                Some("VAEL_CRYPTO_LIMIT"),
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
                    Some("VAEL_CRYPTO_LIMIT"),
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
fn op_vael_crypto_hash_create<'s, 'i>(
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

#[op2(fast)]
fn op_vael_crypto_hash_release(state: &mut OpState, #[smi] id: ResourceId) {
    let _ = state.resource_table.take::<DigestResource>(id);
}

#[op2]
fn op_vael_crypto_hmac_create<'s, 'i>(
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
fn op_vael_crypto_hash_update<'s, 'i>(
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
fn op_vael_crypto_hash_digest<'s, 'i>(
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
        let _ = state.resource_table.take::<DigestResource>(id);
        let bytes = encode_digest(&digest.finalize(), encoding);
        Ok((encoding, bytes))
    })();
    outcome_digest(scope, result)
}

#[allow(clippy::too_many_arguments)]
#[op2]
fn op_vael_crypto_async_submit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    algorithm: v8::Local<'s, v8::Value>,
    kind: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
    data: v8::Local<'s, v8::Value>,
    encoding: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
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
        #[cfg(feature = "test-support")]
        let _isolate_probe = ProbeGuard::install(Some(state.borrow::<CryptoExecutor>().probe()));

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

        let job = WorkerOwned {
            data,
            key,
            _reservation: reservation,
        };
        let join = state.borrow::<CryptoExecutor>().spawn_blocking(move || {
            let job = job;
            let result = job.run(algorithm, kind, encoding);
            drop(job);
            result
        });
        Ok(state.resource_table.add(AsyncDigestJob {
            handle: RefCell::new(Some(join)),
        }))
    })();
    outcome_id(scope, result)
}

#[op2]
#[buffer]
pub(crate) async fn op_vael_crypto_async_join(
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
    let _abort = AbortOnDrop(handle.abort_handle());
    handle.await.unwrap_or_default()
}

deno_core::extension!(
    vaeljs_crypto_hash,
    ops = [
        op_vael_crypto_hash_create,
        op_vael_crypto_hmac_create,
        op_vael_crypto_hash_release,
        op_vael_crypto_hash_update,
        op_vael_crypto_hash_digest,
        op_vael_crypto_async_submit,
        op_vael_crypto_async_join,
        password::op_vael_crypto_pbkdf2,
        password::op_vael_crypto_pbkdf2_async_submit,
        password::op_vael_crypto_password_async_join,
        password::op_vael_crypto_bcrypt_hash,
        password::op_vael_crypto_bcrypt_hash_async_submit,
        password::op_vael_crypto_bcrypt_verify,
        password::op_vael_crypto_bcrypt_verify_async_submit
    ],
    state = |state| {
        state.put(CryptoExecutor::capture());
    },
    docs = "SHA-2, HMAC, PBKDF2, and bcrypt ops behind `Vael.getBuiltInModule(\"vael:crypto\")`."
);

#[cfg(feature = "test-support")]
#[op2]
#[string]
fn op_vael_crypto_test_worker_placement(state: &mut OpState) -> String {
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
fn op_vael_crypto_test_shamir_placement(state: &mut OpState) -> String {
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
fn op_vael_crypto_test_password_placement(state: &mut OpState) -> String {
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
    vaeljs_crypto_hash_test,
    ops = [
        op_vael_crypto_test_worker_placement,
        op_vael_crypto_test_password_placement,
        op_vael_crypto_test_shamir_placement
    ],
    docs = "Test-only worker-placement probes for `vael:crypto`."
);

#[cfg(feature = "test-support")]
pub(crate) fn test_support_extension() -> deno_core::Extension {
    vaeljs_crypto_hash_test::init()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test: Tokio blocking-pool pending-byte and pending-job budget enforcement.
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

    // Unit test: pending-job cap enforcement for zero-byte jobs.
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

    // Unit test: reservation RAII release on error and panic unwind.
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

    // Unit test: open-object cap enforcement on runtime Budget.
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

    // Unit test: WorkerOwned field-order drop guard for reservation lifetime.
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
        assert_eq!(budget.lock().pending_jobs, 1);
        assert_eq!(budget.lock().pending_bytes, 8);
        drop(job);
        let state = budget.lock();
        assert_eq!(state.pending_jobs, 0);
        assert_eq!(state.pending_bytes, 0);
        drop(state);

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

    // Unit test: JS adapter error code and class mapping.
    #[test]
    fn errors_have_stable_codes_and_no_input_bytes() {
        assert_eq!(
            HashError::new(HashErrorCode::Algorithm).code().as_str(),
            Some("VAEL_CRYPTO_ALGORITHM")
        );
        assert_eq!(
            HashError::new(HashErrorCode::Encoding).code().as_str(),
            Some("VAEL_CRYPTO_ENCODING")
        );
        assert_eq!(
            HashError::new(HashErrorCode::Digested).code().as_str(),
            Some("VAEL_CRYPTO_DIGESTED")
        );
        assert_eq!(
            HashError::new(HashErrorCode::Busy).code().as_str(),
            Some("VAEL_CRYPTO_BUSY")
        );
        assert_eq!(
            HashError::new(HashErrorCode::NoExecutor).code().as_str(),
            Some("VAEL_CRYPTO_NO_EXECUTOR")
        );
        assert_eq!(
            HashError::new(HashErrorCode::TooManyObjects)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_TOO_MANY_OBJECTS")
        );
        assert_eq!(
            HashError::new(HashErrorCode::Internal).code().as_str(),
            Some("VAEL_CRYPTO_INTERNAL")
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

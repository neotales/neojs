//! PBKDF2 and bcrypt password hashing ops for `vael:crypto`.
//!
//! Pure PBKDF2 and bcrypt logic, bounds, and zeroizing copies live in
//! [`vaeljs_crypto_core::password`]. This module retains V8 secret input copying,
//! Tokio blocking-pool worker dispatch, shared budget reservations, and
//! `deno_core` ops.

use std::cell::RefCell;
use std::rc::Rc;

use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_core::v8;
use tokio::runtime::Handle;
use tokio::runtime::RuntimeFlavor;
use tokio::task::JoinHandle;
use vaeljs_crypto_core::password::{
    BCRYPT_HASH_BYTES, DEFAULT_BCRYPT_COST, MAX_BCRYPT_COST, MAX_BCRYPT_PASSWORD_BYTES,
    MAX_PBKDF2_INPUT_BYTES, MAX_PBKDF2_ITERATIONS, MAX_PBKDF2_OUTPUT_BYTES, MIN_BCRYPT_COST,
    PasswordError as CorePasswordError, PasswordErrorCode as CorePasswordErrorCode, bcrypt_hash,
    bcrypt_verify, is_valid_bcrypt_hash, pbkdf2_zeroizing,
};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PasswordErrorCode {
    Algorithm,
    Limit,
    Busy,
    NoExecutor,
    Random,
    Detached,
    Internal,
    Type,
    Range,
}

impl PasswordErrorCode {
    fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Algorithm => "VAEL_CRYPTO_ALGORITHM",
            Self::Limit => "VAEL_CRYPTO_LIMIT",
            Self::Busy => "VAEL_CRYPTO_BUSY",
            Self::NoExecutor => "VAEL_CRYPTO_NO_EXECUTOR",
            Self::Random => "VAEL_CRYPTO_RANDOM",
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

impl From<CorePasswordError> for PasswordError {
    fn from(error: CorePasswordError) -> Self {
        let code = match error.code() {
            CorePasswordErrorCode::Algorithm => PasswordErrorCode::Algorithm,
            CorePasswordErrorCode::Limit => PasswordErrorCode::Limit,
            CorePasswordErrorCode::Random => PasswordErrorCode::Random,
            CorePasswordErrorCode::Range => PasswordErrorCode::Range,
            CorePasswordErrorCode::Internal | _ => PasswordErrorCode::Internal,
        };
        Self { code }
    }
}

const WORKER_OK: u8 = 0;
const WORKER_RANDOM: u8 = 1;
const WORKER_LIMIT: u8 = 2;
const WORKER_INTERNAL: u8 = 3;

fn worker_success(payload: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(payload.len() + 1);
    output.push(WORKER_OK);
    output.extend_from_slice(payload);
    output
}

fn worker_error(error: PasswordError) -> Vec<u8> {
    let tag = match error.code() {
        PasswordErrorCode::Random => WORKER_RANDOM,
        PasswordErrorCode::Limit => WORKER_LIMIT,
        _ => WORKER_INTERNAL,
    };
    vec![tag]
}

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

fn read_secret_input(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>, PasswordError> {
    if value.is_string() {
        let string = super::checked_string(value)?;
        let length = string.utf8_length(scope);
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

fn derive(
    algorithm: Algorithm,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> Result<Zeroizing<Vec<u8>>, PasswordError> {
    #[cfg(feature = "test-support")]
    super::record_pbkdf2_placement();
    Ok(pbkdf2_zeroizing(
        algorithm, password, salt, iterations, length,
    )?)
}

fn hash(password: &[u8], cost: u32) -> Result<String, PasswordError> {
    #[cfg(feature = "test-support")]
    super::record_bcrypt_placement();
    Ok(bcrypt_hash(password, cost)?)
}

fn verify(password: &[u8], hash: &str) -> bool {
    if password.len() > MAX_BCRYPT_PASSWORD_BYTES || !is_valid_bcrypt_hash(hash) {
        return false;
    }
    #[cfg(feature = "test-support")]
    super::record_bcrypt_placement();
    bcrypt_verify(password, hash)
}

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
    result: Result<Zeroizing<Vec<u8>>, PasswordError>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    match result {
        Ok(bytes) => {
            let v8_bytes = bytes_to_v8(scope, bytes.to_vec());
            drop(bytes);
            match v8_bytes {
                Some(value) => {
                    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                    set(scope, object, "value", value.into());
                }
                None => set_failure(scope, object, PasswordError::new(PasswordErrorCode::Limit)),
            }
        }
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

fn read_cost(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<u32, PasswordError> {
    if value.is_undefined() {
        return Ok(DEFAULT_BCRYPT_COST);
    }
    read_count(scope, value, MIN_BCRYPT_COST, MAX_BCRYPT_COST)
}

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
    ) -> Result<Zeroizing<Vec<u8>>, PasswordError> {
        derive(
            algorithm,
            self.password.as_slice(),
            self.salt.as_slice(),
            iterations,
            length,
        )
    }
}

struct BcryptHashJob {
    password: Zeroizing<Vec<u8>>,
    _reservation: PendingReservation,
}

impl BcryptHashJob {
    fn run(self, cost: u32) -> Result<String, PasswordError> {
        hash(self.password.as_slice(), cost)
    }
}

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

struct AsyncPasswordJob {
    handle: RefCell<Option<JoinHandle<Vec<u8>>>>,
}

impl Drop for AsyncPasswordJob {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.get_mut().take() {
            handle.abort();
        }
    }
}

impl Resource for AsyncPasswordJob {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "vael::crypto::AsyncPassword".into()
    }
}

#[op2]
pub(super) fn op_vael_crypto_pbkdf2<'s, 'i>(
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
pub(super) fn op_vael_crypto_pbkdf2_async_submit<'s, 'i>(
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
        #[cfg(feature = "test-support")]
        let _isolate_probe = ProbeGuard::install(Some(state.borrow::<CryptoExecutor>().probe()));

        let password_length = input_length(scope, password, MAX_PBKDF2_INPUT_BYTES)?;
        let salt_length = input_length(scope, salt, MAX_PBKDF2_INPUT_BYTES)?;
        let pending_bytes = password_length
            .checked_add(salt_length)
            .ok_or_else(|| PasswordError::new(PasswordErrorCode::Busy))?;
        let reservation = budget.reserve_pending(pending_bytes)?;
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
                Ok(result) => worker_success(&result),
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
pub(super) fn op_vael_crypto_bcrypt_hash<'s, 'i>(
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
pub(super) fn op_vael_crypto_bcrypt_hash_async_submit<'s, 'i>(
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
                Ok(hash) => worker_success(hash.as_bytes()),
                Err(error) => worker_error(error),
            });
        Ok(state.resource_table.add(AsyncPasswordJob {
            handle: RefCell::new(Some(join)),
        }))
    })();
    outcome_id(scope, result)
}

#[op2]
pub(super) fn op_vael_crypto_bcrypt_verify<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    password: v8::Local<'s, v8::Value>,
    hash: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        let hash = checked_string(hash)?;
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
pub(super) fn op_vael_crypto_bcrypt_verify_async_submit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    password: v8::Local<'s, v8::Value>,
    hash: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = (|| {
        check_executor()?;
        let hash = checked_string(hash)?;
        let password_length = input_length(scope, password, usize::MAX)?;
        if hash.length() != BCRYPT_HASH_BYTES || password_length > MAX_BCRYPT_PASSWORD_BYTES {
            return Ok(VerifyAsync::Immediate(false));
        }
        let hash = hash.to_rust_string_lossy(scope);
        if !is_valid_bcrypt_hash(&hash) {
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
            .spawn_blocking(move || worker_success(&[u8::from(job.run())]));
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

#[op2]
#[buffer]
pub(super) async fn op_vael_crypto_password_async_join(
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
    let _abort = AbortOnDrop(handle.abort_handle());
    handle.await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::MAX_PENDING_ASYNC_BYTES;
    use super::super::MAX_PENDING_ASYNC_JOBS;
    use super::*;

    // Unit test: zeroizing copy guard on short copies and over-limit inputs.
    #[test]
    fn zeroizing_copy_rejects_a_short_copy_and_an_over_limit_length() {
        let error = copy_zeroizing(8, 8, PasswordErrorCode::Detached, |_| 7).unwrap_err();
        assert_eq!(error.code(), PasswordErrorCode::Detached);

        let error = copy_zeroizing(9, 8, PasswordErrorCode::Detached, |_| 9).unwrap_err();
        assert_eq!(error.code(), PasswordErrorCode::Limit);

        let bytes = copy_zeroizing(4, 4, PasswordErrorCode::Detached, |destination| {
            destination.copy_from_slice(b"test");
            4
        })
        .unwrap();
        assert_eq!(bytes.as_slice(), b"test");
    }

    // Unit test: JS adapter error code and class mapping.
    #[test]
    fn error_codes_are_stable_and_never_echo_input() {
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Algorithm)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_ALGORITHM")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Limit).code().as_str(),
            Some("VAEL_CRYPTO_LIMIT")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Busy).code().as_str(),
            Some("VAEL_CRYPTO_BUSY")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::NoExecutor)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_NO_EXECUTOR")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Random)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_RANDOM")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Detached)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_DETACHED")
        );
        assert_eq!(
            PasswordError::new(PasswordErrorCode::Internal)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_INTERNAL")
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

    // Unit test: password job admission into shared Budget.
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

    // Unit test: documented budget/bound constant values.
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

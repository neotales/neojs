//! Shamir secret sharing over GF(256) ops for `vael:crypto`.
//!
//! Pure GF(256) polynomial evaluation, Lagrange interpolation, CSPRNG
//! coefficient generation, and packed splitting live in
//! [`vaeljs_crypto_core::shamir`]. This module retains V8 share-plan validation,
//! `SharedArrayBuffer` rejection, zeroizing V8 backing-store deleters, Tokio
//! blocking-pool jobs, and `deno_core` ops.

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
use vaeljs_crypto_core::shamir::{
    MAX_SHAMIR_PACKED_BYTES, MAX_SHAMIR_SECRET_BYTES, MAX_SHAMIR_SHARES, ShamirError as CoreError,
    ShamirErrorCode as CoreErrorCode, ShamirShare, check_total, noise_size, parse_split_payload,
    shamir_combine, shamir_split, shamir_split_packed, validate_split,
};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShamirErrorCode {
    Limit,
    Random,
    DuplicateIndex,
    LengthMismatch,
    Detached,
    Shared,
    Type,
    Range,
    Busy,
    NoExecutor,
    Internal,
}

impl ShamirErrorCode {
    fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Limit => "VAEL_CRYPTO_LIMIT",
            Self::Random => "VAEL_CRYPTO_RANDOM",
            Self::DuplicateIndex => "VAEL_CRYPTO_SHAMIR_DUPLICATE",
            Self::LengthMismatch => "VAEL_CRYPTO_SHAMIR_LENGTH",
            Self::Detached => "VAEL_CRYPTO_DETACHED",
            Self::Shared => "VAEL_CRYPTO_SHAMIR_SHARED",
            Self::Busy => "VAEL_CRYPTO_BUSY",
            Self::NoExecutor => "VAEL_CRYPTO_NO_EXECUTOR",
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

impl From<CoreError> for ShamirError {
    fn from(error: CoreError) -> Self {
        let code = match error.code() {
            CoreErrorCode::Limit => ShamirErrorCode::Limit,
            CoreErrorCode::Random => ShamirErrorCode::Random,
            CoreErrorCode::DuplicateIndex => ShamirErrorCode::DuplicateIndex,
            CoreErrorCode::LengthMismatch => ShamirErrorCode::LengthMismatch,
            CoreErrorCode::Range => ShamirErrorCode::Range,
            CoreErrorCode::Internal | _ => ShamirErrorCode::Internal,
        };
        Self::new(code)
    }
}

struct SharePlan<'s> {
    index: u8,
    view: View<'s>,
}

#[cfg(feature = "test-support")]
pub(crate) fn fail_next_shamir_random() {
    vaeljs_crypto_core::test_support::fail_next_shamir_random();
}

fn zeroed_vec(length: usize) -> Result<Vec<u8>, ShamirError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    bytes.resize(length, 0);
    Ok(bytes)
}

fn copied_slice(source: &[u8]) -> Result<Vec<u8>, ShamirError> {
    let mut bytes = zeroed_vec(source.len())?;
    bytes.copy_from_slice(source);
    Ok(bytes)
}

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

fn split(
    secret: &[u8],
    threshold: usize,
    share_count: usize,
) -> Result<Vec<ShamirShare>, ShamirError> {
    validate_split(secret.len(), threshold, share_count)?;
    #[cfg(feature = "test-support")]
    record_shamir_placement();
    Ok(shamir_split(secret, threshold, share_count)?)
}

fn combine(shares: &[ShamirShare]) -> Result<Zeroizing<Vec<u8>>, ShamirError> {
    #[cfg(feature = "test-support")]
    record_shamir_placement();
    Ok(shamir_combine(shares)?)
}

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

fn read_optional_threshold(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Option<usize>, ShamirError> {
    if value.is_undefined() {
        return Ok(None);
    }
    read_integer(scope, value, 2, MAX_SHAMIR_SHARES as u32).map(Some)
}

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

fn copy_shares(plans: &[SharePlan<'_>]) -> Result<Vec<ShamirShare>, ShamirError> {
    let mut shares = Vec::new();
    shares
        .try_reserve_exact(plans.len())
        .map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    for plan in plans {
        shares.push(ShamirShare::new(plan.index, copy_view(&plan.view)?));
    }
    Ok(shares)
}

fn split_packed(
    secret: &[u8],
    threshold: usize,
    share_count: usize,
) -> Result<SensitiveVec, ShamirError> {
    validate_split(secret.len(), threshold, share_count)?;
    #[cfg(feature = "test-support")]
    record_shamir_placement();
    let mut packed = shamir_split_packed(secret, threshold, share_count)?;
    Ok(SensitiveVec::new(std::mem::take(&mut *packed)))
}

#[cfg(feature = "test-support")]
static ZEROIZED_V8_BUFFERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "test-support")]
static ZEROIZED_WORKER_RESULTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(feature = "test-support")]
pub(crate) fn zeroized_v8_buffers() -> usize {
    ZEROIZED_V8_BUFFERS.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(feature = "test-support")]
pub(crate) fn zeroized_worker_results() -> usize {
    ZEROIZED_WORKER_RESULTS.load(std::sync::atomic::Ordering::Relaxed)
}

struct VecOwner {
    bytes: Vec<u8>,
    #[cfg(any(test, feature = "test-support"))]
    capacity_initialized: bool,
}

impl VecOwner {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            #[cfg(any(test, feature = "test-support"))]
            capacity_initialized: false,
        }
    }

    #[cfg(test)]
    fn new_with_initialized_capacity(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            capacity_initialized: true,
        }
    }
}

unsafe extern "C" fn wipe_vec_backing_store(
    _data: *mut c_void,
    _byte_length: usize,
    deleter_data: *mut c_void,
) {
    if deleter_data.is_null() {
        return;
    }
    // SAFETY: `deleter_data` is the `Box<VecOwner>` handed to V8, and V8 calls
    // this deleter exactly once.
    let mut owner = unsafe { Box::from_raw(deleter_data.cast::<VecOwner>()) };
    let initialized = owner.bytes.len();
    owner.bytes.zeroize();
    #[cfg(any(test, debug_assertions, feature = "test-support"))]
    {
        let all = unsafe { std::slice::from_raw_parts(owner.bytes.as_ptr(), initialized) };
        assert!(
            all.iter().all(|byte| *byte == 0),
            "the initialized vector bytes were not zeroized"
        );
        #[cfg(any(test, feature = "test-support"))]
        if owner.capacity_initialized {
            let all =
                unsafe { std::slice::from_raw_parts(owner.bytes.as_ptr(), owner.bytes.capacity()) };
            assert!(
                all.iter().all(|byte| *byte == 0),
                "the fully initialized vector capacity was not zeroized"
            );
        }
    }
    #[cfg(feature = "test-support")]
    ZEROIZED_V8_BUFFERS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

fn zeroizing_bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    mut bytes: Zeroizing<Vec<u8>>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let length = bytes.len();
    if length == 0 {
        let mut empty = std::mem::take(&mut *bytes);
        empty.zeroize();
        drop(empty);
        return bytes_to_v8(scope, Vec::new());
    }
    let vec = std::mem::take(&mut *bytes);
    let original_pointer = vec.as_ptr();
    let mut owner = Box::new(VecOwner::new(vec));
    debug_assert_eq!(owner.bytes.len(), length);
    debug_assert_eq!(owner.bytes.as_ptr(), original_pointer);
    let data = owner.bytes.as_mut_ptr().cast::<c_void>();
    let context = Box::into_raw(owner).cast::<c_void>();
    // SAFETY: `data` points into the vector allocation owned by `context`.
    let store = unsafe {
        v8::ArrayBuffer::new_backing_store_from_ptr(data, length, wipe_vec_backing_store, context)
    };
    let store = store.make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length)
}

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

fn shares_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    shares: Vec<ShamirShare>,
) -> Result<v8::Local<'s, v8::Array>, ShamirError> {
    let count =
        i32::try_from(shares.len()).map_err(|_| ShamirError::new(ShamirErrorCode::Limit))?;
    let array = v8::Array::new(scope, count);
    for (position, share) in shares.into_iter().enumerate() {
        let index = share.index();
        let value = zeroizing_bytes_to_v8(scope, share.into_value())
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
    }
    Ok(array)
}

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

fn success<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
    set(scope, object, "value", value);
    object.into()
}

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

fn outcome_secret<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    result: Result<Zeroizing<Vec<u8>>, ShamirError>,
) -> v8::Local<'s, v8::Value> {
    match result {
        Ok(bytes) => match zeroizing_bytes_to_v8(scope, bytes) {
            Some(value) => success(scope, value.into()),
            None => failure(scope, ShamirError::new(ShamirErrorCode::Limit)),
        },
        Err(error) => failure(scope, error),
    }
}

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

#[op2]
fn op_vael_crypto_shamir_split<'s, 'i>(
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

#[op2]
fn op_vael_crypto_shamir_split_async_submit<'s, 'i>(
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

#[op2]
fn op_vael_crypto_shamir_combine<'s, 'i>(
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

#[op2]
fn op_vael_crypto_shamir_combine_async_submit<'s, 'i>(
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

fn validate_plan_total(plans: &[SharePlan<'_>]) -> Result<usize, ShamirError> {
    let length = validate_plan(plans)?;
    length
        .checked_mul(plans.len())
        .ok_or_else(|| ShamirError::new(ShamirErrorCode::Limit))
}

struct SensitiveVec {
    bytes: Option<Vec<u8>>,
    #[cfg(any(test, feature = "test-support"))]
    capacity_initialized: bool,
}

impl SensitiveVec {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Some(bytes),
            #[cfg(any(test, feature = "test-support"))]
            capacity_initialized: false,
        }
    }

    fn empty() -> Self {
        Self {
            bytes: None,
            #[cfg(any(test, feature = "test-support"))]
            capacity_initialized: false,
        }
    }

    #[cfg(test)]
    fn new_with_initialized_capacity(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Some(bytes),
            capacity_initialized: true,
        }
    }

    fn take(&mut self) -> Option<Vec<u8>> {
        self.bytes.take()
    }

    fn as_slice(&self) -> &[u8] {
        self.bytes.as_deref().unwrap_or(&[])
    }

    fn len(&self) -> usize {
        self.bytes.as_ref().map_or(0, Vec::len)
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for SensitiveVec {
    fn drop(&mut self) {
        if let Some(mut bytes) = self.bytes.take() {
            let initialized = bytes.len();
            bytes.zeroize();
            #[cfg(any(test, debug_assertions, feature = "test-support"))]
            {
                let all = unsafe { std::slice::from_raw_parts(bytes.as_ptr(), initialized) };
                assert!(
                    all.iter().all(|byte| *byte == 0),
                    "sensitive bytes were not cleared before drop"
                );
                #[cfg(any(test, feature = "test-support"))]
                if self.capacity_initialized {
                    let all =
                        unsafe { std::slice::from_raw_parts(bytes.as_ptr(), bytes.capacity()) };
                    assert!(
                        all.iter().all(|byte| *byte == 0),
                        "fully initialized sensitive capacity was not cleared"
                    );
                }
            }
            #[cfg(feature = "test-support")]
            ZEROIZED_WORKER_RESULTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

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

struct CombineJob {
    shares: Vec<ShamirShare>,
    _reservation: PendingReservation,
}

impl CombineJob {
    fn run(self) -> WorkerResult {
        match combine(&self.shares) {
            Ok(mut secret) => {
                WorkerResult::success(SensitiveVec::new(std::mem::take(&mut *secret)))
            }
            Err(error) => WorkerResult::error(error),
        }
    }
}

struct AsyncShamirJob {
    handle: RefCell<Option<JoinHandle<WorkerResult>>>,
    result: RefCell<Option<WorkerResult>>,
}

impl Drop for AsyncShamirJob {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.get_mut().take() {
            handle.abort();
        }
    }
}

impl Resource for AsyncShamirJob {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "vael::crypto::AsyncShamir".into()
    }
}

struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[op2]
#[buffer]
async fn op_vael_crypto_shamir_async_join(
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
    }
    if status == 0 {
        Vec::new()
    } else {
        vec![status]
    }
}

#[op2]
fn op_vael_crypto_shamir_take_result<'s, 'i>(
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
    vaeljs_crypto_shamir,
    ops = [
        op_vael_crypto_shamir_split,
        op_vael_crypto_shamir_split_async_submit,
        op_vael_crypto_shamir_combine,
        op_vael_crypto_shamir_combine_async_submit,
        op_vael_crypto_shamir_async_join,
        op_vael_crypto_shamir_take_result
    ],
    docs = "GF(256) Shamir secret sharing ops behind `Vael.getBuiltInModule(\"vael:crypto\")`."
);

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test: SensitiveVec Drop full-capacity zeroization boundary check.
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
        drop(SensitiveVec::new_with_initialized_capacity(vec));
        #[cfg(feature = "test-support")]
        assert_eq!(
            zeroized_worker_results(),
            before + 1,
            "the clear-boundary counter did not advance after a wiped drop"
        );
    }

    // Unit test: V8 backing-store deleter full-capacity zeroization check.
    #[test]
    fn deleter_wipes_excess_capacity_from_the_original_vec_context() {
        let mut vec = Vec::with_capacity(32);
        vec.extend_from_slice(b"secret");
        let length = vec.len();
        let capacity = vec.capacity();
        assert!(capacity > length);
        unsafe {
            std::ptr::write_bytes(vec.as_mut_ptr().add(length), 0xaa, capacity - length);
        }
        let data = vec.as_mut_ptr().cast::<c_void>();
        let context =
            Box::into_raw(Box::new(VecOwner::new_with_initialized_capacity(vec))).cast::<c_void>();
        // SAFETY: `data`/`context` match the layout the deleter expects.
        unsafe {
            wipe_vec_backing_store(data, length, context);
        }
    }

    // Unit test: VecOwner capacity preservation and wipe check.
    #[test]
    fn vec_owner_preserves_and_wipes_the_original_capacity() {
        let mut vec = Vec::with_capacity(32);
        vec.extend_from_slice(b"secret");
        assert!(vec.capacity() > vec.len());
        let pointer = vec.as_ptr();
        let capacity = vec.capacity();
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

    // Unit test: async worker pending-byte accounting exact match.
    #[test]
    fn async_pending_bytes_match_worker_peak() {
        assert_eq!(split_pending_bytes(1000, 3, 5).unwrap(), 8013);
        assert_eq!(split_pending_bytes(100, 255, 255).unwrap(), 51263);
        assert_eq!(combine_pending_bytes(5000, 1000, 5).unwrap(), 6005);
    }
}

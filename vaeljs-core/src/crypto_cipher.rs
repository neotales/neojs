//! Bounded, stateful AES-256-CBC and AES-256-GCM cipher ops for `vael:crypto`.
//!
//! Pure cipher state management, padding, AEAD tag handling, and per-runtime
//! buffer/open-object budgets live in [`vaeljs_crypto_core::cipher`]. This
//! module owns V8 typed-array/options validation, `deno_core::Resource`
//! lifetime, and host op serialization.

use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_core::v8;
use vaeljs_crypto_core::cipher::{
    CipherAlgorithm as Algorithm, CipherBudget, CipherDirection as Direction,
    CipherError as CoreCipherError, CipherErrorCode as CoreCipherErrorCode, CipherLimits,
    CipherState, OpenReservation,
};

use crate::crypto::CryptoError;
use crate::crypto::CryptoErrorCode;
use crate::crypto::KEY_LEN;
use crate::crypto::TAG_LEN;
use crate::crypto::bytes_to_v8;
use crate::crypto::checked_view;
use crate::crypto::copy_view;
use crate::crypto::response;
use crate::crypto::set;
use crate::crypto::set_failure_parts;

/// Stable machine-readable cipher failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CipherErrorCode {
    Algorithm,
    Key,
    Iv,
    Tag,
    MissingTag,
    Authentication,
    Padding,
    State,
    Closed,
    Unsupported,
    Limit,
    TooManyObjects,
    Busy,
    Detached,
    Internal,
    Type,
    Options,
    Range,
}

impl CipherErrorCode {
    fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::Algorithm => "VAEL_CRYPTO_ALGORITHM",
            Self::Key => "VAEL_CRYPTO_KEY",
            Self::Iv => "VAEL_CRYPTO_IV",
            Self::Tag | Self::MissingTag => "VAEL_CRYPTO_TAG",
            Self::Authentication => "VAEL_CRYPTO_AUTH",
            Self::Padding => "VAEL_CRYPTO_PADDING",
            Self::State => "VAEL_CRYPTO_STATE",
            Self::Closed => "VAEL_CRYPTO_CLOSED",
            Self::Unsupported => "VAEL_CRYPTO_UNSUPPORTED",
            Self::Limit => "VAEL_CRYPTO_LIMIT",
            Self::TooManyObjects => "VAEL_CRYPTO_TOO_MANY_OBJECTS",
            Self::Busy => "VAEL_CRYPTO_BUSY",
            Self::Detached => "VAEL_CRYPTO_DETACHED",
            Self::Internal => "VAEL_CRYPTO_INTERNAL",
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

impl From<CoreCipherError> for CipherError {
    fn from(error: CoreCipherError) -> Self {
        let code = match error.code() {
            CoreCipherErrorCode::Algorithm => CipherErrorCode::Algorithm,
            CoreCipherErrorCode::Key => CipherErrorCode::Key,
            CoreCipherErrorCode::Iv => CipherErrorCode::Iv,
            CoreCipherErrorCode::Tag => CipherErrorCode::Tag,
            CoreCipherErrorCode::MissingTag => CipherErrorCode::MissingTag,
            CoreCipherErrorCode::Authentication => CipherErrorCode::Authentication,
            CoreCipherErrorCode::Padding => CipherErrorCode::Padding,
            CoreCipherErrorCode::State => CipherErrorCode::State,
            CoreCipherErrorCode::Closed => CipherErrorCode::Closed,
            CoreCipherErrorCode::Unsupported => CipherErrorCode::Unsupported,
            CoreCipherErrorCode::Limit => CipherErrorCode::Limit,
            CoreCipherErrorCode::TooManyObjects => CipherErrorCode::TooManyObjects,
            CoreCipherErrorCode::Busy => CipherErrorCode::Busy,
            CoreCipherErrorCode::Internal | _ => CipherErrorCode::Internal,
        };
        Self::new(code)
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
        "vael::crypto::Cipher".into()
    }
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

fn plain_object_prototype<'s>(
    scope: &v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Value>, CipherError> {
    let object = v8::Object::new(scope);
    object.get_prototype(scope).ok_or_else(options_error)
}

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
fn op_vael_crypto_cipher_create<'s, 'i>(
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
fn op_vael_crypto_cipher_update<'s, 'i>(
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
        Ok(cipher.update(&bytes)?)
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_crypto_cipher_final<'s, 'i>(
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
        Ok(cipher.finalize()?)
    })();
    outcome_bytes(scope, result)
}

#[op2]
fn op_vael_crypto_cipher_set_aad<'s, 'i>(
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
        if cipher.algorithm() != Algorithm::Aes256Gcm {
            return Err(CipherError::new(CipherErrorCode::Unsupported));
        }
        let view = checked_view(scope, aad)?;
        if view.length > cipher.limits().max_aad {
            return Err(CipherError::new(CipherErrorCode::Limit));
        }
        let bytes = copy_view(&view)?;
        Ok(cipher.set_aad(bytes)?)
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_crypto_cipher_set_tag<'s, 'i>(
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
        Ok(cipher.set_tag(&bytes)?)
    })();
    outcome_unit(scope, result)
}

#[op2]
fn op_vael_crypto_cipher_get_tag<'s, 'i>(
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
fn op_vael_crypto_cipher_set_padding<'s, 'i>(
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
        Ok(cipher.set_padding(enabled)?)
    })();
    outcome_unit(scope, result)
}

/// Drop one cipher object's native state explicitly.
#[op2(fast)]
fn op_vael_crypto_cipher_release(state: &mut OpState, #[smi] id: ResourceId) {
    let _ = state.resource_table.take::<CipherResource>(id);
}

deno_core::extension!(
    vaeljs_crypto_cipher,
    ops = [
        op_vael_crypto_cipher_create,
        op_vael_crypto_cipher_update,
        op_vael_crypto_cipher_final,
        op_vael_crypto_cipher_set_aad,
        op_vael_crypto_cipher_set_tag,
        op_vael_crypto_cipher_get_tag,
        op_vael_crypto_cipher_set_padding,
        op_vael_crypto_cipher_release
    ],
    state = |state| {
        state.put(CipherBudgetHandle::default());
    },
    docs = "Bounded AES-256-CBC and AES-256-GCM cipher ops behind `Vael.getBuiltInModule(\"vael:crypto\")`."
);

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test: JS adapter error code and class mapping.
    #[test]
    fn errors_have_stable_codes_and_safe_messages() {
        assert_eq!(
            CipherError::new(CipherErrorCode::Algorithm).code().as_str(),
            Some("VAEL_CRYPTO_ALGORITHM")
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::MissingTag)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_TAG")
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::Padding).code().as_str(),
            Some("VAEL_CRYPTO_PADDING")
        );
        assert_eq!(
            CipherError::new(CipherErrorCode::State).code().as_str(),
            Some("VAEL_CRYPTO_STATE")
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

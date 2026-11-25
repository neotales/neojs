//! Synchronous AES-256-GCM authenticated encryption for `vael:crypto`.
//!
//! Pure cryptographic logic (NEO1 envelope construction, associated-data
//! binding, size policy, and fallible OS CSPRNG calls) lives in
//! [`vaeljs_crypto_core::envelope`]. This module retains only the V8 typed-array
//! validation, result-object protocol, and `deno_core` ops.

use deno_core::op2;
use deno_core::v8;

pub(crate) use vaeljs_crypto_core::envelope::{
    CryptoLimits, EnvelopeError, EnvelopeErrorCode, KEY_LEN, MAX_ASSOCIATED_DATA_BYTES,
    MAX_ENVELOPE_BYTES, MAX_RANDOM_BYTES, TAG_LEN, decrypt, encrypt, generate_envelope_key,
    random_bytes,
};

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
            Self::Key => "VAEL_CRYPTO_KEY",
            Self::Format => "VAEL_CRYPTO_FORMAT",
            Self::Authentication => "VAEL_CRYPTO_AUTH",
            Self::Random => "VAEL_CRYPTO_RANDOM",
            Self::Limit => "VAEL_CRYPTO_LIMIT",
            Self::Detached => "VAEL_CRYPTO_DETACHED",
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

impl From<EnvelopeError> for CryptoError {
    fn from(error: EnvelopeError) -> Self {
        let code = match error.code() {
            EnvelopeErrorCode::Key => CryptoErrorCode::Key,
            EnvelopeErrorCode::Format => CryptoErrorCode::Format,
            EnvelopeErrorCode::Authentication => CryptoErrorCode::Authentication,
            EnvelopeErrorCode::Random => CryptoErrorCode::Random,
            EnvelopeErrorCode::Limit => CryptoErrorCode::Limit,
            _ => CryptoErrorCode::Limit,
        };
        Self::new(code)
    }
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
pub(crate) fn copy_view(view: &View<'_>) -> Result<Vec<u8>, CryptoError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(view.length)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    bytes.resize(view.length, 0);
    let written = view.view.copy_contents(&mut bytes);
    if written != view.length {
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
fn op_vael_crypto_generate_envelope_key<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
) -> v8::Local<'s, v8::Value> {
    outcome(
        scope,
        generate_envelope_key().map(Vec::from).map_err(Into::into),
    )
}

#[op2]
fn op_vael_crypto_random_bytes<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    length: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let result = random_length(scope, length).and_then(|len| random_bytes(len).map_err(Into::into));
    outcome(scope, result)
}

#[op2]
fn op_vael_crypto_encrypt<'s, 'i>(
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
        Ok(encrypt(&key, plaintext, &associated_data, limits)?)
    })();
    outcome(scope, result)
}

#[op2]
fn op_vael_crypto_decrypt<'s, 'i>(
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
        Ok(decrypt(&key, envelope, &associated_data, limits)?)
    })();
    outcome(scope, result)
}

deno_core::extension!(
    vaeljs_crypto,
    ops = [
        op_vael_crypto_generate_envelope_key,
        op_vael_crypto_random_bytes,
        op_vael_crypto_encrypt,
        op_vael_crypto_decrypt
    ],
    docs = "Synchronous AES-256-GCM ops behind `Vael.getBuiltInModule(\"vael:crypto\")`."
);

#[cfg(test)]
mod tests {
    use super::*;

    // Unit test: JS adapter error code and class mapping.
    #[test]
    fn errors_have_stable_codes_and_no_input_bytes() {
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Key).code().as_str(),
            Some("VAEL_CRYPTO_KEY")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Format).code().as_str(),
            Some("VAEL_CRYPTO_FORMAT")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Authentication)
                .code()
                .as_str(),
            Some("VAEL_CRYPTO_AUTH")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Random).code().as_str(),
            Some("VAEL_CRYPTO_RANDOM")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Limit).code().as_str(),
            Some("VAEL_CRYPTO_LIMIT")
        );
        assert_eq!(
            CryptoError::new(CryptoErrorCode::Detached).code().as_str(),
            Some("VAEL_CRYPTO_DETACHED")
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

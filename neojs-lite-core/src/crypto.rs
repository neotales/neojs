//! The QuickJS adapter for the synchronous `neo:crypto` envelope slice.
//!
//! This module is compiled only with the `crypto` feature. It registers a
//! QuickJS native module whose four exports call the shared
//! `neojs-crypto-core` backend:
//!
//! * `generateEnvelopeKey()` returns 32 CSPRNG bytes.
//! * `randomBytes(length)` returns up to 8 MiB of CSPRNG bytes.
//! * `encrypt(key, plaintext[, associatedData])` emits a NEO1 envelope.
//! * `decrypt(key, envelope[, associatedData])` opens a NEO1 envelope.
//!
//! The module is reachable only as `neo:crypto` (the `neo:` scheme is matched
//! case-insensitively). Neo built-ins always require the prefix; a bare
//! `crypto` specifier is not an alias and stays a normal bare specifier.
//!
//! There is no `Neo` object and no introspection global in Lite yet. The module
//! is a native QuickJS module, so no host function is installed on `globalThis`
//! and no JavaScript shim can be replaced before the module is imported.
//!
//! Lite has no host asynchronous work, so this slice deliberately exports no
//! Promise-returning function: `hashAsync`, `hmacAsync`, `pbkdf2Async`, bcrypt,
//! hashes, ciphers, Shamir sharing, `subtle`, and Node's `generateKey` are not
//! present. Importing one of those names is a link error, not a function that
//! rejects or a resolved Promise.
//!
//! Argument validation happens in Rust with QuickJS's internal class and buffer
//! APIs. A `Uint8Array` is recognized by its internal slot, not by
//! `Symbol.hasInstance`, a prototype, or a `byteLength` getter. The view's
//! byte range is copied once, with no JavaScript running between reading its
//! length and copying it, so a detached or out-of-bounds view is rejected
//! rather than read stale bytes. Error messages are fixed and never contain
//! key, plaintext, ciphertext, or associated-data bytes.
//!
//! # Resizable buffers
//!
//! Fixed-length views over a resizable `ArrayBuffer` are copied at their own
//! range, and growing the buffer does not change the view. Shared
//! `SharedArrayBuffer` views are copied normally.
//!
//! Two resizable-buffer cases differ from the V8 adapter, because QuickJS's C
//! buffer accessor does not expose the range V8 does. The adapter fails closed
//! rather than guessing:
//!
//! * A fixed-length view shrunk until it no longer fits reports length zero to
//!   JavaScript but has no readable byte range. It is rejected with
//!   `NEO_CRYPTO_DETACHED`; V8 encrypts it as empty.
//! * A length-tracking view (`new Uint8Array(resizableBuffer)`) reports its
//!   grown length to JavaScript, but the binding's accessor keeps returning
//!   the length it had when the view was created. The adapter copies that
//!   original in-bounds range; V8 copies the grown range. Use a fixed-length
//!   view for an exact range.

use std::ffi::CStr;
use std::ptr::NonNull;

use neojs_crypto_core::{
    CryptoError, CryptoErrorCode, CryptoLimits, KEY_LEN, MAX_ASSOCIATED_DATA_BYTES,
    MAX_RANDOM_BYTES, decrypt, encrypt, generate_envelope_key, random_bytes,
};
use rquickjs::function::{Exhaustive, Opt};
use rquickjs::module::{Declarations, Exports, ModuleDef};
use rquickjs::{Ctx, Exception, Function, Module, TypedArray, Value, qjs};

use crate::builtins::CRYPTO_SPECIFIER;

/// Register the `neo:crypto` native module in this context.
///
/// QuickJS stores the module in the context's loaded-module list, so a later
/// `import "neo:crypto"` reaches that one instance. The module is not evaluated
/// until it is imported.
pub(crate) fn register(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    Module::declare_def::<CryptoModule, _>(ctx.clone(), CRYPTO_SPECIFIER)?;
    Ok(())
}

/// The four-export native module.
struct CryptoModule;

impl ModuleDef for CryptoModule {
    fn declare<'js>(decl: &Declarations<'js>) -> rquickjs::Result<()> {
        decl.declare("generateEnvelopeKey")?
            .declare("randomBytes")?
            .declare("encrypt")?
            .declare("decrypt")?;
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> rquickjs::Result<()> {
        exports.export("generateEnvelopeKey", make_generate(ctx)?)?;
        exports.export("randomBytes", make_random(ctx)?)?;
        exports.export("encrypt", make_encrypt(ctx)?)?;
        exports.export("decrypt", make_decrypt(ctx)?)?;
        Ok(())
    }
}

/// Build `generateEnvelopeKey()`, which takes exactly zero arguments.
fn make_generate<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, _: Exhaustive| -> rquickjs::Result<TypedArray<'js, u8>> {
            let key = generate_envelope_key().map_err(|error| throw_crypto(&ctx, error))?;
            TypedArray::new(ctx, key.to_vec())
        },
    )?
    .with_name("generateEnvelopeKey")
}

/// Build `randomBytes(length)`, which takes exactly one argument.
fn make_random<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>,
         _: Exhaustive,
         length: Value<'js>|
         -> rquickjs::Result<TypedArray<'js, u8>> {
            let length = random_length(&length).map_err(|error| throw_crypto(&ctx, error))?;
            let bytes = random_bytes(length).map_err(|error| throw_crypto(&ctx, error))?;
            TypedArray::new(ctx, bytes)
        },
    )?
    .with_name("randomBytes")
}

/// Build `encrypt(key, plaintext[, associatedData])`.
fn make_encrypt<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>,
         _: Exhaustive,
         key: Value<'js>,
         plaintext: Value<'js>,
         associated_data: Opt<Value<'js>>|
         -> rquickjs::Result<TypedArray<'js, u8>> {
            let limits = CryptoLimits::default();
            let key = key_bytes(key).map_err(|error| throw_crypto(&ctx, error))?;
            let plaintext = bytes_value(plaintext, limits.max_plaintext)
                .map_err(|error| throw_crypto(&ctx, error))?;
            let associated_data = associated_data_bytes(associated_data)
                .map_err(|error| throw_crypto(&ctx, error))?;
            let envelope = encrypt(&key, plaintext, &associated_data, limits)
                .map_err(|error| throw_crypto(&ctx, error))?;
            TypedArray::new(ctx, envelope)
        },
    )?
    .with_name("encrypt")
}

/// Build `decrypt(key, envelope[, associatedData])`.
fn make_decrypt<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Function<'js>> {
    Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>,
         _: Exhaustive,
         key: Value<'js>,
         envelope: Value<'js>,
         associated_data: Opt<Value<'js>>|
         -> rquickjs::Result<TypedArray<'js, u8>> {
            let limits = CryptoLimits::default();
            let key = key_bytes(key).map_err(|error| throw_crypto(&ctx, error))?;
            let envelope =
                bytes_value(envelope, limits.max_plaintext + neojs_crypto_core::OVERHEAD)
                    .map_err(|error| throw_crypto(&ctx, error))?;
            let associated_data = associated_data_bytes(associated_data)
                .map_err(|error| throw_crypto(&ctx, error))?;
            let plaintext = decrypt(&key, envelope, &associated_data, limits)
                .map_err(|error| throw_crypto(&ctx, error))?;
            TypedArray::new(ctx, plaintext)
        },
    )?
    .with_name("decrypt")
}

/// Read the exact 32-byte key from a `Uint8Array` without invoking JavaScript.
fn key_bytes<'js>(value: Value<'js>) -> Result<Vec<u8>, CryptoError> {
    let view = typed_array(value)?;
    let Some(raw) = view.as_raw() else {
        return Err(CryptoError::new(CryptoErrorCode::Detached));
    };
    if raw.len() != KEY_LEN {
        return Err(CryptoError::new(CryptoErrorCode::Key));
    }
    copy_raw(raw)
}

/// Read a `Uint8Array` after checking its byte length against `max`.
fn bytes_value<'js>(value: Value<'js>, max: usize) -> Result<Vec<u8>, CryptoError> {
    let view = typed_array(value)?;
    let Some(raw) = view.as_raw() else {
        return Err(CryptoError::new(CryptoErrorCode::Detached));
    };
    if raw.len() > max {
        return Err(CryptoError::new(CryptoErrorCode::Limit));
    }
    copy_raw(raw)
}

/// Read the optional trailing associated-data argument.
///
/// Absent and `undefined` both mean an empty value. `null` and every other
/// non-`Uint8Array` value are a `TypeError`, checked before any byte is copied.
fn associated_data_bytes<'js>(value: Opt<Value<'js>>) -> Result<Vec<u8>, CryptoError> {
    match value.0 {
        None => Ok(Vec::new()),
        Some(value) if value.is_undefined() => Ok(Vec::new()),
        Some(value) => bytes_value(value, MAX_ASSOCIATED_DATA_BYTES),
    }
}

/// Convert a value to `Uint8Array` using the internal class check.
fn typed_array<'js>(value: Value<'js>) -> Result<TypedArray<'js, u8>, CryptoError> {
    TypedArray::<u8>::from_value(value).map_err(|_| CryptoError::new(CryptoErrorCode::Type))
}

/// Copy a view's byte range immediately, before any JavaScript can run.
fn copy_raw(raw: NonNull<[u8]>) -> Result<Vec<u8>, CryptoError> {
    let length = raw.len();
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| CryptoError::new(CryptoErrorCode::Limit))?;
    bytes.resize(length, 0);
    if length != 0 {
        // SAFETY: `raw` names `length` readable bytes in a QuickJS-owned
        // ArrayBuffer. The `TypedArray` that produced it is still alive and no
        // JavaScript runs between reading the range and this copy. `bytes` is
        // allocated for exactly `length` bytes. If another thread mutates a
        // SharedArrayBuffer concurrently, the copy is still within bounds; Lite
        // does not expose host threads.
        unsafe {
            std::ptr::copy_nonoverlapping(raw.as_ptr() as *const u8, bytes.as_mut_ptr(), length);
        }
    }
    Ok(bytes)
}

/// Validate a `randomBytes` length without invoking JavaScript hooks.
///
/// The value must already be a primitive number, so no `valueOf` or `toString`
/// conversion runs. Integers from 0 through [`MAX_RANDOM_BYTES`] are accepted;
/// everything else is a `RangeError`.
fn random_length(value: &Value<'_>) -> Result<usize, CryptoError> {
    if !value.is_number() {
        return Err(CryptoError::new(CryptoErrorCode::Type));
    }
    let Some(number) = value.as_number() else {
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

/// Throw the JavaScript exception described by a core or validation error.
///
/// Argument errors are `TypeError` or `RangeError` and carry no code. Other
/// failures are `Error` whose `message` and `code` are own data properties,
/// defined without consulting `Error.prototype` or `Object.prototype`, so an
/// inherited setter cannot intercept them.
fn throw_crypto(ctx: &Ctx<'_>, error: CryptoError) -> rquickjs::Error {
    match error.code() {
        CryptoErrorCode::Type => Exception::throw_type(ctx, error.message()),
        CryptoErrorCode::Range => Exception::throw_range(ctx, error.message()),
        code => throw_coded(
            ctx,
            code.as_str().unwrap_or("NEO_CRYPTO_ERROR"),
            error.message(),
        ),
    }
}

/// Create and throw an `Error` with own `message` and `code` properties.
///
/// The raw QuickJS calls are used instead of `Exception::from_message` because
/// `from_message` sets the message through the ordinary property path, which
/// can reach an inherited accessor. `JS_DefinePropertyValueStr` defines the
/// property on the new error object directly.
fn throw_coded(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    // SAFETY: `ctx` is live for the call. `JS_NewError` returns either a new
    // error object or `JS_EXCEPTION`; the tag check handles the latter. Both
    // property definitions consume the string values they are given. `JS_Throw`
    // consumes the error object and installs it as the pending exception.
    unsafe {
        let raw = ctx.as_raw().as_ptr();
        let error = qjs::JS_NewError(raw);
        if qjs::JS_VALUE_GET_TAG(error) == qjs::JS_TAG_EXCEPTION {
            return rquickjs::Error::Exception;
        }
        define_own_string(raw, error, c"message", message);
        define_own_string(raw, error, c"code", code);
        qjs::JS_Throw(raw, error);
        rquickjs::Error::Exception
    }
}

/// Define an own string, writable, enumerable, configurable data property.
///
/// # Safety
///
/// `raw` must be the live context that owns `object`.
unsafe fn define_own_string(
    raw: *mut qjs::JSContext,
    object: qjs::JSValue,
    name: &CStr,
    value: &str,
) {
    // SAFETY: the caller guarantees the context is live. `JS_NewStringLen`
    // copies the Rust bytes into a QuickJS string. On failure it leaves a
    // pending exception, which the caller replaces with the thrown error.
    let text =
        unsafe { qjs::JS_NewStringLen(raw, value.as_ptr().cast(), value.len() as qjs::size_t) };
    if unsafe { qjs::JS_VALUE_GET_TAG(text) } == qjs::JS_TAG_EXCEPTION {
        return;
    }
    let flags =
        (qjs::JS_PROP_WRITABLE | qjs::JS_PROP_ENUMERABLE | qjs::JS_PROP_CONFIGURABLE) as i32;
    // SAFETY: `object` is the error object from `JS_NewError`, `name` is a
    // static C string, and `JS_DefinePropertyValueStr` consumes `text`.
    unsafe {
        qjs::JS_DefinePropertyValueStr(raw, object, name.as_ptr(), text, flags);
    }
}

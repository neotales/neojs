//! Native `vael:crypto` synchronous bindings for QuickJS.
//!
//! All pure cryptographic primitives, bounded stateful `CipherTable` /
//! `DigestTable` handle tables, password hashing, and Shamir secret sharing
//! live in [`vaeljs_crypto_core`]. This module exposes the 13 synchronous
//! exports in [`EXPORTS`] on one frozen registry object per context and serves
//! those exact function objects from both `Vael.getBuiltInModule("vael:crypto")`
//! and `import ... from "vael:crypto"`.

mod cipher;
mod hash;
mod password;
mod shamir;

use rquickjs::{
    Ctx, Exception, Function, Object, Result, TypedArray, Value, function::Rest, module::ModuleDef,
    object::Property, qjs,
};
use vaeljs_crypto_core::Zeroizing;
use vaeljs_crypto_core::envelope::{
    CryptoLimits, EnvelopeError, EnvelopeErrorCode, KEY_LEN, MAX_ASSOCIATED_DATA_BYTES,
    MAX_ENVELOPE_BYTES, MAX_PLAINTEXT_BYTES, MAX_RANDOM_BYTES, decrypt as core_decrypt,
    encrypt as core_encrypt, generate_envelope_key as core_generate_envelope_key,
    random_bytes as core_random_bytes,
};

use crate::native::{freeze_native, throw_coded_error, with_uint8_array_bytes_ex};

pub(super) use self::cipher::build_cipher_proto;
pub(super) use self::hash::build_hash_proto;

/// Canonical module specifier for `vael:crypto`.
pub(super) const SPECIFIER: &str = "vael:crypto";

/// Synchronous exports exposed by `vael:crypto` in QuickJS (`Vael.engine === "vael-lite"`).
pub(super) const EXPORTS: &[&str] = &[
    "bcryptHash",
    "bcryptVerify",
    "createDecryptor",
    "createEncryptor",
    "createHash",
    "createHmac",
    "decrypt",
    "encrypt",
    "generateEnvelopeKey",
    "pbkdf2",
    "randomBytes",
    "shamirCombine",
    "shamirSplit",
];

pub(super) fn define_method<'js>(
    object: &Object<'js>,
    name: &str,
    func: Function<'js>,
    length: usize,
) -> Result<()> {
    let func = func.with_name(name)?.with_length(length)?;
    object.prop(
        name,
        Property::from(func).writable().enumerable().configurable(),
    )?;
    Ok(())
}

/// Creates the frozen public registry object for `vael:crypto`.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    define_method(
        &object,
        "bcryptHash",
        Function::new(ctx.clone(), password::bcrypt_hash)?,
        2,
    )?;
    define_method(
        &object,
        "bcryptVerify",
        Function::new(ctx.clone(), password::bcrypt_verify)?,
        2,
    )?;
    define_method(
        &object,
        "createDecryptor",
        Function::new(ctx.clone(), cipher::create_decryptor)?,
        4,
    )?;
    define_method(
        &object,
        "createEncryptor",
        Function::new(ctx.clone(), cipher::create_encryptor)?,
        4,
    )?;
    define_method(
        &object,
        "createHash",
        Function::new(ctx.clone(), hash::create_hash)?,
        1,
    )?;
    define_method(
        &object,
        "createHmac",
        Function::new(ctx.clone(), hash::create_hmac)?,
        2,
    )?;
    define_method(&object, "decrypt", Function::new(ctx.clone(), decrypt)?, 3)?;
    define_method(&object, "encrypt", Function::new(ctx.clone(), encrypt)?, 3)?;
    define_method(
        &object,
        "generateEnvelopeKey",
        Function::new(ctx.clone(), generate_envelope_key)?,
        0,
    )?;
    define_method(
        &object,
        "pbkdf2",
        Function::new(ctx.clone(), password::pbkdf2)?,
        5,
    )?;
    define_method(
        &object,
        "randomBytes",
        Function::new(ctx.clone(), random_bytes)?,
        1,
    )?;
    define_method(
        &object,
        "shamirCombine",
        Function::new(ctx.clone(), shamir::shamir_combine)?,
        2,
    )?;
    define_method(
        &object,
        "shamirSplit",
        Function::new(ctx.clone(), shamir::shamir_split)?,
        3,
    )?;
    freeze_native(ctx, &object)?;
    Ok(object)
}

/// The native QuickJS module served for `vael:crypto`.
pub(super) struct CryptoModule;

impl ModuleDef for CryptoModule {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let Some(crypto) = state.crypto.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = crypto.clone().restore(ctx)?;
        for name in EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        Ok(())
    }
}

pub(super) fn throw_crypto_error(ctx: &Ctx<'_>, code: &str, message: &str) -> rquickjs::Error {
    throw_coded_error(ctx, None, code, message)
}

pub(super) fn detached_error(ctx: &Ctx<'_>) -> rquickjs::Error {
    throw_crypto_error(
        ctx,
        "VAEL_CRYPTO_DETACHED",
        "input is detached or out of bounds",
    )
}

pub(super) fn shared_buffer_error(ctx: &Ctx<'_>) -> rquickjs::Error {
    throw_crypto_error(
        ctx,
        "VAEL_CRYPTO_SHAMIR_SHARED",
        "SharedArrayBuffer-backed input cannot be copied coherently",
    )
}

pub(super) fn limit_error(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    throw_crypto_error(ctx, "VAEL_CRYPTO_LIMIT", message)
}

fn throw_envelope_error(ctx: &Ctx<'_>, error: EnvelopeError) -> rquickjs::Error {
    let code = match error.code() {
        EnvelopeErrorCode::Key => "VAEL_CRYPTO_KEY",
        EnvelopeErrorCode::Format => "VAEL_CRYPTO_FORMAT",
        EnvelopeErrorCode::Authentication => "VAEL_CRYPTO_AUTH",
        EnvelopeErrorCode::Random => "VAEL_CRYPTO_RANDOM",
        EnvelopeErrorCode::Limit => "VAEL_CRYPTO_LIMIT",
        _ => "VAEL_CRYPTO_LIMIT",
    };
    throw_crypto_error(ctx, code, error.message())
}

pub(super) fn bytes_to_uint8_array<'js>(
    ctx: &Ctx<'js>,
    bytes: &[u8],
) -> Result<TypedArray<'js, u8>> {
    TypedArray::<u8>::new_copy(ctx.clone(), bytes)
}

pub(super) fn read_uint8_bytes<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    max_len: usize,
    limit_msg: &str,
    allow_shared: bool,
) -> Result<Zeroizing<Vec<u8>>> {
    with_uint8_array_bytes_ex(ctx, value, type_msg, detached_error, |slice, is_shared| {
        if !allow_shared && is_shared {
            return Err(shared_buffer_error(ctx));
        }
        if slice.len() > max_len {
            return Err(limit_error(ctx, limit_msg));
        }
        let mut out = Zeroizing::new(Vec::new());
        out.try_reserve_exact(slice.len())
            .map_err(|_| limit_error(ctx, limit_msg))?;
        out.extend_from_slice(slice);
        Ok(out)
    })
}

pub(super) fn read_uint8_exact<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    expected_len: usize,
    wrong_len_code: &str,
    wrong_len_msg: &str,
) -> Result<Zeroizing<Vec<u8>>> {
    with_uint8_array_bytes_ex(ctx, value, type_msg, detached_error, |slice, _is_shared| {
        if slice.len() != expected_len {
            return Err(throw_crypto_error(ctx, wrong_len_code, wrong_len_msg));
        }
        let mut out = Zeroizing::new(Vec::new());
        out.try_reserve_exact(slice.len())
            .map_err(|_| limit_error(ctx, "input exceeds the crypto size limit"))?;
        out.extend_from_slice(slice);
        Ok(out)
    })
}

struct CStringUtf16Guard(*mut qjs::JSContext, *const u16);

impl Drop for CStringUtf16Guard {
    fn drop(&mut self) {
        unsafe { qjs::JS_FreeCStringUTF16(self.0, self.1) };
    }
}

pub(super) fn with_string_utf16<'js, T>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    f: impl FnOnce(&[u16]) -> Result<T>,
) -> Result<T> {
    if !value.is_string() {
        return Err(Exception::throw_type(ctx, type_msg));
    }
    let raw = ctx.as_raw().as_ptr();
    let mut len: qjs::size_t = 0;
    let ptr = unsafe { qjs::JS_ToCStringLenUTF16(raw, &mut len, value.as_raw()) };
    if ptr.is_null() {
        if ctx.has_exception() {
            return Err(rquickjs::Error::Exception);
        }
        return Err(throw_crypto_error(
            ctx,
            "VAEL_CRYPTO_INTERNAL",
            "failed to read string",
        ));
    }
    let _guard = CStringUtf16Guard(raw, ptr);
    // SAFETY: `ptr` points to `len` UTF-16 code units valid until `_guard` drops.
    let units = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    f(units)
}

pub(super) fn utf8_len_with_replacement(units: &[u16]) -> usize {
    char::decode_utf16(units.iter().copied())
        .map(|res| match res {
            Ok(ch) => ch.len_utf8(),
            Err(_) => char::REPLACEMENT_CHARACTER.len_utf8(),
        })
        .sum()
}

pub(super) fn read_utf8_bytes<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    limit: usize,
    limit_msg: &str,
) -> Result<Zeroizing<Vec<u8>>> {
    with_string_utf16(ctx, value, type_msg, |units| {
        let byte_len = utf8_len_with_replacement(units);
        if byte_len > limit {
            return Err(limit_error(ctx, limit_msg));
        }
        let mut out = Zeroizing::new(Vec::new());
        out.try_reserve_exact(byte_len)
            .map_err(|_| limit_error(ctx, limit_msg))?;
        let mut buf = Zeroizing::new([0u8; 4]);
        for res in char::decode_utf16(units.iter().copied()) {
            let ch = res.unwrap_or(char::REPLACEMENT_CHARACTER);
            out.extend_from_slice(ch.encode_utf8(&mut *buf).as_bytes());
        }
        Ok(out)
    })
}

pub(super) fn read_ascii_or_utf8_string<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    max_units: usize,
    on_over_limit: impl FnOnce(&Ctx<'js>) -> rquickjs::Error,
) -> Result<String> {
    with_string_utf16(ctx, value, type_msg, |units| {
        if units.len() > max_units {
            return Err(on_over_limit(ctx));
        }
        let mut out = String::with_capacity(units.len());
        for res in char::decode_utf16(units.iter().copied()) {
            out.push(res.unwrap_or(char::REPLACEMENT_CHARACTER));
        }
        Ok(out)
    })
}

pub(super) fn read_integer(
    ctx: &Ctx<'_>,
    value: &Value<'_>,
    minimum: usize,
    maximum: usize,
    type_msg: &str,
    range_msg: &str,
) -> Result<usize> {
    if !value.is_number() {
        return Err(Exception::throw_type(ctx, type_msg));
    }
    let Some(number) = value.as_number() else {
        return Err(Exception::throw_type(ctx, type_msg));
    };
    if !number.is_finite()
        || number.trunc() != number
        || number < minimum as f64
        || number > maximum as f64
    {
        return Err(Exception::throw_range(ctx, range_msg));
    }
    Ok(number as usize)
}

fn generate_envelope_key<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<TypedArray<'js, u8>> {
    if !args.0.is_empty() {
        return Err(Exception::throw_type(
            &ctx,
            "generateEnvelopeKey takes no arguments",
        ));
    }
    let key =
        Zeroizing::new(core_generate_envelope_key().map_err(|e| throw_envelope_error(&ctx, e))?);
    bytes_to_uint8_array(&ctx, key.as_slice())
}

fn random_bytes<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<TypedArray<'js, u8>> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "randomBytes takes exactly one argument",
        ));
    }
    let len = read_integer(
        &ctx,
        &args.0[0],
        0,
        MAX_RANDOM_BYTES,
        "length must be a number",
        "length must be a non-negative safe integer within the limit",
    )?;
    let bytes = Zeroizing::new(core_random_bytes(len).map_err(|e| throw_envelope_error(&ctx, e))?);
    bytes_to_uint8_array(&ctx, &bytes)
}

fn read_optional_aad<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>) -> Result<Vec<u8>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_undefined() {
        return Ok(Vec::new());
    }
    let mut aad = read_uint8_bytes(
        ctx,
        value,
        "associatedData must be a Uint8Array",
        MAX_ASSOCIATED_DATA_BYTES,
        "input exceeds the crypto size limit",
        true,
    )?;
    Ok(std::mem::take(&mut *aad))
}

fn encrypt<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<TypedArray<'js, u8>> {
    if args.0.len() < 2 || args.0.len() > 3 {
        return Err(Exception::throw_type(
            &ctx,
            "encrypt takes two or three arguments",
        ));
    }
    let key = read_uint8_exact(
        &ctx,
        &args.0[0],
        "key must be a Uint8Array",
        KEY_LEN,
        "VAEL_CRYPTO_KEY",
        "key must be exactly 32 bytes",
    )?;
    let mut plaintext = read_uint8_bytes(
        &ctx,
        &args.0[1],
        "plaintext must be a Uint8Array",
        MAX_PLAINTEXT_BYTES,
        "input exceeds the crypto size limit",
        true,
    )?;
    let aad = read_optional_aad(&ctx, args.0.get(2))?;
    let envelope = core_encrypt(
        &key,
        std::mem::take(&mut *plaintext),
        &aad,
        CryptoLimits::default(),
    )
    .map_err(|e| throw_envelope_error(&ctx, e))?;
    TypedArray::<u8>::new(ctx, envelope)
}

fn decrypt<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<TypedArray<'js, u8>> {
    if args.0.len() < 2 || args.0.len() > 3 {
        return Err(Exception::throw_type(
            &ctx,
            "decrypt takes two or three arguments",
        ));
    }
    let key = read_uint8_exact(
        &ctx,
        &args.0[0],
        "key must be a Uint8Array",
        KEY_LEN,
        "VAEL_CRYPTO_KEY",
        "key must be exactly 32 bytes",
    )?;
    let mut envelope = read_uint8_bytes(
        &ctx,
        &args.0[1],
        "envelope must be a Uint8Array",
        MAX_ENVELOPE_BYTES,
        "input exceeds the crypto size limit",
        true,
    )?;
    let aad = read_optional_aad(&ctx, args.0.get(2))?;
    let plaintext = Zeroizing::new(
        core_decrypt(
            &key,
            std::mem::take(&mut *envelope),
            &aad,
            CryptoLimits::default(),
        )
        .map_err(|e| throw_envelope_error(&ctx, e))?,
    );
    bytes_to_uint8_array(&ctx, &plaintext)
}

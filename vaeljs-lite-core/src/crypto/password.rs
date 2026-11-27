//! Synchronous PBKDF2-HMAC and bcrypt password hashing bindings for QuickJS.

use rquickjs::{Ctx, Exception, Result, TypedArray, Value, function::Rest};
use vaeljs_crypto_core::Zeroizing;
use vaeljs_crypto_core::password::{
    BCRYPT_HASH_BYTES, DEFAULT_BCRYPT_COST, MAX_BCRYPT_COST, MAX_BCRYPT_PASSWORD_BYTES,
    MAX_PBKDF2_INPUT_BYTES, MAX_PBKDF2_ITERATIONS, MAX_PBKDF2_OUTPUT_BYTES, MIN_BCRYPT_COST,
    PasswordError, PasswordErrorCode, bcrypt_hash as core_bcrypt_hash,
    bcrypt_verify as core_bcrypt_verify, is_valid_bcrypt_hash,
    pbkdf2_zeroizing as core_pbkdf2_zeroizing,
};

use super::hash::parse_hash_algorithm;
use super::{
    bytes_to_uint8_array, detached_error, read_integer, read_uint8_bytes, read_utf8_bytes,
    throw_crypto_error, utf8_len_with_replacement, with_string_utf16,
};
use crate::native::with_uint8_array_bytes_ex;

fn throw_password_error(ctx: &Ctx<'_>, error: PasswordError) -> rquickjs::Error {
    match error.code().as_str() {
        Some(code) => throw_crypto_error(ctx, code, error.message()),
        None => match error.code() {
            PasswordErrorCode::Range => Exception::throw_range(ctx, error.message()),
            _ => throw_crypto_error(ctx, "VAEL_CRYPTO_INTERNAL", error.message()),
        },
    }
}

fn read_secret_input<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    limit: usize,
) -> Result<Zeroizing<Vec<u8>>> {
    if value.is_string() {
        read_utf8_bytes(
            ctx,
            value,
            "expected a Uint8Array or a string",
            limit,
            "input exceeds the password hashing size limit",
        )
    } else {
        read_uint8_bytes(
            ctx,
            value,
            "expected a Uint8Array or a string",
            limit,
            "input exceeds the password hashing size limit",
            true,
        )
    }
}

pub(super) fn pbkdf2<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<TypedArray<'js, u8>> {
    if args.0.len() != 5 {
        return Err(Exception::throw_type(
            &ctx,
            "pbkdf2 takes exactly five arguments",
        ));
    }
    let algorithm = parse_hash_algorithm(
        &ctx,
        &args.0[0],
        "VAEL_CRYPTO_ALGORITHM",
        "unsupported password hashing algorithm",
    )?;
    let iterations = read_integer(
        &ctx,
        &args.0[3],
        1,
        MAX_PBKDF2_ITERATIONS as usize,
        "iterations must be a number",
        "numeric argument is outside its documented range",
    )? as u32;
    let length = read_integer(
        &ctx,
        &args.0[4],
        1,
        MAX_PBKDF2_OUTPUT_BYTES,
        "length must be a number",
        "numeric argument is outside its documented range",
    )?;
    let password = read_secret_input(&ctx, &args.0[1], MAX_PBKDF2_INPUT_BYTES)?;
    let salt = read_secret_input(&ctx, &args.0[2], MAX_PBKDF2_INPUT_BYTES)?;

    let derived = core_pbkdf2_zeroizing(algorithm, &password, &salt, iterations, length)
        .map_err(|e| throw_password_error(&ctx, e))?;
    bytes_to_uint8_array(&ctx, &derived)
}

pub(super) fn bcrypt_hash<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    if args.0.is_empty() || args.0.len() > 2 {
        return Err(Exception::throw_type(
            &ctx,
            "bcryptHash takes one or two arguments",
        ));
    }
    let cost = match args.0.get(1) {
        None => DEFAULT_BCRYPT_COST,
        Some(v) if v.is_undefined() => DEFAULT_BCRYPT_COST,
        Some(v) => read_integer(
            &ctx,
            v,
            MIN_BCRYPT_COST as usize,
            MAX_BCRYPT_COST as usize,
            "cost must be a number",
            "numeric argument is outside its documented range",
        )? as u32,
    };
    let password = read_secret_input(&ctx, &args.0[0], MAX_BCRYPT_PASSWORD_BYTES)?;
    core_bcrypt_hash(&password, cost).map_err(|e| throw_password_error(&ctx, e))
}

pub(super) fn bcrypt_verify<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<bool> {
    if args.0.len() != 2 {
        return Err(Exception::throw_type(
            &ctx,
            "bcryptVerify takes exactly two arguments",
        ));
    }
    let hash_val = &args.0[1];
    if !hash_val.is_string() {
        return Err(Exception::throw_type(&ctx, "hash must be a string"));
    }

    let password_val = &args.0[0];
    let password_too_long = if password_val.is_string() {
        with_string_utf16(
            &ctx,
            password_val,
            "expected a Uint8Array or a string",
            |units| Ok(utf8_len_with_replacement(units) > MAX_BCRYPT_PASSWORD_BYTES),
        )?
    } else {
        with_uint8_array_bytes_ex(
            &ctx,
            password_val,
            "expected a Uint8Array or a string",
            detached_error,
            |slice, _| Ok(slice.len() > MAX_BCRYPT_PASSWORD_BYTES),
        )?
    };
    if password_too_long {
        return Ok(false);
    }

    let Some(hash) = with_string_utf16(&ctx, hash_val, "hash must be a string", |units| {
        if units.len() != BCRYPT_HASH_BYTES {
            return Ok(None);
        }
        let mut out = String::with_capacity(BCRYPT_HASH_BYTES);
        for res in char::decode_utf16(units.iter().copied()) {
            let ch = res.unwrap_or(char::REPLACEMENT_CHARACTER);
            if out.len() + ch.len_utf8() > BCRYPT_HASH_BYTES {
                return Ok(None);
            }
            out.push(ch);
        }
        if out.len() != BCRYPT_HASH_BYTES || !is_valid_bcrypt_hash(&out) {
            return Ok(None);
        }
        Ok(Some(out))
    })?
    else {
        return Ok(false);
    };

    let password = read_secret_input(&ctx, password_val, MAX_BCRYPT_PASSWORD_BYTES)?;
    Ok(core_bcrypt_verify(&password, &hash))
}

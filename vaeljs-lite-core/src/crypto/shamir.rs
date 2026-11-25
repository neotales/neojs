//! Synchronous GF(256) Shamir secret sharing bindings for QuickJS.

use rquickjs::{
    Array, Ctx, Exception, Object, Result, TypedArray, Value, function::Rest, object::Property, qjs,
};
use vaeljs_crypto_core::Zeroizing;
use vaeljs_crypto_core::shamir::{
    MAX_SHAMIR_SECRET_BYTES, MAX_SHAMIR_SHARES, ShamirError, ShamirErrorCode, ShamirShare,
    check_total, shamir_combine as core_shamir_combine, shamir_split as core_shamir_split,
    validate_split,
};

use super::{
    bytes_to_uint8_array, detached_error, limit_error, read_integer, read_uint8_bytes,
    shared_buffer_error, throw_crypto_error,
};
use crate::native::{
    freeze_native, own_array_element, own_data_property, with_uint8_array_bytes_ex,
};

struct SharePlan<'js> {
    index: u8,
    value: Value<'js>,
    length: usize,
}

fn throw_shamir_error(ctx: &Ctx<'_>, error: ShamirError) -> rquickjs::Error {
    match error.code().as_str() {
        Some(code) => throw_crypto_error(ctx, code, error.message()),
        None => match error.code() {
            ShamirErrorCode::Range => Exception::throw_range(ctx, error.message()),
            _ => throw_crypto_error(ctx, "VAEL_CRYPTO_INTERNAL", error.message()),
        },
    }
}

pub(super) fn shamir_split<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Array<'js>> {
    if args.0.len() != 3 {
        return Err(Exception::throw_type(
            &ctx,
            "shamirSplit takes exactly three arguments",
        ));
    }
    let threshold = read_integer(
        &ctx,
        &args.0[1],
        2,
        MAX_SHAMIR_SHARES,
        "threshold must be a number",
        "numeric argument is outside its documented range",
    )?;
    let share_count = read_integer(
        &ctx,
        &args.0[2],
        2,
        MAX_SHAMIR_SHARES,
        "shareCount must be a number",
        "numeric argument is outside its documented range",
    )?;

    let secret_len = with_uint8_array_bytes_ex(
        &ctx,
        &args.0[0],
        "secret must be a Uint8Array",
        detached_error,
        |slice, is_shared| {
            if is_shared {
                return Err(shared_buffer_error(&ctx));
            }
            Ok(slice.len())
        },
    )?;
    validate_split(secret_len, threshold, share_count).map_err(|e| throw_shamir_error(&ctx, e))?;
    let secret = read_uint8_bytes(
        &ctx,
        &args.0[0],
        "secret must be a Uint8Array",
        MAX_SHAMIR_SECRET_BYTES,
        "input exceeds the Shamir size limit",
        false,
    )?;

    let shares = core_shamir_split(&secret, threshold, share_count)
        .map_err(|e| throw_shamir_error(&ctx, e))?;

    let out = Array::new(ctx.clone())?;
    for (position, share) in shares.into_iter().enumerate() {
        let index = share.index();
        let bytes = bytes_to_uint8_array(&ctx, &share.into_value())?;
        let record = Object::new(ctx.clone())?;
        record.prop(
            "index",
            Property::from(f64::from(index))
                .writable()
                .enumerable()
                .configurable(),
        )?;
        record.prop(
            "value",
            Property::from(bytes).writable().enumerable().configurable(),
        )?;
        freeze_native(&ctx, &record)?;
        out.as_object().prop(
            position.to_string(),
            Property::from(record)
                .writable()
                .enumerable()
                .configurable(),
        )?;
    }
    Ok(out)
}

pub(super) fn shamir_combine<'js>(
    ctx: Ctx<'js>,
    args: Rest<Value<'js>>,
) -> Result<TypedArray<'js, u8>> {
    if args.0.is_empty() || args.0.len() > 2 {
        return Err(Exception::throw_type(
            &ctx,
            "shamirCombine takes one or two arguments",
        ));
    }
    let threshold = match args.0.get(1) {
        None => None,
        Some(v) if v.is_undefined() => None,
        Some(v) => Some(read_integer(
            &ctx,
            v,
            2,
            MAX_SHAMIR_SHARES,
            "threshold must be a number",
            "numeric argument is outside its documented range",
        )?),
    };

    let shares_val = &args.0[0];
    if shares_val.is_proxy() || !unsafe { qjs::JS_IsArray(shares_val.as_raw()) } {
        return Err(Exception::throw_type(&ctx, "shares must be an Array"));
    }
    let array = shares_val
        .as_array()
        .ok_or_else(|| Exception::throw_type(&ctx, "shares must be an Array"))?;
    let Some(len_val) = own_data_property(
        &ctx,
        shares_val.as_raw(),
        c"length",
        "shares length must be an own data property",
    )?
    else {
        return Err(Exception::throw_type(
            &ctx,
            "shares length must be an own data property",
        ));
    };
    let count = read_integer(
        &ctx,
        &len_val,
        2,
        MAX_SHAMIR_SHARES,
        "shares length must be a number",
        "numeric argument is outside its documented range",
    )?;

    let mut plans = Vec::new();
    plans
        .try_reserve_exact(count)
        .map_err(|_| limit_error(&ctx, "input exceeds the Shamir size limit"))?;

    for pos in 0..count {
        let Some(element) = own_array_element(
            &ctx,
            array,
            pos as u32,
            "share elements must be plain data properties",
        )?
        else {
            return Err(Exception::throw_type(&ctx, "shares must not contain holes"));
        };
        if !element.is_object() || element.is_proxy() {
            return Err(Exception::throw_type(&ctx, "each share must be an object"));
        }
        let raw_el = element.as_raw();
        let Some(index_val) = own_data_property(
            &ctx,
            raw_el,
            c"index",
            "share index must be an own data property",
        )?
        else {
            return Err(Exception::throw_type(&ctx, "share must have an index"));
        };
        let index = read_integer(
            &ctx,
            &index_val,
            1,
            MAX_SHAMIR_SHARES,
            "share index must be a number",
            "numeric argument is outside its documented range",
        )? as u8;

        let Some(value_val) = own_data_property(
            &ctx,
            raw_el,
            c"value",
            "share value must be an own data property",
        )?
        else {
            return Err(Exception::throw_type(&ctx, "share must have a value"));
        };
        let length = with_uint8_array_bytes_ex(
            &ctx,
            &value_val,
            "share value must be a Uint8Array",
            detached_error,
            |slice, is_shared| {
                if is_shared {
                    return Err(shared_buffer_error(&ctx));
                }
                Ok(slice.len())
            },
        )?;
        plans.push(SharePlan {
            index,
            value: value_val,
            length,
        });
    }

    let expected_len = plans[0].length;
    if expected_len == 0 {
        return Err(Exception::throw_range(
            &ctx,
            "numeric argument is outside its documented range",
        ));
    }
    if expected_len > MAX_SHAMIR_SECRET_BYTES {
        return Err(limit_error(&ctx, "input exceeds the Shamir size limit"));
    }
    check_total(expected_len, plans.len()).map_err(|e| throw_shamir_error(&ctx, e))?;

    let mut seen = [false; 256];
    for plan in &plans {
        let slot = usize::from(plan.index);
        if seen[slot] {
            return Err(throw_shamir_error(
                &ctx,
                ShamirError::new(ShamirErrorCode::DuplicateIndex),
            ));
        }
        seen[slot] = true;
        if plan.length == 0 {
            return Err(Exception::throw_range(
                &ctx,
                "numeric argument is outside its documented range",
            ));
        }
        if plan.length != expected_len {
            return Err(throw_shamir_error(
                &ctx,
                ShamirError::new(ShamirErrorCode::LengthMismatch),
            ));
        }
    }

    if let Some(threshold) = threshold
        && plans.len() < threshold
    {
        return Err(Exception::throw_range(
            &ctx,
            "numeric argument is outside its documented range",
        ));
    }

    let mut shares = Vec::new();
    shares
        .try_reserve_exact(plans.len())
        .map_err(|_| limit_error(&ctx, "input exceeds the Shamir size limit"))?;
    for plan in &plans {
        let bytes = read_uint8_bytes(
            &ctx,
            &plan.value,
            "share value must be a Uint8Array",
            MAX_SHAMIR_SECRET_BYTES,
            "input exceeds the Shamir size limit",
            false,
        )?;
        shares.push(ShamirShare::from_zeroizing(plan.index, bytes));
    }

    let recovered: Zeroizing<Vec<u8>> =
        core_shamir_combine(&shares).map_err(|e| throw_shamir_error(&ctx, e))?;
    bytes_to_uint8_array(&ctx, &recovered)
}

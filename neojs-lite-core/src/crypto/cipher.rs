//! Bounded stateful AES-256-CBC and AES-256-GCM cipher bindings for QuickJS.

use std::cell::Cell;
use std::rc::Rc;

use neojs_crypto_core::Zeroizing;
use neojs_crypto_core::cipher::{
    CipherAlgorithm, CipherDirection, CipherError, CipherErrorCode, MAX_CIPHER_AAD_BYTES,
};
use neojs_crypto_core::envelope::{KEY_LEN, TAG_LEN};
use rquickjs::{
    Ctx, Exception, Function, Object, Result, TypedArray, Value,
    function::{Rest, This},
    object::Property,
};

use super::{
    bytes_to_uint8_array, read_ascii_or_utf8_string, read_uint8_bytes, read_uint8_exact,
    read_utf8_bytes, throw_crypto_error,
};
use crate::native::{freeze_native, parse_options, set_prototype};

#[derive(Clone, Copy)]
struct CipherHandle {
    id: u32,
    is_encrypt: bool,
    is_gcm: bool,
    finalized: bool,
    closed: bool,
    obj_ptr: usize,
}

fn cipher_ctor<'js>(ctx: Ctx<'js>, _args: Rest<Value<'js>>) -> Result<()> {
    Err(Exception::throw_type(
        &ctx,
        "Cipher objects cannot be constructed directly; use createEncryptor() or createDecryptor()",
    ))
}

/// Builds the frozen `Cipher.prototype` with a non-constructible `Cipher` constructor.
pub(crate) fn build_cipher_proto<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let ctor = Function::new(ctx.clone(), cipher_ctor)?
        .with_name("Cipher")?
        .with_length(0)?;
    freeze_native(ctx, ctor.as_object().ok_or(rquickjs::Error::Unknown)?)?;
    let proto = Object::new(ctx.clone())?;
    proto.prop(
        "constructor",
        Property::from(ctor).writable().configurable(),
    )?;
    freeze_native(ctx, &proto)?;
    Ok(proto)
}

fn throw_cipher_error(ctx: &Ctx<'_>, error: CipherError) -> rquickjs::Error {
    throw_crypto_error(ctx, error.code().as_str(), error.message())
}

fn parse_cipher_algorithm<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<CipherAlgorithm> {
    let name = read_ascii_or_utf8_string(ctx, value, "algorithm must be a string", 32, |ctx| {
        throw_crypto_error(ctx, "NEO_CRYPTO_ALGORITHM", "unsupported cipher algorithm")
    })?;
    CipherAlgorithm::parse(&name).ok_or_else(|| {
        throw_crypto_error(ctx, "NEO_CRYPTO_ALGORITHM", "unsupported cipher algorithm")
    })
}

fn parse_cipher_options<'js>(
    ctx: &Ctx<'js>,
    algorithm: CipherAlgorithm,
    value: Option<&Value<'js>>,
) -> Result<bool> {
    let type_msg = "options must be a plain or null-prototype object with own enumerable data properties; only CBC accepts padding";
    let mut padding = true;
    parse_options(ctx, value, type_msg, |ctx, key, field| {
        if algorithm == CipherAlgorithm::Aes256Gcm {
            return Err(Exception::throw_type(ctx, type_msg));
        }
        if key != "padding" {
            return Err(Exception::throw_type(ctx, type_msg));
        }
        if field.is_undefined() {
            return Ok(());
        }
        let Some(enabled) = field.as_bool() else {
            return Err(Exception::throw_type(ctx, type_msg));
        };
        padding = enabled;
        Ok(())
    })?;
    Ok(padding)
}

pub(super) fn create_encryptor<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() < 3 || args.0.len() > 4 {
        return Err(Exception::throw_type(
            &ctx,
            "createEncryptor takes three or four arguments",
        ));
    }
    create_cipher(&ctx, CipherDirection::Encrypt, &args.0)
}

pub(super) fn create_decryptor<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() < 3 || args.0.len() > 4 {
        return Err(Exception::throw_type(
            &ctx,
            "createDecryptor takes three or four arguments",
        ));
    }
    create_cipher(&ctx, CipherDirection::Decrypt, &args.0)
}

fn create_cipher<'js>(
    ctx: &Ctx<'js>,
    direction: CipherDirection,
    args: &[Value<'js>],
) -> Result<Object<'js>> {
    let algorithm = parse_cipher_algorithm(ctx, &args[0])?;
    let padding = parse_cipher_options(ctx, algorithm, args.get(3))?;
    let key = read_uint8_exact(
        ctx,
        &args[1],
        "key must be a Uint8Array",
        KEY_LEN,
        "NEO_CRYPTO_KEY",
        "key must be exactly 32 bytes",
    )?;
    let iv = read_uint8_exact(
        ctx,
        &args[2],
        "iv must be a Uint8Array",
        algorithm.iv_len(),
        "NEO_CRYPTO_IV",
        "iv has the wrong length for the cipher algorithm",
    )?;

    let id = {
        let state = ctx
            .userdata::<crate::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let mut table = state.cipher_table.borrow_mut();
        table
            .create(direction, algorithm, &key, &iv, padding)
            .map_err(|e| throw_cipher_error(ctx, e))?
    };

    build_cipher_object(
        ctx,
        id,
        direction == CipherDirection::Encrypt,
        algorithm == CipherAlgorithm::Aes256Gcm,
    )
}

fn check_receiver<'js>(ctx: &Ctx<'js>, this: &Value<'js>, expected_ptr: usize) -> Result<()> {
    let valid = this
        .as_object()
        .is_some_and(|obj| unsafe { obj.as_raw().u.ptr as usize } == expected_ptr);
    if !valid {
        return Err(Exception::throw_type(ctx, "invalid Cipher receiver"));
    }
    Ok(())
}

fn assert_open(ctx: &Ctx<'_>, handle: &Cell<CipherHandle>) -> Result<()> {
    if handle.get().closed {
        return Err(throw_cipher_error(
            ctx,
            CipherError::new(CipherErrorCode::Closed),
        ));
    }
    Ok(())
}

fn assert_buffering(ctx: &Ctx<'_>, handle: &Cell<CipherHandle>) -> Result<()> {
    assert_open(ctx, handle)?;
    if handle.get().finalized {
        return Err(throw_crypto_error(
            ctx,
            "NEO_CRYPTO_STATE",
            "the cipher has already been finalized",
        ));
    }
    Ok(())
}

fn parse_input_encoding<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_undefined() {
        return Ok(());
    }
    let name = read_ascii_or_utf8_string(ctx, value, "encoding must be a string", 16, |ctx| {
        Exception::throw_type(ctx, "unsupported input encoding")
    })?;
    if name.eq_ignore_ascii_case("utf8") || name.eq_ignore_ascii_case("utf-8") {
        Ok(())
    } else {
        Err(Exception::throw_type(ctx, "unsupported input encoding"))
    }
}

fn build_cipher_object<'js>(
    ctx: &Ctx<'js>,
    id: u32,
    is_encrypt: bool,
    is_gcm: bool,
) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    let proto = {
        let state = ctx
            .userdata::<crate::neo::NeoState>()
            .ok_or(rquickjs::Error::Unknown)?;
        state
            .cipher_proto
            .as_ref()
            .ok_or(rquickjs::Error::Unknown)?
            .clone()
            .restore(ctx)?
    };
    set_prototype(ctx, &obj, &proto)?;

    let obj_ptr = unsafe { obj.as_raw().u.ptr as usize };
    let handle = Rc::new(Cell::new(CipherHandle {
        id,
        is_encrypt,
        is_gcm,
        finalized: false,
        closed: false,
        obj_ptr,
    }));

    let h = Rc::clone(&handle);
    let update_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              this: This<Value<'js>>,
              args: Rest<Value<'js>>|
              -> Result<TypedArray<'js, u8>> {
            check_receiver(&ctx, &this.0, h.get().obj_ptr)?;
            if args.0.is_empty() || args.0.len() > 2 {
                return Err(Exception::throw_type(
                    &ctx,
                    "update takes one or two arguments",
                ));
            }
            assert_buffering(&ctx, &h)?;
            let data = &args.0[0];
            let encoding = args.0.get(1);
            if data.is_string() {
                parse_input_encoding(&ctx, encoding)?;
            } else if encoding.is_some_and(|v| !v.is_undefined()) {
                return Err(Exception::throw_type(
                    &ctx,
                    "encoding must not be supplied for binary input",
                ));
            }
            let cid = h.get().id;
            let remaining = {
                let state = ctx
                    .userdata::<crate::neo::NeoState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let table = state.cipher_table.borrow();
                table
                    .get(cid)
                    .map_err(|e| throw_cipher_error(&ctx, e))?
                    .remaining_input()
            };
            let bytes = if data.is_string() {
                read_utf8_bytes(
                    &ctx,
                    data,
                    "expected a Uint8Array or a string",
                    remaining,
                    "input exceeds the cipher size limit",
                )?
            } else {
                read_uint8_bytes(
                    &ctx,
                    data,
                    "expected a Uint8Array or a string",
                    remaining,
                    "input exceeds the cipher size limit",
                    true,
                )?
            };
            {
                let state = ctx
                    .userdata::<crate::neo::NeoState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let mut table = state.cipher_table.borrow_mut();
                table
                    .get_mut(cid)
                    .map_err(|e| throw_cipher_error(&ctx, e))?
                    .update(&bytes)
                    .map_err(|e| throw_cipher_error(&ctx, e))?;
            }
            TypedArray::<u8>::new(ctx, [])
        },
    )?
    .with_name("update")?
    .with_length(2)?;

    let h = Rc::clone(&handle);
    let final_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              this: This<Value<'js>>,
              args: Rest<Value<'js>>|
              -> Result<TypedArray<'js, u8>> {
            check_receiver(&ctx, &this.0, h.get().obj_ptr)?;
            if !args.0.is_empty() {
                return Err(Exception::throw_type(&ctx, "final takes no arguments"));
            }
            assert_buffering(&ctx, &h)?;
            let mut cur = h.get();
            cur.finalized = true;
            h.set(cur);
            let output = {
                let state = ctx
                    .userdata::<crate::neo::NeoState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let mut table = state.cipher_table.borrow_mut();
                Zeroizing::new(
                    table
                        .get_mut(cur.id)
                        .map_err(|e| throw_cipher_error(&ctx, e))?
                        .finalize()
                        .map_err(|e| throw_cipher_error(&ctx, e))?,
                )
            };
            bytes_to_uint8_array(&ctx, output)
        },
    )?
    .with_name("final")?
    .with_length(0)?;

    let h = Rc::clone(&handle);
    let set_aad_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              this: This<Value<'js>>,
              args: Rest<Value<'js>>|
              -> Result<Value<'js>> {
            check_receiver(&ctx, &this.0, h.get().obj_ptr)?;
            if args.0.len() != 1 {
                return Err(Exception::throw_type(
                    &ctx,
                    "setAAD takes exactly one argument",
                ));
            }
            assert_buffering(&ctx, &h)?;
            let cur = h.get();
            if !cur.is_gcm {
                return Err(throw_crypto_error(
                    &ctx,
                    "NEO_CRYPTO_UNSUPPORTED",
                    "associated data is only supported for aes-256-gcm",
                ));
            }
            let aad = read_uint8_bytes(
                &ctx,
                &args.0[0],
                "aad must be a Uint8Array",
                MAX_CIPHER_AAD_BYTES,
                "input exceeds the cipher size limit",
                true,
            )?;
            {
                let state = ctx
                    .userdata::<crate::neo::NeoState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let mut table = state.cipher_table.borrow_mut();
                table
                    .get_mut(cur.id)
                    .map_err(|e| throw_cipher_error(&ctx, e))?
                    .set_aad(aad.to_vec())
                    .map_err(|e| throw_cipher_error(&ctx, e))?;
            }
            Ok(this.0)
        },
    )?
    .with_name("setAAD")?
    .with_length(1)?;

    let h = Rc::clone(&handle);
    let set_tag_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              this: This<Value<'js>>,
              args: Rest<Value<'js>>|
              -> Result<Value<'js>> {
            check_receiver(&ctx, &this.0, h.get().obj_ptr)?;
            if args.0.len() != 1 {
                return Err(Exception::throw_type(
                    &ctx,
                    "setAuthTag takes exactly one argument",
                ));
            }
            assert_buffering(&ctx, &h)?;
            let cur = h.get();
            if !cur.is_gcm || cur.is_encrypt {
                return Err(throw_crypto_error(
                    &ctx,
                    "NEO_CRYPTO_UNSUPPORTED",
                    "setAuthTag is only supported on aes-256-gcm decryptors",
                ));
            }
            let tag = read_uint8_exact(
                &ctx,
                &args.0[0],
                "tag must be a Uint8Array",
                TAG_LEN,
                "NEO_CRYPTO_TAG",
                "authentication tag must be exactly 16 bytes",
            )?;
            {
                let state = ctx
                    .userdata::<crate::neo::NeoState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let mut table = state.cipher_table.borrow_mut();
                table
                    .get_mut(cur.id)
                    .map_err(|e| throw_cipher_error(&ctx, e))?
                    .set_tag(&tag)
                    .map_err(|e| throw_cipher_error(&ctx, e))?;
            }
            Ok(this.0)
        },
    )?
    .with_name("setAuthTag")?
    .with_length(1)?;

    let h = Rc::clone(&handle);
    let get_tag_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              this: This<Value<'js>>,
              args: Rest<Value<'js>>|
              -> Result<TypedArray<'js, u8>> {
            check_receiver(&ctx, &this.0, h.get().obj_ptr)?;
            if !args.0.is_empty() {
                return Err(Exception::throw_type(&ctx, "getAuthTag takes no arguments"));
            }
            assert_open(&ctx, &h)?;
            let cur = h.get();
            if !cur.is_gcm || !cur.is_encrypt {
                return Err(throw_crypto_error(
                    &ctx,
                    "NEO_CRYPTO_UNSUPPORTED",
                    "getAuthTag is only supported on aes-256-gcm encryptors",
                ));
            }
            if !cur.finalized {
                return Err(throw_crypto_error(
                    &ctx,
                    "NEO_CRYPTO_STATE",
                    "final must be called before getAuthTag",
                ));
            }
            let tag = {
                let state = ctx
                    .userdata::<crate::neo::NeoState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let table = state.cipher_table.borrow();
                table
                    .get(cur.id)
                    .map_err(|e| throw_cipher_error(&ctx, e))?
                    .auth_tag()
                    .map_err(|e| throw_cipher_error(&ctx, e))?
            };
            TypedArray::<u8>::new(ctx, tag.as_slice())
        },
    )?
    .with_name("getAuthTag")?
    .with_length(0)?;

    let h = Rc::clone(&handle);
    let set_padding_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>,
              this: This<Value<'js>>,
              args: Rest<Value<'js>>|
              -> Result<Value<'js>> {
            check_receiver(&ctx, &this.0, h.get().obj_ptr)?;
            if args.0.len() != 1 {
                return Err(Exception::throw_type(
                    &ctx,
                    "setAutoPadding takes exactly one argument",
                ));
            }
            assert_buffering(&ctx, &h)?;
            let cur = h.get();
            if cur.is_gcm {
                return Err(throw_crypto_error(
                    &ctx,
                    "NEO_CRYPTO_UNSUPPORTED",
                    "padding is not supported for aes-256-gcm",
                ));
            }
            let Some(enabled) = args.0[0].as_bool() else {
                return Err(Exception::throw_type(
                    &ctx,
                    "setAutoPadding expects a boolean",
                ));
            };
            {
                let state = ctx
                    .userdata::<crate::neo::NeoState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let mut table = state.cipher_table.borrow_mut();
                table
                    .get_mut(cur.id)
                    .map_err(|e| throw_cipher_error(&ctx, e))?
                    .set_padding(enabled)
                    .map_err(|e| throw_cipher_error(&ctx, e))?;
            }
            Ok(this.0)
        },
    )?
    .with_name("setAutoPadding")?
    .with_length(1)?;

    let h = Rc::clone(&handle);
    let close_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<()> {
            check_receiver(&ctx, &this.0, h.get().obj_ptr)?;
            if !args.0.is_empty() {
                return Err(Exception::throw_type(&ctx, "close takes no arguments"));
            }
            close_cipher_handle(&ctx, &h)
        },
    )?
    .with_name("close")?
    .with_length(0)?;

    let h = handle;
    let dispose_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<()> {
            check_receiver(&ctx, &this.0, h.get().obj_ptr)?;
            if !args.0.is_empty() {
                return Err(Exception::throw_type(&ctx, "dispose takes no arguments"));
            }
            close_cipher_handle(&ctx, &h)
        },
    )?
    .with_name("dispose")?
    .with_length(0)?;

    obj.set("update", update_fn)?;
    obj.set("final", final_fn)?;
    obj.set("setAAD", set_aad_fn)?;
    obj.set("setAuthTag", set_tag_fn)?;
    obj.set("getAuthTag", get_tag_fn)?;
    obj.set("setAutoPadding", set_padding_fn)?;
    obj.set("close", close_fn)?;
    obj.set("dispose", dispose_fn)?;
    freeze_native(ctx, &obj)?;
    Ok(obj)
}

fn close_cipher_handle(ctx: &Ctx<'_>, handle: &Cell<CipherHandle>) -> Result<()> {
    let mut cur = handle.get();
    if cur.closed {
        return Ok(());
    }
    cur.closed = true;
    handle.set(cur);
    let state = ctx
        .userdata::<crate::neo::NeoState>()
        .ok_or(rquickjs::Error::Unknown)?;
    state.cipher_table.borrow_mut().close(cur.id);
    Ok(())
}

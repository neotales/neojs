//! Incremental SHA-2 hash and HMAC bindings for QuickJS.

use std::cell::Cell;

use rquickjs::{
    Class, Constructor, Ctx, Exception, Function, IntoJs, JsLifetime, Object, Result,
    String as JsString, Value,
    class::{JsClass, Readable, Trace, Tracer},
    function::{Rest, This},
    object::Property,
};
use vaeljs_crypto_core::Zeroizing;
use vaeljs_crypto_core::hash::{
    HashAlgorithm, HashError, HashErrorCode, MAX_HMAC_KEY_BYTES, OutputEncoding,
};

use super::{
    bytes_to_uint8_array, define_method, read_ascii_or_utf8_string, read_uint8_bytes,
    read_utf8_bytes, throw_crypto_error,
};
use crate::native::freeze_native;

#[derive(Clone, Copy)]
struct HashHandle {
    id: u32,
    digested: bool,
    closed: bool,
}

struct HashBrand(Cell<HashHandle>);

impl<'js> Trace<'js> for HashBrand {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

unsafe impl<'js> JsLifetime<'js> for HashBrand {
    type Changed<'to> = HashBrand;
}

impl<'js> JsClass<'js> for HashBrand {
    const NAME: &'static str = "Hash";
    type Mutable = Readable;

    fn constructor(_ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        Ok(None)
    }
}

struct HashSlotGuard<'a, 'js> {
    ctx: &'a Ctx<'js>,
    id: Option<u32>,
}

impl Drop for HashSlotGuard<'_, '_> {
    fn drop(&mut self) {
        if let Some(id) = self.id
            && let Some(state) = self.ctx.userdata::<crate::vael::VaelState>()
        {
            state.digest_table.borrow_mut().close(id);
        }
    }
}

fn hash_ctor<'js>(ctx: Ctx<'js>, _args: Rest<Value<'js>>) -> Result<()> {
    Err(Exception::throw_type(
        &ctx,
        "Hash objects cannot be constructed directly; use createHash() or createHmac()",
    ))
}

/// Builds the frozen `Hash.prototype` with a non-constructible `Hash` constructor.
pub(crate) fn build_hash_proto<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let ctor = Function::new(ctx.clone(), hash_ctor)?
        .with_name("Hash")?
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

fn throw_hash_error(ctx: &Ctx<'_>, error: HashError) -> rquickjs::Error {
    let code = match error.code() {
        HashErrorCode::Digested | HashErrorCode::Closed => "VAEL_CRYPTO_DIGESTED",
        other => other.as_str(),
    };
    throw_crypto_error(ctx, code, error.message())
}

pub(in crate::crypto) fn parse_hash_algorithm<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    code: &str,
    msg: &str,
) -> Result<HashAlgorithm> {
    let name = read_ascii_or_utf8_string(ctx, value, "algorithm must be a string", 16, |ctx| {
        throw_crypto_error(ctx, code, msg)
    })?;
    HashAlgorithm::parse(&name).ok_or_else(|| throw_crypto_error(ctx, code, msg))
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

fn parse_output_encoding<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
) -> Result<OutputEncoding> {
    let Some(value) = value else {
        return Ok(OutputEncoding::Binary);
    };
    if value.is_undefined() {
        return Ok(OutputEncoding::Binary);
    }
    let name = read_ascii_or_utf8_string(ctx, value, "encoding must be a string", 16, |ctx| {
        Exception::throw_type(ctx, "unsupported digest encoding")
    })?;
    OutputEncoding::parse(&name)
        .ok_or_else(|| Exception::throw_type(ctx, "unsupported digest encoding"))
}

pub(super) fn create_hash<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "createHash takes exactly one argument",
        ));
    }
    let algorithm = parse_hash_algorithm(
        &ctx,
        &args.0[0],
        "VAEL_CRYPTO_ALGORITHM",
        "unsupported hash algorithm",
    )?;
    let id = {
        let state = ctx
            .userdata::<crate::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let mut table = state.digest_table.borrow_mut();
        table
            .create_hash(algorithm)
            .map_err(|e| throw_hash_error(&ctx, e))?
    };
    build_hash_object(&ctx, id)
}

pub(super) fn create_hmac<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 2 {
        return Err(Exception::throw_type(
            &ctx,
            "createHmac takes exactly two arguments",
        ));
    }
    let algorithm = parse_hash_algorithm(
        &ctx,
        &args.0[0],
        "VAEL_CRYPTO_ALGORITHM",
        "unsupported hash algorithm",
    )?;
    let key_val = &args.0[1];
    let key = if key_val.is_string() {
        read_utf8_bytes(
            &ctx,
            key_val,
            "expected a Uint8Array or a string",
            MAX_HMAC_KEY_BYTES,
            "input exceeds the hash size limit",
        )?
    } else {
        read_uint8_bytes(
            &ctx,
            key_val,
            "expected a Uint8Array or a string",
            MAX_HMAC_KEY_BYTES,
            "input exceeds the hash size limit",
            true,
        )?
    };
    let id = {
        let state = ctx
            .userdata::<crate::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let mut table = state.digest_table.borrow_mut();
        table
            .create_hmac(algorithm, &key)
            .map_err(|e| throw_hash_error(&ctx, e))?
    };
    build_hash_object(&ctx, id)
}

fn check_receiver<'js>(ctx: &Ctx<'js>, this: &Value<'js>) -> Result<Class<'js, HashBrand>> {
    this.as_object()
        .and_then(Class::<HashBrand>::from_object)
        .ok_or_else(|| Exception::throw_type(ctx, "invalid Hash receiver"))
}

fn check_active(ctx: &Ctx<'_>, handle: &Cell<HashHandle>) -> Result<u32> {
    let cur = handle.get();
    if cur.closed {
        return Err(throw_crypto_error(
            ctx,
            "VAEL_CRYPTO_CLOSED",
            "the hash object is closed",
        ));
    }
    if cur.digested {
        return Err(throw_crypto_error(
            ctx,
            "VAEL_CRYPTO_DIGESTED",
            "digest has already been called",
        ));
    }
    Ok(cur.id)
}

fn build_hash_object<'js>(ctx: &Ctx<'js>, id: u32) -> Result<Object<'js>> {
    let mut slot_guard = HashSlotGuard { ctx, id: Some(id) };
    let proto = {
        let state = ctx
            .userdata::<crate::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        state
            .hash_proto
            .as_ref()
            .ok_or(rquickjs::Error::Unknown)?
            .clone()
            .restore(ctx)?
    };
    let cls = Class::instance_proto(
        HashBrand(Cell::new(HashHandle {
            id,
            digested: false,
            closed: false,
        })),
        proto,
    )?;

    let update_fn = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<Value<'js>> {
            let cls = check_receiver(&ctx, &this.0)?;
            let h = &cls.borrow().0;
            if args.0.is_empty() || args.0.len() > 2 {
                return Err(Exception::throw_type(
                    &ctx,
                    "update takes one or two arguments",
                ));
            }
            let did = check_active(&ctx, h)?;
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
            let remaining = {
                let state = ctx
                    .userdata::<crate::vael::VaelState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let table = state.digest_table.borrow();
                table
                    .remaining(did)
                    .map_err(|e| throw_hash_error(&ctx, e))?
            };
            let bytes = if data.is_string() {
                read_utf8_bytes(
                    &ctx,
                    data,
                    "expected a Uint8Array or a string",
                    remaining,
                    "input exceeds the hash size limit",
                )?
            } else {
                read_uint8_bytes(
                    &ctx,
                    data,
                    "expected a Uint8Array or a string",
                    remaining,
                    "input exceeds the hash size limit",
                    true,
                )?
            };
            {
                let state = ctx
                    .userdata::<crate::vael::VaelState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let mut table = state.digest_table.borrow_mut();
                table
                    .update(did, &bytes)
                    .map_err(|e| throw_hash_error(&ctx, e))?;
            }
            Ok(this.0)
        },
    )?;

    let digest_fn = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<Value<'js>> {
            let cls = check_receiver(&ctx, &this.0)?;
            let h = &cls.borrow().0;
            if args.0.len() > 1 {
                return Err(Exception::throw_type(
                    &ctx,
                    "digest takes at most one argument",
                ));
            }
            let did = check_active(&ctx, h)?;
            let encoding = parse_output_encoding(&ctx, args.0.first())?;
            let mut cur = h.get();
            cur.digested = true;
            h.set(cur);

            let bytes = Zeroizing::new({
                let state = ctx
                    .userdata::<crate::vael::VaelState>()
                    .ok_or(rquickjs::Error::Unknown)?;
                let mut table = state.digest_table.borrow_mut();
                table
                    .digest(did, encoding)
                    .map_err(|e| throw_hash_error(&ctx, e))?
            });
            match encoding {
                OutputEncoding::Binary => {
                    let ta = bytes_to_uint8_array(&ctx, &bytes)?;
                    Ok(ta.into_value())
                }
                OutputEncoding::Latin1 => {
                    let text: String = bytes.iter().copied().map(char::from).collect();
                    JsString::from_str(ctx, &text).map(JsString::into_value)
                }
                OutputEncoding::Hex | OutputEncoding::Base64 | OutputEncoding::Base64Url => {
                    let text = std::str::from_utf8(&bytes).map_err(|_| {
                        throw_crypto_error(&ctx, "VAEL_CRYPTO_INTERNAL", "invalid encoded digest")
                    })?;
                    text.into_js(&ctx)
                }
            }
        },
    )?;

    let close_fn = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<()> {
            let cls = check_receiver(&ctx, &this.0)?;
            if !args.0.is_empty() {
                return Err(Exception::throw_type(&ctx, "close takes no arguments"));
            }
            close_hash_handle(&ctx, &cls.borrow().0)
        },
    )?;

    let dispose_fn = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>, this: This<Value<'js>>, args: Rest<Value<'js>>| -> Result<()> {
            let cls = check_receiver(&ctx, &this.0)?;
            if !args.0.is_empty() {
                return Err(Exception::throw_type(&ctx, "dispose takes no arguments"));
            }
            close_hash_handle(&ctx, &cls.borrow().0)
        },
    )?;

    define_method(&cls, "update", update_fn, 2)?;
    define_method(&cls, "digest", digest_fn, 1)?;
    define_method(&cls, "close", close_fn, 0)?;
    define_method(&cls, "dispose", dispose_fn, 0)?;
    freeze_native(ctx, &cls)?;
    slot_guard.id = None;
    Ok((*cls).clone())
}

fn close_hash_handle(ctx: &Ctx<'_>, handle: &Cell<HashHandle>) -> Result<()> {
    let mut cur = handle.get();
    if cur.closed {
        return Ok(());
    }
    cur.closed = true;
    handle.set(cur);
    let state = ctx
        .userdata::<crate::vael::VaelState>()
        .ok_or(rquickjs::Error::Unknown)?;
    state.digest_table.borrow_mut().close(cur.id);
    Ok(())
}

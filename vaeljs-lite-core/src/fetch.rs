//! Native `vael:fetch` synchronous bindings for QuickJS.
//!
//! All SSRF guards, redirect validation, bounded reads, atomic temporary-file
//! streaming, SHA-256 verification, and `VAEL_FETCH_*` error codes live in
//! [`vaeljs_fetch_core`]. This module wires synchronous QuickJS host
//! operations into `fetch.js` (which captures ECMAScript primordials at realm
//! initialization), builds one frozen registry object per context, and serves
//! those exact exports from both `Vael.getBuiltInModule("vael:fetch")` and
//! `import ... from "vael:fetch"` without installing `globalThis.fetch`.

use std::mem::MaybeUninit;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rquickjs::{
    Array, Ctx, Exception, Function, Object, Result, String as JsString, TypedArray, Value,
    function::Rest, module::ModuleDef, object::Property, qjs,
};
use vaeljs_fetch_core::{
    DownloadOptions, DownloadProgress, FetchBody, FetchError, FetchErrorCode, FetchOptions,
    MAX_ALLOWED_DOWNLOAD_BYTES, MAX_ALLOWED_REDIRECTS, MAX_ALLOWED_RESPONSE_BYTES, MAX_TIMEOUT,
    Zeroizing, download_sync_cancellable, fetch_sync,
};

use crate::native::{
    freeze_native, get_prototype, own_array_element, parse_options, with_uint8_array_bytes,
};

/// Canonical module specifier for `vael:fetch`.
pub(super) const SPECIFIER: &str = vaeljs_fetch_core::MODULE_SPECIFIER;

/// Exact sorted named exports of `vael:fetch`.
pub(super) const EXPORTS: &[&str] = &["FetchError", "downloadSync", "fetchSync"];

const FACTORY_SOURCE: &str = include_str!("fetch.js");
const MAX_PAIR_COUNT: u32 = 4096;
const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;
const MAX_JSON_DEPTH: usize = 128;
const MAX_JSON_NODES: usize = 100_000;

fn set_null_proto<'js>(ctx: &Ctx<'js>, obj: &Object<'js>) -> Result<()> {
    let rc = unsafe { qjs::JS_SetPrototype(ctx.as_raw().as_ptr(), obj.as_raw(), qjs::JS_NULL) };
    if rc < 0 {
        return Err(if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            rquickjs::Error::Unknown
        });
    }
    Ok(())
}

struct PropertyEnumGuard {
    raw: *mut qjs::JSContext,
    tab: *mut qjs::JSPropertyEnum,
    len: u32,
}

impl Drop for PropertyEnumGuard {
    fn drop(&mut self) {
        if !self.tab.is_null() {
            // SAFETY: `tab` and `len` were returned by `JS_GetOwnPropertyNames`
            // on the active `JSContext`.
            unsafe { qjs::JS_FreePropertyEnum(self.raw, self.tab, self.len) };
        }
    }
}

/// Creates the frozen public registry object for `vael:fetch`.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let factory: Function<'js> = ctx.eval(FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    host.set(
        "fetchSync",
        Function::new(ctx.clone(), host_fetch_sync)?
            .with_name("fetchSync")?
            .with_length(2)?,
    )?;
    host.set(
        "downloadSync",
        Function::new(ctx.clone(), host_download_sync)?
            .with_name("downloadSync")?
            .with_length(3)?,
    )?;
    host.set(
        "decodeUtf8Strict",
        Function::new(ctx.clone(), host_decode_utf8_strict)?
            .with_name("decodeUtf8Strict")?
            .with_length(1)?,
    )?;
    let module: Object<'js> = factory.call((host,))?;
    freeze_native(ctx, &module)?;
    Ok(module)
}

fn read_strict_string<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    on_not_string: impl FnOnce() -> FetchError,
    on_surrogate: impl FnOnce() -> FetchError,
) -> std::result::Result<String, FetchError> {
    if !value.is_string() {
        return Err(on_not_string());
    }
    let Ok(js_str) = JsString::from_value(value.clone()) else {
        return Err(on_not_string());
    };
    match js_str.to_string() {
        Ok(text) => Ok(text),
        Err(rquickjs::Error::FromJs { .. } | rquickjs::Error::Utf8(_)) => Err(on_surrogate()),
        Err(_) => {
            if ctx.has_exception() {
                let _ = ctx.catch();
            }
            Err(on_not_string())
        }
    }
}

fn error_object<'js>(ctx: &Ctx<'js>, err: &FetchError) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    set_null_proto(ctx, &obj)?;
    obj.prop(
        "ok",
        Property::from(false).writable().enumerable().configurable(),
    )?;
    obj.prop(
        "code",
        Property::from(err.code().as_str())
            .writable()
            .enumerable()
            .configurable(),
    )?;
    obj.prop(
        "message",
        Property::from(err.to_string())
            .writable()
            .enumerable()
            .configurable(),
    )?;
    if let FetchError::HttpStatus { status, .. } = err {
        obj.prop(
            "status",
            Property::from(f64::from(*status))
                .writable()
                .enumerable()
                .configurable(),
        )?;
    }
    if let Some(url) = err.url() {
        obj.prop(
            "url",
            Property::from(url).writable().enumerable().configurable(),
        )?;
    }
    Ok(obj)
}

fn invalid_option(reason: &'static str) -> FetchError {
    FetchError::InvalidOption { reason }
}

fn read_strict_string_type<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &'static str,
) -> Result<String> {
    read_strict_string(
        ctx,
        value,
        || invalid_option(type_msg),
        || invalid_option("string contains an unpaired UTF-16 surrogate"),
    )
    .map_err(|err| Exception::throw_type(ctx, &err.to_string()))
}

fn parse_string_pairs<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    label: &'static str,
) -> Result<std::result::Result<Vec<(String, String)>, FetchError>> {
    if value.is_undefined() {
        return Ok(Ok(Vec::new()));
    }
    if !value.is_object()
        || value.is_proxy()
        || unsafe { qjs::JS_IsFunction(ctx.as_raw().as_ptr(), value.as_raw()) }
    {
        return Err(Exception::throw_type(ctx, label));
    }
    if unsafe { qjs::JS_IsArray(value.as_raw()) } {
        let Some(arr) = value.as_array() else {
            return Err(Exception::throw_type(ctx, label));
        };
        let len = arr.len();
        if len > MAX_PAIR_COUNT as usize {
            return Ok(Err(invalid_option("too many header or query entries")));
        }
        let mut pairs = Vec::with_capacity(len);
        for idx in 0..len as u32 {
            let elem =
                own_array_element(ctx, arr, idx, "header/query entry must not use an accessor")?;
            let Some(elem) = elem else {
                return Err(Exception::throw_type(ctx, label));
            };
            if !elem.is_object() || elem.is_proxy() || !unsafe { qjs::JS_IsArray(elem.as_raw()) } {
                return Err(Exception::throw_type(ctx, label));
            }
            let Some(pair_arr) = elem.as_array() else {
                return Err(Exception::throw_type(ctx, label));
            };
            if pair_arr.len() != 2 {
                return Err(Exception::throw_type(ctx, label));
            }
            let Some(k_val) =
                own_array_element(ctx, pair_arr, 0, "entry key must not use an accessor")?
            else {
                return Err(Exception::throw_type(ctx, label));
            };
            let Some(v_val) =
                own_array_element(ctx, pair_arr, 1, "entry value must not use an accessor")?
            else {
                return Err(Exception::throw_type(ctx, label));
            };
            let key = read_strict_string_type(ctx, &k_val, label)?;
            let val = read_strict_string_type(ctx, &v_val, label)?;
            pairs.push((key, val));
        }
        return Ok(Ok(pairs));
    }

    let mut pairs = Vec::new();
    let mut over_limit = false;
    parse_options(ctx, Some(value), label, |ctx, key, val| {
        if val.is_undefined() {
            return Ok(());
        }
        if pairs.len() >= MAX_PAIR_COUNT as usize {
            over_limit = true;
            return Ok(());
        }
        let v = read_strict_string_type(ctx, val, label)?;
        pairs.push((key.to_owned(), v));
        Ok(())
    })?;
    if over_limit {
        return Ok(Err(invalid_option("too many header or query entries")));
    }
    Ok(Ok(pairs))
}

fn parse_timeout_ms<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
) -> Result<std::result::Result<Option<Duration>, FetchError>> {
    if value.is_undefined() {
        return Ok(Ok(None));
    }
    let Some(num) = value.as_number().filter(|_| value.is_number()) else {
        return Err(Exception::throw_type(ctx, "timeoutMs must be a number"));
    };
    let max_ms = MAX_TIMEOUT.as_secs_f64() * 1000.0;
    if !num.is_finite() || num <= 0.0 || num > max_ms {
        return Ok(Err(invalid_option(
            "timeoutMs must be between 0 and 600000 ms",
        )));
    }
    let dur = Duration::from_secs_f64(num / 1000.0);
    if dur.is_zero() {
        return Ok(Err(invalid_option("timeoutMs must be greater than zero")));
    }
    Ok(Ok(Some(dur)))
}

fn parse_bounded_usize<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    min: usize,
    max: usize,
    type_msg: &'static str,
    reason: &'static str,
) -> Result<std::result::Result<Option<usize>, FetchError>> {
    if value.is_undefined() {
        return Ok(Ok(None));
    }
    let Some(num) = value.as_number().filter(|_| value.is_number()) else {
        return Err(Exception::throw_type(ctx, type_msg));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < min as f64 || num > max as f64 {
        return Ok(Err(invalid_option(reason)));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Ok(Some(num as usize)))
}

fn parse_bounded_u64<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    min: u64,
    max: u64,
    type_msg: &'static str,
    reason: &'static str,
) -> Result<std::result::Result<Option<u64>, FetchError>> {
    if value.is_undefined() {
        return Ok(Ok(None));
    }
    let Some(num) = value.as_number().filter(|_| value.is_number()) else {
        return Err(Exception::throw_type(ctx, type_msg));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < min as f64 || num > max as f64 {
        return Ok(Err(invalid_option(reason)));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Ok(Some(num as u64)))
}

fn parse_bool_opt<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &'static str,
) -> Result<Option<bool>> {
    if value.is_undefined() {
        return Ok(None);
    }
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| Exception::throw_type(ctx, type_msg))
}

fn parse_user_agent_opt<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<Option<Option<String>>> {
    if value.is_undefined() {
        return Ok(None);
    }
    if value.is_null() {
        return Ok(Some(None));
    }
    read_strict_string_type(ctx, value, "userAgent must be a string or null").map(|s| Some(Some(s)))
}

fn qjs_to_json<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    depth: usize,
    nodes: &mut usize,
    ancestors: &mut Vec<Value<'js>>,
) -> Result<std::result::Result<serde_json::Value, FetchError>> {
    if depth > MAX_JSON_DEPTH {
        return Ok(Err(invalid_option("json body nesting depth exceeds limit")));
    }
    if *nodes >= MAX_JSON_NODES {
        return Ok(Err(invalid_option("json body exceeds maximum node count")));
    }
    *nodes += 1;

    if value.is_null() {
        return Ok(Ok(serde_json::Value::Null));
    }
    if let Some(b) = value.as_bool() {
        return Ok(Ok(serde_json::Value::Bool(b)));
    }
    if value.is_number() {
        let Some(num) = value.as_number() else {
            return Ok(Err(invalid_option("invalid number in json option")));
        };
        if !num.is_finite() {
            return Ok(Err(invalid_option(
                "json option must not contain NaN or Infinity",
            )));
        }
        if num.fract() == 0.0 && (-MAX_SAFE_INTEGER_F64..=MAX_SAFE_INTEGER_F64).contains(&num) {
            #[allow(clippy::cast_possible_truncation)]
            return Ok(Ok(serde_json::Value::Number(serde_json::Number::from(
                num as i64,
            ))));
        }
        let Some(number) = serde_json::Number::from_f64(num) else {
            return Ok(Err(invalid_option(
                "invalid floating-point number in json option",
            )));
        };
        return Ok(Ok(serde_json::Value::Number(number)));
    }
    if value.is_string() {
        return Ok(read_strict_string(
            ctx,
            value,
            || invalid_option("json string expected"),
            || invalid_option("json option contains an unpaired UTF-16 surrogate"),
        )
        .map(serde_json::Value::String));
    }
    let raw_ctx = ctx.as_raw().as_ptr();
    if value.is_undefined()
        || value.is_symbol()
        || value.is_big_int()
        || value.is_proxy()
        || unsafe { qjs::JS_IsFunction(raw_ctx, value.as_raw()) }
    {
        return Ok(Err(invalid_option(
            "json option contains an unsupported value",
        )));
    }
    if unsafe { qjs::JS_IsArray(value.as_raw()) } {
        let Some(arr) = value.as_array() else {
            return Ok(Err(invalid_option("invalid array in json option")));
        };
        if ancestors.iter().any(|a| a == value) {
            return Ok(Err(invalid_option(
                "json option contains a circular reference",
            )));
        }
        let len = arr.len();
        if len > MAX_JSON_NODES - *nodes {
            return Ok(Err(invalid_option("json array exceeds maximum node count")));
        }
        ancestors.push(value.clone());
        let mut out = Vec::with_capacity(len);
        for idx in 0..len as u32 {
            let elem = match own_array_element(
                ctx,
                arr,
                idx,
                "options must not contain accessor properties",
            ) {
                Ok(e) => e,
                Err(err) => {
                    ancestors.pop();
                    return Err(err);
                }
            };
            match elem {
                Some(v) if !v.is_undefined() => {
                    match qjs_to_json(ctx, &v, depth + 1, nodes, ancestors) {
                        Ok(Ok(parsed)) => out.push(parsed),
                        Ok(Err(fe)) => {
                            ancestors.pop();
                            return Ok(Err(fe));
                        }
                        Err(err) => {
                            ancestors.pop();
                            return Err(err);
                        }
                    }
                }
                _ => out.push(serde_json::Value::Null),
            }
        }
        ancestors.pop();
        return Ok(Ok(serde_json::Value::Array(out)));
    }
    if value.is_object() {
        let proto = get_prototype(ctx, value.as_raw())?;
        let plain = Object::new(ctx.clone())?;
        let plain_proto = get_prototype(ctx, plain.as_raw())?;
        if !proto.is_null() && proto != plain_proto {
            return Ok(Err(invalid_option(
                "json option objects must be plain objects",
            )));
        }
        let raw_val = value.as_raw();
        if ancestors.iter().any(|a| a == value) {
            return Ok(Err(invalid_option(
                "json option contains a circular reference",
            )));
        }
        let mut tab: *mut qjs::JSPropertyEnum = std::ptr::null_mut();
        let mut len: u32 = 0;
        let flags = (qjs::JS_GPN_STRING_MASK | qjs::JS_GPN_SYMBOL_MASK) as std::ffi::c_int;
        let rc =
            unsafe { qjs::JS_GetOwnPropertyNames(raw_ctx, &mut tab, &mut len, raw_val, flags) };
        let _guard = PropertyEnumGuard {
            raw: raw_ctx,
            tab,
            len,
        };
        if rc < 0 {
            if ctx.has_exception() {
                let _ = ctx.catch();
            }
            return Ok(Err(invalid_option(
                "failed to enumerate json object properties",
            )));
        }
        ancestors.push(value.clone());
        let mut map = serde_json::Map::new();
        for i in 0..len {
            let atom = unsafe { (*tab.add(i as usize)).atom };
            let atom_val =
                unsafe { Value::from_raw(ctx.clone(), qjs::JS_AtomToValue(raw_ctx, atom)) };
            if atom_val.is_symbol() {
                continue;
            }
            let mut desc = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
            let found =
                unsafe { qjs::JS_GetOwnProperty(raw_ctx, desc.as_mut_ptr(), raw_val, atom) };
            if found <= 0 {
                if ctx.has_exception() {
                    let _ = ctx.catch();
                }
                continue;
            }
            let desc = unsafe { desc.assume_init() };
            let field_val = unsafe { Value::from_raw(ctx.clone(), desc.value) };
            let _getter = unsafe { Value::from_raw(ctx.clone(), desc.getter) };
            let _setter = unsafe { Value::from_raw(ctx.clone(), desc.setter) };
            if (desc.flags as u32 & qjs::JS_PROP_GETSET) != 0 {
                ancestors.pop();
                return Err(Exception::throw_type(
                    ctx,
                    "options must not contain accessor properties",
                ));
            }
            if (desc.flags as u32 & qjs::JS_PROP_ENUMERABLE) == 0 || field_val.is_undefined() {
                continue;
            }
            let key = match read_strict_string(
                ctx,
                &atom_val,
                || invalid_option("json property key must be a string"),
                || invalid_option("json property key contains an unpaired UTF-16 surrogate"),
            ) {
                Ok(k) => k,
                Err(fe) => {
                    ancestors.pop();
                    return Ok(Err(fe));
                }
            };
            match qjs_to_json(ctx, &field_val, depth + 1, nodes, ancestors) {
                Ok(Ok(parsed)) => {
                    map.insert(key, parsed);
                }
                Ok(Err(fe)) => {
                    ancestors.pop();
                    return Ok(Err(fe));
                }
                Err(err) => {
                    ancestors.pop();
                    return Err(err);
                }
            }
        }
        ancestors.pop();
        return Ok(Ok(serde_json::Value::Object(map)));
    }
    Ok(Err(invalid_option(
        "json option contains an unsupported value",
    )))
}

fn parse_body_or_json<'js>(
    ctx: &Ctx<'js>,
    body_val: Option<&Value<'js>>,
    json_val: Option<&Value<'js>>,
) -> Result<std::result::Result<FetchBody, FetchError>> {
    let has_body = body_val.is_some_and(|v| !v.is_undefined());
    let has_json = json_val.is_some_and(|v| !v.is_undefined());
    if has_body && has_json {
        return Ok(Err(invalid_option(
            "body and json options are mutually exclusive",
        )));
    }
    if let Some(body) = body_val
        && !body.is_undefined()
    {
        if body.is_string() {
            return Ok(read_strict_string(
                ctx,
                body,
                || invalid_option("body must be a string or Uint8Array"),
                || invalid_option("body string contains an unpaired UTF-16 surrogate"),
            )
            .map(FetchBody::Text));
        }
        let bytes =
            with_uint8_array_bytes(ctx, body, "body must be a string or Uint8Array", |slice| {
                Ok(slice.to_vec())
            })?;
        return Ok(Ok(FetchBody::Bytes(bytes)));
    }
    if let Some(json) = json_val
        && !json.is_undefined()
    {
        let mut nodes = 0usize;
        let mut ancestors = Vec::new();
        return match qjs_to_json(ctx, json, 0, &mut nodes, &mut ancestors)? {
            Ok(parsed) => Ok(Ok(FetchBody::Json(parsed))),
            Err(e) => Ok(Err(e)),
        };
    }
    Ok(Ok(FetchBody::None))
}

#[derive(Default)]
struct RawFetchFields<'js> {
    method: Option<Value<'js>>,
    headers: Option<Value<'js>>,
    query: Option<Value<'js>>,
    bearer_token: Option<Value<'js>>,
    body: Option<Value<'js>>,
    json: Option<Value<'js>>,
    timeout_ms: Option<Value<'js>>,
    max_response_bytes: Option<Value<'js>>,
    max_redirects: Option<Value<'js>>,
    allow_private_ips: Option<Value<'js>>,
    user_agent: Option<Value<'js>>,
}

fn build_fetch_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
) -> Result<std::result::Result<FetchOptions, FetchError>> {
    let mut fields = RawFetchFields::default();
    let mut unknown = false;
    parse_options(
        ctx,
        value,
        "options must be a plain object",
        |_ctx, key, val| {
            if unknown {
                return Ok(());
            }
            match key {
                "method" => fields.method = Some(val.clone()),
                "headers" => fields.headers = Some(val.clone()),
                "query" => fields.query = Some(val.clone()),
                "bearerToken" => fields.bearer_token = Some(val.clone()),
                "body" => fields.body = Some(val.clone()),
                "json" => fields.json = Some(val.clone()),
                "timeoutMs" => fields.timeout_ms = Some(val.clone()),
                "maxResponseBytes" => fields.max_response_bytes = Some(val.clone()),
                "maxRedirects" => fields.max_redirects = Some(val.clone()),
                "allowPrivateIps" => fields.allow_private_ips = Some(val.clone()),
                "userAgent" => fields.user_agent = Some(val.clone()),
                _ => unknown = true,
            }
            Ok(())
        },
    )?;
    if unknown {
        return Err(Exception::throw_type(ctx, "unknown fetch option"));
    }

    let mut opts = FetchOptions::new();

    if let Some(ref val) = fields.method
        && !val.is_undefined()
    {
        opts.method = read_strict_string_type(ctx, val, "method must be a string")?;
    }

    if let Some(ref val) = fields.headers {
        match parse_string_pairs(
            ctx,
            val,
            "headers must be a plain object or [name, value] array",
        )? {
            Ok(pairs) => opts.headers = pairs,
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.query {
        match parse_string_pairs(
            ctx,
            val,
            "query must be a plain object or [key, value] array",
        )? {
            Ok(pairs) => opts.query = pairs,
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.bearer_token
        && !val.is_undefined()
    {
        let token = read_strict_string_type(ctx, val, "bearerToken must be a string")?;
        opts.bearer_token = Some(Zeroizing::new(token));
    }

    match parse_body_or_json(ctx, fields.body.as_ref(), fields.json.as_ref())? {
        Ok(body) => opts.body = body,
        Err(e) => return Ok(Err(e)),
    }

    if let Some(ref val) = fields.timeout_ms {
        match parse_timeout_ms(ctx, val)? {
            Ok(Some(dur)) => opts.timeout = dur,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.max_response_bytes {
        match parse_bounded_usize(
            ctx,
            val,
            1,
            MAX_ALLOWED_RESPONSE_BYTES,
            "maxResponseBytes must be a number",
            "maxResponseBytes must be an integer between 1 and 268435456",
        )? {
            Ok(Some(n)) => opts.max_response_bytes = n,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.max_redirects {
        match parse_bounded_usize(
            ctx,
            val,
            0,
            MAX_ALLOWED_REDIRECTS,
            "maxRedirects must be a number",
            "maxRedirects must be an integer between 0 and 20",
        )? {
            Ok(Some(n)) => opts.max_redirects = n,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.allow_private_ips
        && let Some(b) = parse_bool_opt(ctx, val, "allowPrivateIps must be a boolean")?
    {
        opts.allow_private_ips = b;
    }

    if let Some(ref val) = fields.user_agent
        && let Some(ua) = parse_user_agent_opt(ctx, val)?
    {
        opts.user_agent = ua;
    }

    Ok(Ok(opts))
}

#[derive(Default)]
struct RawDownloadFields<'js> {
    headers: Option<Value<'js>>,
    query: Option<Value<'js>>,
    bearer_token: Option<Value<'js>>,
    timeout_ms: Option<Value<'js>>,
    max_redirects: Option<Value<'js>>,
    allow_private_ips: Option<Value<'js>>,
    user_agent: Option<Value<'js>>,
    max_bytes: Option<Value<'js>>,
    expected_sha256: Option<Value<'js>>,
    overwrite: Option<Value<'js>>,
    on_progress: Option<Value<'js>>,
}

fn build_download_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
) -> Result<std::result::Result<(DownloadOptions, Option<Function<'js>>), FetchError>> {
    let mut fields = RawDownloadFields::default();
    let mut unknown = false;
    parse_options(
        ctx,
        value,
        "options must be a plain object",
        |_ctx, key, val| {
            if unknown {
                return Ok(());
            }
            match key {
                "headers" => fields.headers = Some(val.clone()),
                "query" => fields.query = Some(val.clone()),
                "bearerToken" => fields.bearer_token = Some(val.clone()),
                "timeoutMs" => fields.timeout_ms = Some(val.clone()),
                "maxRedirects" => fields.max_redirects = Some(val.clone()),
                "allowPrivateIps" => fields.allow_private_ips = Some(val.clone()),
                "userAgent" => fields.user_agent = Some(val.clone()),
                "maxBytes" => fields.max_bytes = Some(val.clone()),
                "expectedSha256" => fields.expected_sha256 = Some(val.clone()),
                "overwrite" => fields.overwrite = Some(val.clone()),
                "onProgress" => fields.on_progress = Some(val.clone()),
                _ => unknown = true,
            }
            Ok(())
        },
    )?;
    if unknown {
        return Err(Exception::throw_type(ctx, "unknown download option"));
    }

    let mut opts = DownloadOptions::new();

    if let Some(ref val) = fields.headers {
        match parse_string_pairs(
            ctx,
            val,
            "headers must be a plain object or [name, value] array",
        )? {
            Ok(pairs) => opts.headers = pairs,
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.query {
        match parse_string_pairs(
            ctx,
            val,
            "query must be a plain object or [key, value] array",
        )? {
            Ok(pairs) => opts.query = pairs,
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.bearer_token
        && !val.is_undefined()
    {
        let token = read_strict_string_type(ctx, val, "bearerToken must be a string")?;
        opts.bearer_token = Some(Zeroizing::new(token));
    }

    if let Some(ref val) = fields.timeout_ms {
        match parse_timeout_ms(ctx, val)? {
            Ok(Some(dur)) => opts.timeout = dur,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.max_redirects {
        match parse_bounded_usize(
            ctx,
            val,
            0,
            MAX_ALLOWED_REDIRECTS,
            "maxRedirects must be a number",
            "maxRedirects must be an integer between 0 and 20",
        )? {
            Ok(Some(n)) => opts.max_redirects = n,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.allow_private_ips
        && let Some(b) = parse_bool_opt(ctx, val, "allowPrivateIps must be a boolean")?
    {
        opts.allow_private_ips = b;
    }

    if let Some(ref val) = fields.user_agent
        && let Some(ua) = parse_user_agent_opt(ctx, val)?
    {
        opts.user_agent = ua;
    }

    if let Some(ref val) = fields.max_bytes {
        match parse_bounded_u64(
            ctx,
            val,
            1,
            MAX_ALLOWED_DOWNLOAD_BYTES,
            "maxBytes must be a number",
            "maxBytes must be an integer between 1 and 4294967296",
        )? {
            Ok(Some(n)) => opts.max_bytes = n,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.expected_sha256
        && !val.is_undefined()
    {
        opts.expected_sha256 = Some(read_strict_string_type(
            ctx,
            val,
            "expectedSha256 must be a string",
        )?);
    }

    if let Some(ref val) = fields.overwrite
        && let Some(b) = parse_bool_opt(ctx, val, "overwrite must be a boolean")?
    {
        opts.overwrite = b;
    }

    let on_progress = if let Some(ref val) = fields.on_progress
        && !val.is_undefined()
    {
        let Some(func) = val.as_function().cloned() else {
            return Err(Exception::throw_type(ctx, "onProgress must be a function"));
        };
        Some(func)
    } else {
        None
    };

    Ok(Ok((opts, on_progress)))
}

fn host_fetch_sync<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 2 {
        return Err(Exception::throw_type(
            &ctx,
            "internal fetchSync arity error",
        ));
    }
    let url = read_strict_string_type(&ctx, &args.0[0], "URL must be a string")?;

    let opts = match build_fetch_options(&ctx, args.0.get(1))? {
        Ok(opts) => opts,
        Err(e) => return error_object(&ctx, &e),
    };

    let response = match fetch_sync(&url, &opts) {
        Ok(resp) => resp,
        Err(e) => return error_object(&ctx, &e),
    };

    let headers_arr = Array::new(ctx.clone())?;
    for (idx, (k, v)) in response.headers.into_iter().enumerate() {
        let pair = Array::new(ctx.clone())?;
        pair.as_object().prop(
            0u32,
            Property::from(k).writable().enumerable().configurable(),
        )?;
        pair.as_object().prop(
            1u32,
            Property::from(v).writable().enumerable().configurable(),
        )?;
        headers_arr.as_object().prop(
            idx as u32,
            Property::from(pair).writable().enumerable().configurable(),
        )?;
    }

    let body = TypedArray::<u8>::new(ctx.clone(), response.body)?;
    let out = Object::new(ctx.clone())?;
    set_null_proto(&ctx, &out)?;
    out.prop(
        "ok",
        Property::from(true).writable().enumerable().configurable(),
    )?;
    out.prop(
        "status",
        Property::from(f64::from(response.status))
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "statusText",
        Property::from(response.status_text)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "isOk",
        Property::from(response.ok)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "url",
        Property::from(response.url)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "redirected",
        Property::from(response.redirected)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "headers",
        Property::from(headers_arr)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "body",
        Property::from(body).writable().enumerable().configurable(),
    )?;
    Ok(out)
}

fn host_download_sync<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 3 {
        return Err(Exception::throw_type(
            &ctx,
            "internal downloadSync arity error",
        ));
    }
    let url = read_strict_string_type(&ctx, &args.0[0], "URL must be a string")?;
    let dest_path = read_strict_string_type(&ctx, &args.0[1], "destPath must be a string")?;

    let (opts, on_progress) = match build_download_options(&ctx, args.0.get(2))? {
        Ok(pair) => pair,
        Err(e) => return error_object(&ctx, &e),
    };

    let cancel = AtomicBool::new(false);
    let mut callback_error: Option<Value<'js>> = None;
    let result = if let Some(func) = on_progress {
        let mut cb = |progress: DownloadProgress| -> std::result::Result<(), FetchError> {
            if callback_error.is_some() {
                return Err(FetchError::Io {
                    reason: "aborted by onProgress".to_owned(),
                });
            }
            let obj = Object::new(ctx.clone()).map_err(|_| FetchError::Io {
                reason: "failed to allocate progress object".to_owned(),
            })?;
            set_null_proto(&ctx, &obj).map_err(|_| FetchError::Io {
                reason: "failed to initialize progress object".to_owned(),
            })?;
            obj.prop(
                "bytesWritten",
                Property::from(progress.bytes_written as f64).enumerable(),
            )
            .map_err(|_| FetchError::Io {
                reason: "failed to set progress bytesWritten".to_owned(),
            })?;
            let total = match progress.total_bytes {
                Some(n) => Value::new_number(ctx.clone(), n as f64),
                None => Value::new_null(ctx.clone()),
            };
            obj.prop("totalBytes", Property::from(total).enumerable())
                .map_err(|_| FetchError::Io {
                    reason: "failed to set progress totalBytes".to_owned(),
                })?;
            freeze_native(&ctx, &obj).map_err(|_| FetchError::Io {
                reason: "failed to freeze progress object".to_owned(),
            })?;
            if let Err(err) = func.call::<_, Value<'js>>((obj,)) {
                if matches!(err, rquickjs::Error::Exception) && ctx.has_exception() {
                    callback_error = Some(ctx.catch());
                }
                cancel.store(true, Ordering::Release);
                return Err(FetchError::Io {
                    reason: "aborted by onProgress".to_owned(),
                });
            }
            Ok(())
        };
        download_sync_cancellable(
            &url,
            Path::new(&dest_path),
            &opts,
            Some(&cancel),
            Some(&mut cb),
        )
    } else {
        download_sync_cancellable(&url, Path::new(&dest_path), &opts, None, None)
    };

    if let Some(exc) = callback_error {
        return Err(ctx.throw(exc));
    }

    let info = match result {
        Ok(info) => info,
        Err(e) => return error_object(&ctx, &e),
    };

    let out = Object::new(ctx.clone())?;
    set_null_proto(&ctx, &out)?;
    out.prop(
        "ok",
        Property::from(true).writable().enumerable().configurable(),
    )?;
    out.prop(
        "path",
        Property::from(info.path.to_string_lossy().into_owned())
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "status",
        Property::from(f64::from(info.status))
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "url",
        Property::from(info.url)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "bytesWritten",
        Property::from(info.bytes_written as f64)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "sha256",
        Property::from(info.sha256)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    let content_type = match info.content_type {
        Some(ct) => JsString::from_str(ctx.clone(), &ct)?.into_value(),
        None => Value::new_null(ctx.clone()),
    };
    out.prop(
        "contentType",
        Property::from(content_type)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    let etag = match info.etag {
        Some(et) => JsString::from_str(ctx.clone(), &et)?.into_value(),
        None => Value::new_null(ctx.clone()),
    };
    out.prop(
        "etag",
        Property::from(etag).writable().enumerable().configurable(),
    )?;
    Ok(out)
}

fn host_decode_utf8_strict<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "decodeUtf8Strict takes one argument",
        ));
    }
    let decoded = with_uint8_array_bytes(&ctx, &args.0[0], "expected Uint8Array", |slice| {
        Ok(std::str::from_utf8(slice).map(str::to_owned))
    })?;
    match decoded {
        Ok(text) => {
            let out = Object::new(ctx.clone())?;
            set_null_proto(&ctx, &out)?;
            out.prop(
                "ok",
                Property::from(true).writable().enumerable().configurable(),
            )?;
            out.prop(
                "text",
                Property::from(text).writable().enumerable().configurable(),
            )?;
            Ok(out)
        }
        Err(_) => error_object(&ctx, &FetchError::InvalidUtf8),
    }
}

/// The native QuickJS module served for `vael:fetch`.
pub(super) struct FetchModule;

impl ModuleDef for FetchModule {
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
        let Some(fetch) = state.fetch.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = fetch.clone().restore(ctx)?;
        for name in EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        Ok(())
    }
}

// Keep unused error code imports honest for exhaustive lint coverage.
const _: fn(FetchErrorCode) -> &'static str = FetchErrorCode::as_str;

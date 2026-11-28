//! Native `vael:fetch` synchronous bindings for QuickJS.
//!
//! All SSRF guards, redirect validation, bounded reads, atomic temporary-file
//! streaming, SHA-256 verification, and `VAEL_FETCH_*` error codes live in
//! [`vaeljs_fetch_core`]. This module wires synchronous QuickJS host
//! operations into `fetch.js` (which captures ECMAScript primordials at realm
//! initialization), builds one frozen registry object per context, and serves
//! those exact exports from both `Vael.getBuiltInModule("vael:fetch")` and
//! `import ... from "vael:fetch"` without installing `globalThis.fetch`.

use std::path::Path;
use std::time::Duration;

use rquickjs::{
    Array, Ctx, Exception, Function, Object, Result, String as JsString, TypedArray, Value,
    function::Rest, module::ModuleDef, object::Property, qjs,
};
use vaeljs_fetch_core::{
    DownloadOptions, DownloadProgress, FetchBody, FetchError, FetchErrorCode, FetchOptions,
    MAX_ALLOWED_DOWNLOAD_BYTES, MAX_ALLOWED_REDIRECTS, MAX_ALLOWED_RESPONSE_BYTES, MAX_TIMEOUT,
    Zeroizing, download_sync, fetch_sync,
};

use crate::native::{freeze_native, own_array_element, parse_options, with_uint8_array_bytes};

/// Canonical module specifier for `vael:fetch`.
pub(super) const SPECIFIER: &str = vaeljs_fetch_core::MODULE_SPECIFIER;

/// Exact sorted named exports of `vael:fetch`.
pub(super) const EXPORTS: &[&str] = &["FetchError", "downloadSync", "fetchSync"];

const FACTORY_SOURCE: &str = include_str!("fetch.js");
const MAX_PAIR_COUNT: u32 = 4096;

/// Creates the frozen public registry object for `vael:fetch`.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let factory: Function<'js> = ctx.eval(FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    host.set(
        "fetchSync",
        Function::new(ctx.clone(), host_fetch_sync)?
            .with_name("fetchSync")?
            .with_length(3)?,
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
    Ok(obj)
}

fn invalid_option(reason: &'static str) -> FetchError {
    FetchError::InvalidOption { reason }
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
        return Ok(Err(invalid_option(label)));
    }
    if unsafe { qjs::JS_IsArray(value.as_raw()) } {
        let Some(arr) = value.as_array() else {
            return Ok(Err(invalid_option(label)));
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
                return Ok(Err(invalid_option(label)));
            };
            if !elem.is_object() || elem.is_proxy() || !unsafe { qjs::JS_IsArray(elem.as_raw()) } {
                return Ok(Err(invalid_option(label)));
            }
            let Some(pair_arr) = elem.as_array() else {
                return Ok(Err(invalid_option(label)));
            };
            if pair_arr.len() != 2 {
                return Ok(Err(invalid_option(label)));
            }
            let Some(k_val) =
                own_array_element(ctx, pair_arr, 0, "entry key must not use an accessor")?
            else {
                return Ok(Err(invalid_option(label)));
            };
            let Some(v_val) =
                own_array_element(ctx, pair_arr, 1, "entry value must not use an accessor")?
            else {
                return Ok(Err(invalid_option(label)));
            };
            let key = match read_strict_string(
                ctx,
                &k_val,
                || invalid_option(label),
                || invalid_option("entry key contains an unpaired UTF-16 surrogate"),
            ) {
                Ok(k) => k,
                Err(e) => return Ok(Err(e)),
            };
            let val = match read_strict_string(
                ctx,
                &v_val,
                || invalid_option(label),
                || invalid_option("entry value contains an unpaired UTF-16 surrogate"),
            ) {
                Ok(v) => v,
                Err(e) => return Ok(Err(e)),
            };
            pairs.push((key, val));
        }
        return Ok(Ok(pairs));
    }

    let mut pairs = Vec::new();
    let mut bad: Option<FetchError> = None;
    parse_options(
        ctx,
        Some(value),
        "options entry must be a plain object",
        |ctx, key, val| {
            if bad.is_some() || val.is_undefined() {
                return Ok(());
            }
            if pairs.len() >= MAX_PAIR_COUNT as usize {
                bad = Some(invalid_option("too many header or query entries"));
                return Ok(());
            }
            match read_strict_string(
                ctx,
                val,
                || invalid_option(label),
                || invalid_option("entry value contains an unpaired UTF-16 surrogate"),
            ) {
                Ok(v) => pairs.push((key.to_owned(), v)),
                Err(e) => bad = Some(e),
            }
            Ok(())
        },
    )?;
    if let Some(err) = bad {
        return Ok(Err(err));
    }
    Ok(Ok(pairs))
}

fn parse_timeout_ms(value: &Value<'_>) -> std::result::Result<Option<Duration>, FetchError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Some(num) = value.as_number().filter(|_| value.is_number()) else {
        return Err(invalid_option("timeoutMs must be a positive finite number"));
    };
    let max_ms = MAX_TIMEOUT.as_secs_f64() * 1000.0;
    if !num.is_finite() || num <= 0.0 || num > max_ms {
        return Err(invalid_option("timeoutMs must be between 0 and 600000 ms"));
    }
    let dur = Duration::from_secs_f64(num / 1000.0);
    if dur.is_zero() {
        return Err(invalid_option("timeoutMs must be greater than zero"));
    }
    Ok(Some(dur))
}

fn parse_bounded_usize(
    value: &Value<'_>,
    min: usize,
    max: usize,
    reason: &'static str,
) -> std::result::Result<Option<usize>, FetchError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Some(num) = value.as_number().filter(|_| value.is_number()) else {
        return Err(invalid_option(reason));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < min as f64 || num > max as f64 {
        return Err(invalid_option(reason));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Some(num as usize))
}

fn parse_bounded_u64(
    value: &Value<'_>,
    min: u64,
    max: u64,
    reason: &'static str,
) -> std::result::Result<Option<u64>, FetchError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Some(num) = value.as_number().filter(|_| value.is_number()) else {
        return Err(invalid_option(reason));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < min as f64 || num > max as f64 {
        return Err(invalid_option(reason));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Some(num as u64))
}

fn parse_bool_opt(
    value: &Value<'_>,
    reason: &'static str,
) -> std::result::Result<Option<bool>, FetchError> {
    if value.is_undefined() {
        return Ok(None);
    }
    value
        .as_bool()
        .map(Some)
        .ok_or_else(|| invalid_option(reason))
}

fn parse_user_agent_opt<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
) -> std::result::Result<Option<Option<String>>, FetchError> {
    if value.is_undefined() {
        return Ok(None);
    }
    if value.is_null() {
        return Ok(Some(None));
    }
    read_strict_string(
        ctx,
        value,
        || invalid_option("userAgent must be a string or null"),
        || invalid_option("userAgent contains an unpaired UTF-16 surrogate"),
    )
    .map(|s| Some(Some(s)))
}

fn parse_body_or_json<'js>(
    ctx: &Ctx<'js>,
    body_val: Option<&Value<'js>>,
    json_val: Option<&Value<'js>>,
    json_stringify: &Function<'js>,
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
        return match with_uint8_array_bytes(
            ctx,
            body,
            "body must be a string or Uint8Array",
            |slice| Ok(slice.to_vec()),
        ) {
            Ok(bytes) => Ok(Ok(FetchBody::Bytes(bytes))),
            Err(_) => {
                if ctx.has_exception() {
                    let _ = ctx.catch();
                }
                Ok(Err(invalid_option("body must be a string or Uint8Array")))
            }
        };
    }
    if let Some(json) = json_val
        && !json.is_undefined()
    {
        let serialized: Value<'js> = match json_stringify.call((json.clone(),)) {
            Ok(v) => v,
            Err(_) => {
                if ctx.has_exception() {
                    let _ = ctx.catch();
                }
                return Ok(Err(invalid_option("json option could not be serialized")));
            }
        };
        if serialized.is_undefined() {
            return Ok(Err(invalid_option("json option is not JSON-serializable")));
        }
        let text = match read_strict_string(
            ctx,
            &serialized,
            || invalid_option("json option could not be serialized"),
            || invalid_option("json option contains an unpaired UTF-16 surrogate"),
        ) {
            Ok(t) => t,
            Err(e) => return Ok(Err(e)),
        };
        let parsed: serde_json::Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return Ok(Err(invalid_option("json option produced invalid JSON"))),
        };
        return Ok(Ok(FetchBody::Json(parsed)));
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
    json_stringify: &Function<'js>,
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
        return Ok(Err(invalid_option("unknown fetch option")));
    }

    let mut opts = FetchOptions::new();

    if let Some(ref val) = fields.method
        && !val.is_undefined()
    {
        match read_strict_string(
            ctx,
            val,
            || invalid_option("method must be a string"),
            || invalid_option("method contains an unpaired UTF-16 surrogate"),
        ) {
            Ok(m) => opts.method = m,
            Err(e) => return Ok(Err(e)),
        }
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
        match read_strict_string(
            ctx,
            val,
            || invalid_option("bearerToken must be a string"),
            || invalid_option("bearerToken contains an unpaired UTF-16 surrogate"),
        ) {
            Ok(token) => opts.bearer_token = Some(Zeroizing::new(token)),
            Err(e) => return Ok(Err(e)),
        }
    }

    match parse_body_or_json(
        ctx,
        fields.body.as_ref(),
        fields.json.as_ref(),
        json_stringify,
    )? {
        Ok(body) => opts.body = body,
        Err(e) => return Ok(Err(e)),
    }

    if let Some(ref val) = fields.timeout_ms {
        match parse_timeout_ms(val) {
            Ok(Some(dur)) => opts.timeout = dur,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.max_response_bytes {
        match parse_bounded_usize(
            val,
            1,
            MAX_ALLOWED_RESPONSE_BYTES,
            "maxResponseBytes must be an integer between 1 and 268435456",
        ) {
            Ok(Some(n)) => opts.max_response_bytes = n,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.max_redirects {
        match parse_bounded_usize(
            val,
            0,
            MAX_ALLOWED_REDIRECTS,
            "maxRedirects must be an integer between 0 and 20",
        ) {
            Ok(Some(n)) => opts.max_redirects = n,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.allow_private_ips {
        match parse_bool_opt(val, "allowPrivateIps must be a boolean") {
            Ok(Some(b)) => opts.allow_private_ips = b,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.user_agent {
        match parse_user_agent_opt(ctx, val) {
            Ok(Some(ua)) => opts.user_agent = ua,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
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
        return Ok(Err(invalid_option("unknown download option")));
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
        match read_strict_string(
            ctx,
            val,
            || invalid_option("bearerToken must be a string"),
            || invalid_option("bearerToken contains an unpaired UTF-16 surrogate"),
        ) {
            Ok(token) => opts.bearer_token = Some(Zeroizing::new(token)),
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.timeout_ms {
        match parse_timeout_ms(val) {
            Ok(Some(dur)) => opts.timeout = dur,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.max_redirects {
        match parse_bounded_usize(
            val,
            0,
            MAX_ALLOWED_REDIRECTS,
            "maxRedirects must be an integer between 0 and 20",
        ) {
            Ok(Some(n)) => opts.max_redirects = n,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.allow_private_ips {
        match parse_bool_opt(val, "allowPrivateIps must be a boolean") {
            Ok(Some(b)) => opts.allow_private_ips = b,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.user_agent {
        match parse_user_agent_opt(ctx, val) {
            Ok(Some(ua)) => opts.user_agent = ua,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.max_bytes {
        match parse_bounded_u64(
            val,
            1,
            MAX_ALLOWED_DOWNLOAD_BYTES,
            "maxBytes must be an integer between 1 and 4294967296",
        ) {
            Ok(Some(n)) => opts.max_bytes = n,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.expected_sha256
        && !val.is_undefined()
    {
        match read_strict_string(
            ctx,
            val,
            || invalid_option("expectedSha256 must be a 64-character hex string"),
            || invalid_option("expectedSha256 contains an unpaired UTF-16 surrogate"),
        ) {
            Ok(sha) => opts.expected_sha256 = Some(sha),
            Err(e) => return Ok(Err(e)),
        }
    }

    if let Some(ref val) = fields.overwrite {
        match parse_bool_opt(val, "overwrite must be a boolean") {
            Ok(Some(b)) => opts.overwrite = b,
            Ok(None) => {}
            Err(e) => return Ok(Err(e)),
        }
    }

    let on_progress = if let Some(ref val) = fields.on_progress
        && !val.is_undefined()
    {
        let Some(func) = val.as_function().cloned() else {
            return Ok(Err(invalid_option("onProgress must be a function")));
        };
        Some(func)
    } else {
        None
    };

    Ok(Ok((opts, on_progress)))
}

fn host_fetch_sync<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 3 {
        return Err(Exception::throw_type(
            &ctx,
            "internal fetchSync arity error",
        ));
    }
    let json_stringify = args.0[2]
        .as_function()
        .cloned()
        .ok_or(rquickjs::Error::Unknown)?;
    let url = match read_strict_string(
        &ctx,
        &args.0[0],
        || FetchError::InvalidUrl {
            reason: "URL must be a string".to_owned(),
        },
        || FetchError::InvalidUrl {
            reason: "URL contains an unpaired UTF-16 surrogate".to_owned(),
        },
    ) {
        Ok(u) => u,
        Err(e) => return error_object(&ctx, &e),
    };

    let opts = match build_fetch_options(&ctx, args.0.get(1), &json_stringify)? {
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
    let url = match read_strict_string(
        &ctx,
        &args.0[0],
        || FetchError::InvalidUrl {
            reason: "URL must be a string".to_owned(),
        },
        || FetchError::InvalidUrl {
            reason: "URL contains an unpaired UTF-16 surrogate".to_owned(),
        },
    ) {
        Ok(u) => u,
        Err(e) => return error_object(&ctx, &e),
    };

    let dest_path = match read_strict_string(
        &ctx,
        &args.0[1],
        || invalid_option("destPath must be a string"),
        || invalid_option("destPath contains an unpaired UTF-16 surrogate"),
    ) {
        Ok(p) => p,
        Err(e) => return error_object(&ctx, &e),
    };

    let (opts, on_progress) = match build_download_options(&ctx, args.0.get(2))? {
        Ok(pair) => pair,
        Err(e) => return error_object(&ctx, &e),
    };

    let mut callback_error: Option<Value<'js>> = None;
    let result = if let Some(func) = on_progress {
        let mut cb = |progress: DownloadProgress| {
            if callback_error.is_some() {
                return;
            }
            let Ok(obj) = Object::new(ctx.clone()) else {
                return;
            };
            let _ = obj.prop(
                "bytesWritten",
                Property::from(progress.bytes_written as f64)
                    .writable()
                    .enumerable()
                    .configurable(),
            );
            let total = match progress.total_bytes {
                Some(n) => Value::new_number(ctx.clone(), n as f64),
                None => Value::new_null(ctx.clone()),
            };
            let _ = obj.prop(
                "totalBytes",
                Property::from(total).writable().enumerable().configurable(),
            );
            if let Err(err) = func.call::<_, Value<'js>>((obj,))
                && matches!(err, rquickjs::Error::Exception)
                && ctx.has_exception()
            {
                callback_error = Some(ctx.catch());
            }
        };
        download_sync(&url, Path::new(&dest_path), &opts, Some(&mut cb))
    } else {
        download_sync(&url, Path::new(&dest_path), &opts, None)
    };

    if let Some(exc) = callback_error {
        return Err(ctx.throw(exc));
    }

    let info = match result {
        Ok(info) => info,
        Err(e) => return error_object(&ctx, &e),
    };

    let out = Object::new(ctx.clone())?;
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

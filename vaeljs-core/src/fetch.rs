//! V8/`deno_core` host ops for `vael:fetch`.
//!
//! All SSRF guarding, URL/header/option validation, redirect handling,
//! credential redaction, bounded body reading, and atomic streaming downloads
//! live in the engine-neutral [`vaeljs_fetch_core`] crate. This module parses
//! V8 arguments without invoking getters or Proxy traps, dispatches blocking
//! I/O on `tokio::task::spawn_blocking` for async variants, and returns
//! null-prototype outcome objects to [`crate::builtins`].

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use deno_core::FromV8;
use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::ToV8;
use deno_core::op2;
use deno_core::v8;
use deno_error::JsErrorBox;
use tokio::sync::{Mutex as AsyncMutex, mpsc, oneshot};
use vaeljs_fetch_core::{
    DownloadOptions, DownloadProgress, DownloadResult, FetchBody, FetchError, FetchOptions,
    FetchResponse, Zeroizing, download_sync_cancellable, fetch_sync,
};

const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;
const MAX_JSON_DEPTH: usize = 128;
const MAX_JSON_NODES: usize = 100_000;
const MAX_PAIR_ENTRIES: usize = 4_096;
const MAX_OPEN_DOWNLOADS: usize = 64;
const DOWNLOAD_PROGRESS_CHANNEL_CAP: usize = 1_024;

#[derive(Debug)]
enum FetchBindingError {
    Type(&'static str),
    Fetch(FetchError),
}

impl From<FetchError> for FetchBindingError {
    fn from(err: FetchError) -> Self {
        Self::Fetch(err)
    }
}

/// Per-isolate Tokio driver handle captured at isolate creation for fallback
/// when `run_file_async` is polled outside an ambient Tokio runtime.
struct FetchState {
    driver: Option<tokio::runtime::Handle>,
    open_downloads: Arc<AtomicUsize>,
}

impl FetchState {
    fn capture() -> Self {
        Self {
            driver: tokio::runtime::Handle::try_current().ok(),
            open_downloads: Arc::new(AtomicUsize::new(0)),
        }
    }
}

fn spawn_blocking_work<F, R>(
    state: &Rc<RefCell<OpState>>,
    work: F,
) -> Result<tokio::task::JoinHandle<R>, FetchBindingError>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        return Ok(tokio::task::spawn_blocking(work));
    }
    let driver = state
        .borrow()
        .try_borrow::<FetchState>()
        .and_then(|s| s.driver.clone());
    if let Some(driver) = driver {
        return Ok(driver.spawn_blocking(work));
    }
    Err(FetchBindingError::Fetch(FetchError::Io {
        reason: "async fetch requires a Tokio runtime".to_owned(),
    }))
}

type ProgressChannelItem = (DownloadProgress, std::sync::mpsc::SyncSender<bool>);

struct DownloadStreamResource {
    progress_rx: AsyncMutex<mpsc::Receiver<ProgressChannelItem>>,
    pending_ack: AsyncMutex<Option<std::sync::mpsc::SyncSender<bool>>>,
    result_rx: AsyncMutex<Option<oneshot::Receiver<Result<DownloadResult, FetchError>>>>,
    cancel: Arc<AtomicBool>,
    open_downloads: Arc<AtomicUsize>,
}

impl Drop for DownloadStreamResource {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Ok(mut guard) = self.pending_ack.try_lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(false);
        }
        self.open_downloads.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Resource for DownloadStreamResource {
    fn name(&self) -> Cow<'_, str> {
        "vael::fetch::DownloadStream".into()
    }
}

// ---------------------------------------------------------------------------
// Null-prototype V8 result helpers
// ---------------------------------------------------------------------------

fn response_obj<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = v8::null(scope);
    v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[])
}

fn set<'s>(
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

fn set_string<'s>(
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

fn set_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: bool,
) {
    set(scope, object, name, v8::Boolean::new(scope, value).into());
}

fn set_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: f64,
) {
    set(scope, object, name, v8::Number::new(scope, value).into());
}

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    err: &FetchBindingError,
) {
    set_bool(scope, object, "ok", false);
    match err {
        FetchBindingError::Type(msg) => {
            set_string(scope, object, "name", "TypeError");
            set_string(scope, object, "message", msg);
        }
        FetchBindingError::Fetch(fetch_err) => {
            set_string(scope, object, "name", "FetchError");
            set_string(scope, object, "code", fetch_err.code().as_str());
            set_string(scope, object, "message", &fetch_err.to_string());
            if let FetchError::HttpStatus { status, .. } = fetch_err {
                set_number(scope, object, "status", f64::from(*status));
            }
            if let Some(url) = fetch_err.url() {
                set_string(scope, object, "url", url);
            }
        }
    }
}

fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Result<v8::Local<'s, v8::Value>, FetchBindingError> {
    let length = bytes.len();
    let array = if length == 0 {
        let buffer = v8::ArrayBuffer::new(scope, 0);
        v8::Uint8Array::new(scope, buffer, 0, 0)
    } else {
        let store = v8::ArrayBuffer::new_backing_store_from_bytes(bytes.into_boxed_slice());
        let store = store.make_shared();
        let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
        v8::Uint8Array::new(scope, buffer, 0, length)
    };
    array.map(Into::into).ok_or_else(|| {
        FetchBindingError::Fetch(FetchError::Io {
            reason: "failed to allocate Uint8Array".to_owned(),
        })
    })
}

fn headers_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    headers: &[(String, String)],
) -> v8::Local<'s, v8::Array> {
    let mut elements = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        let k = v8::String::new(scope, name)
            .map(Into::into)
            .unwrap_or_else(|| v8::String::empty(scope).into());
        let v = v8::String::new(scope, value)
            .map(Into::into)
            .unwrap_or_else(|| v8::String::empty(scope).into());
        let pair = v8::Array::new_with_elements(scope, &[k, v]);
        elements.push(pair.into());
    }
    v8::Array::new_with_elements(scope, &elements)
}

fn fetch_response_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resp: FetchResponse,
) -> Result<v8::Local<'s, v8::Object>, FetchBindingError> {
    let body = bytes_to_v8(scope, resp.body)?;
    let headers = headers_to_v8(scope, &resp.headers);
    let obj = response_obj(scope);
    set_number(scope, obj, "status", f64::from(resp.status));
    set_string(scope, obj, "statusText", &resp.status_text);
    set_bool(scope, obj, "ok", resp.ok);
    set_string(scope, obj, "url", &resp.url);
    set_bool(scope, obj, "redirected", resp.redirected);
    set(scope, obj, "headers", headers.into());
    set(scope, obj, "body", body);
    Ok(obj)
}

fn download_result_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    res: &DownloadResult,
) -> v8::Local<'s, v8::Object> {
    let obj = response_obj(scope);
    set_string(scope, obj, "path", &res.path.to_string_lossy());
    set_number(scope, obj, "status", f64::from(res.status));
    set_string(scope, obj, "url", &res.url);
    #[allow(clippy::cast_precision_loss)]
    set_number(scope, obj, "bytesWritten", res.bytes_written as f64);
    set_string(scope, obj, "sha256", &res.sha256);
    match &res.content_type {
        Some(ct) => set_string(scope, obj, "contentType", ct),
        None => set(scope, obj, "contentType", v8::null(scope).into()),
    }
    match &res.etag {
        Some(etag) => set_string(scope, obj, "etag", etag),
        None => set(scope, obj, "etag", v8::null(scope).into()),
    }
    obj
}

fn progress_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    progress: DownloadProgress,
) -> v8::Local<'s, v8::Object> {
    let obj = response_obj(scope);
    #[allow(clippy::cast_precision_loss)]
    set_number(scope, obj, "bytesWritten", progress.bytes_written as f64);
    match progress.total_bytes {
        #[allow(clippy::cast_precision_loss)]
        Some(total) => set_number(scope, obj, "totalBytes", total as f64),
        None => set(scope, obj, "totalBytes", v8::null(scope).into()),
    }
    obj
}

// ---------------------------------------------------------------------------
// Strict V8 argument & prototype-safe option parsing
// ---------------------------------------------------------------------------

fn copy_strict_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<String, FetchBindingError> {
    if !value.is_string() {
        return Err(FetchBindingError::Type(type_msg));
    }
    let string =
        v8::Local::<v8::String>::try_from(value).map_err(|_| FetchBindingError::Type(type_msg))?;
    let length = string.utf8_length(scope);
    let capacity =
        length
            .checked_add(4)
            .ok_or(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "string exceeds maximum buffer length",
            }))?;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(capacity).map_err(|_| {
        FetchBindingError::Fetch(FetchError::Io {
            reason: "failed to allocate string buffer".to_owned(),
        })
    })?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    buffer.truncate(written);
    String::from_utf8(buffer).map_err(|_| {
        FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: "string contains an unpaired UTF-16 surrogate",
        })
    })
}

fn copy_strict_secret_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<Zeroizing<String>, FetchBindingError> {
    if !value.is_string() {
        return Err(FetchBindingError::Type(type_msg));
    }
    let string =
        v8::Local::<v8::String>::try_from(value).map_err(|_| FetchBindingError::Type(type_msg))?;
    let length = string.utf8_length(scope);
    let capacity =
        length
            .checked_add(4)
            .ok_or(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "bearerToken exceeds maximum buffer length",
            }))?;
    let mut buffer = Zeroizing::new(Vec::new());
    buffer.try_reserve_exact(capacity).map_err(|_| {
        FetchBindingError::Fetch(FetchError::Io {
            reason: "failed to allocate bearerToken buffer".to_owned(),
        })
    })?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    buffer.truncate(written);
    let text = std::str::from_utf8(&buffer).map_err(|_| {
        FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: "bearerToken contains an unpaired UTF-16 surrogate",
        })
    })?;
    Ok(Zeroizing::new(text.to_owned()))
}

fn read_uint8_array(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<Vec<u8>, FetchBindingError> {
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(FetchBindingError::Type(type_msg));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(FetchBindingError::Type("Uint8Array buffer is detached"));
    };
    if buffer.was_detached() {
        return Err(FetchBindingError::Type("Uint8Array buffer is detached"));
    }
    let length = view.byte_length();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| {
        FetchBindingError::Fetch(FetchError::Io {
            reason: "failed to allocate byte buffer".to_owned(),
        })
    })?;
    bytes.resize(length, 0);
    if length <= i32::MAX as usize {
        if view.copy_contents(&mut bytes) != length {
            return Err(FetchBindingError::Type(
                "Uint8Array buffer is detached or out of bounds",
            ));
        }
    } else {
        let store = buffer.get_backing_store();
        let offset = view.byte_offset();
        let end = offset.checked_add(length).ok_or(FetchBindingError::Type(
            "Uint8Array buffer is out of bounds",
        ))?;
        let slice = store.get(offset..end).ok_or(FetchBindingError::Type(
            "Uint8Array buffer is out of bounds",
        ))?;
        for (dst, src) in bytes.iter_mut().zip(slice.iter()) {
            *dst = src.get();
        }
    }
    Ok(bytes)
}

fn plain_object_prototype<'s>(
    scope: &v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Value>, FetchBindingError> {
    let object = v8::Object::new(scope);
    object
        .get_prototype(scope)
        .ok_or(FetchBindingError::Type("options must be a plain object"))
}

fn own_descriptor_field<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, FetchBindingError> {
    let Some(key) = v8::String::new(scope, name) else {
        return Err(FetchBindingError::Type("options must be a plain object"));
    };
    let key: v8::Local<'s, v8::Name> = key.into();
    match object.has_own_property(scope, key) {
        Some(true) => object
            .get(scope, key.into())
            .map(Some)
            .ok_or(FetchBindingError::Type("options must be a plain object")),
        Some(false) => Ok(None),
        None => Err(FetchBindingError::Type("options must be a plain object")),
    }
}

fn own_descriptor_boolean(
    scope: &v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
) -> Result<bool, FetchBindingError> {
    let Some(value) = own_descriptor_field(scope, object, name)? else {
        return Err(FetchBindingError::Type("options must be a plain object"));
    };
    if !value.is_boolean() {
        return Err(FetchBindingError::Type("options must be a plain object"));
    }
    Ok(value.boolean_value(scope))
}

fn visit_plain_object_own<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    type_msg: &'static str,
    mut on_field: impl FnMut(
        &v8::PinScope<'s, '_>,
        &str,
        v8::Local<'s, v8::Value>,
    ) -> Result<(), FetchBindingError>,
) -> Result<(), FetchBindingError> {
    if !value.is_object() || value.is_proxy() || value.is_array() || value.is_function() {
        return Err(FetchBindingError::Type(type_msg));
    }
    let object =
        v8::Local::<v8::Object>::try_from(value).map_err(|_| FetchBindingError::Type(type_msg))?;
    let Some(prototype) = object.get_prototype(scope) else {
        return Err(FetchBindingError::Type(type_msg));
    };
    if !prototype.is_null() && prototype != plain_object_prototype(scope)? {
        return Err(FetchBindingError::Type(type_msg));
    }
    let args = v8::GetPropertyNamesArgs {
        mode: v8::KeyCollectionMode::OwnOnly,
        property_filter: v8::PropertyFilter::ALL_PROPERTIES,
        index_filter: v8::IndexFilter::IncludeIndices,
        key_conversion: v8::KeyConversionMode::ConvertToString,
    };
    let Some(names) = object.get_own_property_names(scope, args) else {
        return Err(FetchBindingError::Type(type_msg));
    };
    for index in 0..names.length() {
        let Some(name_value) = names.get_index(scope, index) else {
            return Err(FetchBindingError::Type(type_msg));
        };
        if name_value.is_symbol() {
            return Err(FetchBindingError::Type(
                "options must not contain symbol properties",
            ));
        }
        let Ok(name) = v8::Local::<v8::Name>::try_from(name_value) else {
            return Err(FetchBindingError::Type(type_msg));
        };
        let Some(descriptor_value) = object.get_own_property_descriptor(scope, name) else {
            return Err(FetchBindingError::Type(type_msg));
        };
        let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor_value) else {
            return Err(FetchBindingError::Type(type_msg));
        };
        if !own_descriptor_boolean(scope, descriptor, "enumerable")? {
            return Err(FetchBindingError::Type(
                "options must not contain non-enumerable properties",
            ));
        }
        if own_descriptor_field(scope, descriptor, "get")?.is_some()
            || own_descriptor_field(scope, descriptor, "set")?.is_some()
        {
            return Err(FetchBindingError::Type(
                "options must not contain accessor properties",
            ));
        }
        let Ok(key_str) = v8::Local::<v8::String>::try_from(name_value) else {
            return Err(FetchBindingError::Type(type_msg));
        };
        let key = copy_strict_string(scope, key_str.into(), type_msg)?;
        let Some(field_value) = own_descriptor_field(scope, descriptor, "value")? else {
            return Err(FetchBindingError::Type(type_msg));
        };
        on_field(scope, &key, field_value)?;
    }
    Ok(())
}

fn parse_string_pairs(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    label: &'static str,
) -> Result<Vec<(String, String)>, FetchBindingError> {
    if value.is_undefined() {
        return Ok(Vec::new());
    }
    if value.is_proxy() {
        return Err(FetchBindingError::Type(label));
    }
    if value.is_array() {
        let arr =
            v8::Local::<v8::Array>::try_from(value).map_err(|_| FetchBindingError::Type(label))?;
        let len = arr.length() as usize;
        if len > MAX_PAIR_ENTRIES {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "too many entries",
            }));
        }
        let arr_obj: v8::Local<'_, v8::Object> = arr.into();
        let mut out = Vec::with_capacity(len);
        for index in 0..arr.length() {
            let entry_val = read_own_array_element(scope, arr_obj, index, label)?;
            if !entry_val.is_array() || entry_val.is_proxy() {
                return Err(FetchBindingError::Type(label));
            }
            let entry = v8::Local::<v8::Array>::try_from(entry_val)
                .map_err(|_| FetchBindingError::Type(label))?;
            if entry.length() != 2 {
                return Err(FetchBindingError::Type(label));
            }
            let entry_obj: v8::Local<'_, v8::Object> = entry.into();
            let k_val = read_own_array_element(scope, entry_obj, 0, label)?;
            let v_val = read_own_array_element(scope, entry_obj, 1, label)?;
            let k = copy_strict_string(scope, k_val, label)?;
            let v = copy_strict_string(scope, v_val, label)?;
            out.push((k, v));
        }
        return Ok(out);
    }

    let mut out = Vec::new();
    visit_plain_object_own(scope, value, label, |scope, key, val| {
        if out.len() >= MAX_PAIR_ENTRIES {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "too many entries",
            }));
        }
        if val.is_undefined() {
            return Ok(());
        }
        let v = copy_strict_string(scope, val, label)?;
        out.push((key.to_owned(), v));
        Ok(())
    })?;
    Ok(out)
}

fn read_own_array_element<'s>(
    scope: &v8::PinScope<'s, '_>,
    array_obj: v8::Local<'s, v8::Object>,
    index: u32,
    type_msg: &'static str,
) -> Result<v8::Local<'s, v8::Value>, FetchBindingError> {
    let idx_num = v8::Integer::new_from_unsigned(scope, index);
    let Some(key_str) = idx_num.to_string(scope) else {
        return Err(FetchBindingError::Type(type_msg));
    };
    let key: v8::Local<'s, v8::Name> = key_str.into();
    let Some(desc_val) = array_obj.get_own_property_descriptor(scope, key) else {
        return Err(FetchBindingError::Type(type_msg));
    };
    let Ok(desc) = v8::Local::<v8::Object>::try_from(desc_val) else {
        return Err(FetchBindingError::Type(type_msg));
    };
    if own_descriptor_field(scope, desc, "get")?.is_some()
        || own_descriptor_field(scope, desc, "set")?.is_some()
    {
        return Err(FetchBindingError::Type(
            "options must not contain accessor properties",
        ));
    }
    own_descriptor_field(scope, desc, "value")?.ok_or(FetchBindingError::Type(type_msg))
}

fn parse_bool(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    msg: &'static str,
) -> Result<Option<bool>, FetchBindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    if !value.is_boolean() {
        return Err(FetchBindingError::Type(msg));
    }
    Ok(Some(value.boolean_value(scope)))
}

fn parse_timeout_ms(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Option<Duration>, FetchBindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    if !value.is_number() {
        return Err(FetchBindingError::Type(
            "options.timeoutMs must be a number",
        ));
    }
    let Some(num) = value.number_value(scope) else {
        return Err(FetchBindingError::Type(
            "options.timeoutMs must be a number",
        ));
    };
    if !num.is_finite() || num <= 0.0 || num > MAX_SAFE_INTEGER_F64 {
        return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: "timeoutMs must be a positive finite number",
        }));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let millis = num.round() as u64;
    if millis == 0 {
        return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: "timeoutMs must be greater than zero",
        }));
    }
    Ok(Some(Duration::from_millis(millis)))
}

fn parse_safe_u64(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
    invalid_reason: &'static str,
) -> Result<Option<u64>, FetchBindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    if !value.is_number() {
        return Err(FetchBindingError::Type(type_msg));
    }
    let Some(num) = value.number_value(scope) else {
        return Err(FetchBindingError::Type(type_msg));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < 0.0 || num > MAX_SAFE_INTEGER_F64 {
        return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: invalid_reason,
        }));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(Some(num as u64))
}

fn is_disallowed_json_object(value: v8::Local<'_, v8::Value>) -> bool {
    value.is_function()
        || value.is_proxy()
        || value.is_map()
        || value.is_set()
        || value.is_weak_map()
        || value.is_weak_set()
        || value.is_arguments_object()
        || value.is_generator_object()
        || value.is_module_namespace_object()
        || value.is_map_iterator()
        || value.is_set_iterator()
        || value.is_wasm_memory_object()
        || value.is_wasm_module_object()
        || value.is_external()
        || value.is_date()
        || value.is_reg_exp()
        || value.is_promise()
        || value.is_native_error()
        || value.is_array_buffer()
        || value.is_shared_array_buffer()
        || value.is_array_buffer_view()
        || value.is_symbol_object()
        || value.is_big_int_object()
        || value.is_number_object()
        || value.is_string_object()
        || value.is_boolean_object()
}

fn v8_to_json<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    depth: usize,
    nodes: &mut usize,
    ancestors: &mut Vec<v8::Local<'s, v8::Object>>,
) -> Result<serde_json::Value, FetchBindingError> {
    if depth > MAX_JSON_DEPTH {
        return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: "json body nesting depth exceeds limit",
        }));
    }
    if *nodes >= MAX_JSON_NODES {
        return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: "json body exceeds maximum node count",
        }));
    }
    *nodes += 1;

    if value.is_null() {
        return Ok(serde_json::Value::Null);
    }
    if value.is_boolean() {
        return Ok(serde_json::Value::Bool(value.boolean_value(scope)));
    }
    if value.is_number() {
        let Some(num) = value.number_value(scope) else {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "invalid number in json option",
            }));
        };
        if !num.is_finite() {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "json option must not contain NaN or Infinity",
            }));
        }
        if num.fract() == 0.0 && (-MAX_SAFE_INTEGER_F64..=MAX_SAFE_INTEGER_F64).contains(&num) {
            #[allow(clippy::cast_possible_truncation)]
            let int = num as i64;
            return Ok(serde_json::Value::Number(serde_json::Number::from(int)));
        }
        let number = serde_json::Number::from_f64(num).ok_or(FetchBindingError::Fetch(
            FetchError::InvalidOption {
                reason: "invalid floating-point number in json option",
            },
        ))?;
        return Ok(serde_json::Value::Number(number));
    }
    if value.is_string() {
        let text = copy_strict_string(scope, value, "json string expected")?;
        return Ok(serde_json::Value::String(text));
    }
    if value.is_undefined()
        || value.is_symbol()
        || value.is_big_int()
        || is_disallowed_json_object(value)
    {
        return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: "json option contains an unsupported value",
        }));
    }
    if value.is_array() {
        let arr = v8::Local::<v8::Array>::try_from(value).map_err(|_| {
            FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "invalid array in json option",
            })
        })?;
        let obj: v8::Local<'s, v8::Object> = arr.into();
        if ancestors.contains(&obj) {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "json option contains a circular reference",
            }));
        }
        let len = arr.length() as usize;
        if len > MAX_JSON_NODES - *nodes {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "json array exceeds maximum node count",
            }));
        }
        ancestors.push(obj);
        let mut out = Vec::with_capacity(len);
        for index in 0..arr.length() {
            let idx_num = v8::Integer::new_from_unsigned(scope, index);
            let Some(key_str) = idx_num.to_string(scope) else {
                ancestors.pop();
                return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                    reason: "failed to index json array",
                }));
            };
            let key: v8::Local<'s, v8::Name> = key_str.into();
            match obj.get_own_property_descriptor(scope, key) {
                Some(desc_val) if desc_val.is_object() => {
                    let Ok(desc) = v8::Local::<v8::Object>::try_from(desc_val) else {
                        ancestors.pop();
                        return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                            reason: "invalid array element descriptor in json option",
                        }));
                    };
                    if own_descriptor_field(scope, desc, "get")?.is_some()
                        || own_descriptor_field(scope, desc, "set")?.is_some()
                    {
                        ancestors.pop();
                        return Err(FetchBindingError::Type(
                            "options must not contain accessor properties",
                        ));
                    }
                    let elem = own_descriptor_field(scope, desc, "value")?
                        .unwrap_or_else(|| v8::undefined(scope).into());
                    if elem.is_undefined() {
                        out.push(serde_json::Value::Null);
                    } else {
                        match v8_to_json(scope, elem, depth + 1, nodes, ancestors) {
                            Ok(val) => out.push(val),
                            Err(err) => {
                                ancestors.pop();
                                return Err(err);
                            }
                        }
                    }
                }
                _ => out.push(serde_json::Value::Null),
            }
        }
        ancestors.pop();
        return Ok(serde_json::Value::Array(out));
    }
    if value.is_object() {
        let obj = v8::Local::<v8::Object>::try_from(value).map_err(|_| {
            FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "invalid object in json option",
            })
        })?;
        let Some(prototype) = obj.get_prototype(scope) else {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "invalid object prototype in json option",
            }));
        };
        if !prototype.is_null() && prototype != plain_object_prototype(scope)? {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "json option objects must be plain objects",
            }));
        }
        if ancestors.contains(&obj) {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "json option contains a circular reference",
            }));
        }
        let args = v8::GetPropertyNamesArgs {
            mode: v8::KeyCollectionMode::OwnOnly,
            property_filter: v8::PropertyFilter::ALL_PROPERTIES,
            index_filter: v8::IndexFilter::IncludeIndices,
            key_conversion: v8::KeyConversionMode::ConvertToString,
        };
        let Some(names) = obj.get_own_property_names(scope, args) else {
            return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                reason: "failed to enumerate json object properties",
            }));
        };
        ancestors.push(obj);
        let mut map = serde_json::Map::new();
        for index in 0..names.length() {
            let Some(name_value) = names.get_index(scope, index) else {
                ancestors.pop();
                return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                    reason: "failed to read json property name",
                }));
            };
            if name_value.is_symbol() {
                continue;
            }
            let Ok(name) = v8::Local::<v8::Name>::try_from(name_value) else {
                ancestors.pop();
                return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                    reason: "invalid property name in json option",
                }));
            };
            let Some(desc_val) = obj.get_own_property_descriptor(scope, name) else {
                ancestors.pop();
                return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                    reason: "failed to read property descriptor in json option",
                }));
            };
            let Ok(desc) = v8::Local::<v8::Object>::try_from(desc_val) else {
                ancestors.pop();
                return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
                    reason: "invalid property descriptor in json option",
                }));
            };
            if !own_descriptor_boolean(scope, desc, "enumerable")? {
                continue;
            }
            if own_descriptor_field(scope, desc, "get")?.is_some()
                || own_descriptor_field(scope, desc, "set")?.is_some()
            {
                ancestors.pop();
                return Err(FetchBindingError::Type(
                    "options must not contain accessor properties",
                ));
            }
            let Some(field_val) = own_descriptor_field(scope, desc, "value")? else {
                continue;
            };
            if field_val.is_undefined() {
                continue;
            }
            let key =
                match copy_strict_string(scope, name_value, "json property key must be a string") {
                    Ok(k) => k,
                    Err(err) => {
                        ancestors.pop();
                        return Err(err);
                    }
                };
            match v8_to_json(scope, field_val, depth + 1, nodes, ancestors) {
                Ok(v) => {
                    map.insert(key, v);
                }
                Err(err) => {
                    ancestors.pop();
                    return Err(err);
                }
            }
        }
        ancestors.pop();
        return Ok(serde_json::Value::Object(map));
    }

    Err(FetchBindingError::Fetch(FetchError::InvalidOption {
        reason: "json option contains an unsupported value",
    }))
}

fn parse_fetch_options(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<FetchOptions, FetchBindingError> {
    let mut opts = FetchOptions::default();
    if value.is_undefined() {
        return Ok(opts);
    }
    let mut saw_body = false;
    let mut saw_json = false;

    visit_plain_object_own(
        scope,
        value,
        "fetch options must be a plain object",
        |scope, key, val| match key {
            "method" => {
                if !val.is_undefined() {
                    opts.method =
                        copy_strict_string(scope, val, "options.method must be a string")?;
                }
                Ok(())
            }
            "headers" => {
                opts.headers = parse_string_pairs(
                    scope,
                    val,
                    "options.headers must be a plain object or [name, value][]",
                )?;
                Ok(())
            }
            "query" => {
                opts.query = parse_string_pairs(
                    scope,
                    val,
                    "options.query must be a plain object or [key, value][]",
                )?;
                Ok(())
            }
            "bearerToken" => {
                if !val.is_undefined() {
                    opts.bearer_token = Some(copy_strict_secret_string(
                        scope,
                        val,
                        "options.bearerToken must be a string",
                    )?);
                }
                Ok(())
            }
            "body" => {
                if val.is_undefined() {
                    return Ok(());
                }
                saw_body = true;
                if val.is_string() {
                    let text = copy_strict_string(
                        scope,
                        val,
                        "options.body must be a string or Uint8Array",
                    )?;
                    opts.body = FetchBody::Text(text);
                } else if val.is_uint8_array() {
                    let bytes = read_uint8_array(
                        scope,
                        val,
                        "options.body must be a string or Uint8Array",
                    )?;
                    opts.body = FetchBody::Bytes(bytes);
                } else {
                    return Err(FetchBindingError::Type(
                        "options.body must be a string or Uint8Array",
                    ));
                }
                Ok(())
            }
            "json" => {
                if val.is_undefined() {
                    return Ok(());
                }
                saw_json = true;
                let mut nodes = 0usize;
                let mut ancestors = Vec::new();
                let json_val = v8_to_json(scope, val, 0, &mut nodes, &mut ancestors)?;
                opts.body = FetchBody::Json(json_val);
                Ok(())
            }
            "timeoutMs" => {
                if let Some(dur) = parse_timeout_ms(scope, val)? {
                    opts.timeout = dur;
                }
                Ok(())
            }
            "maxResponseBytes" => {
                if let Some(n) = parse_safe_u64(
                    scope,
                    val,
                    "options.maxResponseBytes must be a number",
                    "maxResponseBytes must be a non-negative integer",
                )? {
                    opts.max_response_bytes = usize::try_from(n).unwrap_or(usize::MAX);
                }
                Ok(())
            }
            "maxRedirects" => {
                if let Some(n) = parse_safe_u64(
                    scope,
                    val,
                    "options.maxRedirects must be a number",
                    "maxRedirects must be a non-negative integer",
                )? {
                    opts.max_redirects = usize::try_from(n).unwrap_or(usize::MAX);
                }
                Ok(())
            }
            "allowPrivateIps" => {
                if let Some(b) =
                    parse_bool(scope, val, "options.allowPrivateIps must be a boolean")?
                {
                    opts.allow_private_ips = b;
                }
                Ok(())
            }
            "userAgent" => {
                if val.is_undefined() {
                    return Ok(());
                }
                if val.is_null() {
                    opts.user_agent = None;
                } else {
                    opts.user_agent = Some(copy_strict_string(
                        scope,
                        val,
                        "options.userAgent must be a string or null",
                    )?);
                }
                Ok(())
            }
            _ => Err(FetchBindingError::Type("unknown fetch option")),
        },
    )?;

    if saw_body && saw_json {
        return Err(FetchBindingError::Fetch(FetchError::InvalidOption {
            reason: "body and json options are mutually exclusive",
        }));
    }

    Ok(opts)
}

fn parse_download_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(DownloadOptions, Option<v8::Local<'s, v8::Function>>), FetchBindingError> {
    let mut opts = DownloadOptions::default();
    let mut progress_fn: Option<v8::Local<'s, v8::Function>> = None;
    if value.is_undefined() {
        return Ok((opts, progress_fn));
    }

    visit_plain_object_own(
        scope,
        value,
        "download options must be a plain object",
        |scope, key, val| match key {
            "headers" => {
                opts.headers = parse_string_pairs(
                    scope,
                    val,
                    "options.headers must be a plain object or [name, value][]",
                )?;
                Ok(())
            }
            "query" => {
                opts.query = parse_string_pairs(
                    scope,
                    val,
                    "options.query must be a plain object or [key, value][]",
                )?;
                Ok(())
            }
            "bearerToken" => {
                if !val.is_undefined() {
                    opts.bearer_token = Some(copy_strict_secret_string(
                        scope,
                        val,
                        "options.bearerToken must be a string",
                    )?);
                }
                Ok(())
            }
            "timeoutMs" => {
                if let Some(dur) = parse_timeout_ms(scope, val)? {
                    opts.timeout = dur;
                }
                Ok(())
            }
            "maxRedirects" => {
                if let Some(n) = parse_safe_u64(
                    scope,
                    val,
                    "options.maxRedirects must be a number",
                    "maxRedirects must be a non-negative integer",
                )? {
                    opts.max_redirects = usize::try_from(n).unwrap_or(usize::MAX);
                }
                Ok(())
            }
            "allowPrivateIps" => {
                if let Some(b) =
                    parse_bool(scope, val, "options.allowPrivateIps must be a boolean")?
                {
                    opts.allow_private_ips = b;
                }
                Ok(())
            }
            "userAgent" => {
                if val.is_undefined() {
                    return Ok(());
                }
                if val.is_null() {
                    opts.user_agent = None;
                } else {
                    opts.user_agent = Some(copy_strict_string(
                        scope,
                        val,
                        "options.userAgent must be a string or null",
                    )?);
                }
                Ok(())
            }
            "maxBytes" => {
                if let Some(n) = parse_safe_u64(
                    scope,
                    val,
                    "options.maxBytes must be a number",
                    "maxBytes must be a non-negative integer",
                )? {
                    opts.max_bytes = n;
                }
                Ok(())
            }
            "expectedSha256" => {
                if !val.is_undefined() {
                    opts.expected_sha256 = Some(copy_strict_string(
                        scope,
                        val,
                        "options.expectedSha256 must be a string",
                    )?);
                }
                Ok(())
            }
            "overwrite" => {
                if let Some(b) = parse_bool(scope, val, "options.overwrite must be a boolean")? {
                    opts.overwrite = b;
                }
                Ok(())
            }
            "onProgress" => {
                if !val.is_undefined() {
                    let Ok(func) = v8::Local::<v8::Function>::try_from(val) else {
                        return Err(FetchBindingError::Type(
                            "options.onProgress must be a function",
                        ));
                    };
                    progress_fn = Some(func);
                }
                Ok(())
            }
            _ => Err(FetchBindingError::Type("unknown download option")),
        },
    )?;

    Ok((opts, progress_fn))
}

// ---------------------------------------------------------------------------
// FromV8 / ToV8 types for async ops
// ---------------------------------------------------------------------------

struct FetchStrArg(Result<String, FetchBindingError>);

impl<'a> FromV8<'a> for FetchStrArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        Ok(Self(copy_strict_string(scope, value, "expected a string")))
    }
}

struct FetchOptsArg(Result<FetchOptions, FetchBindingError>);

impl<'a> FromV8<'a> for FetchOptsArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        Ok(Self(parse_fetch_options(scope, value)))
    }
}

enum FetchOutcome {
    Response(FetchResponse),
    Err(FetchBindingError),
}

impl<'a> ToV8<'a> for FetchOutcome {
    type Error = JsErrorBox;

    fn to_v8<'i>(
        self,
        scope: &mut v8::PinScope<'a, 'i>,
    ) -> Result<v8::Local<'a, v8::Value>, Self::Error> {
        let obj = response_obj(scope);
        match self {
            Self::Response(resp) => match fetch_response_to_v8(scope, resp) {
                Ok(val) => {
                    set_bool(scope, obj, "ok", true);
                    set(scope, obj, "value", val.into());
                }
                Err(err) => set_failure(scope, obj, &err),
            },
            Self::Err(err) => set_failure(scope, obj, &err),
        }
        Ok(obj.into())
    }
}

enum DownloadNextOutcome {
    Progress(DownloadProgress),
    Done(DownloadResult),
    Err(FetchBindingError),
}

impl<'a> ToV8<'a> for DownloadNextOutcome {
    type Error = JsErrorBox;

    fn to_v8<'i>(
        self,
        scope: &mut v8::PinScope<'a, 'i>,
    ) -> Result<v8::Local<'a, v8::Value>, Self::Error> {
        let obj = response_obj(scope);
        match self {
            Self::Progress(p) => {
                set_bool(scope, obj, "ok", true);
                set_string(scope, obj, "kind", "progress");
                let val = progress_to_v8(scope, p);
                set(scope, obj, "value", val.into());
            }
            Self::Done(res) => {
                set_bool(scope, obj, "ok", true);
                set_string(scope, obj, "kind", "done");
                let val = download_result_to_v8(scope, &res);
                set(scope, obj, "value", val.into());
            }
            Self::Err(err) => set_failure(scope, obj, &err),
        }
        Ok(obj.into())
    }
}

// ---------------------------------------------------------------------------
// Host ops
// ---------------------------------------------------------------------------

#[op2]
fn op_vael_fetch_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    url: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let url = match copy_strict_string(scope, url, "url must be a string") {
        Ok(u) => u,
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let opts = match parse_fetch_options(scope, options) {
        Ok(o) => o,
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let obj = response_obj(scope);
    match fetch_sync(&url, &opts) {
        Ok(resp) => match fetch_response_to_v8(scope, resp) {
            Ok(val) => {
                set_bool(scope, obj, "ok", true);
                set(scope, obj, "value", val.into());
            }
            Err(err) => set_failure(scope, obj, &err),
        },
        Err(err) => set_failure(scope, obj, &FetchBindingError::Fetch(err)),
    }
    obj.into()
}

#[op2]
async fn op_vael_fetch_async(
    state: Rc<RefCell<OpState>>,
    #[scoped] url: FetchStrArg,
    #[scoped] options: FetchOptsArg,
) -> FetchOutcome {
    let url = match url.0 {
        Ok(u) => u,
        Err(FetchBindingError::Type(_)) => {
            return FetchOutcome::Err(FetchBindingError::Type("url must be a string"));
        }
        Err(err) => return FetchOutcome::Err(err),
    };
    let opts = match options.0 {
        Ok(o) => o,
        Err(err) => return FetchOutcome::Err(err),
    };
    let join = match spawn_blocking_work(&state, move || fetch_sync(&url, &opts)) {
        Ok(handle) => handle,
        Err(err) => return FetchOutcome::Err(err),
    };
    match join.await {
        Ok(Ok(resp)) => FetchOutcome::Response(resp),
        Ok(Err(err)) => FetchOutcome::Err(FetchBindingError::Fetch(err)),
        Err(_) => FetchOutcome::Err(FetchBindingError::Fetch(FetchError::Io {
            reason: "fetch worker task panicked or was cancelled".to_owned(),
        })),
    }
}

#[op2]
fn op_vael_fetch_download_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    url: v8::Local<'s, v8::Value>,
    dest_path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let url = match copy_strict_string(scope, url, "url must be a string") {
        Ok(u) => u,
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let dest = match copy_strict_string(scope, dest_path, "destPath must be a string") {
        Ok(d) => PathBuf::from(d),
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let (opts, progress_fn) = match parse_download_options(scope, options) {
        Ok(pair) => pair,
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };

    let mut caught_exc: Option<v8::Global<v8::Value>> = None;
    let result = if let Some(cb_fn) = progress_fn {
        let mut cb = |p: DownloadProgress| -> Result<(), FetchError> {
            v8::tc_scope!(let tc, scope);
            let p_obj = progress_to_v8(tc, p);
            let _ = p_obj.set_integrity_level(tc, v8::IntegrityLevel::Frozen);
            let undef: v8::Local<'_, v8::Value> = v8::undefined(tc).into();
            let _ = cb_fn.call(tc, undef, &[p_obj.into()]);
            if tc.has_caught() {
                if let Some(exc) = tc.exception() {
                    caught_exc = Some(v8::Global::new(tc, exc));
                }
                return Err(FetchError::Io {
                    reason: "download aborted by onProgress callback".to_owned(),
                });
            }
            Ok(())
        };
        download_sync_cancellable(&url, &dest, &opts, None, Some(&mut cb))
    } else {
        download_sync_cancellable(&url, &dest, &opts, None, None)
    };

    if let Some(exc_global) = caught_exc {
        let exc = v8::Local::new(scope, exc_global);
        scope.throw_exception(exc);
        return v8::undefined(scope).into();
    }

    let obj = response_obj(scope);
    match result {
        Ok(res) => {
            set_bool(scope, obj, "ok", true);
            let res_v8 = download_result_to_v8(scope, &res);
            set(scope, obj, "value", res_v8.into());
        }
        Err(err) => set_failure(scope, obj, &FetchBindingError::Fetch(err)),
    }
    obj.into()
}

#[op2]
fn op_vael_fetch_download_start<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    url: v8::Local<'s, v8::Value>,
    dest_path: v8::Local<'s, v8::Value>,
    options: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let url = match copy_strict_string(scope, url, "url must be a string") {
        Ok(u) => u,
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let dest = match copy_strict_string(scope, dest_path, "destPath must be a string") {
        Ok(d) => PathBuf::from(d),
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let (opts, progress_fn) = match parse_download_options(scope, options) {
        Ok(pair) => pair,
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let has_progress = progress_fn.is_some();

    let open_downloads = state
        .try_borrow::<FetchState>()
        .map(|s| Arc::clone(&s.open_downloads))
        .unwrap_or_default();
    let prev_open = open_downloads.fetch_add(1, Ordering::Relaxed);
    if prev_open >= MAX_OPEN_DOWNLOADS {
        open_downloads.fetch_sub(1, Ordering::Relaxed);
        let obj = response_obj(scope);
        set_failure(
            scope,
            obj,
            &FetchBindingError::Fetch(FetchError::Io {
                reason: "maximum number of open concurrent downloads exceeded".to_owned(),
            }),
        );
        return obj.into();
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_worker = Arc::clone(&cancel);
    let (progress_tx, progress_rx) =
        mpsc::channel::<ProgressChannelItem>(DOWNLOAD_PROGRESS_CHANNEL_CAP);
    let (result_tx, result_rx) = oneshot::channel::<Result<DownloadResult, FetchError>>();

    let worker = move || {
        let res = if has_progress {
            let mut cb = |p: DownloadProgress| -> Result<(), FetchError> {
                let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel::<bool>(1);
                if progress_tx.blocking_send((p, ack_tx)).is_err() {
                    return Err(FetchError::Io {
                        reason: "download cancelled".to_owned(),
                    });
                }
                match ack_rx.recv() {
                    Ok(true) => Ok(()),
                    _ => Err(FetchError::Io {
                        reason: "download aborted by onProgress callback".to_owned(),
                    }),
                }
            };
            download_sync_cancellable(&url, &dest, &opts, Some(&cancel_worker), Some(&mut cb))
        } else {
            download_sync_cancellable(&url, &dest, &opts, Some(&cancel_worker), None)
        };
        drop(progress_tx);
        let _ = result_tx.send(res);
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::spawn_blocking(worker);
    } else if let Some(driver) = state
        .try_borrow::<FetchState>()
        .and_then(|s| s.driver.clone())
    {
        driver.spawn_blocking(worker);
    } else {
        open_downloads.fetch_sub(1, Ordering::Relaxed);
        let obj = response_obj(scope);
        set_failure(
            scope,
            obj,
            &FetchBindingError::Fetch(FetchError::Io {
                reason: "async download requires a Tokio runtime".to_owned(),
            }),
        );
        return obj.into();
    }

    let rid = state.resource_table.add(DownloadStreamResource {
        progress_rx: AsyncMutex::new(progress_rx),
        pending_ack: AsyncMutex::new(None),
        result_rx: AsyncMutex::new(Some(result_rx)),
        cancel,
        open_downloads,
    });

    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set_number(scope, obj, "id", f64::from(rid));
    obj.into()
}

#[op2]
async fn op_vael_fetch_download_next(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
) -> DownloadNextOutcome {
    let resource = {
        let borrowed = state.borrow();
        let Ok(res) = borrowed.resource_table.get::<DownloadStreamResource>(rid) else {
            return DownloadNextOutcome::Err(FetchBindingError::Fetch(FetchError::Io {
                reason: "download handle is closed".to_owned(),
            }));
        };
        res
    };

    if let Some(prev_ack) = resource.pending_ack.lock().await.take() {
        let _ = prev_ack.send(true);
    }

    let mut progress_rx = resource.progress_rx.lock().await;
    if let Ok((progress, ack_tx)) = progress_rx.try_recv() {
        *resource.pending_ack.lock().await = Some(ack_tx);
        return DownloadNextOutcome::Progress(progress);
    }
    if let Some((progress, ack_tx)) = progress_rx.recv().await {
        *resource.pending_ack.lock().await = Some(ack_tx);
        return DownloadNextOutcome::Progress(progress);
    }
    drop(progress_rx);

    let mut result_guard = resource.result_rx.lock().await;
    let Some(result_rx) = result_guard.take() else {
        return DownloadNextOutcome::Err(FetchBindingError::Fetch(FetchError::Io {
            reason: "download already completed".to_owned(),
        }));
    };
    match result_rx.await {
        Ok(Ok(res)) => DownloadNextOutcome::Done(res),
        Ok(Err(err)) => DownloadNextOutcome::Err(FetchBindingError::Fetch(err)),
        Err(_) => DownloadNextOutcome::Err(FetchBindingError::Fetch(FetchError::Io {
            reason: "download worker task terminated unexpectedly".to_owned(),
        })),
    }
}

#[op2(fast)]
fn op_vael_fetch_download_close(state: &mut OpState, #[smi] rid: ResourceId) {
    if let Ok(res) = state.resource_table.take::<DownloadStreamResource>(rid) {
        res.cancel.store(true, Ordering::Release);
        if let Ok(mut guard) = res.pending_ack.try_lock()
            && let Some(tx) = guard.take()
        {
            let _ = tx.send(false);
        }
    }
}

#[op2]
fn op_vael_fetch_decode_utf8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let bytes = match read_uint8_array(scope, value, "body must be a Uint8Array") {
        Ok(b) => b,
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let obj = response_obj(scope);
    match std::str::from_utf8(&bytes) {
        Ok(text) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", text);
        }
        Err(_) => {
            set_failure(
                scope,
                obj,
                &FetchBindingError::Fetch(FetchError::InvalidUtf8),
            );
        }
    }
    obj.into()
}

deno_core::extension!(
    vaeljs_fetch,
    ops = [
        op_vael_fetch_sync,
        op_vael_fetch_async,
        op_vael_fetch_download_sync,
        op_vael_fetch_download_start,
        op_vael_fetch_download_next,
        op_vael_fetch_download_close,
        op_vael_fetch_decode_utf8,
    ],
    state = |state| {
        state.put(FetchState::capture());
    },
    docs = "SSRF-guarded HTTP fetch and atomic streaming download ops behind `Vael.getBuiltInModule(\"vael:fetch\")`."
);

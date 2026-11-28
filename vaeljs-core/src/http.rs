//! Host ops and pure-Rust HTTP/1.1 transport for `vael:http`.
//!
//! All deterministic HTTP routing, static-file resolution, cookie
//! parsing/formatting, and RFC 9110 header validation live in
//! [`vaeljs_http_core`]. This module wires `vaeljs-http-core`, `hyper 1`, and
//! `tokio::net::TcpListener` into `vaeljs-core` behind the `vael-http` feature.

#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::net::SocketAddr;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::op2;
use deno_core::v8;
use http::header::{HeaderName, HeaderValue};
use http::{Response as HttpLibResponse, StatusCode};
use http_body_util::{BodyExt as _, Full, LengthLimitError, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;
use tokio::sync::{Mutex as AsyncMutex, Notify, Semaphore, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use vaeljs_http_core::{
    DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_HEADER_BYTES, HttpCoreError, RouteMatch, Router, SameSite,
    SetCookie, format_set_cookie, parse_cookies, resolve_static_relative_path,
    validate_header_name, validate_header_value,
};

/// Maximum concurrent `ServeMux` router instances per isolate.
const MAX_ROUTERS: usize = 1024;

/// Maximum concurrent live HTTP servers per isolate.
const MAX_SERVERS: usize = 64;

/// Maximum concurrent TCP connections accepted per server.
const MAX_CONCURRENT_CONNECTIONS: usize = 1024;

/// Maximum concurrent live `IncomingRequestResource` slots across an isolate.
const MAX_PENDING_REQUESTS: usize = 1024;

/// Bounded queue depth between Hyper connection tasks and the JS accept loop.
const REQUEST_CHANNEL_CAPACITY: usize = 256;

/// Maximum number of response headers accepted for one HTTP response.
const MAX_RESPONSE_HEADERS: usize = 256;

/// Maximum static file size read into memory for one response (64 MiB).
const MAX_STATIC_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Maximum number of query/form parameters decoded in one call.
const MAX_QUERY_PARAMS: usize = 1024;

/// Hard upper bound for configurable `maxBodyBytes` (256 MiB).
const MAX_ALLOWED_BODY_BYTES: usize = 256 * 1024 * 1024;

/// Timeout for receiving HTTP/1.1 request headers on a connection.
const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Timeout for reading an incoming HTTP/1.1 request body.
const DEFAULT_BODY_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Graceful shutdown drain window for active HTTP/1.1 connections.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Default body size limit exposed to `vael:http` (`DEFAULT_MAX_BODY_BYTES`).
const DEFAULT_BODY_LIMIT: usize = DEFAULT_MAX_BODY_BYTES;

#[derive(Default)]
struct ResourceCounts {
    routers: AtomicUsize,
    servers: AtomicUsize,
    pending_requests: AtomicUsize,
}

/// Per-isolate HTTP host state stored in `OpState`.
pub(crate) struct HttpState {
    driver: Option<tokio::runtime::Handle>,
    counts: Arc<ResourceCounts>,
    header_read_timeout: Duration,
    body_read_timeout: Duration,
}

impl HttpState {
    fn capture() -> Self {
        Self {
            driver: tokio::runtime::Handle::try_current().ok(),
            counts: Arc::new(ResourceCounts::default()),
            header_read_timeout: HEADER_READ_TIMEOUT,
            body_read_timeout: DEFAULT_BODY_READ_TIMEOUT,
        }
    }
}

struct RouterResource {
    router: RefCell<Router>,
    counts: Arc<ResourceCounts>,
}

impl Drop for RouterResource {
    fn drop(&mut self) {
        self.counts.routers.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Resource for RouterResource {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "vael::http::Router".into()
    }
}

struct IncomingRequest {
    method: String,
    is_head: bool,
    url: String,
    path: String,
    remote_addr: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    resp_tx: oneshot::Sender<HttpResponseData>,
}

struct HttpResponseData {
    status: u16,
    is_head: bool,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct IncomingRequestResource {
    is_head: bool,
    data: RefCell<Option<IncomingRequestData>>,
    resp_tx: RefCell<Option<oneshot::Sender<HttpResponseData>>>,
    counts: Arc<ResourceCounts>,
}

impl Drop for IncomingRequestResource {
    fn drop(&mut self) {
        self.counts.pending_requests.fetch_sub(1, Ordering::Relaxed);
    }
}

struct IncomingRequestData {
    method: String,
    url: String,
    path: String,
    remote_addr: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resource for IncomingRequestResource {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "vael::http::Request".into()
    }
}

struct ActiveConnGuard {
    active_conns: Arc<AtomicUsize>,
    conns_drained: Arc<Notify>,
}

impl ActiveConnGuard {
    fn new(active_conns: Arc<AtomicUsize>, conns_drained: Arc<Notify>) -> Self {
        active_conns.fetch_add(1, Ordering::AcqRel);
        Self {
            active_conns,
            conns_drained,
        }
    }
}

impl Drop for ActiveConnGuard {
    fn drop(&mut self) {
        if self.active_conns.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.conns_drained.notify_waiters();
        }
    }
}

struct ServerInner {
    rx: AsyncMutex<mpsc::Receiver<IncomingRequest>>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    accept_task: StdMutex<Option<JoinHandle<()>>>,
    active_conns: Arc<AtomicUsize>,
    conns_drained: Arc<Notify>,
    counts: Arc<ResourceCounts>,
}

struct HttpServerResource {
    inner: Arc<ServerInner>,
}

impl Drop for HttpServerResource {
    fn drop(&mut self) {
        let _ = self.inner.shutdown_tx.send(true);
        if let Ok(mut guard) = self.inner.accept_task.lock()
            && let Some(handle) = guard.take()
        {
            handle.abort();
        }
        self.inner.counts.servers.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Resource for HttpServerResource {
    fn name(&self) -> std::borrow::Cow<'_, str> {
        "vael::http::Server".into()
    }
}

// ---------------------------------------------------------------------------
// V8 response helpers
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

fn set_number<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: f64,
) {
    set(scope, object, name, v8::Number::new(scope, value).into());
}

fn set_bool<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: bool,
) {
    set(scope, object, name, v8::Boolean::new(scope, value).into());
}

fn failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    code: Option<&str>,
    message: &str,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", false);
    set_string(scope, obj, "name", name);
    if let Some(code) = code {
        set_string(scope, obj, "code", code);
    }
    set_string(scope, obj, "message", message);
    obj.into()
}

fn core_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    err: &HttpCoreError,
) -> v8::Local<'s, v8::Value> {
    failure(scope, "Error", Some(err.code().as_str()), &err.to_string())
}

fn ok_unit<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    obj.into()
}

fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Value>> {
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
    array.map(Into::into)
}

fn pairs_to_v8_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pairs: &[(String, String)],
) -> v8::Local<'s, v8::Array> {
    let arr = v8::Array::new(scope, pairs.len() as i32);
    for (idx, (k, v)) in pairs.iter().enumerate() {
        let pair = v8::Array::new(scope, 2);
        if let Some(ks) = v8::String::new(scope, k) {
            pair.set_index(scope, 0, ks.into());
        }
        if let Some(vs) = v8::String::new(scope, v) {
            pair.set_index(scope, 1, vs.into());
        }
        arr.set_index(scope, idx as u32, pair.into());
    }
    arr
}

fn copy_strict_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<String, (&'static str, &'static str)> {
    let Ok(string) = v8::Local::<v8::String>::try_from(value) else {
        return Err(("TypeError", type_msg));
    };
    let length = string.utf8_length(scope);
    let capacity = length
        .checked_add(4)
        .ok_or(("RangeError", "string length overflow"))?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| ("RangeError", "failed to allocate string buffer"))?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    buffer.truncate(written);
    String::from_utf8(buffer)
        .map_err(|_| ("TypeError", "string contains an unpaired UTF-16 surrogate"))
}

fn read_uint8_array(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<Vec<u8>, (&'static str, &'static str)> {
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(("TypeError", type_msg));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(("TypeError", "Uint8Array buffer is detached"));
    };
    if buffer.was_detached() {
        return Err(("TypeError", "Uint8Array buffer is detached"));
    }
    let length = view.byte_length();
    let offset = view.byte_offset();
    let Some(end) = offset.checked_add(length) else {
        return Err((
            "TypeError",
            "Uint8Array buffer is detached or out of bounds",
        ));
    };
    if end > buffer.byte_length() {
        return Err((
            "TypeError",
            "Uint8Array buffer is detached or out of bounds",
        ));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| ("RangeError", "failed to allocate byte buffer"))?;
    bytes.resize(length, 0);
    if length <= i32::MAX as usize {
        if view.copy_contents(&mut bytes) != length {
            return Err((
                "TypeError",
                "Uint8Array buffer is detached or out of bounds",
            ));
        }
    } else {
        let store = buffer.get_backing_store();
        let slice = store.get(offset..end).ok_or((
            "TypeError",
            "Uint8Array buffer is detached or out of bounds",
        ))?;
        for (dst, src) in bytes.iter_mut().zip(slice.iter()) {
            *dst = src.get();
        }
    }
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Router & Policy Ops
// ---------------------------------------------------------------------------

#[op2(fast)]
fn op_vael_http_default_max_body_bytes() -> f64 {
    DEFAULT_BODY_LIMIT as f64
}

#[op2]
fn op_vael_http_random_id<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Value> {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_RANDOM"),
            "OS entropy source failed",
        );
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    );
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set_string(scope, obj, "id", &id);
    obj.into()
}

#[op2]
fn op_vael_http_router_create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
) -> v8::Local<'s, v8::Value> {
    let counts = {
        let Some(http_state) = state.try_borrow::<HttpState>() else {
            return failure(
                scope,
                "Error",
                Some("VAEL_HTTP_INTERNAL"),
                "HTTP state unavailable",
            );
        };
        Arc::clone(&http_state.counts)
    };
    let prev = counts.routers.fetch_add(1, Ordering::Relaxed);
    if prev >= MAX_ROUTERS {
        counts.routers.fetch_sub(1, Ordering::Relaxed);
        return failure(
            scope,
            "RangeError",
            Some("VAEL_HTTP_LIMIT"),
            "maximum number of ServeMux routers exceeded",
        );
    }
    let rid = state.resource_table.add(RouterResource {
        router: RefCell::new(Router::new()),
        counts,
    });
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set_number(scope, obj, "rid", f64::from(rid));
    obj.into()
}

#[op2]
fn op_vael_http_router_add<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    #[smi] router_rid: ResourceId,
    #[string] pattern: String,
    #[smi] handler_id: u32,
) -> v8::Local<'s, v8::Value> {
    let Ok(resource) = state.resource_table.get::<RouterResource>(router_rid) else {
        return failure(scope, "Error", Some("VAEL_HTTP_CLOSED"), "router is closed");
    };
    let mut router = resource.router.borrow_mut();
    match router.add(&pattern, handler_id as usize) {
        Ok(()) => ok_unit(scope),
        Err(err) => core_failure(scope, &err),
    }
}

#[op2]
fn op_vael_http_router_match<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    #[smi] router_rid: ResourceId,
    #[string] method: String,
    #[string] path: String,
) -> v8::Local<'s, v8::Value> {
    let Ok(resource) = state.resource_table.get::<RouterResource>(router_rid) else {
        return failure(scope, "Error", Some("VAEL_HTTP_CLOSED"), "router is closed");
    };
    let router = resource.router.borrow();
    let obj = response_obj(scope);
    match router.match_route(&method, &path) {
        RouteMatch::Matched { handler_id, params } => {
            set_number(scope, obj, "status", 200.0);
            set_number(scope, obj, "handlerId", handler_id as f64);
            let params_arr = pairs_to_v8_array(scope, &params);
            set(scope, obj, "params", params_arr.into());
        }
        RouteMatch::MethodNotAllowed { allow } => {
            set_number(scope, obj, "status", 405.0);
            let allow_arr = v8::Array::new(scope, allow.len() as i32);
            for (idx, method_name) in allow.iter().enumerate() {
                if let Some(ms) = v8::String::new(scope, method_name) {
                    allow_arr.set_index(scope, idx as u32, ms.into());
                }
            }
            set(scope, obj, "allow", allow_arr.into());
        }
        RouteMatch::NotFound => {
            set_number(scope, obj, "status", 404.0);
        }
    }
    obj.into()
}

#[cfg(unix)]
const fn unix_o_nofollow() -> i32 {
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))]
    {
        0x0100
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        any(
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "m68k"
        )
    ))]
    {
        0x8000
    }
    #[cfg(all(
        any(target_os = "linux", target_os = "android"),
        not(any(
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "m68k"
        ))
    ))]
    {
        0x20000
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly",
        target_os = "linux",
        target_os = "android"
    )))]
    {
        0x20000
    }
}

fn is_symlink_or_reparse(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

fn open_static_file_nofollow(path: &Path) -> std::io::Result<File> {
    let pre_meta = std::fs::symlink_metadata(path)?;
    if is_symlink_or_reparse(&pre_meta) || !pre_meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "static path is not a regular file",
        ));
    }
    let mut opts = OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.custom_flags(unix_o_nofollow());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        opts.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = opts.open(path)?;
    let post_meta = file.metadata()?;
    if is_symlink_or_reparse(&post_meta) || !post_meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "static path is not a regular file",
        ));
    }
    Ok(file)
}

#[op2]
fn op_vael_http_static_resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    #[string] root_dir: String,
    #[string] rel_path: String,
    #[string] index_file: String,
    has_index: bool,
    head_only: bool,
) -> v8::Local<'s, v8::Value> {
    let index = if has_index {
        Some(index_file.as_str())
    } else {
        None
    };
    let resolved = match resolve_static_relative_path(Path::new(&root_dir), &rel_path, index) {
        Ok(file) => file,
        Err(HttpCoreError::NotFound) => {
            let obj = response_obj(scope);
            set_bool(scope, obj, "ok", false);
            set_number(scope, obj, "status", 404.0);
            set_string(scope, obj, "message", "Not Found");
            return obj.into();
        }
        Err(err @ HttpCoreError::InvalidPath { .. }) => {
            let obj = response_obj(scope);
            set_bool(scope, obj, "ok", false);
            set_number(scope, obj, "status", 400.0);
            set_string(scope, obj, "code", err.code().as_str());
            set_string(scope, obj, "message", "Bad Request");
            return obj.into();
        }
        Err(err) => {
            let obj = response_obj(scope);
            set_bool(scope, obj, "ok", false);
            set_number(scope, obj, "status", 500.0);
            set_string(scope, obj, "code", err.code().as_str());
            set_string(scope, obj, "message", "Internal Server Error");
            return obj.into();
        }
    };

    let Ok(file) = open_static_file_nofollow(&resolved.path) else {
        let obj = response_obj(scope);
        set_bool(scope, obj, "ok", false);
        set_number(scope, obj, "status", 404.0);
        set_string(scope, obj, "message", "Not Found");
        return obj.into();
    };
    let Ok(meta) = file.metadata() else {
        let obj = response_obj(scope);
        set_bool(scope, obj, "ok", false);
        set_number(scope, obj, "status", 500.0);
        set_string(scope, obj, "message", "Internal Server Error");
        return obj.into();
    };
    if is_symlink_or_reparse(&meta) || !meta.is_file() {
        let obj = response_obj(scope);
        set_bool(scope, obj, "ok", false);
        set_number(scope, obj, "status", 404.0);
        set_string(scope, obj, "message", "Not Found");
        return obj.into();
    }
    let meta_size = meta.len();
    if meta_size > MAX_STATIC_FILE_BYTES {
        let obj = response_obj(scope);
        set_bool(scope, obj, "ok", false);
        set_number(scope, obj, "status", 413.0);
        set_string(scope, obj, "message", "Payload Too Large");
        return obj.into();
    }

    let (body, actual_size) = if head_only {
        (Vec::new(), meta_size)
    } else {
        let mut buf = Vec::new();
        let cap = (meta_size.min(MAX_STATIC_FILE_BYTES)) as usize;
        if buf.try_reserve_exact(cap).is_err() {
            let obj = response_obj(scope);
            set_bool(scope, obj, "ok", false);
            set_number(scope, obj, "status", 500.0);
            set_string(scope, obj, "message", "Internal Server Error");
            return obj.into();
        }
        if file
            .take(MAX_STATIC_FILE_BYTES + 1)
            .read_to_end(&mut buf)
            .is_err()
        {
            let obj = response_obj(scope);
            set_bool(scope, obj, "ok", false);
            set_number(scope, obj, "status", 500.0);
            set_string(scope, obj, "message", "Internal Server Error");
            return obj.into();
        }
        if buf.len() as u64 > MAX_STATIC_FILE_BYTES {
            let obj = response_obj(scope);
            set_bool(scope, obj, "ok", false);
            set_number(scope, obj, "status", 413.0);
            set_string(scope, obj, "message", "Payload Too Large");
            return obj.into();
        }
        let actual_len = buf.len() as u64;
        (buf, actual_len)
    };

    let Some(v8_bytes) = bytes_to_v8(scope, body) else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_INTERNAL"),
            "failed to allocate buffer",
        );
    };
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set_string(scope, obj, "mimeType", resolved.mime_type);
    set_number(scope, obj, "size", actual_size as f64);
    set(scope, obj, "body", v8_bytes);
    obj.into()
}

#[op2]
fn op_vael_http_parse_cookies<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    header: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let raw = match copy_strict_string(scope, header, "cookie header must be a string") {
        Ok(s) => s,
        Err((name, msg)) => return failure(scope, name, None, msg),
    };
    match parse_cookies(&raw) {
        Ok(pairs) => {
            let obj = response_obj(scope);
            set_bool(scope, obj, "ok", true);
            let arr = pairs_to_v8_array(scope, &pairs);
            set(scope, obj, "cookies", arr.into());
            obj.into()
        }
        Err(err) => core_failure(scope, &err),
    }
}

#[op2]
#[allow(clippy::too_many_arguments)]
fn op_vael_http_format_set_cookie<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
    path: v8::Local<'s, v8::Value>,
    domain: v8::Local<'s, v8::Value>,
    max_age: v8::Local<'s, v8::Value>,
    http_only: v8::Local<'s, v8::Value>,
    secure: v8::Local<'s, v8::Value>,
    same_site: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let cookie_name = match copy_strict_string(scope, name, "cookie.name must be a string") {
        Ok(s) => s,
        Err((err_name, msg)) => return failure(scope, err_name, None, msg),
    };
    let cookie_value = match copy_strict_string(scope, value, "cookie.value must be a string") {
        Ok(s) => s,
        Err((err_name, msg)) => return failure(scope, err_name, None, msg),
    };

    let mut cookie = SetCookie::new(cookie_name, cookie_value);

    if !path.is_undefined() {
        match copy_strict_string(scope, path, "cookie.path must be a string") {
            Ok(s) => cookie = cookie.path(s),
            Err((err_name, msg)) => return failure(scope, err_name, None, msg),
        }
    }
    if !domain.is_undefined() {
        match copy_strict_string(scope, domain, "cookie.domain must be a string") {
            Ok(s) => cookie = cookie.domain(s),
            Err((err_name, msg)) => return failure(scope, err_name, None, msg),
        }
    }
    if !max_age.is_undefined() {
        let Ok(num) = v8::Local::<v8::Number>::try_from(max_age) else {
            return failure(scope, "TypeError", None, "cookie.maxAge must be an integer");
        };
        let n = num.value();
        if !n.is_finite() || n.fract() != 0.0 || n < (i64::MIN as f64) || n > (i64::MAX as f64) {
            return failure(
                scope,
                "RangeError",
                None,
                "cookie.maxAge must be a finite integer",
            );
        }
        cookie = cookie.max_age(n as i64);
    }
    if !http_only.is_undefined() {
        let Ok(flag) = v8::Local::<v8::Boolean>::try_from(http_only) else {
            return failure(
                scope,
                "TypeError",
                None,
                "cookie.httpOnly must be a boolean",
            );
        };
        cookie = cookie.http_only(flag.is_true());
    }
    if !secure.is_undefined() {
        let Ok(flag) = v8::Local::<v8::Boolean>::try_from(secure) else {
            return failure(scope, "TypeError", None, "cookie.secure must be a boolean");
        };
        cookie = cookie.secure(flag.is_true());
    }
    if !same_site.is_undefined() {
        let s = match copy_strict_string(scope, same_site, "cookie.sameSite must be a string") {
            Ok(s) => s,
            Err((err_name, msg)) => return failure(scope, err_name, None, msg),
        };
        let parsed = if s.eq_ignore_ascii_case("lax") {
            SameSite::Lax
        } else if s.eq_ignore_ascii_case("strict") {
            SameSite::Strict
        } else if s.eq_ignore_ascii_case("none") {
            SameSite::None
        } else {
            return failure(
                scope,
                "TypeError",
                None,
                "cookie.sameSite must be \"Lax\", \"Strict\", or \"None\"",
            );
        };
        cookie = cookie.same_site(parsed);
    }

    match format_set_cookie(&cookie) {
        Ok(formatted) => {
            let obj = response_obj(scope);
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", &formatted);
            obj.into()
        }
        Err(err) => core_failure(scope, &err),
    }
}

#[op2]
fn op_vael_http_validate_header<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let header_name = match copy_strict_string(scope, name, "header name must be a string") {
        Ok(s) => s,
        Err((err_name, msg)) => return failure(scope, err_name, None, msg),
    };
    let header_value = match copy_strict_string(scope, value, "header value must be a string") {
        Ok(s) => s,
        Err((err_name, msg)) => return failure(scope, err_name, None, msg),
    };
    if let Err(err) = validate_header_name(&header_name) {
        return core_failure(scope, &err);
    }
    if let Err(err) = validate_header_value(&header_value) {
        return core_failure(scope, &err);
    }
    ok_unit(scope)
}

#[op2]
fn op_vael_http_encode_utf8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let text = match copy_strict_string(scope, value, "body must be a string") {
        Ok(s) => s,
        Err((err_name, msg)) => return failure(scope, err_name, None, msg),
    };
    let Some(bytes) = bytes_to_v8(scope, text.into_bytes()) else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_INTERNAL"),
            "failed to allocate buffer",
        );
    };
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set(scope, obj, "bytes", bytes);
    obj.into()
}

#[op2]
fn op_vael_http_decode_utf8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let bytes = match read_uint8_array(scope, value, "bytes must be a Uint8Array") {
        Ok(b) => b,
        Err((err_name, msg)) => return failure(scope, err_name, None, msg),
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_INVALID_UTF8"),
            "bytes are not valid UTF-8",
        );
    };
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set_string(scope, obj, "text", text);
    obj.into()
}

#[op2]
fn op_vael_http_parse_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    #[string] raw: String,
) -> v8::Local<'s, v8::Value> {
    let trimmed = raw.strip_prefix('?').unwrap_or(&raw);
    let mut pairs = Vec::new();
    if !trimmed.is_empty() {
        for part in trimmed.split('&') {
            if part.is_empty() {
                continue;
            }
            if pairs.len() >= MAX_QUERY_PARAMS {
                return failure(
                    scope,
                    "RangeError",
                    Some("VAEL_HTTP_LIMIT"),
                    "query parameter count exceeds limit",
                );
            }
            let (raw_k, raw_v) = match part.split_once('=') {
                Some((k, v)) => (k, v),
                None => (part, ""),
            };
            pairs.push((decode_form_component(raw_k), decode_form_component(raw_v)));
        }
    }
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    let arr = pairs_to_v8_array(scope, &pairs);
    set(scope, obj, "params", arr.into());
    obj.into()
}

fn decode_form_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Live HTTP/1.1 Server Ops (hyper 1 + tokio::net::TcpListener)
// ---------------------------------------------------------------------------

#[op2]
fn op_vael_http_listen<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    host: v8::Local<'s, v8::Value>,
    port: v8::Local<'s, v8::Value>,
    max_body_bytes: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let host = match copy_strict_string(scope, host, "options.host must be a string") {
        Ok(h) => h,
        Err((name, msg)) => return failure(scope, name, None, msg),
    };
    if host.is_empty() || host.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return failure(scope, "TypeError", None, "options.host is invalid");
    }

    let Ok(port_num) = v8::Local::<v8::Number>::try_from(port) else {
        return failure(scope, "TypeError", None, "options.port must be an integer");
    };
    let port_val = port_num.value();
    if !port_val.is_finite() || port_val.fract() != 0.0 || !(0.0..=65535.0).contains(&port_val) {
        return failure(
            scope,
            "RangeError",
            None,
            "options.port must be an integer between 0 and 65535",
        );
    }
    let port_u16 = port_val as u16;

    let Ok(max_body_num) = v8::Local::<v8::Number>::try_from(max_body_bytes) else {
        return failure(
            scope,
            "TypeError",
            None,
            "options.maxBodyBytes must be a positive integer",
        );
    };
    let max_body_val = max_body_num.value();
    if !max_body_val.is_finite()
        || max_body_val.fract() != 0.0
        || max_body_val < 1.0
        || max_body_val > (MAX_ALLOWED_BODY_BYTES as f64)
    {
        return failure(
            scope,
            "RangeError",
            None,
            "options.maxBodyBytes is out of range",
        );
    }
    let max_body = max_body_val as usize;

    let (driver_opt, counts, header_read_timeout, body_read_timeout) = {
        let Some(http_state) = state.try_borrow::<HttpState>() else {
            return failure(
                scope,
                "Error",
                Some("VAEL_HTTP_INTERNAL"),
                "HTTP state unavailable",
            );
        };
        (
            http_state.driver.clone(),
            Arc::clone(&http_state.counts),
            http_state.header_read_timeout,
            http_state.body_read_timeout,
        )
    };

    let Some(driver) = driver_opt else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_NO_EXECUTOR"),
            "Tokio runtime driver is unavailable",
        );
    };

    let prev = counts.servers.fetch_add(1, Ordering::Relaxed);
    if prev >= MAX_SERVERS {
        counts.servers.fetch_sub(1, Ordering::Relaxed);
        return failure(
            scope,
            "RangeError",
            Some("VAEL_HTTP_LIMIT"),
            "maximum number of live HTTP servers exceeded",
        );
    }

    let std_listener = match std::net::TcpListener::bind((host.as_str(), port_u16)) {
        Ok(l) => l,
        Err(err) => {
            counts.servers.fetch_sub(1, Ordering::Relaxed);
            return failure(
                scope,
                "Error",
                Some("VAEL_HTTP_BIND"),
                &format!("failed to bind listener: {err}"),
            );
        }
    };
    if let Err(err) = std_listener.set_nonblocking(true) {
        counts.servers.fetch_sub(1, Ordering::Relaxed);
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_BIND"),
            &format!("failed to configure non-blocking listener: {err}"),
        );
    }

    let (listener, local_addr) = {
        let _guard = driver.enter();
        let listener = match TcpListener::from_std(std_listener) {
            Ok(l) => l,
            Err(err) => {
                counts.servers.fetch_sub(1, Ordering::Relaxed);
                return failure(
                    scope,
                    "Error",
                    Some("VAEL_HTTP_BIND"),
                    &format!("failed to register listener with Tokio: {err}"),
                );
            }
        };
        let local_addr = match listener.local_addr() {
            Ok(addr) => addr,
            Err(err) => {
                counts.servers.fetch_sub(1, Ordering::Relaxed);
                return failure(
                    scope,
                    "Error",
                    Some("VAEL_HTTP_BIND"),
                    &format!("failed to query listener address: {err}"),
                );
            }
        };
        (listener, local_addr)
    };

    let (req_tx, req_rx) = mpsc::channel::<IncomingRequest>(REQUEST_CHANNEL_CAPACITY);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let conn_semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    let active_conns = Arc::new(AtomicUsize::new(0));
    let conns_drained = Arc::new(Notify::new());

    let accept_driver = driver.clone();
    let accept_shutdown = shutdown_rx.clone();
    let accept_active_conns = Arc::clone(&active_conns);
    let accept_conns_drained = Arc::clone(&conns_drained);
    let accept_task = driver.spawn(run_accept_loop(
        listener,
        req_tx,
        accept_shutdown,
        accept_driver,
        max_body,
        header_read_timeout,
        body_read_timeout,
        conn_semaphore,
        accept_active_conns,
        accept_conns_drained,
    ));

    let inner = Arc::new(ServerInner {
        rx: AsyncMutex::new(req_rx),
        shutdown_tx,
        shutdown_rx,
        accept_task: StdMutex::new(Some(accept_task)),
        active_conns,
        conns_drained,
        counts,
    });

    let server_rid = state.resource_table.add(HttpServerResource { inner });

    let bound_host = local_addr.ip().to_string();
    let bound_port = local_addr.port();
    let addr_str = local_addr.to_string();
    let url_str = format!("http://{local_addr}");

    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set_number(scope, obj, "serverRid", f64::from(server_rid));
    set_string(scope, obj, "host", &bound_host);
    set_number(scope, obj, "port", f64::from(bound_port));
    set_string(scope, obj, "addr", &addr_str);
    set_string(scope, obj, "url", &url_str);
    obj.into()
}

#[allow(clippy::too_many_arguments)]
async fn run_accept_loop(
    listener: TcpListener,
    req_tx: mpsc::Sender<IncomingRequest>,
    mut shutdown_rx: watch::Receiver<bool>,
    driver: tokio::runtime::Handle,
    max_body_bytes: usize,
    header_read_timeout: Duration,
    body_read_timeout: Duration,
    conn_semaphore: Arc<Semaphore>,
    active_conns: Arc<AtomicUsize>,
    conns_drained: Arc<Notify>,
) {
    loop {
        if *shutdown_rx.borrow() {
            break;
        }
        let permit = tokio::select! {
            biased;
            res = shutdown_rx.changed() => {
                if res.is_err() || *shutdown_rx.borrow() {
                    break;
                }
                continue;
            }
            res = Arc::clone(&conn_semaphore).acquire_owned() => match res {
                Ok(permit) => permit,
                Err(_) => break,
            },
        };

        let accepted = tokio::select! {
            biased;
            res = shutdown_rx.changed() => {
                if res.is_err() || *shutdown_rx.borrow() {
                    break;
                }
                continue;
            }
            res = listener.accept() => res,
        };

        let (stream, remote_addr) = match accepted {
            Ok(pair) => pair,
            Err(_) => {
                drop(permit);
                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
        };

        let conn_guard =
            ActiveConnGuard::new(Arc::clone(&active_conns), Arc::clone(&conns_drained));
        let conn_tx = req_tx.clone();
        let conn_shutdown = shutdown_rx.clone();
        driver.spawn(async move {
            let _conn_guard = conn_guard;
            let _permit = permit;
            serve_connection(
                stream,
                remote_addr,
                conn_tx,
                conn_shutdown,
                max_body_bytes,
                header_read_timeout,
                body_read_timeout,
            )
            .await;
        });
    }
    drop(listener);
}

async fn serve_connection(
    stream: tokio::net::TcpStream,
    remote_addr: SocketAddr,
    req_tx: mpsc::Sender<IncomingRequest>,
    mut shutdown_rx: watch::Receiver<bool>,
    max_body_bytes: usize,
    header_read_timeout: Duration,
    body_read_timeout: Duration,
) {
    let io = TokioIo::new(stream);
    let service = service_fn(move |req: hyper::Request<Incoming>| {
        let tx = req_tx.clone();
        Box::pin(handle_hyper_request(
            req,
            remote_addr,
            tx,
            max_body_bytes,
            body_read_timeout,
        ))
    });

    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(header_read_timeout)
        .max_buf_size(DEFAULT_MAX_HEADER_BYTES.max(16 * 1024));

    let mut conn = builder.serve_connection(io, service);

    let res = tokio::select! {
        res = &mut conn => Some(res),
        _ = wait_for_shutdown(&mut shutdown_rx) => {
            Pin::new(&mut conn).graceful_shutdown();
            let _ = tokio::time::timeout(GRACEFUL_SHUTDOWN_TIMEOUT, &mut conn).await;
            None
        }
    };

    if let Some(Err(err)) = res
        && err.is_timeout()
    {
        let parts = conn.into_parts();
        let mut stream = parts.io.into_inner();
        let _ = stream
            .write_all(
                b"HTTP/1.1 408 Request Timeout\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 15\r\nConnection: close\r\n\r\nRequest Timeout",
            )
            .await;
        let _ = stream.shutdown().await;
    }
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    while !*shutdown_rx.borrow() {
        if shutdown_rx.changed().await.is_err() {
            return;
        }
    }
}

async fn handle_hyper_request(
    req: hyper::Request<Incoming>,
    remote_addr: SocketAddr,
    req_tx: mpsc::Sender<IncomingRequest>,
    max_body_bytes: usize,
    body_read_timeout: Duration,
) -> Result<HttpLibResponse<Full<Bytes>>, std::convert::Infallible> {
    let method = req.method().as_str().to_owned();
    let is_head = *req.method() == hyper::Method::HEAD;
    let url = req
        .uri()
        .path_and_query()
        .map_or_else(|| "/".to_owned(), |pq| pq.as_str().to_owned());
    let path = req.uri().path().to_owned();

    let mut total_header_bytes: usize = 0;
    let mut headers = Vec::with_capacity(req.headers().len());
    for (name, value) in req.headers() {
        let Ok(value_str) = value.to_str() else {
            return Ok(plain_response(
                StatusCode::BAD_REQUEST,
                "invalid non-ASCII header value",
            ));
        };
        if validate_header_value(value_str).is_err() {
            return Ok(plain_response(
                StatusCode::BAD_REQUEST,
                "invalid header control bytes",
            ));
        }
        let name_str = name.as_str().to_ascii_lowercase();
        total_header_bytes = total_header_bytes
            .saturating_add(name_str.len())
            .saturating_add(value_str.len());
        if total_header_bytes > DEFAULT_MAX_HEADER_BYTES {
            return Ok(plain_response(
                StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
                "Request Header Fields Too Large",
            ));
        }
        if name_str == "content-length"
            && let Ok(declared_len) = value_str.trim().parse::<u64>()
            && declared_len > max_body_bytes as u64
        {
            return Ok(plain_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Payload Too Large",
            ));
        }
        headers.push((name_str, value_str.to_owned()));
    }

    let collected = match tokio::time::timeout(
        body_read_timeout,
        Limited::new(req.into_body(), max_body_bytes).collect(),
    )
    .await
    {
        Ok(Ok(collected)) => collected,
        Ok(Err(err)) => {
            if err.downcast_ref::<LengthLimitError>().is_some() {
                return Ok(plain_response(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Payload Too Large",
                ));
            }
            return Ok(plain_response(StatusCode::BAD_REQUEST, "Bad Request"));
        }
        Err(_) => {
            return Ok(plain_response(
                StatusCode::REQUEST_TIMEOUT,
                "Request Timeout",
            ));
        }
    };
    let body = collected.to_bytes().to_vec();

    let (resp_tx, resp_rx) = oneshot::channel::<HttpResponseData>();
    let incoming = IncomingRequest {
        method,
        is_head,
        url,
        path,
        remote_addr: remote_addr.to_string(),
        headers,
        body,
        resp_tx,
    };

    if req_tx.send(incoming).await.is_err() {
        return Ok(plain_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "Service Unavailable",
        ));
    }

    match resp_rx.await {
        Ok(data) => Ok(build_hyper_response(data)),
        Err(_) => Ok(plain_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
        )),
    }
}

fn plain_response(status: StatusCode, text: &'static str) -> HttpLibResponse<Full<Bytes>> {
    let mut resp = HttpLibResponse::new(Full::new(Bytes::from_static(text.as_bytes())));
    *resp.status_mut() = status;
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    resp
}

fn sanitize_response_headers(
    headers: Vec<(String, String)>,
    status: u16,
    body_len: usize,
    is_head: bool,
) -> Vec<(String, String)> {
    let allow_custom_head_cl =
        is_head && body_len == 0 && status >= 200 && status != 204 && status != 304;
    let forbid_cl = status < 200 || status == 204 || status == 304;
    let mut out = Vec::with_capacity(headers.len());
    let mut kept_cl = false;

    for (name, value) in headers {
        if name.eq_ignore_ascii_case("transfer-encoding") {
            continue;
        }
        if name.eq_ignore_ascii_case("content-length") {
            if forbid_cl || kept_cl {
                continue;
            }
            let trimmed = value.trim();
            if !trimmed.is_empty()
                && trimmed.bytes().all(|b| b.is_ascii_digit())
                && let Ok(parsed) = trimmed.parse::<u64>()
                && (allow_custom_head_cl || parsed == body_len as u64)
            {
                kept_cl = true;
                out.push((name, trimmed.to_owned()));
            }
            continue;
        }
        out.push((name, value));
    }
    out
}

fn build_hyper_response(data: HttpResponseData) -> HttpLibResponse<Full<Bytes>> {
    let Ok(status) = StatusCode::from_u16(data.status) else {
        return plain_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
    };
    let headers =
        sanitize_response_headers(data.headers, data.status, data.body.len(), data.is_head);
    let mut resp = HttpLibResponse::new(Full::new(Bytes::from(data.body)));
    *resp.status_mut() = status;
    for (name, value) in headers {
        if validate_header_name(&name).is_err() || validate_header_value(&value).is_err() {
            return plain_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        }
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            return plain_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        };
        let Ok(header_value) = HeaderValue::from_str(&value) else {
            return plain_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error");
        };
        resp.headers_mut().append(header_name, header_value);
    }
    resp
}

#[op2]
async fn op_vael_http_next_request(
    state: Rc<RefCell<OpState>>,
    #[smi] server_rid: ResourceId,
) -> f64 {
    let inner = {
        let state = state.borrow();
        let Ok(res) = state.resource_table.get::<HttpServerResource>(server_rid) else {
            return -1.0;
        };
        Arc::clone(&res.inner)
    };

    let mut shutdown_rx = inner.shutdown_rx.clone();
    loop {
        let incoming = {
            let mut rx = inner.rx.lock().await;
            tokio::select! {
                biased;
                _ = wait_for_shutdown(&mut shutdown_rx) => None,
                req = rx.recv() => req,
            }
        };

        let Some(req) = incoming else {
            return -1.0;
        };

        let prev = inner
            .counts
            .pending_requests
            .fetch_add(1, Ordering::Relaxed);
        if prev >= MAX_PENDING_REQUESTS {
            inner
                .counts
                .pending_requests
                .fetch_sub(1, Ordering::Relaxed);
            let _ = req.resp_tx.send(HttpResponseData {
                status: 503,
                is_head: req.is_head,
                headers: vec![(
                    "content-type".to_owned(),
                    "text/plain; charset=utf-8".to_owned(),
                )],
                body: b"Service Unavailable".to_vec(),
            });
            continue;
        }

        let res = IncomingRequestResource {
            is_head: req.is_head,
            data: RefCell::new(Some(IncomingRequestData {
                method: req.method,
                url: req.url,
                path: req.path,
                remote_addr: req.remote_addr,
                headers: req.headers,
                body: req.body,
            })),
            resp_tx: RefCell::new(Some(req.resp_tx)),
            counts: Arc::clone(&inner.counts),
        };
        let mut state = state.borrow_mut();
        let rid = state.resource_table.add(res);
        return f64::from(rid);
    }
}

#[op2]
fn op_vael_http_take_request<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    #[smi] req_rid: ResourceId,
) -> v8::Local<'s, v8::Value> {
    let Ok(res) = state.resource_table.get::<IncomingRequestResource>(req_rid) else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_CLOSED"),
            "request resource missing",
        );
    };
    let Some(data) = res.data.borrow_mut().take() else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_CONSUMED"),
            "request already taken",
        );
    };

    let Some(body_v8) = bytes_to_v8(scope, data.body) else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_INTERNAL"),
            "failed to allocate body",
        );
    };
    let headers_v8 = pairs_to_v8_array(scope, &data.headers);

    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set_string(scope, obj, "method", &data.method);
    set_string(scope, obj, "url", &data.url);
    set_string(scope, obj, "path", &data.path);
    set_string(scope, obj, "remoteAddr", &data.remote_addr);
    set(scope, obj, "headers", headers_v8.into());
    set(scope, obj, "body", body_v8);
    obj.into()
}

#[op2]
fn op_vael_http_respond<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    #[smi] req_rid: ResourceId,
    status: v8::Local<'s, v8::Value>,
    headers: v8::Local<'s, v8::Value>,
    body: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let is_head = {
        let Ok(res) = state.resource_table.get::<IncomingRequestResource>(req_rid) else {
            return failure(
                scope,
                "Error",
                Some("VAEL_HTTP_CLOSED"),
                "request already responded or closed",
            );
        };
        res.is_head
    };

    let Ok(status_num) = v8::Local::<v8::Number>::try_from(status) else {
        return failure(
            scope,
            "TypeError",
            None,
            "response status must be an integer",
        );
    };
    let status_val = status_num.value();
    if !status_val.is_finite()
        || status_val.fract() != 0.0
        || !(100.0..=599.0).contains(&status_val)
    {
        return failure(
            scope,
            "RangeError",
            None,
            "response status must be between 100 and 599",
        );
    }
    let status_u16 = status_val as u16;

    let Ok(headers_arr) = v8::Local::<v8::Array>::try_from(headers) else {
        return failure(
            scope,
            "TypeError",
            None,
            "response headers must be an array",
        );
    };
    let header_len = headers_arr.length() as usize;
    if header_len > MAX_RESPONSE_HEADERS {
        return failure(
            scope,
            "RangeError",
            Some("VAEL_HTTP_LIMIT"),
            "response header count exceeds limit",
        );
    }
    let mut parsed_headers = Vec::with_capacity(header_len);
    for idx in 0..header_len {
        let Some(entry_val) = headers_arr.get_index(scope, idx as u32) else {
            return failure(scope, "TypeError", None, "invalid header entry");
        };
        let Ok(pair) = v8::Local::<v8::Array>::try_from(entry_val) else {
            return failure(
                scope,
                "TypeError",
                None,
                "header entry must be a [name, value] pair",
            );
        };
        if pair.length() != 2 {
            return failure(
                scope,
                "TypeError",
                None,
                "header entry must be a [name, value] pair",
            );
        }
        let Some(k_val) = pair.get_index(scope, 0) else {
            return failure(scope, "TypeError", None, "header name missing");
        };
        let Some(v_val) = pair.get_index(scope, 1) else {
            return failure(scope, "TypeError", None, "header value missing");
        };
        let k = match copy_strict_string(scope, k_val, "header name must be a string") {
            Ok(s) => s,
            Err((err_name, msg)) => return failure(scope, err_name, None, msg),
        };
        let v = match copy_strict_string(scope, v_val, "header value must be a string") {
            Ok(s) => s,
            Err((err_name, msg)) => return failure(scope, err_name, None, msg),
        };
        if let Err(err) = validate_header_name(&k) {
            return core_failure(scope, &err);
        }
        if let Err(err) = validate_header_value(&v) {
            return core_failure(scope, &err);
        }
        parsed_headers.push((k, v));
    }

    let body_bytes = if body.is_null() || body.is_undefined() {
        Vec::new()
    } else if body.is_string() {
        match copy_strict_string(scope, body, "body must be a string or Uint8Array") {
            Ok(s) => s.into_bytes(),
            Err((err_name, msg)) => return failure(scope, err_name, None, msg),
        }
    } else {
        match read_uint8_array(scope, body, "body must be a string or Uint8Array") {
            Ok(b) => b,
            Err((err_name, msg)) => return failure(scope, err_name, None, msg),
        }
    };

    let sanitized_headers =
        sanitize_response_headers(parsed_headers, status_u16, body_bytes.len(), is_head);

    let Ok(res) = state
        .resource_table
        .take::<IncomingRequestResource>(req_rid)
    else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_CLOSED"),
            "request already responded or closed",
        );
    };

    if let Some(resp_tx) = res.resp_tx.borrow_mut().take() {
        let _ = resp_tx.send(HttpResponseData {
            status: status_u16,
            is_head,
            headers: sanitized_headers,
            body: body_bytes,
        });
    }
    ok_unit(scope)
}

#[cfg(feature = "test-support")]
#[op2]
fn op_vael_http_set_test_timeouts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    header_timeout_ms: v8::Local<'s, v8::Value>,
    body_timeout_ms: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let parse_ms =
        |val: v8::Local<'s, v8::Value>, name: &str| -> Result<Duration, (&'static str, String)> {
            let Ok(num) = v8::Local::<v8::Number>::try_from(val) else {
                return Err(("TypeError", format!("{name} must be a positive integer")));
            };
            let ms = num.value();
            if !ms.is_finite() || ms.fract() != 0.0 || !(1.0..=600_000.0).contains(&ms) {
                return Err((
                    "RangeError",
                    format!("{name} must be an integer between 1 and 600000"),
                ));
            }
            Ok(Duration::from_millis(ms as u64))
        };

    let header_timeout = match parse_ms(header_timeout_ms, "headerTimeoutMs") {
        Ok(d) => d,
        Err((kind, msg)) => return failure(scope, kind, None, &msg),
    };
    let body_timeout = match parse_ms(body_timeout_ms, "bodyTimeoutMs") {
        Ok(d) => d,
        Err((kind, msg)) => return failure(scope, kind, None, &msg),
    };

    let Some(http_state) = state.try_borrow_mut::<HttpState>() else {
        return failure(
            scope,
            "Error",
            Some("VAEL_HTTP_INTERNAL"),
            "HTTP state unavailable",
        );
    };
    http_state.header_read_timeout = header_timeout;
    http_state.body_read_timeout = body_timeout;
    ok_unit(scope)
}

#[op2]
async fn op_vael_http_close(state: Rc<RefCell<OpState>>, #[smi] server_rid: ResourceId) -> bool {
    let server = {
        let mut state = state.borrow_mut();
        let Ok(res) = state.resource_table.take::<HttpServerResource>(server_rid) else {
            return false;
        };
        res
    };

    let _ = server.inner.shutdown_tx.send(true);
    let accept_handle = server
        .inner
        .accept_task
        .lock()
        .ok()
        .and_then(|mut guard| guard.take());
    if let Some(handle) = accept_handle {
        let _ = handle.await;
    }

    {
        let mut rx = server.inner.rx.lock().await;
        rx.close();
        while let Ok(req) = rx.try_recv() {
            let _ = req.resp_tx.send(HttpResponseData {
                status: 503,
                is_head: req.is_head,
                headers: vec![(
                    "content-type".to_owned(),
                    "text/plain; charset=utf-8".to_owned(),
                )],
                body: b"Service Unavailable".to_vec(),
            });
        }
    }

    let deadline = tokio::time::Instant::now() + GRACEFUL_SHUTDOWN_TIMEOUT;
    loop {
        let notified = server.inner.conns_drained.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if server.inner.active_conns.load(Ordering::Acquire) == 0 {
            break;
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            break;
        }
    }

    true
}

deno_core::extension!(
    vaeljs_http,
    ops = [
        op_vael_http_default_max_body_bytes,
        op_vael_http_random_id,
        op_vael_http_router_create,
        op_vael_http_router_add,
        op_vael_http_router_match,
        op_vael_http_static_resolve,
        op_vael_http_parse_cookies,
        op_vael_http_format_set_cookie,
        op_vael_http_validate_header,
        op_vael_http_encode_utf8,
        op_vael_http_decode_utf8,
        op_vael_http_parse_query,
        op_vael_http_listen,
        op_vael_http_next_request,
        op_vael_http_take_request,
        op_vael_http_respond,
        op_vael_http_close,
    ],
    state = |state| {
        state.put(HttpState::capture());
    },
    docs = "HTTP/1.1 server, Go-style ServeMux router, static files, and cookies behind `Vael.getBuiltInModule(\"vael:http\")`."
);

#[cfg(feature = "test-support")]
deno_core::extension!(
    vaeljs_http_test,
    ops = [op_vael_http_set_test_timeouts],
    docs = "Test-only timeout override for `vael:http`, compiled only with `test-support`."
);

/// The test-only `vael:http` extension.
#[cfg(feature = "test-support")]
pub(crate) fn test_support_extension() -> deno_core::Extension {
    vaeljs_http_test::init()
}

#[cfg(test)]
mod tests {
    use super::open_static_file_nofollow;

    #[test]
    fn static_open_nofollow_opens_regular_file_and_rejects_symlink() {
        let dir = tempfile::tempdir().expect("temp dir");
        let real = dir.path().join("real.txt");
        std::fs::write(&real, b"hello").expect("write real");

        let opened = open_static_file_nofollow(&real).expect("regular file opens");
        assert!(opened.metadata().expect("meta").is_file());

        #[cfg(unix)]
        {
            let link = dir.path().join("link.txt");
            std::os::unix::fs::symlink(&real, &link).expect("symlink");
            assert!(
                open_static_file_nofollow(&link).is_err(),
                "O_NOFOLLOW must refuse opening a symlink"
            );
        }
    }
}

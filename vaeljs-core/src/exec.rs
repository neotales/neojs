//! V8/`deno_core` host ops for `vael:exec`.
//!
//! All argv-only process spawning, bounded concurrent pipe draining, timeout
//! escalation, safe `PATH` lookup (`which` / `which_all`), and symlink-safe OS
//! advisory file locking live in the engine-neutral [`vaeljs_exec_core`] crate.
//! This module validates V8 arguments without consulting prototypes, getters,
//! or Proxy traps, tracks live [`ChildProcess`] and [`FileLock`] handles in the
//! isolate's `ResourceTable` for deterministic cleanup on dispose or runtime
//! drop, and offloads blocking operations onto `tokio::task::spawn_blocking`.

#![forbid(unsafe_code)]

use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::OsString;
use std::io::{self, Read as _, Write as _};
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::time::{Duration, Instant};

use deno_core::FromV8;
use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::ToV8;
use deno_core::op2;
use deno_core::v8;
use deno_error::JsErrorBox;
use tokio::sync::Mutex as AsyncMutex;
use vaeljs_exec_core::{
    CommandOutput, CommandSpec, CommandStatus, ExecError, FileLock, LockMode, LockOptions,
    MAX_OUTPUT_BYTES, Signal, SpawnedChild, StdioMode, exec_sync, run_sync, spawn, which,
    which_all,
};

const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;
const MAX_EXEC_ARGS: usize = 16_384;
const MAX_ENV_ENTRIES: usize = 4_096;
const MAX_OPEN_CHILDREN: usize = 256;
const MAX_OPEN_LOCKS: usize = 1_024;
const DEFAULT_READ_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug)]
enum ExecBindingError {
    Type(&'static str),
    Exec {
        err: ExecError,
        file_name: Option<String>,
        allow_not_found_on_path: bool,
    },
}

impl ExecBindingError {
    fn invalid_arg(reason: &'static str) -> Self {
        Self::Exec {
            err: ExecError::InvalidArgument { reason },
            file_name: None,
            allow_not_found_on_path: false,
        }
    }

    fn exec(err: ExecError, file_name: Option<String>, allow_not_found_on_path: bool) -> Self {
        Self::Exec {
            err,
            file_name,
            allow_not_found_on_path,
        }
    }
}

impl From<ExecError> for ExecBindingError {
    fn from(err: ExecError) -> Self {
        Self::Exec {
            err,
            file_name: None,
            allow_not_found_on_path: false,
        }
    }
}

#[derive(Default)]
struct ExecCounts {
    open_children: AtomicUsize,
    open_locks: AtomicUsize,
}

struct ExecState {
    driver: Option<tokio::runtime::Handle>,
    counts: Arc<ExecCounts>,
}

impl ExecState {
    fn capture() -> Self {
        Self {
            driver: tokio::runtime::Handle::try_current().ok(),
            counts: Arc::new(ExecCounts::default()),
        }
    }
}

fn spawn_blocking_work<F, R>(
    state: &Rc<RefCell<OpState>>,
    work: F,
) -> Result<tokio::task::JoinHandle<R>, ExecBindingError>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        return Ok(tokio::task::spawn_blocking(work));
    }
    let driver = state
        .borrow()
        .try_borrow::<ExecState>()
        .and_then(|s| s.driver.clone());
    if let Some(driver) = driver {
        return Ok(driver.spawn_blocking(work));
    }
    Err(ExecBindingError::from(ExecError::Io {
        context: "spawn blocking task",
        source: io::Error::other("async exec requires a Tokio runtime"),
    }))
}

struct ChildInner {
    max_output_bytes: usize,
    timeout: Option<Duration>,
    kill_signal: Signal,
    kill_grace: Duration,
    child: StdMutex<Option<SpawnedChild>>,
    stdin: AsyncMutex<Option<ChildStdin>>,
    stdout: AsyncMutex<Option<ChildStdout>>,
    stderr: AsyncMutex<Option<ChildStderr>>,
    stdin_closed: AtomicBool,
    disposed: AtomicBool,
}

struct ChildProcessResource {
    inner: Arc<ChildInner>,
    counts: Arc<ExecCounts>,
}

impl Drop for ChildProcessResource {
    fn drop(&mut self) {
        self.inner.disposed.store(true, Ordering::Release);
        self.inner.stdin_closed.store(true, Ordering::Release);
        if let Ok(mut guard) = self.inner.child.lock()
            && let Some(mut child) = guard.take()
        {
            let _ = child.kill(Signal::SigKill);
            let _ = child.wait();
        }
        if let Ok(mut stdin_guard) = self.inner.stdin.try_lock() {
            drop(stdin_guard.take());
        }
        if let Ok(mut stdout_guard) = self.inner.stdout.try_lock() {
            drop(stdout_guard.take());
        }
        if let Ok(mut stderr_guard) = self.inner.stderr.try_lock() {
            drop(stderr_guard.take());
        }
        self.counts.open_children.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Resource for ChildProcessResource {
    fn name(&self) -> Cow<'_, str> {
        "vael::exec::ChildProcess".into()
    }
}

struct FileLockResource {
    lock: RefCell<Option<FileLock>>,
    counts: Arc<ExecCounts>,
}

impl Drop for FileLockResource {
    fn drop(&mut self) {
        drop(self.lock.get_mut().take());
        self.counts.open_locks.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Resource for FileLockResource {
    fn name(&self) -> Cow<'_, str> {
        "vael::exec::FileLock".into()
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
    failure: &ExecBindingError,
) {
    set_bool(scope, object, "ok", false);
    match failure {
        ExecBindingError::Type(msg) => {
            set_string(scope, object, "name", "TypeError");
            set_string(scope, object, "message", msg);
        }
        ExecBindingError::Exec {
            err,
            file_name,
            allow_not_found_on_path,
        } => {
            let is_not_found_on_path =
                *allow_not_found_on_path && err.code() == vaeljs_exec_core::ExecErrorCode::NotFound;
            if is_not_found_on_path {
                set_string(scope, object, "name", "NotFoundOnPathError");
            } else {
                set_string(scope, object, "name", "CommandError");
            }
            set_string(scope, object, "code", err.code().as_str());
            set_string(scope, object, "message", &err.to_string());
            if let Some(name) = file_name {
                set_string(scope, object, "fileName", name);
            }
        }
    }
}

fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Result<v8::Local<'s, v8::Value>, ExecBindingError> {
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
        ExecBindingError::from(ExecError::Io {
            context: "allocate Uint8Array",
            source: io::Error::other("failed to allocate Uint8Array"),
        })
    })
}

fn command_output_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    out: CommandOutput,
) -> Result<v8::Local<'s, v8::Object>, ExecBindingError> {
    let stdout = bytes_to_v8(scope, out.stdout)?;
    let stderr = bytes_to_v8(scope, out.stderr)?;
    let obj = response_obj(scope);
    set_number(scope, obj, "code", f64::from(out.code));
    match &out.signal {
        Some(sig) => set_string(scope, obj, "signal", sig),
        None => set(scope, obj, "signal", v8::null(scope).into()),
    }
    set_bool(scope, obj, "success", out.success);
    set(scope, obj, "stdout", stdout);
    set(scope, obj, "stderr", stderr);
    set_bool(scope, obj, "timedOut", out.timed_out);
    set_bool(scope, obj, "truncated", out.truncated);
    Ok(obj)
}

fn command_status_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    status: &CommandStatus,
) -> v8::Local<'s, v8::Object> {
    let obj = response_obj(scope);
    set_number(scope, obj, "code", f64::from(status.code));
    match &status.signal {
        Some(sig) => set_string(scope, obj, "signal", sig),
        None => set(scope, obj, "signal", v8::null(scope).into()),
    }
    set_bool(scope, obj, "success", status.success);
    set_bool(scope, obj, "timedOut", status.timed_out);
    obj
}

// ---------------------------------------------------------------------------
// Strict V8 argument & prototype-safe option parsing
// ---------------------------------------------------------------------------

fn copy_strict_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<String, ExecBindingError> {
    if !value.is_string() {
        return Err(ExecBindingError::Type(type_msg));
    }
    let string =
        v8::Local::<v8::String>::try_from(value).map_err(|_| ExecBindingError::Type(type_msg))?;
    let length = string.utf8_length(scope);
    let capacity = length
        .checked_add(4)
        .ok_or_else(|| ExecBindingError::invalid_arg("string exceeds maximum buffer length"))?;
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(capacity).map_err(|_| {
        ExecBindingError::from(ExecError::Io {
            context: "allocate string buffer",
            source: io::Error::other("failed to allocate string buffer"),
        })
    })?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    buffer.truncate(written);
    String::from_utf8(buffer)
        .map_err(|_| ExecBindingError::invalid_arg("string contains an unpaired UTF-16 surrogate"))
}

fn read_uint8_array(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<Vec<u8>, ExecBindingError> {
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(ExecBindingError::Type(type_msg));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(ExecBindingError::Type("Uint8Array buffer is detached"));
    };
    if buffer.was_detached() {
        return Err(ExecBindingError::Type("Uint8Array buffer is detached"));
    }
    let length = view.byte_length();
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| {
        ExecBindingError::from(ExecError::Io {
            context: "allocate byte buffer",
            source: io::Error::other("failed to allocate byte buffer"),
        })
    })?;
    bytes.resize(length, 0);
    if length <= i32::MAX as usize {
        if view.copy_contents(&mut bytes) != length {
            return Err(ExecBindingError::Type(
                "Uint8Array buffer is detached or out of bounds",
            ));
        }
    } else {
        let store = buffer.get_backing_store();
        let offset = view.byte_offset();
        let end = offset
            .checked_add(length)
            .ok_or(ExecBindingError::Type("Uint8Array buffer is out of bounds"))?;
        let slice = store
            .get(offset..end)
            .ok_or(ExecBindingError::Type("Uint8Array buffer is out of bounds"))?;
        for (dst, src) in bytes.iter_mut().zip(slice.iter()) {
            *dst = src.get();
        }
    }
    Ok(bytes)
}

fn plain_object_prototype<'s>(
    scope: &v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Value>, ExecBindingError> {
    let object = v8::Object::new(scope);
    object
        .get_prototype(scope)
        .ok_or(ExecBindingError::Type("options must be a plain object"))
}

fn own_descriptor_field<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<Option<v8::Local<'s, v8::Value>>, ExecBindingError> {
    let Some(key) = v8::String::new(scope, name) else {
        return Err(ExecBindingError::Type("options must be a plain object"));
    };
    let key: v8::Local<'s, v8::Name> = key.into();
    match object.has_own_property(scope, key) {
        Some(true) => object
            .get(scope, key.into())
            .map(Some)
            .ok_or(ExecBindingError::Type("options must be a plain object")),
        Some(false) => Ok(None),
        None => Err(ExecBindingError::Type("options must be a plain object")),
    }
}

fn own_descriptor_boolean(
    scope: &v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
    name: &str,
) -> Result<bool, ExecBindingError> {
    let Some(value) = own_descriptor_field(scope, object, name)? else {
        return Err(ExecBindingError::Type("options must be a plain object"));
    };
    if !value.is_boolean() {
        return Err(ExecBindingError::Type("options must be a plain object"));
    }
    Ok(value.boolean_value(scope))
}

fn visit_plain_object_own<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    type_msg: &'static str,
    mut on_field: impl FnMut(
        &mut v8::PinScope<'s, '_>,
        &str,
        v8::Local<'s, v8::Value>,
    ) -> Result<(), ExecBindingError>,
) -> Result<(), ExecBindingError> {
    if !value.is_object() || value.is_proxy() || value.is_array() || value.is_function() {
        return Err(ExecBindingError::Type(type_msg));
    }
    let object =
        v8::Local::<v8::Object>::try_from(value).map_err(|_| ExecBindingError::Type(type_msg))?;
    let Some(prototype) = object.get_prototype(scope) else {
        return Err(ExecBindingError::Type(type_msg));
    };
    if !prototype.is_null() && prototype != plain_object_prototype(scope)? {
        return Err(ExecBindingError::Type(type_msg));
    }
    let args = v8::GetPropertyNamesArgs {
        mode: v8::KeyCollectionMode::OwnOnly,
        property_filter: v8::PropertyFilter::ALL_PROPERTIES,
        index_filter: v8::IndexFilter::IncludeIndices,
        key_conversion: v8::KeyConversionMode::ConvertToString,
    };
    let Some(names) = object.get_own_property_names(scope, args) else {
        return Err(ExecBindingError::Type(type_msg));
    };
    for index in 0..names.length() {
        let Some(name_value) = names.get_index(scope, index) else {
            return Err(ExecBindingError::Type(type_msg));
        };
        if name_value.is_symbol() {
            return Err(ExecBindingError::Type(
                "options must not contain symbol properties",
            ));
        }
        let Ok(name) = v8::Local::<v8::Name>::try_from(name_value) else {
            return Err(ExecBindingError::Type(type_msg));
        };
        let Some(descriptor_value) = object.get_own_property_descriptor(scope, name) else {
            return Err(ExecBindingError::Type(type_msg));
        };
        let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor_value) else {
            return Err(ExecBindingError::Type(type_msg));
        };
        if !own_descriptor_boolean(scope, descriptor, "enumerable")? {
            return Err(ExecBindingError::Type(
                "options must not contain non-enumerable properties",
            ));
        }
        if own_descriptor_field(scope, descriptor, "get")?.is_some()
            || own_descriptor_field(scope, descriptor, "set")?.is_some()
        {
            return Err(ExecBindingError::Type(
                "options must not contain accessor properties",
            ));
        }
        let Ok(key_str) = v8::Local::<v8::String>::try_from(name_value) else {
            return Err(ExecBindingError::Type(type_msg));
        };
        let key = copy_strict_string(scope, key_str.into(), type_msg)?;
        let Some(field_value) = own_descriptor_field(scope, descriptor, "value")? else {
            return Err(ExecBindingError::Type(type_msg));
        };
        on_field(scope, &key, field_value)?;
    }
    Ok(())
}

fn read_string_array(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
) -> Result<Vec<String>, ExecBindingError> {
    if !value.is_array() || value.is_proxy() {
        return Err(ExecBindingError::Type(type_msg));
    }
    let arr =
        v8::Local::<v8::Array>::try_from(value).map_err(|_| ExecBindingError::Type(type_msg))?;
    let len = arr.length() as usize;
    if len > MAX_EXEC_ARGS {
        return Err(ExecBindingError::invalid_arg("argument list exceeds limit"));
    }
    let arr_obj: v8::Local<'_, v8::Object> = arr.into();
    let mut out = Vec::with_capacity(len);
    for index in 0..arr.length() {
        let idx_num = v8::Integer::new_from_unsigned(scope, index);
        let Some(key_str) = idx_num.to_string(scope) else {
            return Err(ExecBindingError::Type(type_msg));
        };
        let key: v8::Local<'_, v8::Name> = key_str.into();
        let Some(desc_val) = arr_obj.get_own_property_descriptor(scope, key) else {
            return Err(ExecBindingError::Type(type_msg));
        };
        let Ok(desc) = v8::Local::<v8::Object>::try_from(desc_val) else {
            return Err(ExecBindingError::Type(type_msg));
        };
        if own_descriptor_field(scope, desc, "get")?.is_some()
            || own_descriptor_field(scope, desc, "set")?.is_some()
        {
            return Err(ExecBindingError::Type(
                "arguments must not contain accessor properties",
            ));
        }
        let Some(elem) = own_descriptor_field(scope, desc, "value")? else {
            return Err(ExecBindingError::Type(type_msg));
        };
        out.push(copy_strict_string(scope, elem, type_msg)?);
    }
    Ok(out)
}

fn parse_stdio_mode(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    allow_bytes: bool,
) -> Result<StdioMode, ExecBindingError> {
    if value.is_string() {
        let s = copy_strict_string(scope, value, "stdio mode must be a string")?;
        return match s.as_str() {
            "inherit" => Ok(StdioMode::Inherit),
            "piped" => Ok(StdioMode::Piped),
            "null" => Ok(StdioMode::Null),
            _ if allow_bytes => Ok(StdioMode::Bytes(s.into_bytes())),
            _ => Err(ExecBindingError::invalid_arg(
                "stdio mode must be 'inherit', 'piped', or 'null'",
            )),
        };
    }
    if allow_bytes && value.is_uint8_array() {
        let bytes = read_uint8_array(
            scope,
            value,
            "stdin must be a mode string, string, or Uint8Array",
        )?;
        return Ok(StdioMode::Bytes(bytes));
    }
    Err(ExecBindingError::Type(if allow_bytes {
        "stdin must be 'inherit', 'piped', 'null', a string, or a Uint8Array"
    } else {
        "stdio mode must be 'inherit', 'piped', or 'null'"
    }))
}

fn parse_positive_duration_ms(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
    zero_msg: &'static str,
) -> Result<Duration, ExecBindingError> {
    if !value.is_number() {
        return Err(ExecBindingError::Type(type_msg));
    }
    let Some(num) = value.number_value(scope) else {
        return Err(ExecBindingError::Type(type_msg));
    };
    if !num.is_finite() || num <= 0.0 || num > MAX_SAFE_INTEGER_F64 {
        return Err(ExecBindingError::invalid_arg(zero_msg));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let millis = num.round() as u64;
    if millis == 0 {
        return Err(ExecBindingError::invalid_arg(zero_msg));
    }
    Ok(Duration::from_millis(millis))
}

fn parse_non_negative_duration_ms(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    type_msg: &'static str,
    invalid_msg: &'static str,
) -> Result<Duration, ExecBindingError> {
    if !value.is_number() {
        return Err(ExecBindingError::Type(type_msg));
    }
    let Some(num) = value.number_value(scope) else {
        return Err(ExecBindingError::Type(type_msg));
    };
    if !num.is_finite() || num < 0.0 || num > MAX_SAFE_INTEGER_F64 {
        return Err(ExecBindingError::invalid_arg(invalid_msg));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let millis = num.round() as u64;
    Ok(Duration::from_millis(millis))
}

fn parse_max_output_bytes(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<usize, ExecBindingError> {
    if !value.is_number() {
        return Err(ExecBindingError::Type("maxOutputBytes must be a number"));
    }
    let Some(num) = value.number_value(scope) else {
        return Err(ExecBindingError::Type("maxOutputBytes must be a number"));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < 0.0 || num > MAX_SAFE_INTEGER_F64 {
        return Err(ExecBindingError::invalid_arg(
            "maxOutputBytes must be a non-negative safe integer",
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = num as u64;
    usize::try_from(n).map_err(|_| ExecBindingError::invalid_arg("maxOutputBytes exceeds limit"))
}

fn parse_env_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<(String, String)>, ExecBindingError> {
    let mut pairs = Vec::new();
    visit_plain_object_own(
        scope,
        value,
        "env must be a plain object mapping strings to strings",
        |scope, key, val| {
            if pairs.len() >= MAX_ENV_ENTRIES {
                return Err(ExecBindingError::invalid_arg("too many env entries"));
            }
            if val.is_undefined() {
                return Ok(());
            }
            let v = copy_strict_string(scope, val, "env values must be strings")?;
            pairs.push((key.to_owned(), v));
            Ok(())
        },
    )?;
    Ok(pairs)
}

fn parse_command_spec_obj<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<CommandSpec, ExecBindingError> {
    let mut program: Option<String> = None;
    let mut args: Vec<String> = Vec::new();
    let mut cwd: Option<String> = None;
    let mut clear_env = false;
    let mut env_ops: Vec<(bool, String, Option<String>)> = Vec::new();
    let mut stdin: Option<StdioMode> = None;
    let mut stdout: Option<StdioMode> = None;
    let mut stderr: Option<StdioMode> = None;
    let mut timeout: Option<Duration> = None;
    let mut kill_signal: Option<Signal> = None;
    let mut kill_grace: Option<Duration> = None;
    let mut max_output_bytes: Option<usize> = None;

    visit_plain_object_own(
        scope,
        value,
        "command spec must be a plain object",
        |scope, key, val| match key {
            "program" => {
                program = Some(copy_strict_string(scope, val, "program must be a string")?);
                Ok(())
            }
            "args" => {
                if !val.is_undefined() {
                    args = read_string_array(scope, val, "args must be an array of strings")?;
                }
                Ok(())
            }
            "cwd" => {
                if !val.is_undefined() {
                    cwd = Some(copy_strict_string(scope, val, "cwd must be a string")?);
                }
                Ok(())
            }
            "clearEnv" => {
                if !val.is_undefined() {
                    if !val.is_boolean() {
                        return Err(ExecBindingError::Type("clearEnv must be a boolean"));
                    }
                    clear_env = val.boolean_value(scope);
                }
                Ok(())
            }
            "envOps" => {
                if val.is_undefined() {
                    return Ok(());
                }
                if !val.is_array() || val.is_proxy() {
                    return Err(ExecBindingError::Type("envOps must be an array"));
                }
                let arr = v8::Local::<v8::Array>::try_from(val)
                    .map_err(|_| ExecBindingError::Type("envOps must be an array"))?;
                let arr_obj: v8::Local<'_, v8::Object> = arr.into();
                for i in 0..arr.length() {
                    let idx_num = v8::Integer::new_from_unsigned(scope, i);
                    let Some(k_str) = idx_num.to_string(scope) else {
                        return Err(ExecBindingError::Type("invalid envOp"));
                    };
                    let Some(desc_val) = arr_obj.get_own_property_descriptor(scope, k_str.into())
                    else {
                        return Err(ExecBindingError::Type("invalid envOp"));
                    };
                    let Ok(desc) = v8::Local::<v8::Object>::try_from(desc_val) else {
                        return Err(ExecBindingError::Type("invalid envOp"));
                    };
                    let Some(entry_val) = own_descriptor_field(scope, desc, "value")? else {
                        return Err(ExecBindingError::Type("invalid envOp"));
                    };
                    if !entry_val.is_array() {
                        return Err(ExecBindingError::Type("invalid envOp"));
                    }
                    let entry = v8::Local::<v8::Array>::try_from(entry_val)
                        .map_err(|_| ExecBindingError::Type("invalid envOp"))?;
                    let entry_obj: v8::Local<'_, v8::Object> = entry.into();
                    let read_idx =
                        |idx: u32| -> Result<v8::Local<'_, v8::Value>, ExecBindingError> {
                            let n = v8::Integer::new_from_unsigned(scope, idx);
                            let ks = n
                                .to_string(scope)
                                .ok_or(ExecBindingError::Type("invalid envOp"))?;
                            let dv = entry_obj
                                .get_own_property_descriptor(scope, ks.into())
                                .ok_or(ExecBindingError::Type("invalid envOp"))?;
                            let d = v8::Local::<v8::Object>::try_from(dv)
                                .map_err(|_| ExecBindingError::Type("invalid envOp"))?;
                            own_descriptor_field(scope, d, "value")?
                                .ok_or(ExecBindingError::Type("invalid envOp"))
                        };
                    let op_kind = copy_strict_string(scope, read_idx(0)?, "invalid envOp kind")?;
                    let k = copy_strict_string(scope, read_idx(1)?, "env key must be a string")?;
                    if op_kind == "set" {
                        let v =
                            copy_strict_string(scope, read_idx(2)?, "env value must be a string")?;
                        env_ops.push((true, k, Some(v)));
                    } else if op_kind == "remove" {
                        env_ops.push((false, k, None));
                    } else {
                        return Err(ExecBindingError::Type("invalid envOp kind"));
                    }
                }
                Ok(())
            }
            "stdin" => {
                if !val.is_undefined() {
                    stdin = Some(parse_stdio_mode(scope, val, true)?);
                }
                Ok(())
            }
            "stdout" => {
                if !val.is_undefined() {
                    stdout = Some(parse_stdio_mode(scope, val, false)?);
                }
                Ok(())
            }
            "stderr" => {
                if !val.is_undefined() {
                    stderr = Some(parse_stdio_mode(scope, val, false)?);
                }
                Ok(())
            }
            "timeoutMs" => {
                if !val.is_undefined() {
                    timeout = Some(parse_positive_duration_ms(
                        scope,
                        val,
                        "timeoutMs must be a number",
                        "timeout must be greater than zero",
                    )?);
                }
                Ok(())
            }
            "killSignal" => {
                if !val.is_undefined() {
                    let s = copy_strict_string(scope, val, "killSignal must be a string")?;
                    kill_signal = Some(Signal::parse(&s)?);
                }
                Ok(())
            }
            "killGraceMs" => {
                if !val.is_undefined() {
                    kill_grace = Some(parse_non_negative_duration_ms(
                        scope,
                        val,
                        "killGraceMs must be a number",
                        "killGraceMs must be a non-negative finite number",
                    )?);
                }
                Ok(())
            }
            "maxOutputBytes" => {
                if !val.is_undefined() {
                    max_output_bytes = Some(parse_max_output_bytes(scope, val)?);
                }
                Ok(())
            }
            _ => Err(ExecBindingError::Type("unknown command spec field")),
        },
    )?;

    let program = program.ok_or(ExecBindingError::Type("program must be a string"))?;
    let mut spec = CommandSpec::new(program)?;
    spec = spec.args(args)?;
    if let Some(dir) = cwd {
        spec = spec.cwd(dir)?;
    }
    if clear_env {
        spec = spec.env_clear();
    }
    for (is_set, k, v) in env_ops {
        if is_set {
            spec = spec.env(k, v.unwrap_or_default())?;
        } else {
            spec = spec.env_remove(k)?;
        }
    }
    if let Some(m) = stdin {
        spec = spec.stdin(m);
    }
    if let Some(m) = stdout {
        spec = spec.stdout(m)?;
    }
    if let Some(m) = stderr {
        spec = spec.stderr(m)?;
    }
    if let Some(t) = timeout {
        spec = spec.timeout(t)?;
    }
    if let Some(sig) = kill_signal {
        spec = spec.kill_signal(sig);
    }
    if let Some(grace) = kill_grace {
        spec = spec.kill_grace(grace)?;
    }
    if let Some(max) = max_output_bytes {
        spec = spec.max_output_bytes(max)?;
    }
    Ok(spec)
}

fn parse_which_prepend<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Option<OsString>, ExecBindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    if value.is_string() {
        let s = copy_strict_string(scope, value, "prependPath must be a string")?;
        return Ok(Some(OsString::from(s)));
    }
    let mut prepend: Option<OsString> = None;
    visit_plain_object_own(
        scope,
        value,
        "which options must be a string or plain object",
        |scope, key, val| match key {
            "prependPath" => {
                if !val.is_undefined() {
                    let s = copy_strict_string(scope, val, "options.prependPath must be a string")?;
                    prepend = Some(OsString::from(s));
                }
                Ok(())
            }
            _ => Err(ExecBindingError::Type("unknown which option")),
        },
    )?;
    Ok(prepend)
}

fn parse_lock_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<LockOptions, ExecBindingError> {
    let mut opts = LockOptions::default();
    if value.is_undefined() {
        return Ok(opts);
    }
    let mut timeout_ms: Option<Duration> = None;
    let mut timeout_alias: Option<Duration> = None;

    visit_plain_object_own(
        scope,
        value,
        "lock options must be a plain object",
        |scope, key, val| match key {
            "mode" => {
                if !val.is_undefined() {
                    let s = copy_strict_string(scope, val, "options.mode must be a string")?;
                    opts.mode = match s.as_str() {
                        "exclusive" => LockMode::Exclusive,
                        "shared" => LockMode::Shared,
                        _ => {
                            return Err(ExecBindingError::invalid_arg(
                                "lock mode must be 'exclusive' or 'shared'",
                            ));
                        }
                    };
                }
                Ok(())
            }
            "create" => {
                if !val.is_undefined() {
                    if !val.is_boolean() {
                        return Err(ExecBindingError::Type("options.create must be a boolean"));
                    }
                    opts.create = val.boolean_value(scope);
                }
                Ok(())
            }
            "nonBlocking" => {
                if !val.is_undefined() {
                    if !val.is_boolean() {
                        return Err(ExecBindingError::Type(
                            "options.nonBlocking must be a boolean",
                        ));
                    }
                    opts.non_blocking = val.boolean_value(scope);
                }
                Ok(())
            }
            "timeoutMs" => {
                if !val.is_undefined() {
                    timeout_ms = Some(parse_positive_duration_ms(
                        scope,
                        val,
                        "options.timeoutMs must be a number",
                        "timeoutMs must be greater than zero",
                    )?);
                }
                Ok(())
            }
            "timeout" => {
                if !val.is_undefined() {
                    timeout_alias = Some(parse_positive_duration_ms(
                        scope,
                        val,
                        "options.timeout must be a number",
                        "timeout must be greater than zero",
                    )?);
                }
                Ok(())
            }
            _ => Err(ExecBindingError::Type("unknown lockFile option")),
        },
    )?;

    if timeout_ms.is_some() && timeout_alias.is_some() {
        return Err(ExecBindingError::Type(
            "cannot specify both timeoutMs and timeout",
        ));
    }
    opts.timeout = timeout_ms.or(timeout_alias);
    Ok(opts)
}

// ---------------------------------------------------------------------------
// FromV8 / ToV8 wrappers for async ops
// ---------------------------------------------------------------------------

struct CommandSpecArg(Result<CommandSpec, ExecBindingError>);

impl<'a> FromV8<'a> for CommandSpecArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        Ok(Self(parse_command_spec_obj(scope, value)))
    }
}

struct WhichArgs {
    name: Result<String, ExecBindingError>,
}

impl<'a> FromV8<'a> for WhichArgs {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            name: copy_strict_string(scope, value, "executable name must be a string"),
        })
    }
}

struct WhichPrependArg(Result<Option<OsString>, ExecBindingError>);

impl<'a> FromV8<'a> for WhichPrependArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        Ok(Self(parse_which_prepend(scope, value)))
    }
}

struct LockPathArg(Result<PathBuf, ExecBindingError>);

impl<'a> FromV8<'a> for LockPathArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        Ok(Self(
            copy_strict_string(scope, value, "lock path must be a string").map(PathBuf::from),
        ))
    }
}

struct LockOptsArg(Result<LockOptions, ExecBindingError>);

impl<'a> FromV8<'a> for LockOptsArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        Ok(Self(parse_lock_options(scope, value)))
    }
}

struct StdinDataArg(Result<Vec<u8>, ExecBindingError>);

impl<'a> FromV8<'a> for StdinDataArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        let res = if value.is_string() {
            copy_strict_string(scope, value, "stdin data must be a string or Uint8Array")
                .map(String::into_bytes)
        } else if value.is_uint8_array() {
            read_uint8_array(scope, value, "stdin data must be a string or Uint8Array")
        } else {
            Err(ExecBindingError::Type(
                "stdin data must be a string or Uint8Array",
            ))
        };
        Ok(Self(res))
    }
}

struct ReadMaxBytesArg(Result<usize, ExecBindingError>);

impl<'a> FromV8<'a> for ReadMaxBytesArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        let res = if value.is_undefined() {
            Ok(DEFAULT_READ_CHUNK_BYTES)
        } else {
            parse_max_output_bytes(scope, value)
        };
        Ok(Self(res))
    }
}

enum ExecOpOutcome {
    Unit,
    Output(CommandOutput),
    Status(CommandStatus),
    Bytes(Vec<u8>),
    WhichOne(Option<String>),
    WhichAll(Vec<String>),
    LockCreated { rid: ResourceId, mode: &'static str },
    Err(ExecBindingError),
}

impl<'a> ToV8<'a> for ExecOpOutcome {
    type Error = JsErrorBox;

    fn to_v8<'i>(
        self,
        scope: &mut v8::PinScope<'a, 'i>,
    ) -> Result<v8::Local<'a, v8::Value>, Self::Error> {
        let obj = response_obj(scope);
        match self {
            Self::Unit => set_bool(scope, obj, "ok", true),
            Self::Output(out) => match command_output_to_v8(scope, out) {
                Ok(val) => {
                    set_bool(scope, obj, "ok", true);
                    set(scope, obj, "value", val.into());
                }
                Err(err) => set_failure(scope, obj, &err),
            },
            Self::Status(status) => {
                set_bool(scope, obj, "ok", true);
                let val = command_status_to_v8(scope, &status);
                set(scope, obj, "value", val.into());
            }
            Self::Bytes(bytes) => match bytes_to_v8(scope, bytes) {
                Ok(val) => {
                    set_bool(scope, obj, "ok", true);
                    set(scope, obj, "value", val);
                }
                Err(err) => set_failure(scope, obj, &err),
            },
            Self::WhichOne(path) => {
                set_bool(scope, obj, "ok", true);
                match path {
                    Some(p) => set_string(scope, obj, "value", &p),
                    None => set(scope, obj, "value", v8::null(scope).into()),
                }
            }
            Self::WhichAll(paths) => {
                set_bool(scope, obj, "ok", true);
                let items: Vec<v8::Local<'_, v8::Value>> = paths
                    .iter()
                    .map(|p| {
                        v8::String::new(scope, p)
                            .map(Into::into)
                            .unwrap_or_else(|| v8::String::empty(scope).into())
                    })
                    .collect();
                let arr = v8::Array::new_with_elements(scope, &items);
                set(scope, obj, "value", arr.into());
            }
            Self::LockCreated { rid, mode } => {
                set_bool(scope, obj, "ok", true);
                set_number(scope, obj, "id", f64::from(rid));
                set_string(scope, obj, "mode", mode);
            }
            Self::Err(err) => set_failure(scope, obj, &err),
        }
        Ok(obj.into())
    }
}

// ---------------------------------------------------------------------------
// Eager Command builder validation ops
// ---------------------------------------------------------------------------

#[op2]
fn op_vael_exec_validate_program<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = copy_strict_string(scope, value, "program must be a string")
        .and_then(|p| CommandSpec::new(p).map_err(ExecBindingError::from));
    match res {
        Ok(spec) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", spec.program());
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = copy_strict_string(scope, value, "arg must be a string").and_then(|a| {
        CommandSpec::new("p")?
            .arg(a)
            .map_err(ExecBindingError::from)
    });
    match res {
        Ok(_) => set_bool(scope, obj, "ok", true),
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = read_string_array(scope, value, "args must be an array of strings").and_then(|vs| {
        CommandSpec::new("p")?
            .args(vs.clone())
            .map(|_| vs)
            .map_err(ExecBindingError::from)
    });
    match res {
        Ok(vs) => {
            set_bool(scope, obj, "ok", true);
            let items: Vec<v8::Local<'s, v8::Value>> = vs
                .iter()
                .map(|s| {
                    v8::String::new(scope, s)
                        .map(Into::into)
                        .unwrap_or_else(|| v8::String::empty(scope).into())
                })
                .collect();
            let arr = v8::Array::new_with_elements(scope, &items);
            set(scope, obj, "value", arr.into());
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_cwd<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = copy_strict_string(scope, value, "cwd must be a string").and_then(|dir| {
        CommandSpec::new("p")?
            .cwd(dir)
            .map_err(ExecBindingError::from)
    });
    match res {
        Ok(_) => set_bool(scope, obj, "ok", true),
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_env_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Value>,
    val: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = (|| -> Result<(), ExecBindingError> {
        let k = copy_strict_string(scope, key, "env key must be a string")?;
        let v = copy_strict_string(scope, val, "env value must be a string")?;
        CommandSpec::new("p")?.env(k, v)?;
        Ok(())
    })();
    match res {
        Ok(()) => set_bool(scope, obj, "ok", true),
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_envs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = (|| -> Result<Vec<(String, String)>, ExecBindingError> {
        let pairs = parse_env_object(scope, value)?;
        let mut spec = CommandSpec::new("p")?;
        for (k, v) in &pairs {
            spec = spec.env(k, v)?;
        }
        Ok(pairs)
    })();
    match res {
        Ok(pairs) => {
            set_bool(scope, obj, "ok", true);
            let items: Vec<v8::Local<'s, v8::Value>> = pairs
                .iter()
                .map(|(k, v)| {
                    let ks = v8::String::new(scope, k)
                        .map(Into::into)
                        .unwrap_or_else(|| v8::String::empty(scope).into());
                    let vs = v8::String::new(scope, v)
                        .map(Into::into)
                        .unwrap_or_else(|| v8::String::empty(scope).into());
                    v8::Array::new_with_elements(scope, &[ks, vs]).into()
                })
                .collect();
            let arr = v8::Array::new_with_elements(scope, &items);
            set(scope, obj, "value", arr.into());
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_env_remove<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = copy_strict_string(scope, key, "env key must be a string").and_then(|k| {
        CommandSpec::new("p")?
            .env_remove(k)
            .map_err(ExecBindingError::from)
    });
    match res {
        Ok(_) => set_bool(scope, obj, "ok", true),
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_stdin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    match parse_stdio_mode(scope, value, true) {
        Ok(StdioMode::Inherit) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", "inherit");
        }
        Ok(StdioMode::Piped) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", "piped");
        }
        Ok(StdioMode::Null) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", "null");
        }
        Ok(StdioMode::Bytes(bytes)) => match bytes_to_v8(scope, bytes) {
            Ok(arr) => {
                set_bool(scope, obj, "ok", true);
                set(scope, obj, "value", arr);
            }
            Err(err) => set_failure(scope, obj, &err),
        },
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_stdio<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    match parse_stdio_mode(scope, value, false) {
        Ok(StdioMode::Inherit) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", "inherit");
        }
        Ok(StdioMode::Piped) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", "piped");
        }
        Ok(StdioMode::Null) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", "null");
        }
        Ok(StdioMode::Bytes(_)) => unreachable!(),
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_timeout<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = parse_positive_duration_ms(
        scope,
        value,
        "timeout must be a number",
        "timeout must be greater than zero",
    )
    .and_then(|d| {
        CommandSpec::new("p")?
            .timeout(d)
            .map(|_| d)
            .map_err(ExecBindingError::from)
    });
    match res {
        Ok(d) => {
            set_bool(scope, obj, "ok", true);
            #[allow(clippy::cast_precision_loss)]
            set_number(scope, obj, "value", d.as_millis() as f64);
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_max_output<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = parse_max_output_bytes(scope, value).and_then(|n| {
        CommandSpec::new("p")?
            .max_output_bytes(n)
            .map(|_| n)
            .map_err(ExecBindingError::from)
    });
    match res {
        Ok(n) => {
            set_bool(scope, obj, "ok", true);
            #[allow(clippy::cast_precision_loss)]
            set_number(scope, obj, "value", n as f64);
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_signal<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = copy_strict_string(scope, value, "signal must be a string")
        .and_then(|s| Signal::parse(&s).map_err(ExecBindingError::from));
    match res {
        Ok(sig) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", sig.as_str());
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_validate_kill_grace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = parse_non_negative_duration_ms(
        scope,
        value,
        "killGrace must be a number",
        "killGrace must be a non-negative finite number",
    )
    .and_then(|d| {
        CommandSpec::new("p")?
            .kill_grace(d)
            .map(|_| d)
            .map_err(ExecBindingError::from)
    });
    match res {
        Ok(d) => {
            set_bool(scope, obj, "ok", true);
            #[allow(clippy::cast_precision_loss)]
            set_number(scope, obj, "value", d.as_millis() as f64);
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
fn op_vael_exec_parse_command_options<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    if value.is_undefined() {
        set_bool(scope, obj, "ok", true);
        let empty = response_obj(scope);
        set(scope, obj, "value", empty.into());
        return obj.into();
    }

    let out = response_obj(scope);
    let mut timeout_ms_seen: Option<f64> = None;
    let mut timeout_alias_seen: Option<f64> = None;
    let mut kill_grace_ms_seen: Option<f64> = None;
    let mut kill_grace_alias_seen: Option<f64> = None;

    let res = visit_plain_object_own(
        scope,
        value,
        "command options must be a plain object",
        |scope, key, val| match key {
            "args" => {
                if !val.is_undefined() {
                    let vs =
                        read_string_array(scope, val, "options.args must be an array of strings")?;
                    CommandSpec::new("p")?.args(vs.clone())?;
                    let items: Vec<v8::Local<'s, v8::Value>> = vs
                        .iter()
                        .map(|s| {
                            v8::String::new(scope, s)
                                .map(Into::into)
                                .unwrap_or_else(|| v8::String::empty(scope).into())
                        })
                        .collect();
                    let arr = v8::Array::new_with_elements(scope, &items);
                    set(scope, out, "args", arr.into());
                }
                Ok(())
            }
            "cwd" => {
                if !val.is_undefined() {
                    let dir = copy_strict_string(scope, val, "options.cwd must be a string")?;
                    CommandSpec::new("p")?.cwd(&dir)?;
                    set_string(scope, out, "cwd", &dir);
                }
                Ok(())
            }
            "clearEnv" => {
                if !val.is_undefined() {
                    if !val.is_boolean() {
                        return Err(ExecBindingError::Type("options.clearEnv must be a boolean"));
                    }
                    set_bool(scope, out, "clearEnv", val.boolean_value(scope));
                }
                Ok(())
            }
            "env" => {
                if !val.is_undefined() {
                    let pairs = parse_env_object(scope, val)?;
                    let mut spec = CommandSpec::new("p")?;
                    for (k, v) in &pairs {
                        spec = spec.env(k, v)?;
                    }
                    let items: Vec<v8::Local<'s, v8::Value>> = pairs
                        .iter()
                        .map(|(k, v)| {
                            let ks = v8::String::new(scope, k)
                                .map(Into::into)
                                .unwrap_or_else(|| v8::String::empty(scope).into());
                            let vs = v8::String::new(scope, v)
                                .map(Into::into)
                                .unwrap_or_else(|| v8::String::empty(scope).into());
                            v8::Array::new_with_elements(scope, &[ks, vs]).into()
                        })
                        .collect();
                    let arr = v8::Array::new_with_elements(scope, &items);
                    set(scope, out, "env", arr.into());
                }
                Ok(())
            }
            "envRemove" => {
                if !val.is_undefined() {
                    let keys = read_string_array(
                        scope,
                        val,
                        "options.envRemove must be an array of strings",
                    )?;
                    let mut spec = CommandSpec::new("p")?;
                    for k in &keys {
                        spec = spec.env_remove(k)?;
                    }
                    let items: Vec<v8::Local<'s, v8::Value>> = keys
                        .iter()
                        .map(|s| {
                            v8::String::new(scope, s)
                                .map(Into::into)
                                .unwrap_or_else(|| v8::String::empty(scope).into())
                        })
                        .collect();
                    let arr = v8::Array::new_with_elements(scope, &items);
                    set(scope, out, "envRemove", arr.into());
                }
                Ok(())
            }
            "stdin" => {
                if !val.is_undefined() {
                    match parse_stdio_mode(scope, val, true)? {
                        StdioMode::Inherit => set_string(scope, out, "stdin", "inherit"),
                        StdioMode::Piped => set_string(scope, out, "stdin", "piped"),
                        StdioMode::Null => set_string(scope, out, "stdin", "null"),
                        StdioMode::Bytes(bytes) => {
                            let arr = bytes_to_v8(scope, bytes)?;
                            set(scope, out, "stdin", arr);
                        }
                    }
                }
                Ok(())
            }
            "stdout" => {
                if !val.is_undefined() {
                    match parse_stdio_mode(scope, val, false)? {
                        StdioMode::Inherit => set_string(scope, out, "stdout", "inherit"),
                        StdioMode::Piped => set_string(scope, out, "stdout", "piped"),
                        StdioMode::Null => set_string(scope, out, "stdout", "null"),
                        StdioMode::Bytes(_) => unreachable!(),
                    }
                }
                Ok(())
            }
            "stderr" => {
                if !val.is_undefined() {
                    match parse_stdio_mode(scope, val, false)? {
                        StdioMode::Inherit => set_string(scope, out, "stderr", "inherit"),
                        StdioMode::Piped => set_string(scope, out, "stderr", "piped"),
                        StdioMode::Null => set_string(scope, out, "stderr", "null"),
                        StdioMode::Bytes(_) => unreachable!(),
                    }
                }
                Ok(())
            }
            "timeoutMs" => {
                if !val.is_undefined() {
                    let d = parse_positive_duration_ms(
                        scope,
                        val,
                        "options.timeoutMs must be a number",
                        "timeoutMs must be greater than zero",
                    )?;
                    CommandSpec::new("p")?.timeout(d)?;
                    #[allow(clippy::cast_precision_loss)]
                    {
                        timeout_ms_seen = Some(d.as_millis() as f64);
                    }
                }
                Ok(())
            }
            "timeout" => {
                if !val.is_undefined() {
                    let d = parse_positive_duration_ms(
                        scope,
                        val,
                        "options.timeout must be a number",
                        "timeout must be greater than zero",
                    )?;
                    CommandSpec::new("p")?.timeout(d)?;
                    #[allow(clippy::cast_precision_loss)]
                    {
                        timeout_alias_seen = Some(d.as_millis() as f64);
                    }
                }
                Ok(())
            }
            "killSignal" => {
                if !val.is_undefined() {
                    let s = copy_strict_string(scope, val, "options.killSignal must be a string")?;
                    let sig = Signal::parse(&s)?;
                    set_string(scope, out, "killSignal", sig.as_str());
                }
                Ok(())
            }
            "killGraceMs" => {
                if !val.is_undefined() {
                    let d = parse_non_negative_duration_ms(
                        scope,
                        val,
                        "options.killGraceMs must be a number",
                        "killGraceMs must be a non-negative finite number",
                    )?;
                    CommandSpec::new("p")?.kill_grace(d)?;
                    #[allow(clippy::cast_precision_loss)]
                    {
                        kill_grace_ms_seen = Some(d.as_millis() as f64);
                    }
                }
                Ok(())
            }
            "killGrace" => {
                if !val.is_undefined() {
                    let d = parse_non_negative_duration_ms(
                        scope,
                        val,
                        "options.killGrace must be a number",
                        "killGrace must be a non-negative finite number",
                    )?;
                    CommandSpec::new("p")?.kill_grace(d)?;
                    #[allow(clippy::cast_precision_loss)]
                    {
                        kill_grace_alias_seen = Some(d.as_millis() as f64);
                    }
                }
                Ok(())
            }
            "maxOutputBytes" => {
                if !val.is_undefined() {
                    let n = parse_max_output_bytes(scope, val)?;
                    CommandSpec::new("p")?.max_output_bytes(n)?;
                    #[allow(clippy::cast_precision_loss)]
                    set_number(scope, out, "maxOutputBytes", n as f64);
                }
                Ok(())
            }
            _ => Err(ExecBindingError::Type("unknown command option")),
        },
    );

    if let Err(err) = res {
        set_failure(scope, obj, &err);
        return obj.into();
    }
    if timeout_ms_seen.is_some() && timeout_alias_seen.is_some() {
        set_failure(
            scope,
            obj,
            &ExecBindingError::Type("cannot specify both timeoutMs and timeout"),
        );
        return obj.into();
    }
    if let Some(ms) = timeout_ms_seen.or(timeout_alias_seen) {
        set_number(scope, out, "timeoutMs", ms);
    }
    if kill_grace_ms_seen.is_some() && kill_grace_alias_seen.is_some() {
        set_failure(
            scope,
            obj,
            &ExecBindingError::Type("cannot specify both killGraceMs and killGrace"),
        );
        return obj.into();
    }
    if let Some(ms) = kill_grace_ms_seen.or(kill_grace_alias_seen) {
        set_number(scope, out, "killGraceMs", ms);
    }

    set_bool(scope, obj, "ok", true);
    set(scope, obj, "value", out.into());
    obj.into()
}

// ---------------------------------------------------------------------------
// Command execution ops (sync + async)
// ---------------------------------------------------------------------------

#[op2]
fn op_vael_exec_output_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    spec_val: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let spec = match parse_command_spec_obj(scope, spec_val) {
        Ok(s) => s,
        Err(err) => {
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let program = spec.program().to_owned();
    match exec_sync(&spec) {
        Ok(out) => match command_output_to_v8(scope, out) {
            Ok(val) => {
                set_bool(scope, obj, "ok", true);
                set(scope, obj, "value", val.into());
            }
            Err(err) => set_failure(scope, obj, &err),
        },
        Err(err) => set_failure(
            scope,
            obj,
            &ExecBindingError::exec(err, Some(program), true),
        ),
    }
    obj.into()
}

#[op2]
async fn op_vael_exec_output_async(
    state: Rc<RefCell<OpState>>,
    #[scoped] spec: CommandSpecArg,
) -> ExecOpOutcome {
    let spec = match spec.0 {
        Ok(s) => s,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let program = spec.program().to_owned();
    let join = match spawn_blocking_work(&state, move || exec_sync(&spec)) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    match join.await {
        Ok(Ok(out)) => ExecOpOutcome::Output(out),
        Ok(Err(err)) => ExecOpOutcome::Err(ExecBindingError::exec(err, Some(program), true)),
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "exec worker task",
            source: io::Error::other("exec worker task panicked or was cancelled"),
        })),
    }
}

#[op2]
fn op_vael_exec_status_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    spec_val: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let spec = match parse_command_spec_obj(scope, spec_val) {
        Ok(s) => s,
        Err(err) => {
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let program = spec.program().to_owned();
    match run_sync(&spec) {
        Ok(status) => {
            set_bool(scope, obj, "ok", true);
            let val = command_status_to_v8(scope, &status);
            set(scope, obj, "value", val.into());
        }
        Err(err) => set_failure(
            scope,
            obj,
            &ExecBindingError::exec(err, Some(program), true),
        ),
    }
    obj.into()
}

#[op2]
async fn op_vael_exec_status_async(
    state: Rc<RefCell<OpState>>,
    #[scoped] spec: CommandSpecArg,
) -> ExecOpOutcome {
    let spec = match spec.0 {
        Ok(s) => s,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let program = spec.program().to_owned();
    let join = match spawn_blocking_work(&state, move || run_sync(&spec)) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    match join.await {
        Ok(Ok(status)) => ExecOpOutcome::Status(status),
        Ok(Err(err)) => ExecOpOutcome::Err(ExecBindingError::exec(err, Some(program), true)),
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "run worker task",
            source: io::Error::other("run worker task panicked or was cancelled"),
        })),
    }
}

#[op2]
fn op_vael_exec_decode_utf8_lossy<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let bytes = match read_uint8_array(scope, value, "bytes must be a Uint8Array") {
        Ok(b) => b,
        Err(err) => {
            let obj = response_obj(scope);
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let obj = response_obj(scope);
    set_bool(scope, obj, "ok", true);
    set_string(scope, obj, "value", &text);
    obj.into()
}

// ---------------------------------------------------------------------------
// ChildProcess ops (spawn + streaming stdio + kill + wait + output + dispose)
// ---------------------------------------------------------------------------

#[op2]
fn op_vael_exec_spawn<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    spec_val: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let spec = match parse_command_spec_obj(scope, spec_val) {
        Ok(s) => s,
        Err(err) => {
            set_failure(scope, obj, &err);
            return obj.into();
        }
    };

    let counts = state
        .try_borrow::<ExecState>()
        .map(|s| Arc::clone(&s.counts))
        .unwrap_or_default();
    let prev = counts.open_children.fetch_add(1, Ordering::Relaxed);
    if prev >= MAX_OPEN_CHILDREN {
        counts.open_children.fetch_sub(1, Ordering::Relaxed);
        set_failure(
            scope,
            obj,
            &ExecBindingError::invalid_arg("maximum number of open ChildProcess handles exceeded"),
        );
        return obj.into();
    }

    let program = spec.program().to_owned();
    let max_output_bytes = spec.get_max_output_bytes();
    let timeout = spec.get_timeout();
    let kill_signal = spec.get_kill_signal();
    let kill_grace = spec.get_kill_grace();

    let mut spawned = match spawn(&spec) {
        Ok(child) => child,
        Err(err) => {
            counts.open_children.fetch_sub(1, Ordering::Relaxed);
            set_failure(
                scope,
                obj,
                &ExecBindingError::exec(err, Some(program), true),
            );
            return obj.into();
        }
    };

    let pid = spawned.pid();
    let stdin = spawned.take_stdin();
    let stdout = spawned.take_stdout();
    let stderr = spawned.take_stderr();

    let inner = Arc::new(ChildInner {
        max_output_bytes,
        timeout,
        kill_signal,
        kill_grace,
        child: StdMutex::new(Some(spawned)),
        stdin: AsyncMutex::new(stdin),
        stdout: AsyncMutex::new(stdout),
        stderr: AsyncMutex::new(stderr),
        stdin_closed: AtomicBool::new(false),
        disposed: AtomicBool::new(false),
    });

    let rid = state
        .resource_table
        .add(ChildProcessResource { inner, counts });

    set_bool(scope, obj, "ok", true);
    set_number(scope, obj, "id", f64::from(rid));
    set_number(scope, obj, "pid", f64::from(pid));
    obj.into()
}

fn get_child_inner(
    state: &Rc<RefCell<OpState>>,
    rid: ResourceId,
) -> Result<Arc<ChildInner>, ExecBindingError> {
    let borrowed = state.borrow();
    let res = borrowed
        .resource_table
        .get::<ChildProcessResource>(rid)
        .map_err(|_| ExecBindingError::invalid_arg("child process handle is disposed"))?;
    if res.inner.disposed.load(Ordering::Acquire) {
        return Err(ExecBindingError::invalid_arg(
            "child process handle is disposed",
        ));
    }
    Ok(Arc::clone(&res.inner))
}

#[op2]
async fn op_vael_exec_child_write_stdin(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[scoped] data: StdinDataArg,
) -> ExecOpOutcome {
    let bytes = match data.0 {
        Ok(b) => b,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let inner = match get_child_inner(&state, rid) {
        Ok(i) => i,
        Err(err) => return ExecOpOutcome::Err(err),
    };

    let mut stdin_guard = inner.stdin.lock().await;
    if inner.stdin_closed.load(Ordering::Acquire) || inner.disposed.load(Ordering::Acquire) {
        drop(stdin_guard.take());
        return ExecOpOutcome::Err(ExecBindingError::invalid_arg(
            "child stdin is not piped or already closed",
        ));
    }
    let Some(mut pipe) = stdin_guard.take() else {
        return ExecOpOutcome::Err(ExecBindingError::invalid_arg(
            "child stdin is not piped or already closed",
        ));
    };

    let join = match spawn_blocking_work(&state, move || {
        let res = pipe
            .write_all(&bytes)
            .and_then(|()| pipe.flush())
            .map_err(|err| ExecError::Io {
                context: "write child stdin",
                source: err,
            });
        (pipe, res)
    }) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };

    match join.await {
        Ok((pipe, res)) => {
            if !inner.stdin_closed.load(Ordering::Acquire)
                && !inner.disposed.load(Ordering::Acquire)
            {
                *stdin_guard = Some(pipe);
            }
            match res {
                Ok(()) => ExecOpOutcome::Unit,
                Err(err) => ExecOpOutcome::Err(ExecBindingError::from(err)),
            }
        }
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "write child stdin",
            source: io::Error::other("stdin writer task panicked or was cancelled"),
        })),
    }
}

#[op2]
fn op_vael_exec_child_close_stdin<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    #[smi] rid: ResourceId,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    if let Ok(res) = state.resource_table.get::<ChildProcessResource>(rid) {
        res.inner.stdin_closed.store(true, Ordering::Release);
        if let Ok(mut guard) = res.inner.stdin.try_lock() {
            drop(guard.take());
        }
    }
    set_bool(scope, obj, "ok", true);
    obj.into()
}

fn read_pipe_chunk(
    pipe: &mut impl io::Read,
    max_bytes: usize,
    context: &'static str,
) -> Result<Vec<u8>, ExecError> {
    if max_bytes == 0 {
        return Ok(Vec::new());
    }
    let mut buf = vec![0u8; max_bytes.min(DEFAULT_READ_CHUNK_BYTES)];
    loop {
        match pipe.read(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                return Ok(buf);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => {
                return Err(ExecError::Io {
                    context,
                    source: err,
                });
            }
        }
    }
}

#[op2]
async fn op_vael_exec_child_read_stdout(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[scoped] max_bytes: ReadMaxBytesArg,
) -> ExecOpOutcome {
    let max_bytes = match max_bytes.0 {
        Ok(n) => n,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    if max_bytes > MAX_OUTPUT_BYTES {
        return ExecOpOutcome::Err(ExecBindingError::invalid_arg(
            "maxBytes exceeds hard limit of 64 MiB",
        ));
    }
    let inner = match get_child_inner(&state, rid) {
        Ok(i) => i,
        Err(err) => return ExecOpOutcome::Err(err),
    };

    let mut guard = inner.stdout.lock().await;
    let Some(mut pipe) = guard.take() else {
        return ExecOpOutcome::Err(ExecBindingError::invalid_arg(
            "child stdout is not piped or already taken",
        ));
    };

    let join = match spawn_blocking_work(&state, move || {
        let res = read_pipe_chunk(&mut pipe, max_bytes, "read child stdout");
        (pipe, res)
    }) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };

    match join.await {
        Ok((pipe, res)) => {
            if !inner.disposed.load(Ordering::Acquire) {
                *guard = Some(pipe);
            }
            match res {
                Ok(bytes) => ExecOpOutcome::Bytes(bytes),
                Err(err) => ExecOpOutcome::Err(ExecBindingError::from(err)),
            }
        }
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "read child stdout",
            source: io::Error::other("stdout reader task panicked or was cancelled"),
        })),
    }
}

#[op2]
async fn op_vael_exec_child_read_stderr(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[scoped] max_bytes: ReadMaxBytesArg,
) -> ExecOpOutcome {
    let max_bytes = match max_bytes.0 {
        Ok(n) => n,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    if max_bytes > MAX_OUTPUT_BYTES {
        return ExecOpOutcome::Err(ExecBindingError::invalid_arg(
            "maxBytes exceeds hard limit of 64 MiB",
        ));
    }
    let inner = match get_child_inner(&state, rid) {
        Ok(i) => i,
        Err(err) => return ExecOpOutcome::Err(err),
    };

    let mut guard = inner.stderr.lock().await;
    let Some(mut pipe) = guard.take() else {
        return ExecOpOutcome::Err(ExecBindingError::invalid_arg(
            "child stderr is not piped or already taken",
        ));
    };

    let join = match spawn_blocking_work(&state, move || {
        let res = read_pipe_chunk(&mut pipe, max_bytes, "read child stderr");
        (pipe, res)
    }) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };

    match join.await {
        Ok((pipe, res)) => {
            if !inner.disposed.load(Ordering::Acquire) {
                *guard = Some(pipe);
            }
            match res {
                Ok(bytes) => ExecOpOutcome::Bytes(bytes),
                Err(err) => ExecOpOutcome::Err(ExecBindingError::from(err)),
            }
        }
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "read child stderr",
            source: io::Error::other("stderr reader task panicked or was cancelled"),
        })),
    }
}

#[op2]
fn op_vael_exec_child_kill<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    #[smi] rid: ResourceId,
    signal_val: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let signal = if signal_val.is_undefined() {
        Signal::SigTerm
    } else {
        match copy_strict_string(scope, signal_val, "signal must be a string")
            .and_then(|s| Signal::parse(&s).map_err(ExecBindingError::from))
        {
            Ok(sig) => sig,
            Err(err) => {
                set_failure(scope, obj, &err);
                return obj.into();
            }
        }
    };

    let inner = match state.resource_table.get::<ChildProcessResource>(rid) {
        Ok(res) if !res.inner.disposed.load(Ordering::Acquire) => Arc::clone(&res.inner),
        _ => {
            set_failure(
                scope,
                obj,
                &ExecBindingError::invalid_arg("child process handle is disposed"),
            );
            return obj.into();
        }
    };

    let mut guard = inner
        .child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(ref mut child) = *guard else {
        set_bool(scope, obj, "ok", true);
        return obj.into();
    };
    match child.kill(signal) {
        Ok(()) => set_bool(scope, obj, "ok", true),
        Err(err) => set_failure(scope, obj, &ExecBindingError::from(err)),
    }
    obj.into()
}

fn wait_child_inner_blocking(inner: &ChildInner) -> Result<CommandStatus, ExecError> {
    let (status, _) = wait_with_optional_timeout(inner, None)?;
    Ok(status)
}

#[op2]
fn op_vael_exec_child_try_wait<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    #[smi] rid: ResourceId,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let inner = {
        let Some(res) = state.resource_table.get::<ChildProcessResource>(rid).ok() else {
            set_failure(
                scope,
                obj,
                &ExecBindingError::invalid_arg("child process handle is disposed"),
            );
            return obj.into();
        };
        if res.inner.disposed.load(Ordering::Acquire) {
            set_failure(
                scope,
                obj,
                &ExecBindingError::invalid_arg("child process handle is disposed"),
            );
            return obj.into();
        }
        Arc::clone(&res.inner)
    };

    let mut guard = inner
        .child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(ref mut child) = *guard else {
        set_failure(
            scope,
            obj,
            &ExecBindingError::invalid_arg("child process handle is disposed"),
        );
        return obj.into();
    };
    match child.try_wait() {
        Ok(Some(status)) => {
            set_bool(scope, obj, "ok", true);
            let val = command_status_to_v8(scope, &status);
            set(scope, obj, "value", val.into());
        }
        Ok(None) => {
            set_bool(scope, obj, "ok", true);
            let null_val: v8::Local<'s, v8::Value> = v8::null(scope).into();
            set(scope, obj, "value", null_val);
        }
        Err(err) => set_failure(scope, obj, &ExecBindingError::from(err)),
    }
    obj.into()
}

#[op2]
async fn op_vael_exec_child_wait(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
) -> ExecOpOutcome {
    let inner = match get_child_inner(&state, rid) {
        Ok(i) => i,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let join = match spawn_blocking_work(&state, move || wait_child_inner_blocking(&inner)) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    match join.await {
        Ok(Ok(status)) => ExecOpOutcome::Status(status),
        Ok(Err(err)) => ExecOpOutcome::Err(ExecBindingError::from(err)),
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "wait for child process",
            source: io::Error::other("child wait task panicked or was cancelled"),
        })),
    }
}

#[derive(Default)]
struct PipeDrainState {
    buf: Vec<u8>,
    done: bool,
    err: Option<io::Error>,
}

struct PipeDrainHandle {
    signal: Arc<(StdMutex<PipeDrainState>, Condvar)>,
    #[cfg(unix)]
    shutdown_stream: Option<UnixStream>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl PipeDrainHandle {
    fn collect(&self, deadline: Option<Instant>) -> (Vec<u8>, Option<io::Error>) {
        let (lock, cvar) = &*self.signal;
        let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = match deadline {
            None => cvar
                .wait_while(guard, |s| !s.done)
                .unwrap_or_else(|e| e.into_inner()),
            Some(dl) => {
                let remaining = dl.saturating_duration_since(Instant::now());
                if !guard.done && !remaining.is_zero() {
                    cvar.wait_timeout_while(guard, remaining, |s| !s.done)
                        .map(|(g, _)| g)
                        .unwrap_or_else(|e| e.into_inner().0)
                } else {
                    guard
                }
            }
        };
        (std::mem::take(&mut state.buf), state.err.take())
    }

    fn finish(self, timed_out: bool) {
        #[cfg(unix)]
        let has_shutdown = if let Some(stream) = self.shutdown_stream {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            true
        } else {
            false
        };
        if let Some(join) = self.join {
            #[cfg(unix)]
            {
                if has_shutdown || !timed_out {
                    let _ = join.join();
                }
            }
            #[cfg(not(unix))]
            if !timed_out {
                let _ = join.join();
            }
        }
    }
}

#[cfg(unix)]
fn spawn_unix_pipe_drain<P: Into<OwnedFd>>(
    pipe: P,
    max_bytes: usize,
    exceeded: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> PipeDrainHandle {
    let mut stream = UnixStream::from(pipe.into());
    let _ = stream.set_read_timeout(Some(Duration::from_millis(25)));
    let shutdown_stream = stream.try_clone().ok();
    let signal = Arc::new((StdMutex::new(PipeDrainState::default()), Condvar::new()));
    let worker_signal = Arc::clone(&signal);
    let join = std::thread::spawn(move || {
        let mut scratch = [0u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::Acquire) {
                let (lock, cvar) = &*worker_signal;
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                state.done = true;
                cvar.notify_all();
                break;
            }
            match stream.read(&mut scratch) {
                Ok(0) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
                Ok(n) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let remaining = max_bytes.saturating_sub(state.buf.len());
                    if n > remaining {
                        state.buf.extend_from_slice(&scratch[..remaining]);
                        exceeded.store(true, Ordering::Release);
                        state.done = true;
                        cvar.notify_all();
                        drop(state);
                        drop(stream);
                        break;
                    }
                    state.buf.extend_from_slice(&scratch[..n]);
                }
                Err(err)
                    if matches!(
                        err.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(err) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.err = Some(err);
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
            }
        }
    });
    PipeDrainHandle {
        signal,
        shutdown_stream,
        join: Some(join),
    }
}

#[cfg(not(unix))]
fn spawn_generic_pipe_drain(
    mut pipe: impl io::Read + Send + 'static,
    max_bytes: usize,
    exceeded: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
) -> PipeDrainHandle {
    let signal = Arc::new((StdMutex::new(PipeDrainState::default()), Condvar::new()));
    let worker_signal = Arc::clone(&signal);
    let join = std::thread::spawn(move || {
        let mut scratch = [0u8; 64 * 1024];
        loop {
            if cancel.load(Ordering::Acquire) {
                let (lock, cvar) = &*worker_signal;
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                state.done = true;
                cvar.notify_all();
                break;
            }
            match pipe.read(&mut scratch) {
                Ok(0) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
                Ok(n) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    let remaining = max_bytes.saturating_sub(state.buf.len());
                    if n > remaining {
                        state.buf.extend_from_slice(&scratch[..remaining]);
                        exceeded.store(true, Ordering::Release);
                        state.done = true;
                        cvar.notify_all();
                        drop(state);
                        drop(pipe);
                        break;
                    }
                    state.buf.extend_from_slice(&scratch[..n]);
                }
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => {
                    let (lock, cvar) = &*worker_signal;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.err = Some(err);
                    state.done = true;
                    cvar.notify_all();
                    break;
                }
            }
        }
    });
    PipeDrainHandle {
        signal,
        join: Some(join),
    }
}

fn wait_with_optional_timeout(
    inner: &ChildInner,
    timeout_override: Option<Duration>,
) -> Result<(CommandStatus, bool), ExecError> {
    let deadline = timeout_override
        .or(inner.timeout)
        .and_then(|d| Instant::now().checked_add(d));
    let mut sleep_dur = Duration::from_millis(1);
    let mut timed_out = false;
    let mut escalated = false;
    let mut escalate_at: Option<Instant> = None;

    loop {
        if inner.disposed.load(Ordering::Acquire) {
            return Err(ExecError::InvalidArgument {
                reason: "child process handle is disposed",
            });
        }
        {
            let mut guard = inner
                .child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let Some(ref mut child) = *guard else {
                return Err(ExecError::InvalidArgument {
                    reason: "child process handle is disposed",
                });
            };
            if child.try_wait()?.is_some() {
                let mut status = child.wait()?;
                if timed_out {
                    status.timed_out = true;
                    status.success = false;
                }
                return Ok((status, timed_out));
            }
            let now = Instant::now();
            if !timed_out
                && let Some(dl) = deadline
                && now >= dl
            {
                timed_out = true;
                let _ = child.kill(inner.kill_signal);
                if inner.kill_signal == Signal::SigKill || inner.kill_grace.is_zero() {
                    if inner.kill_signal != Signal::SigKill {
                        let _ = child.kill(Signal::SigKill);
                    }
                    escalated = true;
                } else {
                    escalate_at = now.checked_add(inner.kill_grace);
                }
            }
            if timed_out
                && !escalated
                && let Some(ea) = escalate_at
                && now >= ea
            {
                let _ = child.kill(Signal::SigKill);
                escalated = true;
            }
        }
        std::thread::sleep(sleep_dur);
        sleep_dur = (sleep_dur * 2).min(Duration::from_millis(10));
    }
}

#[derive(Default)]
struct ChildOutputOptions {
    max_output_bytes: Option<usize>,
    timeout: Option<Duration>,
}

struct ChildOutputOptionsArg(Result<ChildOutputOptions, ExecBindingError>);

impl<'a> FromV8<'a> for ChildOutputOptionsArg {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        if value.is_undefined() {
            return Ok(Self(Ok(ChildOutputOptions::default())));
        }
        let mut parsed = ChildOutputOptions::default();
        let res = visit_plain_object_own(
            scope,
            value,
            "waitWithOutput options must be a plain object",
            |scope, key, val| match key {
                "maxOutputBytes" => {
                    if !val.is_undefined() {
                        let n = parse_max_output_bytes(scope, val)?;
                        CommandSpec::new("p")?.max_output_bytes(n)?;
                        parsed.max_output_bytes = Some(n);
                    }
                    Ok(())
                }
                "timeout" | "timeoutMs" => {
                    if !val.is_undefined() {
                        let d = parse_positive_duration_ms(
                            scope,
                            val,
                            "timeout must be a number",
                            "timeout must be greater than zero",
                        )?;
                        CommandSpec::new("p")?.timeout(d)?;
                        parsed.timeout = Some(d);
                    }
                    Ok(())
                }
                _ => Err(ExecBindingError::invalid_arg(
                    "unknown waitWithOutput option",
                )),
            },
        );
        Ok(Self(res.map(|()| parsed)))
    }
}

#[op2]
async fn op_vael_exec_child_output(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[scoped] opts: ChildOutputOptionsArg,
) -> ExecOpOutcome {
    let opts = match opts.0 {
        Ok(o) => o,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let inner = match get_child_inner(&state, rid) {
        Ok(i) => i,
        Err(err) => return ExecOpOutcome::Err(err),
    };

    inner.stdin_closed.store(true, Ordering::Release);
    if let Ok(mut stdin_guard) = inner.stdin.try_lock() {
        drop(stdin_guard.take());
    }
    let stdout_pipe = if let Ok(mut g) = inner.stdout.try_lock() {
        g.take()
    } else {
        None
    };
    let stderr_pipe = if let Ok(mut g) = inner.stderr.try_lock() {
        g.take()
    } else {
        None
    };
    if inner.disposed.load(Ordering::Acquire) {
        return ExecOpOutcome::Err(ExecBindingError::invalid_arg(
            "child process handle is disposed",
        ));
    }

    let overall_deadline = opts
        .timeout
        .or(inner.timeout)
        .and_then(|d| Instant::now().checked_add(d));

    let join = match spawn_blocking_work(&state, move || {
        let max_bytes = opts.max_output_bytes.unwrap_or(inner.max_output_bytes);
        let out_exceeded = Arc::new(AtomicBool::new(false));
        let err_exceeded = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));

        #[cfg(unix)]
        let out_handle = stdout_pipe.map(|p| {
            spawn_unix_pipe_drain(p, max_bytes, Arc::clone(&out_exceeded), Arc::clone(&cancel))
        });
        #[cfg(not(unix))]
        let out_handle = stdout_pipe.map(|p| {
            spawn_generic_pipe_drain(p, max_bytes, Arc::clone(&out_exceeded), Arc::clone(&cancel))
        });

        #[cfg(unix)]
        let err_handle = stderr_pipe.map(|p| {
            spawn_unix_pipe_drain(p, max_bytes, Arc::clone(&err_exceeded), Arc::clone(&cancel))
        });
        #[cfg(not(unix))]
        let err_handle = stderr_pipe.map(|p| {
            spawn_generic_pipe_drain(p, max_bytes, Arc::clone(&err_exceeded), Arc::clone(&cancel))
        });

        let wait_res = wait_with_optional_timeout(&inner, opts.timeout);
        let timed_out = wait_res.as_ref().is_ok_and(|(_, t)| *t);
        let drain_deadline = if timed_out || wait_res.is_err() {
            Instant::now().checked_add(Duration::from_millis(75))
        } else {
            overall_deadline
        };

        let (stdout, out_err) = out_handle
            .as_ref()
            .map_or((Vec::new(), None), |h| h.collect(drain_deadline));
        let (stderr, err_err) = err_handle
            .as_ref()
            .map_or((Vec::new(), None), |h| h.collect(drain_deadline));

        if drain_deadline.is_some() {
            cancel.store(true, Ordering::Release);
        }
        if let Some(h) = out_handle {
            h.finish(drain_deadline.is_some());
        }
        if let Some(h) = err_handle {
            h.finish(drain_deadline.is_some());
        }

        let (status, timed_out) = wait_res?;

        if out_exceeded.load(Ordering::Acquire) {
            return Err(ExecError::OutputTooLarge {
                limit: max_bytes,
                stream: "stdout",
            });
        }
        if err_exceeded.load(Ordering::Acquire) {
            return Err(ExecError::OutputTooLarge {
                limit: max_bytes,
                stream: "stderr",
            });
        }
        if let Some(e) = out_err.or(err_err)
            && !timed_out
        {
            return Err(ExecError::Io {
                context: "read child process output",
                source: e,
            });
        }

        Ok(CommandOutput {
            code: status.code,
            signal: status.signal,
            success: status.success,
            stdout,
            stderr,
            timed_out,
            truncated: false,
        })
    }) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };

    match join.await {
        Ok(Ok(out)) => ExecOpOutcome::Output(out),
        Ok(Err(err)) => ExecOpOutcome::Err(ExecBindingError::from(err)),
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "wait_with_output child process",
            source: io::Error::other("child output task panicked or was cancelled"),
        })),
    }
}

#[op2]
async fn op_vael_exec_child_dispose(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
) -> ExecOpOutcome {
    let resource = {
        let mut borrowed = state.borrow_mut();
        borrowed.resource_table.take::<ChildProcessResource>(rid)
    };
    let Ok(resource) = resource else {
        return ExecOpOutcome::Unit;
    };
    resource.inner.disposed.store(true, Ordering::Release);
    resource.inner.stdin_closed.store(true, Ordering::Release);
    let inner = Arc::clone(&resource.inner);
    if let Ok(join) = spawn_blocking_work(&state, move || {
        let mut guard = inner
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(mut child) = guard.take() {
            let _ = child.kill(Signal::SigKill);
            let _ = child.wait();
        }
    }) {
        let _ = join.await;
    }
    if let Ok(mut g) = resource.inner.stdin.try_lock() {
        drop(g.take());
    }
    if let Ok(mut g) = resource.inner.stdout.try_lock() {
        drop(g.take());
    }
    if let Ok(mut g) = resource.inner.stderr.try_lock() {
        drop(g.take());
    }
    // `resource` drops here and decrements `open_children` (child already taken & reaped).
    drop(resource);
    ExecOpOutcome::Unit
}

// ---------------------------------------------------------------------------
// which / whichAll ops (sync + async)
// ---------------------------------------------------------------------------

#[op2]
fn op_vael_exec_which_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name_val: v8::Local<'s, v8::Value>,
    opts_val: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = (|| -> Result<Option<PathBuf>, ExecBindingError> {
        let name = copy_strict_string(scope, name_val, "executable name must be a string")?;
        let prepend = parse_which_prepend(scope, opts_val)?;
        which(&name, prepend.as_deref()).map_err(ExecBindingError::from)
    })();
    match res {
        Ok(Some(p)) => {
            set_bool(scope, obj, "ok", true);
            set_string(scope, obj, "value", &p.to_string_lossy());
        }
        Ok(None) => {
            set_bool(scope, obj, "ok", true);
            set(scope, obj, "value", v8::null(scope).into());
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
async fn op_vael_exec_which_async(
    state: Rc<RefCell<OpState>>,
    #[scoped] name: WhichArgs,
    #[scoped] prepend: WhichPrependArg,
) -> ExecOpOutcome {
    let name = match name.name {
        Ok(n) => n,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let prepend = match prepend.0 {
        Ok(p) => p,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let join = match spawn_blocking_work(&state, move || which(&name, prepend.as_deref())) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    match join.await {
        Ok(Ok(opt)) => ExecOpOutcome::WhichOne(opt.map(|p| p.to_string_lossy().into_owned())),
        Ok(Err(err)) => ExecOpOutcome::Err(ExecBindingError::from(err)),
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "which lookup",
            source: io::Error::other("which task panicked or was cancelled"),
        })),
    }
}

#[op2]
fn op_vael_exec_which_all_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name_val: v8::Local<'s, v8::Value>,
    opts_val: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let res = (|| -> Result<Vec<PathBuf>, ExecBindingError> {
        let name = copy_strict_string(scope, name_val, "executable name must be a string")?;
        let prepend = parse_which_prepend(scope, opts_val)?;
        which_all(&name, prepend.as_deref()).map_err(ExecBindingError::from)
    })();
    match res {
        Ok(paths) => {
            set_bool(scope, obj, "ok", true);
            let items: Vec<v8::Local<'s, v8::Value>> = paths
                .iter()
                .map(|p| {
                    v8::String::new(scope, &p.to_string_lossy())
                        .map(Into::into)
                        .unwrap_or_else(|| v8::String::empty(scope).into())
                })
                .collect();
            let arr = v8::Array::new_with_elements(scope, &items);
            set(scope, obj, "value", arr.into());
        }
        Err(err) => set_failure(scope, obj, &err),
    }
    obj.into()
}

#[op2]
async fn op_vael_exec_which_all_async(
    state: Rc<RefCell<OpState>>,
    #[scoped] name: WhichArgs,
    #[scoped] prepend: WhichPrependArg,
) -> ExecOpOutcome {
    let name = match name.name {
        Ok(n) => n,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let prepend = match prepend.0 {
        Ok(p) => p,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let join = match spawn_blocking_work(&state, move || which_all(&name, prepend.as_deref())) {
        Ok(h) => h,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    match join.await {
        Ok(Ok(paths)) => ExecOpOutcome::WhichAll(
            paths
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        ),
        Ok(Err(err)) => ExecOpOutcome::Err(ExecBindingError::from(err)),
        Err(_) => ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
            context: "whichAll lookup",
            source: io::Error::other("whichAll task panicked or was cancelled"),
        })),
    }
}

// ---------------------------------------------------------------------------
// FileLock ops (sync + async + unlock)
// ---------------------------------------------------------------------------

const fn lock_mode_str(mode: LockMode) -> &'static str {
    match mode {
        LockMode::Exclusive => "exclusive",
        LockMode::Shared => "shared",
    }
}

#[op2]
fn op_vael_exec_lock_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    path_val: v8::Local<'s, v8::Value>,
    opts_val: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let (path, opts) = match (
        copy_strict_string(scope, path_val, "lock path must be a string"),
        parse_lock_options(scope, opts_val),
    ) {
        (Ok(p), Ok(o)) => (PathBuf::from(p), o),
        (Err(e), _) | (_, Err(e)) => {
            set_failure(scope, obj, &e);
            return obj.into();
        }
    };

    let counts = state
        .try_borrow::<ExecState>()
        .map(|s| Arc::clone(&s.counts))
        .unwrap_or_default();
    let prev = counts.open_locks.fetch_add(1, Ordering::Relaxed);
    if prev >= MAX_OPEN_LOCKS {
        counts.open_locks.fetch_sub(1, Ordering::Relaxed);
        set_failure(
            scope,
            obj,
            &ExecBindingError::invalid_arg("maximum number of open FileLock handles exceeded"),
        );
        return obj.into();
    }

    match FileLock::acquire(&path, &opts) {
        Ok(lock) => {
            let mode = lock_mode_str(lock.mode());
            let rid = state.resource_table.add(FileLockResource {
                lock: RefCell::new(Some(lock)),
                counts,
            });
            set_bool(scope, obj, "ok", true);
            set_number(scope, obj, "id", f64::from(rid));
            set_string(scope, obj, "mode", mode);
        }
        Err(err) => {
            counts.open_locks.fetch_sub(1, Ordering::Relaxed);
            set_failure(scope, obj, &ExecBindingError::from(err));
        }
    }
    obj.into()
}

#[op2]
async fn op_vael_exec_lock_async(
    state: Rc<RefCell<OpState>>,
    #[scoped] path: LockPathArg,
    #[scoped] opts: LockOptsArg,
) -> ExecOpOutcome {
    let path = match path.0 {
        Ok(p) => p,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let opts = match opts.0 {
        Ok(o) => o,
        Err(err) => return ExecOpOutcome::Err(err),
    };
    let counts = state
        .borrow()
        .try_borrow::<ExecState>()
        .map(|s| Arc::clone(&s.counts))
        .unwrap_or_default();
    let prev = counts.open_locks.fetch_add(1, Ordering::Relaxed);
    if prev >= MAX_OPEN_LOCKS {
        counts.open_locks.fetch_sub(1, Ordering::Relaxed);
        return ExecOpOutcome::Err(ExecBindingError::invalid_arg(
            "maximum number of open FileLock handles exceeded",
        ));
    }

    let join = match spawn_blocking_work(&state, move || FileLock::acquire(&path, &opts)) {
        Ok(h) => h,
        Err(err) => {
            counts.open_locks.fetch_sub(1, Ordering::Relaxed);
            return ExecOpOutcome::Err(err);
        }
    };

    match join.await {
        Ok(Ok(lock)) => {
            let mode = lock_mode_str(lock.mode());
            let rid = state.borrow_mut().resource_table.add(FileLockResource {
                lock: RefCell::new(Some(lock)),
                counts,
            });
            ExecOpOutcome::LockCreated { rid, mode }
        }
        Ok(Err(err)) => {
            counts.open_locks.fetch_sub(1, Ordering::Relaxed);
            ExecOpOutcome::Err(ExecBindingError::from(err))
        }
        Err(_) => {
            counts.open_locks.fetch_sub(1, Ordering::Relaxed);
            ExecOpOutcome::Err(ExecBindingError::from(ExecError::Io {
                context: "acquire file lock",
                source: io::Error::other("lock task panicked or was cancelled"),
            }))
        }
    }
}

#[op2]
fn op_vael_exec_lock_unlock<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &mut OpState,
    #[smi] rid: ResourceId,
) -> v8::Local<'s, v8::Value> {
    let obj = response_obj(scope);
    let Ok(resource) = state.resource_table.take::<FileLockResource>(rid) else {
        set_bool(scope, obj, "ok", true);
        return obj.into();
    };
    let taken = resource.lock.borrow_mut().take();
    if let Some(lock) = taken
        && let Err(err) = lock.unlock()
    {
        set_failure(scope, obj, &ExecBindingError::from(err));
        return obj.into();
    }
    set_bool(scope, obj, "ok", true);
    obj.into()
}

deno_core::extension!(
    vaeljs_exec,
    ops = [
        op_vael_exec_validate_program,
        op_vael_exec_validate_arg,
        op_vael_exec_validate_args,
        op_vael_exec_validate_cwd,
        op_vael_exec_validate_env_entry,
        op_vael_exec_validate_envs,
        op_vael_exec_validate_env_remove,
        op_vael_exec_validate_stdin,
        op_vael_exec_validate_stdio,
        op_vael_exec_validate_timeout,
        op_vael_exec_validate_max_output,
        op_vael_exec_validate_signal,
        op_vael_exec_validate_kill_grace,
        op_vael_exec_parse_command_options,
        op_vael_exec_output_sync,
        op_vael_exec_output_async,
        op_vael_exec_status_sync,
        op_vael_exec_status_async,
        op_vael_exec_decode_utf8_lossy,
        op_vael_exec_spawn,
        op_vael_exec_child_write_stdin,
        op_vael_exec_child_close_stdin,
        op_vael_exec_child_read_stdout,
        op_vael_exec_child_read_stderr,
        op_vael_exec_child_kill,
        op_vael_exec_child_try_wait,
        op_vael_exec_child_wait,
        op_vael_exec_child_output,
        op_vael_exec_child_dispose,
        op_vael_exec_which_sync,
        op_vael_exec_which_async,
        op_vael_exec_which_all_sync,
        op_vael_exec_which_all_async,
        op_vael_exec_lock_sync,
        op_vael_exec_lock_async,
        op_vael_exec_lock_unlock,
    ],
    state = |state| {
        state.put(ExecState::capture());
    },
    docs = "Argv-only process execution, PATH lookup, and advisory file locking ops behind `Vael.getBuiltInModule(\"vael:exec\")`."
);

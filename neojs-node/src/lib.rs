#![deny(unsafe_code)]
//! V8 adapter for opt-in Node compatibility.
//!
//! The runtime creates this adapter only from
//! `neojs_core::Runtime::with_node_compat`. It registers one native synthetic
//! ECMAScript module named `node:child_process` through
//! `JsRuntime::register_native_module`. The named exports are native V8
//! function objects; there is no JavaScript implementation, module facade, JSON
//! bridge, or host op.
//!
//! The synchronous functions call the shared, engine-neutral
//! `neojs-node-builtins` runner directly. The runner enforces the executable
//! allowlist, an empty inherited environment, and finite time, input, output,
//! argument, and environment limits. The native functions validate and copy
//! guest values before handing them to Rust; a getter or Proxy value is read at
//! most once and never coerced.
//!
//! The module is registered in the normal module map before user code runs, so
//! static imports, dynamic imports, and repeated imports observe one module
//! instance and the same function objects. It is not added to the `Neo`
//! registry, has no bare alias, and is absent from a default runtime.

use std::collections::BTreeMap;
use std::rc::Rc;

use deno_core::FastStaticString;
use deno_core::JsRuntime;
use deno_core::v8;

pub use neojs_node_builtins::{
    CHILD_PROCESS_SPECIFIER, ChildProcessError, ChildProcessPolicy, Limits, NodeOptions,
    PolicyError, SUPPORTED_MODULES, SyncRequest, SyncResult, is_known_node_bare_name, run_sync,
    unsupported_module_reason,
};

const MAX_INPUT_UNITS: usize = 1_048_576;
const MAX_ARGS: usize = 256;
const MAX_ARG_UNITS: usize = 65_536;
const MAX_ENV_ENTRIES: usize = 128;
const MAX_ENV_BYTES: usize = 65_536;
const MAX_PATH_UNITS: usize = 4_096;
const MAX_OPTION_KEYS: usize = 6;
const MAX_OPTION_SCAN: u32 = 64;
const MAX_ENV_SCAN: u32 = 4_096;
const MAX_OPTION_KEY_UNITS: u32 = MAX_PATH_UNITS as u32;

/// Status values shared with the bounded V8 C ABI.
#[derive(Debug, PartialEq, Eq)]
enum BoundedKeysError {
    TooManyKeys,
    KeyTooLong,
    ScanLimit,
    Unsupported,
    Memory,
    Exception,
}

#[allow(unsafe_code)]
unsafe extern "C" {
    /// Narrow bounded own-enumerable-string-key collector added by the
    /// vendored V8 source patch. `out_array` is a V8 `Array*` handle.
    fn neojs_v8_get_own_enumerable_string_keys_bounded(
        object: *const v8::Object,
        context: *const v8::Context,
        max_keys: u32,
        max_scan: u32,
        out_array: *mut *const v8::Array,
        out_len: *mut u32,
        out_scanned: *mut u32,
    ) -> i32;
}

const _: () = assert!(
    std::mem::size_of::<v8::Local<'static, v8::Array>>() == std::mem::size_of::<*const v8::Array>()
);
const _: () = assert!(
    std::mem::align_of::<v8::Local<'static, v8::Array>>()
        == std::mem::align_of::<*const v8::Array>()
);

/// Reconstructs the V8 local handle returned by the C ABI.
#[allow(unsafe_code)]
fn local_array_from_raw<'s>(ptr: *const v8::Array) -> Option<v8::Local<'s, v8::Array>> {
    if ptr.is_null() {
        return None;
    }
    // The v8 crate uses the same pointer-sized representation for Local in its
    // own C++ bridge (`support.h`). The const assertions above pin the layout.
    Some(unsafe { std::mem::transmute_copy(&ptr) })
}

/// Collects own enumerable string keys through the bounded V8 engine API.
#[allow(unsafe_code)]
fn bounded_keys<'js>(
    scope: &v8::PinScope<'js, '_>,
    object: &v8::Local<'js, v8::Object>,
    max_keys: u32,
    max_scan: u32,
    max_key_units: usize,
) -> Result<Vec<String>, BoundedKeysError> {
    let context = scope.get_current_context();
    let mut array: *const v8::Array = std::ptr::null();
    let mut len = 0u32;
    let mut scanned = 0u32;
    let status = unsafe {
        neojs_v8_get_own_enumerable_string_keys_bounded(
            &**object as *const v8::Object,
            &*context as *const v8::Context,
            max_keys,
            max_scan,
            &mut array,
            &mut len,
            &mut scanned,
        )
    };
    match status {
        0 => {}
        1 => return Err(BoundedKeysError::TooManyKeys),
        2 => return Err(BoundedKeysError::ScanLimit),
        3 => return Err(BoundedKeysError::Unsupported),
        4 => return Err(BoundedKeysError::Memory),
        _ => return Err(BoundedKeysError::Exception),
    }
    let Some(array) = local_array_from_raw(array) else {
        return Err(BoundedKeysError::Exception);
    };
    if array.length() != len || len as usize > max_keys as usize {
        return Err(BoundedKeysError::Exception);
    }
    let mut keys = Vec::with_capacity(len as usize);
    for index in 0..len {
        let value = array
            .get_index(scope, index)
            .ok_or(BoundedKeysError::Exception)?;
        let key =
            v8::Local::<v8::String>::try_from(value).map_err(|_| BoundedKeysError::Exception)?;
        if key.length() > max_key_units {
            return Err(BoundedKeysError::KeyTooLong);
        }
        keys.push(key.to_rust_string_lossy(scope));
    }
    Ok(keys)
}

/// Installs the native `node:child_process` module and its process policy.
///
/// Registration happens before any user module can import the specifier. The
/// module has no default side effects and no global object.
pub fn install(js: &mut JsRuntime, options: NodeOptions) -> Result<(), String> {
    let context = js.main_context();
    let exports = {
        v8::scope!(let scope, js.v8_isolate());
        let context_local = v8::Local::new(scope, context.clone());
        let scope = &mut v8::ContextScope::new(scope, context_local);
        build_exports(scope)?
    };
    js.register_native_module(CHILD_PROCESS_SPECIFIER, exports)
        .map_err(|error| error.to_string())?;
    // Install the policy only after the module registration is accepted. A
    // rejected duplicate must not replace the authority of the existing module.
    v8::scope!(let scope, js.v8_isolate());
    let context_local = v8::Local::new(scope, context);
    context_local.set_slot(Rc::new(options));
    Ok(())
}

fn build_exports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<Vec<(FastStaticString, v8::Global<v8::Value>)>, String> {
    let spawn = native_function(scope, spawn, "spawn")?;
    let spawn_sync = native_function(scope, spawn_sync, "spawnSync")?;
    let exec = native_function(scope, exec, "exec")?;
    let exec_file = native_function(scope, exec_file, "execFile")?;
    let exec_file_sync = native_function(scope, exec_file_sync, "execFileSync")?;
    let exec_sync = native_function(scope, exec_sync, "execSync")?;
    let fork = native_function(scope, fork, "fork")?;
    let child_process = native_function(scope, child_process, "ChildProcess")?;

    let api = v8::Object::new(scope);
    for (name, function) in [
        ("spawn", &spawn),
        ("spawnSync", &spawn_sync),
        ("exec", &exec),
        ("execFile", &exec_file),
        ("execFileSync", &exec_file_sync),
        ("execSync", &exec_sync),
        ("fork", &fork),
        ("ChildProcess", &child_process),
    ] {
        if !define_value(scope, &api, name, (*function).into()) {
            return Err("cannot define a native child_process export".to_string());
        }
    }
    if api.set_integrity_level(scope, v8::IntegrityLevel::Frozen) != Some(true) {
        return Err("cannot freeze the native child_process object".to_string());
    }

    let mut exports = Vec::with_capacity(9);
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("spawn"),
        spawn.into(),
    );
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("spawnSync"),
        spawn_sync.into(),
    );
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("exec"),
        exec.into(),
    );
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("execFile"),
        exec_file.into(),
    );
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("execFileSync"),
        exec_file_sync.into(),
    );
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("execSync"),
        exec_sync.into(),
    );
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("fork"),
        fork.into(),
    );
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("ChildProcess"),
        child_process.into(),
    );
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("default"),
        api.into(),
    );
    Ok(exports)
}

fn native_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    name: &str,
) -> Result<v8::Local<'s, v8::Function>, String> {
    v8::Function::new(scope, callback)
        .ok_or_else(|| format!("cannot allocate the native {name} function"))
}

fn push_export<'s>(
    scope: &v8::PinScope<'s, '_>,
    exports: &mut Vec<(FastStaticString, v8::Global<v8::Value>)>,
    name: FastStaticString,
    value: v8::Local<'s, v8::Value>,
) {
    exports.push((name, v8::Global::new(scope, value)));
}

/// Defines an own data property without consulting the prototype chain.
///
/// `Object::set` would invoke an inherited setter. Native results and errors
/// are built from fresh objects, so `CreateDataProperty` is both the correct
/// JavaScript semantics and immune to a hostile `Object.prototype` or
/// `Array.prototype`.
fn define_value<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    let Some(key) = v8::String::new(scope, name) else {
        return false;
    };
    match object.create_data_property(scope, key.into(), value) {
        Some(true) => true,
        Some(false) => {
            if let Some(message) = v8::String::new(scope, "cannot define a native property") {
                scope.throw_exception(v8::Exception::error(scope, message));
            }
            false
        }
        // An empty result carries a pending V8 exception. Leave it in place.
        None => false,
    }
}

fn get<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, Failure> {
    let key = v8::String::new(scope, name).ok_or(Failure::Pending)?;
    object.get(scope, key.into()).ok_or(Failure::Pending)
}

/// A failure raised by guest validation or the shared runner.
enum Failure {
    /// A coded error this adapter should construct and throw.
    Coded { code: String, message: String },
    /// V8 already has a pending exception (a guest getter threw, or allocation
    /// failed). Return without replacing it.
    Pending,
}

impl Failure {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Coded {
            code: code.into(),
            message: message.into(),
        }
    }

    fn invalid_type(message: impl Into<String>) -> Self {
        Self::new("ERR_INVALID_ARG_TYPE", message)
    }

    fn invalid_value(message: impl Into<String>) -> Self {
        Self::new("ERR_INVALID_ARG_VALUE", message)
    }

    fn unsupported(message: impl Into<String>) -> Self {
        Self::new("ERR_NODE_UNSUPPORTED", message)
    }

    fn unsupported_encoding() -> Self {
        Self::new(
            "ERR_NODE_UNSUPPORTED_ENCODING",
            "node:child_process requires encoding: \"utf8\" in this opt-in slice",
        )
    }

    fn too_large() -> Self {
        Self::new(
            "ERR_NODE_REQUEST_TOO_LARGE",
            "the node:child_process request exceeds the hard size limit",
        )
    }
}

fn throw_failure(scope: &v8::PinScope<'_, '_>, failure: Failure) {
    match failure {
        Failure::Coded { code, message } => throw_coded(scope, &code, &message),
        Failure::Pending => {}
    }
}

fn coded_error<'s>(
    scope: &v8::PinScope<'s, '_>,
    code: &str,
    message: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let message = v8::String::new(scope, message)?;
    let exception = v8::Exception::error(scope, message);
    let object = v8::Local::<v8::Object>::try_from(exception).ok()?;
    let code = string_value(scope, code);
    if !define_value(scope, &object, "code", code) {
        return None;
    }
    Some(object)
}

fn throw_coded(scope: &v8::PinScope<'_, '_>, code: &str, message: &str) {
    if let Some(error) = coded_error(scope, code, message) {
        scope.throw_exception(error.into());
    }
}

fn string_value<'s>(scope: &v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
    v8::String::new(scope, text)
        .map(v8::Local::into)
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn null_or_number<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: Option<i32>,
) -> v8::Local<'s, v8::Value> {
    match value {
        Some(value) => v8::Number::new(scope, f64::from(value)).into(),
        None => v8::null(scope).into(),
    }
}

fn null_or_string<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: Option<&str>,
) -> v8::Local<'s, v8::Value> {
    match value {
        Some(value) => string_value(scope, value),
        None => v8::null(scope).into(),
    }
}

fn output_array<'s>(
    scope: &v8::PinScope<'s, '_>,
    stdout: &str,
    stderr: &str,
) -> Option<v8::Local<'s, v8::Array>> {
    let output = v8::Array::new(scope, 3);
    let object: v8::Local<'s, v8::Object> = output.into();
    let null: v8::Local<'s, v8::Value> = v8::null(scope).into();
    if !define_value(scope, &object, "0", null)
        || !define_value(scope, &object, "1", string_value(scope, stdout))
        || !define_value(scope, &object, "2", string_value(scope, stderr))
    {
        return None;
    }
    Some(output)
}

fn attach_result_fields<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
    result: &SyncResult,
) -> bool {
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    let Some(output) = output_array(scope, &stdout, &stderr) else {
        return false;
    };
    let pid = v8::Number::new(scope, f64::from(result.pid));
    let stdout_value = string_value(scope, &stdout);
    let stderr_value = string_value(scope, &stderr);
    let status = null_or_number(scope, result.status);
    let signal = null_or_string(scope, result.signal_name);
    define_value(scope, object, "pid", pid.into())
        && define_value(scope, object, "output", output.into())
        && define_value(scope, object, "stdout", stdout_value)
        && define_value(scope, object, "stderr", stderr_value)
        && define_value(scope, object, "status", status)
        && define_value(scope, object, "signal", signal)
}

fn spawn_result<'s>(
    scope: &v8::PinScope<'s, '_>,
    result: &SyncResult,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    if !attach_result_fields(scope, &object, result) {
        return None;
    }
    Some(object)
}

fn empty_spawn_result<'s>(
    scope: &v8::PinScope<'s, '_>,
    code: &str,
    message: &str,
) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    let output = output_array(scope, "", "")?;
    let error = coded_error(scope, code, message)?;
    if !define_value(scope, &object, "pid", v8::Number::new(scope, 0.0).into())
        || !define_value(scope, &object, "output", output.into())
        || !define_value(scope, &object, "stdout", string_value(scope, ""))
        || !define_value(scope, &object, "stderr", string_value(scope, ""))
        || !define_value(scope, &object, "status", v8::null(scope).into())
        || !define_value(scope, &object, "signal", v8::null(scope).into())
        || !define_value(scope, &object, "error", error.into())
    {
        return None;
    }
    Some(object)
}

fn throw_with_fields(scope: &v8::PinScope<'_, '_>, code: &str, message: &str, result: &SyncResult) {
    let Some(error) = coded_error(scope, code, message) else {
        return;
    };
    if !attach_result_fields(scope, &error, result) {
        return;
    }
    scope.throw_exception(error.into());
}

fn read_string<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    what: &str,
    max_units: usize,
) -> Result<String, Failure> {
    if !value.is_string() {
        return Err(Failure::invalid_type(format!("{what} must be a string")));
    }
    let string = v8::Local::<v8::String>::try_from(value)
        .map_err(|_| Failure::invalid_type(format!("{what} must be a string")))?;
    if string.length() > max_units {
        return Err(Failure::too_large());
    }
    Ok(string.to_rust_string_lossy(scope))
}

struct NormalizedOptions {
    cwd: Option<String>,
    env: Option<BTreeMap<String, String>>,
    input: Option<String>,
    timeout_ms: Option<u64>,
    max_buffer: Option<usize>,
}

fn read_utf8_options<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<NormalizedOptions, Failure> {
    if value.is_undefined() {
        return Err(Failure::unsupported_encoding());
    }
    if value.is_null() || !value.is_object() || value.is_array() || value.is_function() {
        return Err(Failure::invalid_type("options must be an object"));
    }
    if value.is_proxy() {
        return Err(Failure::unsupported(
            "Proxy options are not supported by the native child_process module",
        ));
    }
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| Failure::invalid_type("options must be an object"))?;

    reject_unknown_options(scope, &object)?;

    // Snapshot every option exactly once; a getter must not be read again.
    let encoding = get(scope, &object, "encoding")?;
    let cwd = get(scope, &object, "cwd")?;
    let env = get(scope, &object, "env")?;
    let input = get(scope, &object, "input")?;
    let timeout = get(scope, &object, "timeout")?;
    let max_buffer = get(scope, &object, "maxBuffer")?;

    if !encoding.is_string() {
        return Err(Failure::unsupported_encoding());
    }
    let encoding = read_string(scope, encoding, "options.encoding", MAX_PATH_UNITS)
        .map_err(|_| Failure::unsupported_encoding())?;
    if !normalized_encoding_is_utf8(&encoding) {
        return Err(Failure::unsupported_encoding());
    }

    let cwd = if cwd.is_undefined() {
        None
    } else {
        let cwd = read_string(scope, cwd, "options.cwd", MAX_PATH_UNITS)?;
        if cwd.contains('\0') {
            return Err(Failure::invalid_value(
                "options.cwd must not contain a NUL byte",
            ));
        }
        Some(cwd)
    };

    let env = if env.is_undefined() {
        None
    } else {
        Some(read_env(scope, env)?)
    };

    let input = if input.is_undefined() {
        None
    } else {
        if !input.is_string() {
            return Err(Failure::unsupported(
                "node:child_process input must be a string in this opt-in slice",
            ));
        }
        Some(read_string(scope, input, "options.input", MAX_INPUT_UNITS)?)
    };

    let timeout = if timeout.is_undefined() {
        None
    } else {
        if !timeout.is_number() {
            return Err(Failure::invalid_value(
                "options.timeout must be a non-negative integer",
            ));
        }
        let number = timeout
            .number_value(scope)
            .ok_or_else(|| Failure::invalid_value("options.timeout is not a valid integer"))?;
        if !number.is_finite() || number.fract() != 0.0 || number < 0.0 || number > u64::MAX as f64
        {
            return Err(Failure::invalid_value(
                "options.timeout must be a non-negative integer",
            ));
        }
        Some(number as u64)
    };

    let max_buffer = if max_buffer.is_undefined() {
        None
    } else {
        if !max_buffer.is_number() {
            return Err(Failure::invalid_value(
                "options.maxBuffer must be a positive integer",
            ));
        }
        let number = max_buffer
            .number_value(scope)
            .ok_or_else(|| Failure::invalid_value("options.maxBuffer is not a valid integer"))?;
        if !number.is_finite()
            || number.fract() != 0.0
            || number <= 0.0
            || number > usize::MAX as f64
        {
            return Err(Failure::invalid_value(
                "options.maxBuffer must be a positive integer",
            ));
        }
        Some(number as usize)
    };

    Ok(NormalizedOptions {
        cwd,
        env,
        input,
        timeout_ms: timeout,
        max_buffer,
    })
}

fn reject_unknown_options<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
) -> Result<(), Failure> {
    let keys = bounded_keys(
        scope,
        object,
        MAX_OPTION_KEYS as u32,
        MAX_OPTION_SCAN,
        MAX_OPTION_KEY_UNITS as usize,
    )
    .map_err(|error| match error {
        BoundedKeysError::TooManyKeys
        | BoundedKeysError::ScanLimit
        | BoundedKeysError::Unsupported => Failure::unsupported("an option is not implemented"),
        BoundedKeysError::KeyTooLong => Failure::too_large(),
        BoundedKeysError::Memory | BoundedKeysError::Exception => Failure::Pending,
    })?;
    for key in keys {
        if !matches!(
            key.as_str(),
            "cwd" | "env" | "input" | "timeout" | "maxBuffer" | "encoding"
        ) {
            return Err(Failure::unsupported("an option is not implemented"));
        }
    }
    Ok(())
}

fn read_env<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<BTreeMap<String, String>, Failure> {
    if value.is_null() || !value.is_object() || value.is_array() || value.is_function() {
        return Err(Failure::invalid_type("options.env must be an object"));
    }
    if value.is_proxy() {
        return Err(Failure::unsupported(
            "Proxy options.env is not supported by the native child_process module",
        ));
    }
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| Failure::invalid_type("options.env must be an object"))?;
    let names = bounded_keys(
        scope,
        &object,
        MAX_ENV_ENTRIES as u32,
        MAX_ENV_SCAN,
        MAX_PATH_UNITS,
    )
    .map_err(|error| match error {
        BoundedKeysError::TooManyKeys
        | BoundedKeysError::KeyTooLong
        | BoundedKeysError::ScanLimit => Failure::too_large(),
        BoundedKeysError::Unsupported => {
            Failure::unsupported("options.env is not supported by the native child_process module")
        }
        BoundedKeysError::Memory | BoundedKeysError::Exception => Failure::Pending,
    })?;

    let mut environment = BTreeMap::new();
    let mut total = 0usize;
    for name in names {
        if name.is_empty() || name.contains('=') || name.contains('\0') {
            return Err(Failure::invalid_value("an environment name is not valid"));
        }
        let key = v8::String::new(scope, &name).ok_or(Failure::Pending)?;
        let value = object.get(scope, key.into()).ok_or(Failure::Pending)?;
        let value = read_string(scope, value, "environment value", MAX_ENV_BYTES)?;
        if value.contains('\0') {
            return Err(Failure::invalid_value(
                "an environment value must not contain a NUL byte",
            ));
        }
        total = total.saturating_add(name.len()).saturating_add(value.len());
        if total > MAX_ENV_BYTES {
            return Err(Failure::too_large());
        }
        environment.insert(name, value);
    }
    Ok(environment)
}

fn normalized_encoding_is_utf8(encoding: &str) -> bool {
    let mut normalized = String::with_capacity(encoding.len());
    let mut removed_hyphen = false;
    for character in encoding.chars() {
        let character = character.to_ascii_lowercase();
        if character == '-' && !removed_hyphen {
            removed_hyphen = true;
            continue;
        }
        normalized.push(character);
    }
    normalized == "utf8"
}

fn read_args<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: Option<v8::Local<'s, v8::Value>>,
    pre_length: Option<v8::Local<'s, v8::Value>>,
) -> Result<Vec<String>, Failure> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| Failure::invalid_type("arguments must be an array"))?;
    let length = match pre_length {
        Some(length) => length,
        None => get(scope, &object, "length")?,
    };
    if !length.is_number() {
        return Err(Failure::invalid_type("arguments must have a valid length"));
    }
    let length = length
        .number_value(scope)
        .ok_or_else(|| Failure::invalid_type("arguments must have a valid length"))?;
    if !length.is_finite() || length.fract() != 0.0 || length < 0.0 {
        return Err(Failure::invalid_type("arguments must have a valid length"));
    }
    if length > MAX_ARGS as f64 {
        return Err(Failure::too_large());
    }
    let length = length as u32;

    let mut arguments = Vec::with_capacity(length as usize);
    let mut total = 0usize;
    for index in 0..length {
        let value = object.get_index(scope, index).ok_or(Failure::Pending)?;
        if value.is_string() {
            let string = v8::Local::<v8::String>::try_from(value)
                .map_err(|_| Failure::invalid_type("an argument is not a valid string"))?;
            if string.length() > MAX_ARG_UNITS {
                return Err(Failure::too_large());
            }
            let text = string.to_rust_string_lossy(scope);
            if text.contains('\0') {
                return Err(Failure::invalid_value(
                    "an argument must not contain a NUL byte",
                ));
            }
            total = total.saturating_add(string.length());
            if total > MAX_ARG_UNITS {
                return Err(Failure::too_large());
            }
            arguments.push(text);
        } else if value.is_number() || value.is_boolean() {
            let string = value.to_string(scope).ok_or(Failure::Pending)?;
            if string.length() > MAX_ARG_UNITS {
                return Err(Failure::too_large());
            }
            total = total.saturating_add(string.length());
            if total > MAX_ARG_UNITS {
                return Err(Failure::too_large());
            }
            arguments.push(string.to_rust_string_lossy(scope));
        } else {
            return Err(Failure::invalid_type(
                "arguments must be strings, numbers, or booleans",
            ));
        }
    }
    Ok(arguments)
}

fn build_request<'s>(
    scope: &v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<SyncRequest, Failure> {
    let file = read_string(scope, args.get(0), "file", MAX_PATH_UNITS)?;
    if file.is_empty() {
        return Err(Failure::invalid_value("file must not be empty"));
    }
    if file.contains('\0') {
        return Err(Failure::invalid_value("file must not contain a NUL byte"));
    }

    let args_value = args.get(1);
    let third = args.get(2);
    let mut pre_length = None;
    let (arguments, options) = if args_value.is_undefined() {
        (None, third)
    } else if args_value.is_array() {
        (Some(args_value), third)
    } else if args_value.is_proxy() {
        // V8's `IsArray` does not unwrap a Proxy. Read the one `length` value
        // this adapter is allowed to read; a numeric length means the Proxy is
        // the argument list, as `Array.isArray` would report for a Proxy over
        // an array. The same value is reused instead of reading it again.
        let object = v8::Local::<v8::Object>::try_from(args_value)
            .map_err(|_| Failure::invalid_type("arguments must be an array"))?;
        let length = get(scope, &object, "length")?;
        if length.is_number() {
            pre_length = Some(length);
            (Some(args_value), third)
        } else {
            (None, args_value)
        }
    } else {
        (None, args_value)
    };
    let arguments = read_args(scope, arguments, pre_length)?;
    let options = read_utf8_options(scope, options)?;

    Ok(SyncRequest {
        file,
        args: arguments,
        cwd: options.cwd,
        env: options.env,
        input: options.input,
        timeout_ms: options.timeout_ms,
        max_buffer: options.max_buffer,
    })
}

fn execute<'s>(
    scope: &v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<SyncResult, Failure> {
    let request = build_request(scope, args)?;
    let options = scope
        .get_current_context()
        .get_slot::<NodeOptions>()
        .ok_or_else(|| {
            Failure::new(
                "ERR_NODE_CHILD_PROCESS",
                "node:child_process is not installed in this runtime",
            )
        })?;
    run_sync(&request, &options).map_err(|error| Failure::new(error.code(), error.message()))
}

fn unsupported_operation(scope: &v8::PinScope<'_, '_>, name: &str) {
    throw_coded(
        scope,
        "ERR_NODE_UNSUPPORTED",
        &format!(
            "node:child_process.{name} is not implemented; use spawnSync or execFileSync with encoding: \"utf8\""
        ),
    );
}

fn spawn<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    unsupported_operation(scope, "spawn");
}

fn exec<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    unsupported_operation(scope, "exec");
}

fn exec_file<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    unsupported_operation(scope, "execFile");
}

fn exec_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    unsupported_operation(scope, "execSync");
}

fn fork<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    unsupported_operation(scope, "fork");
}

fn child_process<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    unsupported_operation(scope, "ChildProcess");
}

fn spawn_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = match execute(scope, &args) {
        Ok(result) => result,
        Err(Failure::Pending) => return,
        Err(Failure::Coded { code, message }) if code == "ENOENT" || code == "EACCES" => {
            let Some(object) = empty_spawn_result(scope, &code, &message) else {
                return;
            };
            rv.set(object.into());
            return;
        }
        Err(Failure::Coded { code, message }) => {
            throw_coded(scope, &code, &message);
            return;
        }
    };

    let Some(object) = spawn_result(scope, &result) else {
        return;
    };
    let error = if result.timed_out {
        Some(("ETIMEDOUT", "child process timed out"))
    } else if result.limit_exceeded {
        Some((
            "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
            "child process output exceeded maxBuffer",
        ))
    } else if result.incomplete {
        Some((
            "ERR_NODE_CHILD_PROCESS_INCOMPLETE",
            "the direct child exited while a descendant still held a piped stream; output may be incomplete",
        ))
    } else if result.io_error {
        Some((
            "ERR_NODE_CHILD_PROCESS_IO",
            "a child process pipe read failed; output may be incomplete",
        ))
    } else {
        None
    };
    if let Some((code, message)) = error {
        let Some(error) = coded_error(scope, code, message) else {
            return;
        };
        if !define_value(scope, &object, "error", error.into()) {
            return;
        }
    }
    rv.set(object.into());
}

fn exec_file_sync<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = match execute(scope, &args) {
        Ok(result) => result,
        Err(failure) => {
            throw_failure(scope, failure);
            return;
        }
    };

    if result.timed_out {
        throw_with_fields(scope, "ETIMEDOUT", "child process timed out", &result);
        return;
    }
    if result.limit_exceeded {
        throw_with_fields(
            scope,
            "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
            "child process output exceeded maxBuffer",
            &result,
        );
        return;
    }
    if result.incomplete {
        throw_with_fields(
            scope,
            "ERR_NODE_CHILD_PROCESS_INCOMPLETE",
            "the direct child exited while a descendant still held a piped stream; output may be incomplete",
            &result,
        );
        return;
    }
    if result.io_error {
        throw_with_fields(
            scope,
            "ERR_NODE_CHILD_PROCESS_IO",
            "a child process pipe read failed; output may be incomplete",
            &result,
        );
        return;
    }
    if result.status != Some(0) {
        throw_with_fields(
            scope,
            "ERR_NODE_CHILD_PROCESS_EXIT",
            "child process exited with a non-zero status",
            &result,
        );
        return;
    }
    rv.set(string_value(
        scope,
        &String::from_utf8_lossy(&result.stdout),
    ));
}

#![forbid(unsafe_code)]
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

/// Installs the native `node:child_process` module and its process policy.
///
/// Registration happens before any user module can import the specifier. The
/// module has no default side effects and no global object.
pub fn install(js: &mut JsRuntime, options: NodeOptions) -> Result<(), String> {
    let context = js.main_context();
    let exports = {
        v8::scope!(let scope, js.v8_isolate());
        let context_local = v8::Local::new(scope, context);
        context_local.set_slot(Rc::new(options));
        let scope = &mut v8::ContextScope::new(scope, context_local);
        build_exports(scope)?
    };
    js.register_native_module(CHILD_PROCESS_SPECIFIER, exports)
        .map_err(|error| error.to_string())?;
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
        set_value(scope, &api, name, (*function).into());
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
    let _ = scope;
    exports.push((name, v8::Global::new(scope, value)));
}

fn set_value<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let key = v8::String::new(scope, name).expect("a native property name is a valid V8 string");
    assert!(
        object.set(scope, key.into(), value) == Some(true),
        "setting a native property failed"
    );
}

fn get<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
    name: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let key = v8::String::new(scope, name)?;
    object.get(scope, key.into())
}

/// A coded failure raised by guest validation or the shared runner.
struct Failure {
    code: String,
    message: String,
}

impl Failure {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
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

fn throw_failure(scope: &mut v8::PinScope<'_, '_>, failure: Failure) {
    throw_coded(scope, &failure.code, &failure.message);
}

fn coded_error<'s>(
    scope: &v8::PinScope<'s, '_>,
    code: &str,
    message: &str,
) -> v8::Local<'s, v8::Object> {
    let message = v8::String::new(scope, message).expect("an error message is a valid V8 string");
    let exception = v8::Exception::error(scope, message);
    let object =
        v8::Local::<v8::Object>::try_from(exception).expect("Exception::error is an object");
    let code = string_value(scope, code);
    set_value(scope, &object, "code", code);
    object
}

fn throw_coded(scope: &mut v8::PinScope<'_, '_>, code: &str, message: &str) {
    let error = coded_error(scope, code, message);
    scope.throw_exception(error.into());
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
) -> v8::Local<'s, v8::Array> {
    let output = v8::Array::new(scope, 3);
    assert!(
        output.set_index(scope, 0, v8::null(scope).into()) == Some(true),
        "setting output[0] failed"
    );
    assert!(
        output.set_index(scope, 1, string_value(scope, stdout)) == Some(true),
        "setting output[1] failed"
    );
    assert!(
        output.set_index(scope, 2, string_value(scope, stderr)) == Some(true),
        "setting output[2] failed"
    );
    output
}

fn attach_result_fields<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
    result: &SyncResult,
) {
    let stdout = String::from_utf8_lossy(&result.stdout);
    let stderr = String::from_utf8_lossy(&result.stderr);
    set_value(
        scope,
        object,
        "pid",
        v8::Number::new(scope, f64::from(result.pid)).into(),
    );
    set_value(
        scope,
        object,
        "output",
        output_array(scope, &stdout, &stderr).into(),
    );
    set_value(scope, object, "stdout", string_value(scope, &stdout));
    set_value(scope, object, "stderr", string_value(scope, &stderr));
    set_value(
        scope,
        object,
        "status",
        null_or_number(scope, result.status),
    );
    set_value(
        scope,
        object,
        "signal",
        null_or_string(scope, result.signal_name),
    );
}

fn spawn_result<'s>(
    scope: &v8::PinScope<'s, '_>,
    result: &SyncResult,
) -> v8::Local<'s, v8::Object> {
    let object = v8::Object::new(scope);
    attach_result_fields(scope, &object, result);
    object
}

fn empty_spawn_result<'s>(
    scope: &v8::PinScope<'s, '_>,
    code: &str,
    message: &str,
) -> v8::Local<'s, v8::Object> {
    let object = v8::Object::new(scope);
    set_value(scope, &object, "pid", v8::Number::new(scope, 0.0).into());
    set_value(scope, &object, "output", output_array(scope, "", "").into());
    set_value(scope, &object, "stdout", string_value(scope, ""));
    set_value(scope, &object, "stderr", string_value(scope, ""));
    set_value(scope, &object, "status", v8::null(scope).into());
    set_value(scope, &object, "signal", v8::null(scope).into());
    let error = coded_error(scope, code, message);
    set_value(scope, &object, "error", error.into());
    object
}

fn throw_with_fields(
    scope: &mut v8::PinScope<'_, '_>,
    code: &str,
    message: &str,
    result: &SyncResult,
) {
    let error = coded_error(scope, code, message);
    attach_result_fields(scope, &error, result);
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
    let encoding = get(scope, &object, "encoding")
        .ok_or_else(|| Failure::invalid_type("options could not be read"))?;
    let cwd = get(scope, &object, "cwd")
        .ok_or_else(|| Failure::invalid_type("options could not be read"))?;
    let env = get(scope, &object, "env")
        .ok_or_else(|| Failure::invalid_type("options could not be read"))?;
    let input = get(scope, &object, "input")
        .ok_or_else(|| Failure::invalid_type("options could not be read"))?;
    let timeout = get(scope, &object, "timeout")
        .ok_or_else(|| Failure::invalid_type("options could not be read"))?;
    let max_buffer = get(scope, &object, "maxBuffer")
        .ok_or_else(|| Failure::invalid_type("options could not be read"))?;

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
    let names = object
        .get_own_property_names(
            scope,
            v8::GetPropertyNamesArgs {
                mode: v8::KeyCollectionMode::OwnOnly,
                property_filter: v8::PropertyFilter::ONLY_ENUMERABLE
                    | v8::PropertyFilter::SKIP_SYMBOLS,
                index_filter: v8::IndexFilter::IncludeIndices,
                key_conversion: v8::KeyConversionMode::ConvertToString,
            },
        )
        .ok_or_else(|| Failure::invalid_type("options must be an object"))?;
    let count = names.length() as usize;
    if count > MAX_OPTION_KEYS {
        return Err(Failure::unsupported("an option is not implemented"));
    }
    for index in 0..count {
        let key = names
            .get_index(scope, index as u32)
            .ok_or_else(|| Failure::invalid_type("an option could not be read"))?;
        let key = v8::Local::<v8::String>::try_from(key)
            .map_err(|_| Failure::invalid_type("an option could not be read"))?;
        if key.length() > MAX_PATH_UNITS {
            return Err(Failure::too_large());
        }
        let key = key.to_rust_string_lossy(scope);
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
    let names = object
        .get_own_property_names(
            scope,
            v8::GetPropertyNamesArgs {
                mode: v8::KeyCollectionMode::OwnOnly,
                property_filter: v8::PropertyFilter::ONLY_ENUMERABLE
                    | v8::PropertyFilter::SKIP_SYMBOLS,
                index_filter: v8::IndexFilter::IncludeIndices,
                key_conversion: v8::KeyConversionMode::ConvertToString,
            },
        )
        .ok_or_else(|| Failure::invalid_type("options.env must be an object"))?;
    let count = names.length() as usize;
    if count > MAX_ENV_ENTRIES {
        return Err(Failure::too_large());
    }

    let mut environment = BTreeMap::new();
    let mut total = 0usize;
    for index in 0..count {
        let key = names
            .get_index(scope, index as u32)
            .ok_or_else(|| Failure::invalid_type("an environment name is not valid"))?;
        let name = v8::Local::<v8::String>::try_from(key)
            .map_err(|_| Failure::invalid_type("an environment name is not valid"))?;
        if name.length() > MAX_PATH_UNITS {
            return Err(Failure::too_large());
        }
        let name = name.to_rust_string_lossy(scope);
        if name.is_empty() || name.contains('=') || name.contains('\0') {
            return Err(Failure::invalid_value("an environment name is not valid"));
        }
        let value = object
            .get(scope, key)
            .ok_or_else(|| Failure::invalid_type("an environment value is not valid"))?;
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
        None => get(scope, &object, "length")
            .ok_or_else(|| Failure::invalid_type("arguments must have a valid length"))?,
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
        let value = object
            .get_index(scope, index)
            .ok_or_else(|| Failure::invalid_type("an argument could not be read"))?;
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
            let string = value
                .to_string(scope)
                .ok_or_else(|| Failure::invalid_type("an argument is not a valid string"))?;
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
        let object = v8::Local::<v8::Object>::try_from(args_value).ok();
        let length = object.and_then(|object| get(scope, &object, "length"));
        if length.is_some_and(|value| value.is_number()) {
            pre_length = length;
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

fn unsupported_operation(scope: &mut v8::PinScope<'_, '_>, name: &str) {
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
        Err(failure) => {
            if failure.code == "ENOENT" || failure.code == "EACCES" {
                rv.set(empty_spawn_result(scope, &failure.code, &failure.message).into());
                return;
            }
            throw_failure(scope, failure);
            return;
        }
    };

    let object = spawn_result(scope, &result);
    if result.timed_out {
        let error = coded_error(scope, "ETIMEDOUT", "child process timed out");
        set_value(scope, &object, "error", error.into());
    } else if result.limit_exceeded {
        let error = coded_error(
            scope,
            "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
            "child process output exceeded maxBuffer",
        );
        set_value(scope, &object, "error", error.into());
    } else if result.incomplete {
        let error = coded_error(
            scope,
            "ERR_NODE_CHILD_PROCESS_INCOMPLETE",
            "the direct child exited while a descendant still held a piped stream; output may be incomplete",
        );
        set_value(scope, &object, "error", error.into());
    } else if result.io_error {
        let error = coded_error(
            scope,
            "ERR_NODE_CHILD_PROCESS_IO",
            "a child process pipe read failed; output may be incomplete",
        );
        set_value(scope, &object, "error", error.into());
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

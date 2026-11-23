#![deny(unsafe_code)]
//! QuickJS adapter for opt-in Node compatibility.
//!
//! This crate exposes `node:child_process` as a native QuickJS C module. The
//! module has no JavaScript source and no JSON bridge: its named exports are
//! Rust functions that validate guest values, call the engine-neutral
//! `neojs-node-builtins` runner, and build QuickJS objects and errors directly.
//!
//! The module is installed only by
//! `neojs_lite_core::Runtime::with_node_compat`, which stores the runtime's
//! [`NodeOptions`] for the native module to clone during evaluation. The module
//! has no other authority. It is not a sandbox, and code in the context can call
//! the exported functions directly.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::ffi::c_char;

use rquickjs::Array;
use rquickjs::Atom;
use rquickjs::Ctx;
use rquickjs::Exception;
use rquickjs::Function;
use rquickjs::Module;
use rquickjs::Object;
use rquickjs::Result;
use rquickjs::Value;
use rquickjs::module::Declarations;
use rquickjs::module::Declared;
use rquickjs::module::Exports;
use rquickjs::module::ModuleDef;
use rquickjs::prelude::Rest;

use neojs_node_builtins::run_sync;

pub use neojs_node_builtins::{
    CHILD_PROCESS_SPECIFIER, ChildProcessError, ChildProcessPolicy, Limits, NodeOptions,
    PolicyError, SUPPORTED_MODULES, SyncRequest, SyncResult, is_known_node_bare_name,
    unsupported_module_reason,
};

const MAX_INPUT_BYTES: usize = 1_048_576;
const MAX_ARGS: usize = 256;
const MAX_ARG_BYTES: usize = 65_536;
const MAX_ENV_ENTRIES: usize = 128;
const MAX_ENV_BYTES: usize = 65_536;
const MAX_PATH_BYTES: usize = 4_096;

thread_local! {
    /// Policies for live QuickJS contexts, keyed by the context pointer.
    ///
    /// The runtime is single-threaded and one context belongs to one runtime.
    /// The pointer is only a map key; it is never dereferenced here.
    static POLICIES: RefCell<HashMap<usize, NodeOptions>> =
        RefCell::new(HashMap::new());
}

/// Stores the policy for the native module in this context.
///
/// Called by `neojs_lite_core::Runtime` before user code runs. Replacing an
/// existing entry is not expected; the runtime owns one context.
pub fn install_options(ctx: &Ctx<'_>, options: NodeOptions) {
    let key = ctx.as_raw().as_ptr() as usize;
    POLICIES.with(|policies| {
        policies.borrow_mut().insert(key, options);
    });
}

/// Removes the policy for this context.
///
/// Called when the runtime is dropped so the map does not retain a policy after
/// the QuickJS context is gone.
pub fn remove_options(ctx: &Ctx<'_>) {
    let key = ctx.as_raw().as_ptr() as usize;
    POLICIES.with(|policies| {
        policies.borrow_mut().remove(&key);
    });
}

fn policy_for(ctx: &Ctx<'_>) -> Option<NodeOptions> {
    let key = ctx.as_raw().as_ptr() as usize;
    POLICIES.with(|policies| policies.borrow().get(&key).cloned())
}

/// The native `node:child_process` module definition.
struct ChildProcessModule;

impl ModuleDef for ChildProcessModule {
    fn declare(decl: &Declarations<'_>) -> Result<()> {
        for name in [
            "spawnSync",
            "execFileSync",
            "spawn",
            "exec",
            "execFile",
            "execSync",
            "fork",
            "ChildProcess",
            "default",
        ] {
            decl.declare(name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
        let options = policy_for(ctx).ok_or_else(|| {
            coded_error(
                ctx,
                "ERR_NODE_CHILD_PROCESS_POLICY",
                "the node:child_process policy is not installed",
            )
        })?;

        let spawn_sync = sync_function(ctx.clone(), options.clone(), false)?;
        let exec_file_sync = sync_function(ctx.clone(), options.clone(), true)?;
        let spawn = unsupported_function(ctx.clone(), "spawn")?;
        let exec = unsupported_function(ctx.clone(), "exec")?;
        let exec_file = unsupported_function(ctx.clone(), "execFile")?;
        let exec_sync = unsupported_function(ctx.clone(), "execSync")?;
        let fork = unsupported_function(ctx.clone(), "fork")?;
        let child_process = unsupported_function(ctx.clone(), "ChildProcess")?;

        exports.export("spawnSync", spawn_sync.clone())?;
        exports.export("execFileSync", exec_file_sync.clone())?;
        exports.export("spawn", spawn.clone())?;
        exports.export("exec", exec.clone())?;
        exports.export("execFile", exec_file.clone())?;
        exports.export("execSync", exec_sync.clone())?;
        exports.export("fork", fork.clone())?;
        exports.export("ChildProcess", child_process.clone())?;

        let default = Object::new(ctx.clone())?;
        default.set("spawnSync", spawn_sync)?;
        default.set("execFileSync", exec_file_sync)?;
        default.set("spawn", spawn)?;
        default.set("exec", exec)?;
        default.set("execFile", exec_file)?;
        default.set("execSync", exec_sync)?;
        default.set("fork", fork)?;
        default.set("ChildProcess", child_process)?;
        exports.export("default", default)?;
        Ok(())
    }
}

/// Declares the native `node:child_process` module.
///
/// `neojs-lite-core` calls this from its loader; no module source is read from
/// disk or generated.
pub fn module<'js>(ctx: Ctx<'js>, name: &str) -> Result<Module<'js, Declared>> {
    if name == CHILD_PROCESS_SPECIFIER {
        Module::declare_def::<ChildProcessModule, _>(ctx, name)
    } else {
        Err(rquickjs::Error::new_loading_message(
            name,
            "the Node module is not implemented in this runtime",
        ))
    }
}

fn sync_function<'js>(
    ctx: Ctx<'js>,
    options: NodeOptions,
    exec_file: bool,
) -> Result<Function<'js>> {
    Function::new(
        ctx,
        move |ctx: Ctx<'js>, args: Rest<Value<'js>>| -> Result<Value<'js>> {
            let arguments = args.into_inner();
            let Some(file_value) = arguments.first() else {
                return Err(invalid(&ctx, "file must be a string"));
            };
            let file = read_string(&ctx, file_value, "file", MAX_PATH_BYTES)?;
            if file.is_empty() {
                return Err(invalid(&ctx, "file must not be empty"));
            }
            if file.contains('\0') {
                return Err(invalid(&ctx, "file must not contain a NUL byte"));
            }

            let (args, options_value) = match arguments.get(1) {
                None => (Vec::new(), None),
                Some(value) if value.is_undefined() => (Vec::new(), arguments.get(2)),
                Some(value) if value.is_array() || value.is_proxy() => {
                    let Some(object) = value.as_object() else {
                        return Err(invalid(&ctx, "arguments must be an array"));
                    };
                    let length = object.get::<_, Value>("length")?;
                    if length.is_number() {
                        (
                            read_array_arguments(&ctx, object, &length)?,
                            arguments.get(2),
                        )
                    } else if value.is_array() {
                        return Err(invalid(&ctx, "arguments must have a valid length"));
                    } else {
                        (Vec::new(), Some(value))
                    }
                }
                Some(value) => (Vec::new(), Some(value)),
            };
            let parsed = read_options(&ctx, options_value, options.clone())?;
            let request = SyncRequest {
                file,
                args,
                cwd: parsed.cwd,
                env: parsed.env,
                input: parsed.input,
                timeout_ms: parsed.timeout_ms,
                max_buffer: parsed.max_buffer,
            };

            match run_sync(&request, &parsed.policy) {
                Ok(result) if exec_file => {
                    if let Some(error) = result_error(&ctx, &result) {
                        return Err(error);
                    }
                    Ok(js_string(&ctx, &String::from_utf8_lossy(&result.stdout))?.into_value())
                }
                Ok(result) => spawn_result(&ctx, result),
                Err(error) if exec_file => Err(child_process_error(&ctx, &error)),
                Err(error) if error.code() == "ENOENT" || error.code() == "EACCES" => {
                    spawn_error_result(&ctx, &error)
                }
                Err(error) => Err(child_process_error(&ctx, &error)),
            }
        },
    )
}

fn unsupported_function<'js>(ctx: Ctx<'js>, name: &'static str) -> Result<Function<'js>> {
    Function::new(ctx, move |ctx: Ctx<'js>| -> Result<Value<'js>> {
        Err(coded_error(
            &ctx,
            "ERR_NODE_UNSUPPORTED",
            &format!("node:child_process.{name} is not implemented; use spawnSync or execFileSync"),
        ))
    })
}

struct ParsedOptions {
    cwd: Option<String>,
    env: Option<BTreeMap<String, String>>,
    input: Option<String>,
    timeout_ms: Option<u64>,
    max_buffer: Option<usize>,
    policy: NodeOptions,
}

fn read_options<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    policy: NodeOptions,
) -> Result<ParsedOptions> {
    let Some(value) = value else {
        return Err(coded_error(
            ctx,
            "ERR_NODE_UNSUPPORTED_ENCODING",
            "node:child_process requires encoding: \"utf8\"",
        ));
    };
    if value.is_undefined() {
        return Err(coded_error(
            ctx,
            "ERR_NODE_UNSUPPORTED_ENCODING",
            "node:child_process requires encoding: \"utf8\"",
        ));
    }
    if !value.is_object() || value.is_array() {
        return Err(invalid(ctx, "options must be an object"));
    }
    if value.is_proxy() {
        return Err(coded_error(
            ctx,
            "ERR_NODE_UNSUPPORTED",
            "Proxy options are not supported by the native child_process module",
        ));
    }
    let object = value.as_object().expect("checked object");

    let known_options = [
        Atom::from_str(ctx.clone(), "cwd")?,
        Atom::from_str(ctx.clone(), "env")?,
        Atom::from_str(ctx.clone(), "input")?,
        Atom::from_str(ctx.clone(), "timeout")?,
        Atom::from_str(ctx.clone(), "maxBuffer")?,
        Atom::from_str(ctx.clone(), "encoding")?,
    ];
    for key in object.keys::<Atom>() {
        let key = key?;
        if !known_options.iter().any(|known| known == &key) {
            return Err(coded_error(
                ctx,
                "ERR_NODE_UNSUPPORTED",
                "an option is not implemented",
            ));
        }
    }

    // Snapshot each option exactly once. A getter/proxy can return a different
    // value on later reads; the snapshot is what is validated and copied.
    let encoding = object.get::<_, Value>("encoding")?;
    let cwd_value = object.get::<_, Value>("cwd")?;
    let env_value = object.get::<_, Value>("env")?;
    let input_value = object.get::<_, Value>("input")?;
    let timeout_value = object.get::<_, Value>("timeout")?;
    let max_buffer_value = object.get::<_, Value>("maxBuffer")?;

    if encoding.is_undefined() {
        return Err(coded_error(
            ctx,
            "ERR_NODE_UNSUPPORTED_ENCODING",
            "node:child_process requires encoding: \"utf8\"",
        ));
    }
    let encoding = read_string(ctx, &encoding, "options.encoding", MAX_PATH_BYTES)?;
    if !(encoding.eq_ignore_ascii_case("utf8") || encoding.eq_ignore_ascii_case("utf-8")) {
        return Err(coded_error(
            ctx,
            "ERR_NODE_UNSUPPORTED_ENCODING",
            "node:child_process requires encoding: \"utf8\"",
        ));
    }

    let cwd = if cwd_value.is_undefined() {
        None
    } else {
        let cwd = read_string(ctx, &cwd_value, "options.cwd", MAX_PATH_BYTES)?;
        if cwd.contains('\0') {
            return Err(invalid(ctx, "options.cwd must not contain a NUL byte"));
        }
        Some(cwd)
    };

    let env = if env_value.is_undefined() {
        None
    } else {
        Some(read_env(ctx, &env_value)?)
    };

    let input = if input_value.is_undefined() {
        None
    } else {
        Some(read_string(
            ctx,
            &input_value,
            "options.input",
            MAX_INPUT_BYTES,
        )?)
    };

    let timeout_ms = if timeout_value.is_undefined() {
        None
    } else {
        Some(read_u64(ctx, &timeout_value, "options.timeout", false)?)
    };
    let max_buffer = if max_buffer_value.is_undefined() {
        None
    } else {
        let value = read_u64(ctx, &max_buffer_value, "options.maxBuffer", true)?;
        usize::try_from(value)
            .map_err(|_| invalid(ctx, "options.maxBuffer is too large"))?
            .into()
    };

    Ok(ParsedOptions {
        cwd,
        env,
        input,
        timeout_ms,
        max_buffer,
        policy,
    })
}

fn read_array_arguments<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    length_value: &Value<'js>,
) -> Result<Vec<String>> {
    if !length_value.is_number() {
        return Err(invalid(ctx, "arguments must have a valid length"));
    }
    let length = length_value.as_number().expect("checked number");
    if !length.is_finite() || length.fract() != 0.0 || length < 0.0 {
        return Err(invalid(ctx, "arguments must have a valid length"));
    }
    if length > MAX_ARGS as f64 {
        return Err(request_too_large(ctx));
    }
    let length = length as usize;
    let mut total = 0usize;
    let mut arguments = Vec::with_capacity(length);
    for index in 0..length {
        let value = object.get::<_, Value>(index as u32)?;
        let text = read_argument(ctx, &value)?;
        total = total.saturating_add(text.len());
        if total > MAX_ARG_BYTES {
            return Err(request_too_large(ctx));
        }
        arguments.push(text);
    }
    Ok(arguments)
}

fn read_argument<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<String> {
    if value.is_string() {
        let text = read_string(ctx, value, "argument", MAX_ARG_BYTES)?;
        if text.contains('\0') {
            return Err(invalid(ctx, "an argument must not contain a NUL byte"));
        }
        Ok(text)
    } else if value.is_number() {
        let number = value.as_number().expect("checked number");
        if !number.is_finite() {
            return Err(invalid(ctx, "an argument number must be finite"));
        }
        Ok(number.to_string())
    } else if value.is_bool() {
        Ok(if value.as_bool().expect("checked bool") {
            "true".to_string()
        } else {
            "false".to_string()
        })
    } else {
        Err(invalid(
            ctx,
            "arguments must be strings, numbers, or booleans",
        ))
    }
}

fn read_env<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<BTreeMap<String, String>> {
    if !value.is_object() || value.is_array() {
        return Err(invalid(ctx, "options.env must be an object"));
    }
    if value.is_proxy() {
        return Err(coded_error(
            ctx,
            "ERR_NODE_UNSUPPORTED",
            "Proxy options.env is not supported by the native child_process module",
        ));
    }
    let object = value.as_object().expect("checked object");
    let mut environment = BTreeMap::new();
    let mut total = 0usize;
    for key in object.keys::<Atom>() {
        let key = key?;
        if environment.len() >= MAX_ENV_ENTRIES {
            return Err(request_too_large(ctx));
        }
        let key_value = key.to_value()?;
        let name = read_string(ctx, &key_value, "environment name", MAX_PATH_BYTES)?;
        if name.is_empty() || name.contains('=') || name.contains('\0') {
            return Err(invalid(ctx, "an environment name is not valid"));
        }
        let value = object.get::<_, Value>(name.as_str())?;
        let value = read_string(ctx, &value, "environment value", MAX_ENV_BYTES)?;
        if value.contains('\0') {
            return Err(invalid(
                ctx,
                "an environment value must not contain a NUL byte",
            ));
        }
        total = total.saturating_add(name.len()).saturating_add(value.len());
        if total > MAX_ENV_BYTES {
            return Err(request_too_large(ctx));
        }
        environment.insert(name, value);
    }
    Ok(environment)
}

fn read_string<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    what: &str,
    max_bytes: usize,
) -> Result<String> {
    if !value.is_string() {
        return Err(invalid(ctx, &format!("{what} must be a string")));
    }
    // QuickJS's public property API returns the code-unit length without
    // copying the string. UTF-8 length is not smaller than the UTF-16 length for
    // any scalar, so this rejects a clearly oversized string before the host
    // copies or converts it.
    let units = string_length(ctx, value).ok_or_else(|| {
        coded_error(
            ctx,
            "ERR_INVALID_ARG_VALUE",
            &format!("{what} length could not be read"),
        )
    })?;
    if units > max_bytes {
        return Err(request_too_large(ctx));
    }
    let text = value.as_string().expect("checked string").to_string()?;
    if text.len() > max_bytes {
        // A UTF-16 length below the bound can still exceed it in UTF-8; the
        // conversion allocation is bounded by four times the pre-check bound.
        return Err(request_too_large(ctx));
    }
    Ok(text)
}

/// Returns a string's UTF-16 code-unit length without converting it to Rust.
///
/// QuickJS's `JS_GetPropertyStr(..., "length")` handles primitive strings
/// directly and returns an integer; it does not copy the string. The returned
/// value is freed exactly once. This is the only unsafe function in this crate:
/// the context and value are live QuickJS handles, the property name is a
/// static C string, and the returned value is released exactly once.
#[allow(unsafe_code)]
fn string_length(ctx: &Ctx<'_>, value: &Value<'_>) -> Option<usize> {
    let raw_ctx = ctx.as_raw().as_ptr();
    let length = unsafe {
        rquickjs::qjs::JS_GetPropertyStr(
            raw_ctx,
            value.as_raw(),
            c"length".as_ptr() as *const c_char,
        )
    };
    let mut units = 0i32;
    let status = unsafe { rquickjs::qjs::JS_ToInt32(raw_ctx, &mut units, length) };
    unsafe { rquickjs::qjs::JS_FreeValue(raw_ctx, length) };
    if status < 0 || units < 0 {
        let _ = ctx.catch();
        None
    } else {
        usize::try_from(units).ok()
    }
}

fn read_u64<'js>(ctx: &Ctx<'js>, value: &Value<'js>, what: &str, positive: bool) -> Result<u64> {
    if !value.is_number() {
        return Err(invalid(ctx, &format!("{what} must be a primitive integer")));
    }
    let number = value.as_number().expect("checked number");
    if !number.is_finite() || number.fract() != 0.0 || number < 0.0 || (positive && number == 0.0) {
        return Err(invalid(ctx, &format!("{what} is not a valid integer")));
    }
    if number > u64::MAX as f64 {
        return Err(invalid(ctx, &format!("{what} is too large")));
    }
    Ok(number as u64)
}

fn spawn_result<'js>(ctx: &Ctx<'js>, result: SyncResult) -> Result<Value<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("pid", Value::new_float(ctx.clone(), f64::from(result.pid)))?;
    object.set(
        "stdout",
        js_string(ctx, &String::from_utf8_lossy(&result.stdout))?,
    )?;
    object.set(
        "stderr",
        js_string(ctx, &String::from_utf8_lossy(&result.stderr))?,
    )?;
    object.set("output", output_array(ctx, &result)?)?;
    object.set("status", option_int(ctx, result.status)?)?;
    object.set("signal", option_string(ctx, result.signal_name)?)?;
    if let Some(error) = result_error_object(ctx, &result)? {
        object.set("error", error)?;
    }
    Ok(object.into_value())
}

fn spawn_error_result<'js>(ctx: &Ctx<'js>, error: &ChildProcessError) -> Result<Value<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set("pid", Value::new_int(ctx.clone(), 0))?;
    let empty = js_string(ctx, "")?;
    object.set("stdout", empty.clone())?;
    object.set("stderr", empty)?;
    let output = Array::new(ctx.clone())?;
    output.set(0, Value::new_null(ctx.clone()))?;
    output.set(1, js_string(ctx, "")?)?;
    output.set(2, js_string(ctx, "")?)?;
    object.set("output", output)?;
    object.set("status", Value::new_null(ctx.clone()))?;
    object.set("signal", Value::new_null(ctx.clone()))?;
    object.set("error", error_object(ctx, error.code(), error.message())?)?;
    Ok(object.into_value())
}

fn spawn_error_parts(result: &SyncResult) -> Option<(&'static str, &'static str)> {
    if result.timed_out {
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
    }
}

fn result_error_parts(result: &SyncResult) -> Option<(&'static str, &'static str)> {
    spawn_error_parts(result).or_else(|| {
        if result.status != Some(0) {
            Some((
                "ERR_NODE_CHILD_PROCESS_EXIT",
                "child process did not exit with status 0",
            ))
        } else {
            None
        }
    })
}

fn result_error<'js>(ctx: &Ctx<'js>, result: &SyncResult) -> Option<rquickjs::Error> {
    let (code, message) = result_error_parts(result)?;
    Some(result_error_with(ctx, code, message, result))
}

fn result_error_object<'js>(ctx: &Ctx<'js>, result: &SyncResult) -> Result<Option<Object<'js>>> {
    let Some((code, message)) = spawn_error_parts(result) else {
        return Ok(None);
    };
    let exception = Exception::from_message(ctx.clone(), message)?;
    let object = exception.into_object();
    fill_result_error(ctx, &object, code, result)?;
    Ok(Some(object))
}

fn fill_result_error<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    code: &str,
    result: &SyncResult,
) -> Result<()> {
    object.set("code", code)?;
    object.set("pid", Value::new_float(ctx.clone(), f64::from(result.pid)))?;
    object.set("status", option_int(ctx, result.status)?)?;
    object.set("signal", option_string(ctx, result.signal_name)?)?;
    object.set(
        "stdout",
        js_string(ctx, &String::from_utf8_lossy(&result.stdout))?,
    )?;
    object.set(
        "stderr",
        js_string(ctx, &String::from_utf8_lossy(&result.stderr))?,
    )?;
    object.set("output", output_array(ctx, result)?)?;
    Ok(())
}

fn result_error_with<'js>(
    ctx: &Ctx<'js>,
    code: &str,
    message: &str,
    result: &SyncResult,
) -> rquickjs::Error {
    let exception = match Exception::from_message(ctx.clone(), message) {
        Ok(exception) => exception,
        Err(error) => return error,
    };
    if let Err(error) = fill_result_error(ctx, exception.as_object(), code, result) {
        return error;
    }
    exception.throw()
}

fn output_array<'js>(ctx: &Ctx<'js>, result: &SyncResult) -> Result<Array<'js>> {
    let output = Array::new(ctx.clone())?;
    output.set(0, Value::new_null(ctx.clone()))?;
    output.set(1, js_string(ctx, &String::from_utf8_lossy(&result.stdout))?)?;
    output.set(2, js_string(ctx, &String::from_utf8_lossy(&result.stderr))?)?;
    Ok(output)
}

fn option_int<'js>(ctx: &Ctx<'js>, value: Option<i32>) -> Result<Value<'js>> {
    Ok(match value {
        Some(value) => Value::new_int(ctx.clone(), value),
        None => Value::new_null(ctx.clone()),
    })
}

fn option_string<'js>(ctx: &Ctx<'js>, value: Option<&str>) -> Result<Value<'js>> {
    Ok(match value {
        Some(value) => js_string(ctx, value)?.into_value(),
        None => Value::new_null(ctx.clone()),
    })
}

fn error_object<'js>(ctx: &Ctx<'js>, code: &str, message: &str) -> Result<Object<'js>> {
    let exception = Exception::from_message(ctx.clone(), message)?;
    let object = exception.into_object();
    object.set("code", code)?;
    Ok(object)
}

fn child_process_error<'js>(ctx: &Ctx<'js>, error: &ChildProcessError) -> rquickjs::Error {
    coded_error(ctx, error.code(), error.message())
}

fn coded_error<'js>(ctx: &Ctx<'js>, code: &str, message: &str) -> rquickjs::Error {
    match error_object(ctx, code, message) {
        Ok(object) => match Exception::from_object(object) {
            Some(exception) => exception.throw(),
            None => rquickjs::Error::Unknown,
        },
        Err(error) => error,
    }
}

fn invalid<'js>(ctx: &Ctx<'js>, message: &str) -> rquickjs::Error {
    coded_error(ctx, "ERR_INVALID_ARG_VALUE", message)
}

fn request_too_large(ctx: &Ctx<'_>) -> rquickjs::Error {
    coded_error(
        ctx,
        "ERR_NODE_REQUEST_TOO_LARGE",
        "the node:child_process request exceeds the hard size limit",
    )
}

fn js_string<'js>(ctx: &Ctx<'js>, text: &str) -> Result<rquickjs::String<'js>> {
    rquickjs::String::from_str(ctx.clone(), text)
}

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
/// The six option keys the native module implements.
const KNOWN_OPTIONS: [&str; 6] = ["cwd", "env", "input", "timeout", "maxBuffer", "encoding"];
/// Most enumerable string keys accepted on an options object.
const MAX_OPTION_KEYS: u32 = 6;
/// Most option key UTF-16 code units accepted before conversion.
const MAX_OPTION_KEY_UNITS: u32 = MAX_PATH_BYTES as u32;
/// Shape slots scanned for options. This covers all slots, including deleted,
/// non-enumerable and symbol slots, so a hostile shape is rejected before any
/// property list is allocated.
const MAX_OPTION_SCAN: u32 = 64;
/// Shape slots scanned for an environment object. It is larger than the entry
/// cap because non-enumerable and symbol slots also count toward the scan.
const MAX_ENV_SCAN: u32 = 4_096;

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
    let object = value.as_object().expect("checked object");

    // Enumerate through the bounded QuickJS helper. It rejects Proxy and every
    // exotic object before any trap can run, counts every shape slot against a
    // scan bound, and allocates only after the key bound passes.
    let keys = bounded_string_keys(
        ctx,
        object,
        MAX_OPTION_KEYS,
        MAX_OPTION_SCAN,
        MAX_OPTION_KEY_UNITS,
    )
    .map_err(|error| bounded_option_error(ctx, error))?;
    for key in keys {
        if !KNOWN_OPTIONS.contains(&key.as_str()) {
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

/// Why the bounded QuickJS key snapshot was rejected.
#[derive(Debug, PartialEq, Eq)]
enum BoundedKeysError {
    /// More enumerable string keys than the caller allows.
    TooManyKeys,
    /// More shape slots than the caller allows.
    ScanLimit,
    /// A Proxy, an exotic object, or a non-string atom.
    Unsupported,
    /// The property snapshot could not be allocated.
    Memory,
    /// A key is not valid UTF-8 after conversion (for example, a lone
    /// surrogate that QuickJS encodes as an invalid UTF-8 sequence).
    InvalidKey,
    /// QuickJS raised an exception or produced an unreadable key.
    Exception,
}

/// Returns the own enumerable string keys of a plain QuickJS object.
///
/// The work happens in the vendored `rquickjs-sys` helper
/// `JS_GetOwnEnumerableStringKeysBounded`, which:
///
/// * rejects Proxy and every object with exotic property handlers before any
///   callback or trap can run, so `ownKeys` is never entered;
/// * counts every shape slot against `max_scan` before allocating anything;
/// * counts enumerable string keys against `max_keys`, with checked arithmetic,
///   before allocating;
/// * preserves the JavaScript order: integer keys ascending, then other string
///   keys in insertion order;
/// * checks each key's UTF-16 length against `max_key_len` before converting it
///   to the UTF-8 C string this function copies.
///
/// The returned atoms are released through an RAII guard on every path. This
/// function is the only property-enumeration path used by the adapter.
#[allow(unsafe_code)]
fn bounded_string_keys<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    max_keys: u32,
    max_scan: u32,
    max_key_len: u32,
) -> std::result::Result<Vec<String>, BoundedKeysError> {
    use rquickjs::qjs;

    // These values mirror the JSBoundedKeysStatus C enum in the vendored
    // QuickJS header. They are part of the audited patch surface.
    const OK: i32 = 0;
    const TOO_MANY_KEYS: i32 = 1;
    const SCAN_LIMIT: i32 = 2;
    const UNSUPPORTED: i32 = 3;
    const MEMORY: i32 = 4;

    /// Frees the atom snapshot even when key conversion or validation fails.
    struct EnumGuard {
        raw: *mut qjs::JSContext,
        tab: *mut qjs::JSPropertyEnum,
        len: u32,
    }

    impl Drop for EnumGuard {
        fn drop(&mut self) {
            if !self.tab.is_null() {
                // SAFETY: the table and length come from a successful bounded
                // enumeration, and this is the only owner of that allocation.
                unsafe {
                    qjs::JS_FreePropertyEnum(self.raw, self.tab, self.len);
                }
            }
        }
    }

    let raw = ctx.as_raw().as_ptr();
    let mut tab: *mut qjs::JSPropertyEnum = std::ptr::null_mut();
    let mut len = 0u32;
    let mut scanned = 0u32;

    // SAFETY: `raw` is the live context belonging to `ctx`, `object` belongs to
    // the same context, and the out pointers are valid stack locations.
    let status = unsafe {
        qjs::JS_GetOwnEnumerableStringKeysBounded(
            raw,
            object.as_raw(),
            max_keys,
            max_scan,
            &mut tab,
            &mut len,
            &mut scanned,
        )
    };

    let guard = EnumGuard { raw, tab, len };
    match status {
        OK => {}
        TOO_MANY_KEYS => return Err(BoundedKeysError::TooManyKeys),
        SCAN_LIMIT => return Err(BoundedKeysError::ScanLimit),
        UNSUPPORTED => return Err(BoundedKeysError::Unsupported),
        MEMORY => return Err(BoundedKeysError::Memory),
        _ => return Err(BoundedKeysError::Exception),
    }

    let mut keys = Vec::with_capacity(len as usize);
    for index in 0..len {
        // SAFETY: the guard holds a table of `len` initialised entries.
        let atom = unsafe { (*guard.tab.add(index as usize)).atom };
        let mut byte_len = 0usize;
        let mut ptr: *const c_char = std::ptr::null();
        // SAFETY: `atom` belongs to `raw` and is held by the guard; `ptr` and
        // `byte_len` are valid stack locations. The helper checks the key's
        // UTF-16 length before it allocates the UTF-8 string.
        let status = unsafe {
            qjs::JS_AtomToCStringLenBounded(raw, &mut byte_len, atom, max_key_len, &mut ptr)
        };
        match status {
            OK => {}
            TOO_MANY_KEYS => return Err(BoundedKeysError::TooManyKeys),
            UNSUPPORTED => return Err(BoundedKeysError::Unsupported),
            _ => return Err(BoundedKeysError::Exception),
        }
        // SAFETY: on success the helper returned a NUL-terminated buffer of
        // `byte_len` bytes owned by this context.
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, byte_len) };
        let key = std::str::from_utf8(bytes).map(str::to_owned);
        // SAFETY: `ptr` came from `JS_AtomToCStringLenBounded` and is freed once.
        unsafe {
            qjs::JS_FreeCString(raw, ptr);
        }
        let key = key.map_err(|_| BoundedKeysError::InvalidKey)?;
        keys.push(key);
    }
    Ok(keys)
}

fn bounded_option_error<'js>(ctx: &Ctx<'js>, error: BoundedKeysError) -> rquickjs::Error {
    match error {
        BoundedKeysError::InvalidKey => {
            coded_error(ctx, "ERR_NODE_UNSUPPORTED", "an option is not implemented")
        }
        BoundedKeysError::Exception => {
            let _ = ctx.catch();
            invalid(ctx, "options could not be read")
        }
        _ => coded_error(ctx, "ERR_NODE_UNSUPPORTED", "an option is not implemented"),
    }
}

fn bounded_env_error<'js>(ctx: &Ctx<'js>, error: BoundedKeysError) -> rquickjs::Error {
    match error {
        BoundedKeysError::Unsupported => coded_error(
            ctx,
            "ERR_NODE_UNSUPPORTED",
            "Proxy options.env is not supported by the native child_process module",
        ),
        BoundedKeysError::InvalidKey => invalid(ctx, "an environment name is not valid"),
        BoundedKeysError::Exception => {
            let _ = ctx.catch();
            request_too_large(ctx)
        }
        _ => request_too_large(ctx),
    }
}

fn read_env<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<BTreeMap<String, String>> {
    if !value.is_object() || value.is_array() {
        return Err(invalid(ctx, "options.env must be an object"));
    }
    let object = value.as_object().expect("checked object");
    let names = bounded_string_keys(
        ctx,
        object,
        MAX_ENV_ENTRIES as u32,
        MAX_ENV_SCAN,
        MAX_PATH_BYTES as u32,
    )
    .map_err(|error| bounded_env_error(ctx, error))?;
    let mut environment = BTreeMap::new();
    let mut total = 0usize;
    for name in names {
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

#[cfg(test)]
mod bounded_key_tests {
    use super::{BoundedKeysError, bounded_string_keys};
    use rquickjs::{Context, Runtime, Value};

    fn with_object<R>(
        script: &str,
        f: impl for<'js> FnOnce(&rquickjs::Ctx<'js>, &rquickjs::Object<'js>) -> R,
    ) -> R {
        let runtime = Runtime::new().expect("runtime");
        let context = Context::full(&runtime).expect("context");
        context.with(|ctx| {
            let value: Value = ctx.eval(script).expect("script");
            let object = value.as_object().expect("object");
            f(&ctx, object)
        })
    }

    fn keys(script: &str, max_keys: u32, max_scan: u32, max_key_len: u32) -> Vec<String> {
        with_object(script, |ctx, object| {
            bounded_string_keys(ctx, object, max_keys, max_scan, max_key_len).expect("keys")
        })
    }

    fn error(script: &str, max_keys: u32, max_scan: u32, max_key_len: u32) -> BoundedKeysError {
        with_object(script, |ctx, object| {
            match bounded_string_keys(ctx, object, max_keys, max_scan, max_key_len) {
                Ok(found) => panic!("expected an error, got {found:?}"),
                Err(error) => error,
            }
        })
    }

    #[test]
    fn integer_keys_are_ascending_then_other_keys_keep_insertion_order() {
        let found = keys(r#"({ b: 1, "10": 2, "2": 3, a: 4, "1": 5 })"#, 16, 64, 64);
        assert_eq!(found, ["1", "2", "10", "b", "a"]);
    }

    #[test]
    fn an_empty_object_needs_no_allocation() {
        assert!(keys("({})", 0, 0, 64).is_empty());
    }

    #[test]
    fn the_key_bound_is_inclusive_and_one_more_is_rejected() {
        let found = keys(
            "(() => { const o = {}; for (let i = 0; i < 128; i += 1) o[`k${i}`] = i; return o; })()",
            128,
            4_096,
            64,
        );
        assert_eq!(found.len(), 128);
        assert_eq!(
            error(
                "(() => { const o = {}; for (let i = 0; i < 129; i += 1) o[`k${i}`] = i; return o; })()",
                128,
                4_096,
                64,
            ),
            BoundedKeysError::TooManyKeys
        );
    }

    #[test]
    fn deleted_and_non_enumerable_slots_are_scanned_but_not_returned() {
        let found = keys(
            r#"(() => {
                const o = {};
                Object.defineProperty(o, "hidden", { value: 1, enumerable: false });
                o.visible = 1;
                o.gone = 1;
                delete o.gone;
                return o;
            })()"#,
            16,
            64,
            64,
        );
        assert_eq!(found, ["visible"]);
    }

    #[test]
    fn symbol_slots_count_against_the_scan_bound_and_are_not_returned() {
        let found = keys(
            "(() => { const o = { visible: 1 }; for (let i = 0; i < 10; i += 1) o[Symbol(`s${i}`)] = i; return o; })()",
            16,
            64,
            64,
        );
        assert_eq!(found, ["visible"]);
        assert_eq!(
            error(
                "(() => { const o = {}; for (let i = 0; i < 100; i += 1) o[Symbol(`s${i}`)] = i; return o; })()",
                16,
                64,
                64,
            ),
            BoundedKeysError::ScanLimit
        );
    }

    #[test]
    fn a_key_longer_than_the_bound_is_rejected_before_conversion() {
        assert_eq!(
            error(r#"({ ok: 1, ["x".repeat(100000)]: 2 })"#, 16, 64, 4_096,),
            BoundedKeysError::TooManyKeys
        );
    }

    #[test]
    fn arrays_and_typed_arrays_are_rejected_instead_of_dropping_indexes() {
        assert_eq!(
            error(r#"["a", "b"]"#, 16, 64, 64),
            BoundedKeysError::Unsupported
        );
        assert_eq!(
            error("new Uint8Array([1, 2])", 16, 64, 64),
            BoundedKeysError::Unsupported
        );
    }

    #[test]
    fn a_lone_surrogate_key_is_rejected_as_invalid() {
        assert_eq!(
            error(r#"({ "\uD800": 1 })"#, 16, 64, 64),
            BoundedKeysError::InvalidKey
        );
    }

    #[test]
    fn a_proxy_is_rejected_before_its_own_keys_trap_runs() {
        let runtime = Runtime::new().expect("runtime");
        let context = Context::full(&runtime).expect("context");
        context.with(|ctx| {
            let value: Value = ctx
                .eval(
                    r#"globalThis.trapCount = 0;
                       new Proxy({ a: 1 }, {
                           ownKeys(target) {
                               globalThis.trapCount += 1;
                               return Reflect.ownKeys(target);
                           },
                       })"#,
                )
                .expect("script");
            let object = value.as_object().expect("object");
            assert_eq!(
                match bounded_string_keys(&ctx, object, 16, 64, 64) {
                    Ok(found) => panic!("expected an error, got {found:?}"),
                    Err(error) => error,
                },
                BoundedKeysError::Unsupported
            );
            let traps: i64 = ctx.eval("globalThis.trapCount").expect("counter");
            assert_eq!(traps, 0);
        });
    }

    #[test]
    fn getters_are_not_read_during_enumeration() {
        let runtime = Runtime::new().expect("runtime");
        let context = Context::full(&runtime).expect("context");
        context.with(|ctx| {
            let value: Value = ctx
                .eval(
                    r#"globalThis.getterCount = 0;
                       ({
                           a: 1,
                           get boom() {
                               globalThis.getterCount += 1;
                               throw new Error("boom");
                           },
                       })"#,
                )
                .expect("script");
            let object = value.as_object().expect("object");
            let found = bounded_string_keys(&ctx, object, 16, 64, 64).expect("keys");
            assert_eq!(found, ["a", "boom"]);
            let gets: i64 = ctx.eval("globalThis.getterCount").expect("counter");
            assert_eq!(gets, 0);
        });
    }

    #[test]
    fn a_throwing_object_conversion_does_not_leak_the_snapshot() {
        // The guard must release the atom table when a later key is rejected;
        // repeating the failing call exercises the cleanup path.
        for _ in 0..64 {
            assert_eq!(
                error(r#"({ ok: 1, ["x".repeat(100000)]: 2 })"#, 16, 64, 4_096,),
                BoundedKeysError::TooManyKeys
            );
        }
    }
}

#![deny(unsafe_code)]
//! V8 adapter for opt-in Node compatibility.
//!
//! The runtime creates this adapter only from
//! `neojs_core::Runtime::with_node_compat`. It registers native synthetic
//! ECMAScript modules named `node:child_process` and `node:path` through
//! `JsRuntime::register_native_module`. The named exports are native V8
//! function objects; there is no JavaScript implementation, module facade, JSON
//! bridge, or host op.
//!
//! `node:child_process` calls the shared, engine-neutral
//! `neojs-node-builtins` runner directly. The runner enforces the executable
//! allowlist, an empty inherited environment, and finite time, input, output,
//! argument, and environment limits. `node:path` calls the shared pure-Rust
//! path algorithms. The native functions validate and copy guest values before
//! handing them to Rust; a getter or Proxy value is read at most once and never
//! coerced.
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
    PATH_SPECIFIER, PolicyError, SUPPORTED_MODULES, SyncRequest, SyncResult,
    is_known_node_bare_name, run_sync, unsupported_module_reason,
};

use neojs_node_builtins::MAX_PATH_AGGREGATE_BYTES;
use neojs_node_builtins::MAX_PATH_ARGS;
use neojs_node_builtins::MAX_PATH_INPUT_BYTES;
use neojs_node_builtins::PATH_TOO_LARGE_CODE;
use neojs_node_builtins::PathFlavor;
use neojs_node_builtins::PathObject;
use neojs_node_builtins::host_cwd;

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
    InvalidKey,
    KeyConversionFailed,
    ScanLimit,
    Unsupported,
    Memory,
    Exception,
}

#[allow(unsafe_code)]
unsafe extern "C" {
    /// Narrow bounded own-enumerable-string-key collector added by the
    /// vendored V8 source patch. Keys are delivered to `sink` while the V8
    /// handle scope is alive; no V8 handle crosses this ABI.
    fn neojs_v8_get_own_enumerable_string_keys_bounded(
        object: *const v8::Object,
        context: *const v8::Context,
        max_keys: u32,
        max_scan: u32,
        max_key_units: u32,
        sink: extern "C" fn(*mut std::ffi::c_void, *const u8, usize),
        sink_data: *mut std::ffi::c_void,
        out_len: *mut u32,
        out_scanned: *mut u32,
    ) -> i32;
}

#[allow(unsafe_code)]
extern "C" fn bounded_key_sink(data: *mut std::ffi::c_void, bytes: *const u8, length: usize) {
    if data.is_null() {
        return;
    }
    let keys = unsafe { &mut *(data as *mut Vec<String>) };
    if length == 0 {
        keys.push(String::new());
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(bytes, length) };
    keys.push(String::from_utf8_lossy(bytes).into_owned());
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
    let mut keys = Vec::new();
    let mut len = 0u32;
    let mut scanned = 0u32;
    let status = unsafe {
        neojs_v8_get_own_enumerable_string_keys_bounded(
            &**object as *const v8::Object,
            &*context as *const v8::Context,
            max_keys,
            max_scan,
            max_key_units as u32,
            bounded_key_sink,
            &mut keys as *mut Vec<String> as *mut std::ffi::c_void,
            &mut len,
            &mut scanned,
        )
    };
    let _ = scanned;
    match status {
        0 => {}
        1 => return Err(BoundedKeysError::TooManyKeys),
        2 => return Err(BoundedKeysError::ScanLimit),
        3 => return Err(BoundedKeysError::Unsupported),
        4 => return Err(BoundedKeysError::Memory),
        5 => return Err(BoundedKeysError::KeyTooLong),
        6 => return Err(BoundedKeysError::InvalidKey),
        7 => return Err(BoundedKeysError::KeyConversionFailed),
        _ => return Err(BoundedKeysError::Exception),
    }
    if keys.len() != len as usize {
        return Err(BoundedKeysError::KeyConversionFailed);
    }
    Ok(keys)
}

/// Installs the native `node:child_process` and `node:path` modules and the
/// process policy.
///
/// Registration happens before any user module can import either specifier.
/// Neither module has a default side effect or a global object.
pub fn install(js: &mut JsRuntime, options: NodeOptions) -> Result<(), String> {
    let context = js.main_context();
    let (child_process_exports, path_exports) = {
        v8::scope!(let scope, js.v8_isolate());
        let context_local = v8::Local::new(scope, context.clone());
        let scope = &mut v8::ContextScope::new(scope, context_local);
        (
            build_child_process_exports(scope)?,
            build_path_exports(scope)?,
        )
    };
    js.register_native_module(CHILD_PROCESS_SPECIFIER, child_process_exports)
        .map_err(|error| error.to_string())?;
    js.register_native_module(PATH_SPECIFIER, path_exports)
        .map_err(|error| error.to_string())?;
    // Install the policy only after the module registration is accepted. A
    // rejected duplicate must not replace the authority of the existing module.
    v8::scope!(let scope, js.v8_isolate());
    let context_local = v8::Local::new(scope, context);
    context_local.set_slot(Rc::new(options));
    Ok(())
}

fn build_child_process_exports<'s>(
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

fn build_path_exports<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<Vec<(FastStaticString, v8::Global<v8::Value>)>, String> {
    let posix = build_path_api(scope, PathFlavor::Posix)?;
    let win32 = build_path_api(scope, PathFlavor::Win32)?;
    let host = if PathFlavor::host() == PathFlavor::Posix {
        posix
    } else {
        win32
    };

    for (object, other_posix, other_win32) in [(&posix, &posix, &win32), (&win32, &posix, &win32)] {
        if !define_value(scope, object, "posix", (*other_posix).into())
            || !define_value(scope, object, "win32", (*other_win32).into())
        {
            return Err("cannot define the native path views".to_string());
        }
    }
    let mut exports = Vec::with_capacity(18);
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("resolve"),
        &host,
        "resolve",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("normalize"),
        &host,
        "normalize",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("isAbsolute"),
        &host,
        "isAbsolute",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("join"),
        &host,
        "join",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("relative"),
        &host,
        "relative",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("toNamespacedPath"),
        &host,
        "toNamespacedPath",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("dirname"),
        &host,
        "dirname",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("basename"),
        &host,
        "basename",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("extname"),
        &host,
        "extname",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("format"),
        &host,
        "format",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("parse"),
        &host,
        "parse",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("sep"),
        &host,
        "sep",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("delimiter"),
        &host,
        "delimiter",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("posix"),
        &host,
        "posix",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("win32"),
        &host,
        "win32",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("matchesGlob"),
        &host,
        "matchesGlob",
    )?;
    push_path_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("_makeLong"),
        &host,
        "_makeLong",
    )?;
    push_export(
        scope,
        &mut exports,
        deno_core::ascii_str!("default"),
        host.into(),
    );
    Ok(exports)
}

fn build_path_api<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    flavor: PathFlavor,
) -> Result<v8::Local<'s, v8::Object>, String> {
    let api = v8::Object::new(scope);
    let normalize = path_function(scope, path_normalize, flavor, "normalize")?;
    let is_absolute = path_function(scope, path_is_absolute, flavor, "isAbsolute")?;
    let join = path_function(scope, path_join, flavor, "join")?;
    let resolve = path_function(scope, path_resolve, flavor, "resolve")?;
    let relative = path_function(scope, path_relative, flavor, "relative")?;
    let to_namespaced = path_function(scope, path_to_namespaced_path, flavor, "toNamespacedPath")?;
    let dirname = path_function(scope, path_dirname, flavor, "dirname")?;
    let basename = path_function(scope, path_basename, flavor, "basename")?;
    let extname = path_function(scope, path_extname, flavor, "extname")?;
    let format = path_function(scope, path_format, flavor, "format")?;
    let parse = path_function(scope, path_parse, flavor, "parse")?;
    let matches_glob = native_function(scope, path_matches_glob, "matchesGlob")?;

    for (name, value) in [
        ("normalize", normalize),
        ("isAbsolute", is_absolute),
        ("join", join),
        ("resolve", resolve),
        ("relative", relative),
        ("toNamespacedPath", to_namespaced),
        ("dirname", dirname),
        ("basename", basename),
        ("extname", extname),
        ("format", format),
        ("parse", parse),
    ] {
        if !define_value(scope, &api, name, value.into()) {
            return Err(format!("cannot define the native path.{name} function"));
        }
    }
    if !define_value(scope, &api, "_makeLong", to_namespaced.into())
        || !define_value(scope, &api, "matchesGlob", matches_glob.into())
    {
        return Err("cannot define the native path aliases".to_string());
    }
    for (name, value) in [("sep", flavor.sep()), ("delimiter", flavor.delimiter())] {
        let Some(value) = v8::String::new(scope, value) else {
            return Err(format!("cannot allocate the native path.{name} constant"));
        };
        if !define_value(scope, &api, name, value.into()) {
            return Err(format!("cannot define the native path.{name} constant"));
        }
    }
    Ok(api)
}

fn path_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
    flavor: PathFlavor,
    name: &str,
) -> Result<v8::Local<'s, v8::Function>, String> {
    let data = v8::Integer::new(scope, flavor as i32).into();
    v8::Function::builder(callback)
        .data(data)
        .build(scope)
        .ok_or_else(|| format!("cannot allocate the native path.{name} function"))
}

fn push_path_export<'s>(
    scope: &v8::PinScope<'s, '_>,
    exports: &mut Vec<(FastStaticString, v8::Global<v8::Value>)>,
    name: FastStaticString,
    object: &v8::Local<'s, v8::Object>,
    key: &str,
) -> Result<(), String> {
    let key = v8::String::new(scope, key)
        .ok_or_else(|| "cannot allocate a path export key".to_string())?;
    let value = object
        .get(scope, key.into())
        .ok_or_else(|| "cannot read a path export".to_string())?;
    exports.push((name, v8::Global::new(scope, value)));
    Ok(())
}

fn path_flavor(
    scope: &v8::PinScope<'_, '_>,
    args: &v8::FunctionCallbackArguments<'_>,
) -> Result<PathFlavor, Failure> {
    let number = args.data().int32_value(scope).ok_or(Failure::Pending)?;
    Ok(if number == 0 {
        PathFlavor::Posix
    } else {
        PathFlavor::Win32
    })
}

fn path_arg<'s>(
    scope: &v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    what: &str,
) -> Result<String, Failure> {
    read_path_string(scope, args.get(index), what)
}

fn read_path_string<'s>(
    scope: &v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    what: &str,
) -> Result<String, Failure> {
    let string = v8::Local::<v8::String>::try_from(value)
        .map_err(|_| Failure::invalid_type(format!("{what} must be a string")))?;
    let length = string.length();
    if length > MAX_PATH_INPUT_BYTES {
        return Err(Failure::path_too_large());
    }
    let mut units = vec![0u16; length];
    if length > 0 {
        string.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
    }
    if !is_well_formed_utf16(&units) {
        return Err(Failure::invalid_value(
            "path strings must be well-formed Unicode",
        ));
    }
    let text = read_string(scope, value, what, MAX_PATH_INPUT_BYTES)?;
    if text.len() > MAX_PATH_INPUT_BYTES {
        return Err(Failure::path_too_large());
    }
    Ok(text)
}

fn is_well_formed_utf16(units: &[u16]) -> bool {
    let mut index = 0usize;
    while index < units.len() {
        let unit = units[index];
        if (0xD800..=0xDBFF).contains(&unit) {
            if index + 1 < units.len() && (0xDC00..=0xDFFF).contains(&units[index + 1]) {
                index += 2;
                continue;
            }
            return false;
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            return false;
        }
        index += 1;
    }
    true
}

fn path_optional_arg<'s>(
    scope: &v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
    what: &str,
) -> Result<Option<String>, Failure> {
    let value = args.get(index);
    if value.is_undefined() {
        Ok(None)
    } else {
        read_path_string(scope, value, what).map(Some)
    }
}

fn set_path_string<'s>(
    scope: &v8::PinScope<'s, '_>,
    rv: &mut v8::ReturnValue<'s, v8::Value>,
    value: String,
) {
    if let Some(value) = v8::String::new(scope, &value) {
        rv.set(value.into());
    }
}

fn set_path_error(scope: &v8::PinScope<'_, '_>, failure: Failure) {
    throw_failure(scope, failure);
}

fn path_normalize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let path = path_arg(scope, &args, 0, "path")?;
        Ok(neojs_node_builtins::normalize(flavor, &path))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_is_absolute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let path = path_arg(scope, &args, 0, "path")?;
        Ok(neojs_node_builtins::is_absolute(flavor, &path))
    })();
    match result {
        Ok(value) => rv.set(v8::Boolean::new(scope, value).into()),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn preflight_path_args<'s>(
    scope: &v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    count: usize,
) -> Result<(), Failure> {
    if count > MAX_PATH_ARGS {
        return Err(Failure::path_too_large());
    }
    let mut total = 0usize;
    for index in 0..count {
        let value = args.get(index as i32);
        if !value.is_string() {
            return Err(Failure::invalid_type("path must be a string"));
        }
        let string = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| Failure::invalid_type("path must be a string"))?;
        total = total.saturating_add(string.utf8_length(scope));
        if total > MAX_PATH_AGGREGATE_BYTES {
            return Err(Failure::path_too_large());
        }
    }
    Ok(())
}

fn path_cwd() -> Result<String, Failure> {
    host_cwd().map_err(|_| Failure::path_cwd())
}

fn path_join<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let count = args.length().max(0) as usize;
        preflight_path_args(scope, &args, count)?;
        let mut paths = Vec::with_capacity(count);
        for index in 0..count {
            paths.push(path_arg(scope, &args, index as i32, "path")?);
        }
        if paths.iter().map(String::len).sum::<usize>() > MAX_PATH_AGGREGATE_BYTES {
            return Err(Failure::path_too_large());
        }
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        Ok(neojs_node_builtins::join(flavor, &refs))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_resolve<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let count = args.length().max(0) as usize;
        preflight_path_args(scope, &args, count)?;
        let mut paths = Vec::with_capacity(count);
        for index in 0..count {
            paths.push(path_arg(scope, &args, index as i32, "path")?);
        }
        if paths.iter().map(String::len).sum::<usize>() > MAX_PATH_AGGREGATE_BYTES {
            return Err(Failure::path_too_large());
        }
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let needs_cwd = !paths
            .iter()
            .any(|path| neojs_node_builtins::is_absolute(flavor, path));
        let cwd = if needs_cwd {
            path_cwd()?
        } else {
            String::new()
        };
        Ok(neojs_node_builtins::resolve(flavor, &cwd, &refs))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_relative<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        preflight_path_args(scope, &args, 2)?;
        let from = path_arg(scope, &args, 0, "from")?;
        let to = path_arg(scope, &args, 1, "to")?;
        if from.len().saturating_add(to.len()) > MAX_PATH_AGGREGATE_BYTES {
            return Err(Failure::path_too_large());
        }
        let needs_cwd = !(neojs_node_builtins::is_absolute(flavor, &from)
            && neojs_node_builtins::is_absolute(flavor, &to));
        let cwd = if needs_cwd {
            path_cwd()?
        } else {
            String::new()
        };
        Ok(neojs_node_builtins::relative(flavor, &cwd, &from, &to))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_dirname<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let path = path_arg(scope, &args, 0, "path")?;
        Ok(neojs_node_builtins::dirname(flavor, &path))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_basename<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let path = path_arg(scope, &args, 0, "path")?;
        let suffix = path_optional_arg(scope, &args, 1, "suffix")?;
        Ok(neojs_node_builtins::basename(
            flavor,
            &path,
            suffix.as_deref(),
        ))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_extname<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let path = path_arg(scope, &args, 0, "path")?;
        Ok(neojs_node_builtins::extname(flavor, &path))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_parse<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let path = path_arg(scope, &args, 0, "path")?;
        Ok(neojs_node_builtins::parse(flavor, &path))
    })();
    match result {
        Ok(parsed) => {
            let object = v8::Object::new(scope);
            for (name, value) in [
                ("root", parsed.root),
                ("dir", parsed.dir),
                ("base", parsed.base),
                ("ext", parsed.ext),
                ("name", parsed.name),
            ] {
                let Some(value) = v8::String::new(scope, &value) else {
                    return;
                };
                if !define_value(scope, &object, name, value.into()) {
                    return;
                }
            }
            rv.set(object.into());
        }
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_format<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let value = args.get(0);
        if !value.is_object() || value.is_null() || value.is_function() {
            return Err(Failure::invalid_type("path.format requires an object"));
        }
        let object = v8::Local::<v8::Object>::try_from(value)
            .map_err(|_| Failure::invalid_type("path.format requires an object"))?;
        let root = read_optional_path_field(scope, &object, "root")?;
        let dir = read_optional_path_field(scope, &object, "dir")?;
        let base = read_optional_path_field(scope, &object, "base")?;
        let ext = read_optional_path_field(scope, &object, "ext")?;
        let name = read_optional_path_field(scope, &object, "name")?;
        let path_object = PathObject {
            root: root.as_deref(),
            dir: dir.as_deref(),
            base: base.as_deref(),
            ext: ext.as_deref(),
            name: name.as_deref(),
        };
        Ok(neojs_node_builtins::format(flavor, &path_object))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn read_optional_path_field<'s>(
    scope: &v8::PinScope<'s, '_>,
    object: &v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<Option<String>, Failure> {
    let key = v8::String::new(scope, name).ok_or(Failure::Pending)?;
    let value = object.get(scope, key.into()).ok_or(Failure::Pending)?;
    if value.is_undefined() {
        return Ok(None);
    }
    if !value.is_string() {
        return Err(Failure::invalid_type(format!(
            "path.format field `{name}` must be a string"
        )));
    }
    read_path_string(scope, value, name).map(Some)
}

fn path_to_namespaced_path<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let result = (|| {
        let flavor = path_flavor(scope, &args)?;
        let path = path_arg(scope, &args, 0, "path")?;
        let needs_cwd =
            flavor == PathFlavor::Win32 && !neojs_node_builtins::is_absolute(flavor, &path);
        let cwd = if needs_cwd {
            path_cwd()?
        } else {
            String::new()
        };
        Ok(neojs_node_builtins::to_namespaced_path(flavor, &cwd, &path))
    })();
    match result {
        Ok(value) => set_path_string(scope, &mut rv, value),
        Err(failure) => set_path_error(scope, failure),
    }
}

fn path_matches_glob<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    throw_coded(
        scope,
        "ERR_NODE_UNSUPPORTED",
        "node:path matchesGlob is not implemented",
    );
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

    fn out_of_memory() -> Self {
        Self::new(
            "ERR_MEMORY_ALLOCATION_FAILED",
            "a property-name buffer could not be allocated",
        )
    }

    fn path_too_large() -> Self {
        Self::new(
            PATH_TOO_LARGE_CODE,
            "the node:path request exceeds the hard size limit",
        )
    }

    fn path_cwd() -> Self {
        Self::new(
            "ERR_NODE_PATH_CWD",
            "the current working directory could not be read",
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
        BoundedKeysError::InvalidKey => Failure::unsupported("an option is not implemented"),
        BoundedKeysError::KeyConversionFailed => {
            Failure::invalid_value("an option name could not be converted to UTF-8")
        }
        BoundedKeysError::Memory => Failure::out_of_memory(),
        BoundedKeysError::Exception => Failure::Pending,
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
        BoundedKeysError::InvalidKey => Failure::invalid_value("an environment name is not valid"),
        BoundedKeysError::Unsupported => {
            Failure::unsupported("options.env is not supported by the native child_process module")
        }
        BoundedKeysError::KeyConversionFailed => {
            Failure::invalid_value("an environment name could not be converted to UTF-8")
        }
        BoundedKeysError::Memory => Failure::out_of_memory(),
        BoundedKeysError::Exception => Failure::Pending,
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

#[cfg(test)]
mod bounded_keys_gc_tests {
    use super::{BoundedKeysError, bounded_keys};
    use deno_core::JsRuntime;
    use deno_core::v8;

    #[allow(unsafe_code)]
    unsafe extern "C" {
        fn neojs_v8_reset_element_scan_probe();
        fn neojs_v8_element_scan_probe() -> u64;
        fn neojs_v8_test_set_key_conversion_failure(enabled: i32);
    }

    #[allow(unsafe_code)]
    #[test]
    fn keys_survive_gc_and_handle_churn_around_the_abi() {
        v8::V8::set_flags_from_string("--expose-gc");
        let mut js = JsRuntime::new(Default::default());
        js.execute_script(
            "<bounded-keys-gc>",
            "globalThis.boundedObject = { alpha: 1, beta: 2, gamma: 3 };",
        )
        .expect("script");

        let context = js.main_context();
        v8::scope!(let scope, js.v8_isolate());
        let context = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "boundedObject").expect("key");
        let value = global.get(scope, key.into()).expect("object");
        let object = v8::Local::<v8::Object>::try_from(value).expect("object");

        scope.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
        let keys = bounded_keys(scope, &object, 16, 64, 64).expect("bounded keys");
        scope.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
        for index in 0..2000 {
            let _ = v8::String::new(scope, &format!("handle-churn-{index}"));
        }
        assert_eq!(keys, ["alpha", "beta", "gamma"]);
    }

    #[allow(unsafe_code)]
    #[test]
    fn element_scan_budget_is_checked_before_any_slot_loop() {
        v8::V8::set_flags_from_string("--expose-gc");
        let mut js = JsRuntime::new(Default::default());
        js.execute_script(
            "<bounded-element-scan>",
            "globalThis.elementObject = { encoding: 'utf8' }; for (let i = 0; i < 100; i += 1) globalThis.elementObject[i] = i;",
        )
        .expect("script");

        let context = js.main_context();
        v8::scope!(let scope, js.v8_isolate());
        let context = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "elementObject").expect("key");
        let value = global.get(scope, key.into()).expect("object");
        let object = v8::Local::<v8::Object>::try_from(value).expect("object");

        unsafe { neojs_v8_reset_element_scan_probe() };
        assert_eq!(
            bounded_keys(scope, &object, 16, 0, 64),
            Err(BoundedKeysError::ScanLimit)
        );
        assert_eq!(
            unsafe { neojs_v8_element_scan_probe() },
            0,
            "element slots were inspected before the scan budget check"
        );

        unsafe { neojs_v8_reset_element_scan_probe() };
        let keys = bounded_keys(scope, &object, 200, 1_000, 64).expect("element keys");
        assert!(keys.iter().any(|key| key == "0"));
        let inspected = unsafe { neojs_v8_element_scan_probe() };
        assert!(
            inspected >= 100,
            "the probe did not count the fast element slots: {inspected}"
        );
    }

    #[allow(unsafe_code)]
    struct ConversionFailureGuard;

    #[allow(unsafe_code)]
    impl Drop for ConversionFailureGuard {
        fn drop(&mut self) {
            unsafe { neojs_v8_test_set_key_conversion_failure(0) };
        }
    }

    #[allow(unsafe_code)]
    #[test]
    fn conversion_failure_status_produces_a_coded_error_not_undefined() {
        let mut js = JsRuntime::new(Default::default());
        js.execute_script(
            "<bounded-key-conversion-failure>",
            "globalThis.options = { env: {} };",
        )
        .expect("script");

        let context = js.main_context();
        v8::scope!(let scope, js.v8_isolate());
        let context = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "options").expect("key");
        let value = global.get(scope, key.into()).expect("object");
        let object = v8::Local::<v8::Object>::try_from(value).expect("object");

        unsafe { neojs_v8_test_set_key_conversion_failure(1) };
        let _guard = ConversionFailureGuard;
        assert_eq!(
            bounded_keys(scope, &object, 16, 64, 64),
            Err(BoundedKeysError::KeyConversionFailed)
        );

        unsafe { neojs_v8_test_set_key_conversion_failure(1) };
        let failure = super::reject_unknown_options(scope, &object)
            .expect_err("conversion failure must become a coded error");
        match failure {
            super::Failure::Coded { code, .. } => {
                assert_eq!(code, "ERR_INVALID_ARG_VALUE");
            }
            super::Failure::Pending => {
                panic!("a conversion failure became Pending and could return undefined");
            }
        }
    }

    #[allow(unsafe_code)]
    fn element_scan_count(elements: usize, barrier: &std::sync::Barrier) -> u64 {
        let mut js = JsRuntime::new(Default::default());
        let source = format!(
            "globalThis.elementObject = {{}}; for (let i = 0; i < {elements}; i += 1) globalThis.elementObject[i] = i;"
        );
        js.execute_script("<bounded-element-thread>", source)
            .expect("script");

        let context = js.main_context();
        v8::scope!(let scope, js.v8_isolate());
        let context = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let global = scope.get_current_context().global(scope);
        let key = v8::String::new(scope, "elementObject").expect("key");
        let value = global.get(scope, key.into()).expect("object");
        let object = v8::Local::<v8::Object>::try_from(value).expect("object");

        unsafe { neojs_v8_reset_element_scan_probe() };
        // Start both scans together so a process-global counter would mix the
        // two isolates' slots instead of passing sequentially.
        barrier.wait();
        let keys = bounded_keys(scope, &object, 512, 2_000, 64).expect("element keys");
        assert_eq!(keys.len(), elements);
        unsafe { neojs_v8_element_scan_probe() }
    }

    #[allow(unsafe_code)]
    #[test]
    fn element_scan_probe_is_thread_local_across_parallel_isolates() {
        let barrier = std::sync::Barrier::new(2);
        let (first, second) = std::thread::scope(|scope| {
            let first = scope.spawn(|| element_scan_count(100, &barrier));
            let second = scope.spawn(|| element_scan_count(200, &barrier));
            (
                first.join().expect("first thread"),
                second.join().expect("second thread"),
            )
        });
        assert!(
            (100..200).contains(&first),
            "first thread observed {first} element slots"
        );
        assert!(
            (200..300).contains(&second),
            "second thread observed {second} element slots"
        );
    }
}

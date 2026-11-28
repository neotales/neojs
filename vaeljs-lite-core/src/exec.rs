//! Native `vael:exec` synchronous bindings for QuickJS.
//!
//! All argv-only process spawning, concurrent bounded stdio draining, timeout
//! escalation, PATH lookup, advisory file locking, and `VAEL_EXEC_*` error
//! codes live in [`vaeljs_exec_core`]. This module wires synchronous QuickJS
//! host operations into `exec.js` (which captures ECMAScript primordials at
//! realm initialization), tracks active [`FileLock`] handles in per-context
//! [`ExecLockTable`] state so locks release on `unlock()`, `[Symbol.dispose]()`,
//! GC, or [`crate::Runtime`] drop, and exports the identical frozen registry
//! object across `Vael.getBuiltInModule("vael:exec")` and
//! `import ... from "vael:exec"`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use rquickjs::{
    Array, Class, Constructor, Ctx, Exception, Function, JsLifetime, Object, Result,
    String as JsString, TypedArray, Value,
    class::{JsClass, Readable, Trace, Tracer},
    function::Rest,
    module::ModuleDef,
    object::Property,
    qjs,
};
use vaeljs_exec_core::{
    CommandSpec, ExecError, ExecErrorCode, FileLock, LockMode, LockOptions, MAX_OUTPUT_BYTES,
    Signal, StdioMode, exec_sync, run_sync, which, which_all,
};

use crate::native::{
    freeze_native, own_array_element, own_data_property, parse_options, with_uint8_array_bytes,
};

/// Canonical module specifier for `vael:exec`.
pub(super) const SPECIFIER: &str = vaeljs_exec_core::SPECIFIER;

/// Exact sorted named exports of `vael:exec`.
pub(super) const EXPORTS: &[&str] = &[
    "Command",
    "CommandError",
    "FileLock",
    "NotFoundOnPathError",
    "Output",
    "cmd",
    "execSync",
    "lockFileSync",
    "runSync",
    "whichAllSync",
    "whichSync",
];

const FACTORY_SOURCE: &str = include_str!("exec.js");
const MAX_ARRAY_LEN: usize = 16_384;
const MAX_OPEN_LOCKS: usize = 1_024;

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

/// Per-context table owning active OS advisory file locks so dropping the
/// QuickJS context releases any locks that were not explicitly unlocked.
#[derive(Default)]
pub(super) struct ExecLockTable {
    locks: BTreeMap<u64, FileLock>,
    next_id: u64,
}

impl ExecLockTable {
    pub(super) fn new() -> Self {
        Self::default()
    }

    fn is_full(&self) -> bool {
        self.locks.len() >= MAX_OPEN_LOCKS
    }

    fn insert(&mut self, lock: FileLock) -> Option<u64> {
        if self.is_full() {
            return None;
        }
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.locks.insert(id, lock);
        Some(id)
    }

    fn take(&mut self, id: u64) -> Option<FileLock> {
        self.locks.remove(&id)
    }

    fn contains(&self, id: u64) -> bool {
        self.locks.contains_key(&id)
    }
}

struct FileLockBrand {
    id: u64,
    table: Weak<RefCell<ExecLockTable>>,
}

impl Drop for FileLockBrand {
    fn drop(&mut self) {
        if let Some(table) = self.table.upgrade() {
            let _ = table.borrow_mut().take(self.id);
        }
    }
}

impl<'js> Trace<'js> for FileLockBrand {
    fn trace<'a>(&self, _tracer: Tracer<'a, 'js>) {}
}

unsafe impl<'js> JsLifetime<'js> for FileLockBrand {
    type Changed<'to> = FileLockBrand;
}

impl<'js> JsClass<'js> for FileLockBrand {
    const NAME: &'static str = "FileLockHandle";
    type Mutable = Readable;

    fn constructor(_ctx: &Ctx<'js>) -> Result<Option<Constructor<'js>>> {
        Ok(None)
    }
}

/// Creates the frozen public registry object for `vael:exec`.
pub(super) fn object<'js>(
    ctx: &Ctx<'js>,
    _locks: &Rc<RefCell<ExecLockTable>>,
) -> Result<Object<'js>> {
    let factory: Function<'js> = ctx.eval(FACTORY_SOURCE)?;
    let host = Object::new(ctx.clone())?;
    host.set("maxOutputBytes", MAX_OUTPUT_BYTES as f64)?;
    host.set(
        "validateSignal",
        Function::new(ctx.clone(), host_validate_signal)?
            .with_name("validateSignal")?
            .with_length(1)?,
    )?;
    host.set(
        "validateTimeoutMs",
        Function::new(ctx.clone(), host_validate_timeout_ms)?
            .with_name("validateTimeoutMs")?
            .with_length(1)?,
    )?;
    host.set(
        "parseStringArray",
        Function::new(ctx.clone(), host_parse_string_array)?
            .with_name("parseStringArray")?
            .with_length(2)?,
    )?;
    host.set(
        "parseEnvObject",
        Function::new(ctx.clone(), host_parse_env_object)?
            .with_name("parseEnvObject")?
            .with_length(1)?,
    )?;
    host.set(
        "parseCommandOptions",
        Function::new(ctx.clone(), host_parse_command_options)?
            .with_name("parseCommandOptions")?
            .with_length(1)?,
    )?;
    host.set(
        "execSync",
        Function::new(ctx.clone(), host_exec_sync)?
            .with_name("execSync")?
            .with_length(1)?,
    )?;
    host.set(
        "runSync",
        Function::new(ctx.clone(), host_run_sync)?
            .with_name("runSync")?
            .with_length(1)?,
    )?;
    host.set(
        "whichSync",
        Function::new(ctx.clone(), host_which_sync)?
            .with_name("whichSync")?
            .with_length(2)?,
    )?;
    host.set(
        "whichAllSync",
        Function::new(ctx.clone(), host_which_all_sync)?
            .with_name("whichAllSync")?
            .with_length(2)?,
    )?;
    host.set(
        "lockFileSync",
        Function::new(ctx.clone(), host_lock_file_sync)?
            .with_name("lockFileSync")?
            .with_length(2)?,
    )?;
    host.set(
        "lockUnlock",
        Function::new(ctx.clone(), host_lock_unlock)?
            .with_name("lockUnlock")?
            .with_length(1)?,
    )?;
    host.set(
        "lockIsLocked",
        Function::new(ctx.clone(), host_lock_is_locked)?
            .with_name("lockIsLocked")?
            .with_length(1)?,
    )?;
    host.set(
        "decodeUtf8Lossy",
        Function::new(ctx.clone(), host_decode_utf8_lossy)?
            .with_name("decodeUtf8Lossy")?
            .with_length(1)?,
    )?;
    host.set(
        "linesLossy",
        Function::new(ctx.clone(), host_lines_lossy)?
            .with_name("linesLossy")?
            .with_length(1)?,
    )?;
    let module: Object<'js> = factory.call((host,))?;
    freeze_native(ctx, &module)?;
    Ok(module)
}

fn error_object<'js>(ctx: &Ctx<'js>, err: &ExecError) -> Result<Object<'js>> {
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
    if matches!(err, ExecError::NotFound { .. }) {
        obj.prop(
            "notFound",
            Property::from(true).writable().enumerable().configurable(),
        )?;
    }
    Ok(obj)
}

fn ok_object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    set_null_proto(ctx, &obj)?;
    obj.prop(
        "ok",
        Property::from(true).writable().enumerable().configurable(),
    )?;
    Ok(obj)
}

fn read_strict_string<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    surrogate_reason: &'static str,
) -> Result<std::result::Result<String, ExecError>> {
    if !value.is_string() {
        return Err(Exception::throw_type(ctx, type_msg));
    }
    let js_str =
        JsString::from_value(value.clone()).map_err(|_| Exception::throw_type(ctx, type_msg))?;
    match js_str.to_string() {
        Ok(text) => Ok(Ok(text)),
        Err(rquickjs::Error::FromJs { .. } | rquickjs::Error::Utf8(_)) => {
            Ok(Err(ExecError::InvalidArgument {
                reason: surrogate_reason,
            }))
        }
        Err(other) => Err(other),
    }
}

fn check_no_nul(text: &str, reason: &'static str) -> std::result::Result<(), ExecError> {
    if text.contains('\0') {
        Err(ExecError::InvalidArgument { reason })
    } else {
        Ok(())
    }
}

fn parse_duration_ms(
    ctx: &Ctx<'_>,
    value: &Value<'_>,
    allow_zero: bool,
    label: &str,
) -> Result<std::result::Result<Duration, ExecError>> {
    if !value.is_number() {
        return Err(Exception::throw_type(
            ctx,
            &format!("{label} must be a number"),
        ));
    }
    let Some(ms) = value.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            &format!("{label} must be a number"),
        ));
    };
    if !ms.is_finite() || (allow_zero && ms < 0.0) || (!allow_zero && ms <= 0.0) {
        return Ok(Err(ExecError::InvalidArgument {
            reason: if allow_zero {
                "duration must be a finite non-negative number"
            } else {
                "timeout must be greater than zero"
            },
        }));
    }
    let secs = ms / 1000.0;
    let Some(dur) = Duration::try_from_secs_f64(secs).ok() else {
        return Ok(Err(ExecError::InvalidArgument {
            reason: "duration overflows monotonic clock",
        }));
    };
    if !allow_zero && dur.is_zero() {
        return Ok(Err(ExecError::InvalidArgument {
            reason: "timeout must be greater than zero",
        }));
    }
    if Instant::now().checked_add(dur).is_none() {
        return Ok(Err(ExecError::InvalidArgument {
            reason: "duration overflows monotonic clock",
        }));
    }
    Ok(Ok(dur))
}

fn parse_string_array_inner<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    label: &str,
    reject_empty: bool,
) -> Result<std::result::Result<Vec<String>, ExecError>> {
    if !value.is_object() || value.is_proxy() || !unsafe { qjs::JS_IsArray(value.as_raw()) } {
        return Err(Exception::throw_type(
            ctx,
            &format!("{label} must be an array of strings"),
        ));
    }
    let Some(arr) = value.as_array() else {
        return Err(Exception::throw_type(
            ctx,
            &format!("{label} must be an array of strings"),
        ));
    };
    let len = arr.len();
    if len > MAX_ARRAY_LEN {
        return Ok(Err(ExecError::InvalidArgument {
            reason: "argument list exceeds maximum length",
        }));
    }
    let mut out = Vec::with_capacity(len);
    for idx in 0..len as u32 {
        let Some(elem) = own_array_element(
            ctx,
            arr,
            idx,
            "array elements must not use accessor properties",
        )?
        else {
            return Err(Exception::throw_type(
                ctx,
                &format!("{label} must not contain sparse holes"),
            ));
        };
        let text = match read_strict_string(
            ctx,
            &elem,
            &format!("{label} elements must be strings"),
            "argument contains an unpaired UTF-16 surrogate",
        )? {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        if reject_empty && text.is_empty() {
            return Ok(Err(ExecError::InvalidArgument {
                reason: "entry must not be empty",
            }));
        }
        if let Err(e) = check_no_nul(&text, "process argument must not contain a NUL byte") {
            return Ok(Err(e));
        }
        out.push(text);
    }
    Ok(Ok(out))
}

fn parse_env_object_inner<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
) -> Result<std::result::Result<Vec<(String, String)>, ExecError>> {
    if value.is_undefined() {
        return Ok(Ok(Vec::new()));
    }
    let mut pairs = Vec::new();
    let mut err_out: Option<ExecError> = None;
    parse_options(
        ctx,
        Some(value),
        "env must be a plain object",
        |ctx, key, val| {
            if err_out.is_some() || val.is_undefined() {
                return Ok(());
            }
            if key.is_empty() || key.contains('=') || key.contains('\0') {
                err_out = Some(ExecError::InvalidArgument {
                    reason: "environment variable key must be non-empty and contain neither '=' nor NUL",
                });
                return Ok(());
            }
            match read_strict_string(
                ctx,
                val,
                "environment variable value must be a string",
                "environment variable value contains an unpaired UTF-16 surrogate",
            )? {
                Ok(v) => {
                    if v.contains('\0') {
                        err_out = Some(ExecError::InvalidArgument {
                            reason: "environment variable value must not contain a NUL byte",
                        });
                    } else {
                        pairs.push((key.to_owned(), v));
                    }
                }
                Err(e) => err_out = Some(e),
            }
            Ok(())
        },
    )?;
    if let Some(e) = err_out {
        return Ok(Err(e));
    }
    Ok(Ok(pairs))
}

fn host_validate_signal<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "signal must be a string"));
    };
    let text = match read_strict_string(
        &ctx,
        val,
        "signal must be a string",
        "signal name contains an unpaired UTF-16 surrogate",
    )? {
        Ok(s) => s,
        Err(e) => return error_object(&ctx, &e),
    };
    match Signal::parse(&text) {
        Ok(sig) => {
            let out = ok_object(&ctx)?;
            out.prop(
                "signal",
                Property::from(sig.as_str())
                    .writable()
                    .enumerable()
                    .configurable(),
            )?;
            Ok(out)
        }
        Err(e) => error_object(&ctx, &e),
    }
}

fn host_validate_timeout_ms<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "timeout must be a number"));
    };
    match parse_duration_ms(&ctx, val, false, "timeout")? {
        Ok(_) => ok_object(&ctx),
        Err(e) => error_object(&ctx, &e),
    }
}

fn host_parse_string_array<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 2 {
        return Err(Exception::throw_type(
            &ctx,
            "parseStringArray requires 2 arguments",
        ));
    }
    let label = JsString::from_value(args.0[1].clone())
        .and_then(|s| s.to_string())
        .unwrap_or_else(|_| "args".to_owned());
    let items = match parse_string_array_inner(&ctx, &args.0[0], &label, false)? {
        Ok(items) => items,
        Err(e) => return error_object(&ctx, &e),
    };
    let arr = Array::new(ctx.clone())?;
    for (idx, item) in items.into_iter().enumerate() {
        arr.as_object().prop(
            idx as u32,
            Property::from(item).writable().enumerable().configurable(),
        )?;
    }
    let out = ok_object(&ctx)?;
    out.prop(
        "items",
        Property::from(arr).writable().enumerable().configurable(),
    )?;
    Ok(out)
}

fn host_parse_env_object<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "env must be a plain object"));
    };
    if val.is_undefined() {
        return Err(Exception::throw_type(&ctx, "env must be a plain object"));
    }
    let pairs = match parse_env_object_inner(&ctx, val)? {
        Ok(pairs) => pairs,
        Err(e) => return error_object(&ctx, &e),
    };
    let arr = pairs_to_js_array(&ctx, pairs)?;
    let out = ok_object(&ctx)?;
    out.prop(
        "pairs",
        Property::from(arr).writable().enumerable().configurable(),
    )?;
    Ok(out)
}

fn pairs_to_js_array<'js>(ctx: &Ctx<'js>, pairs: Vec<(String, String)>) -> Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    for (idx, (k, v)) in pairs.into_iter().enumerate() {
        let pair = Array::new(ctx.clone())?;
        pair.as_object().prop(
            0u32,
            Property::from(k).writable().enumerable().configurable(),
        )?;
        pair.as_object().prop(
            1u32,
            Property::from(v).writable().enumerable().configurable(),
        )?;
        arr.as_object().prop(
            idx as u32,
            Property::from(pair).writable().enumerable().configurable(),
        )?;
    }
    Ok(arr)
}

#[derive(Default)]
struct RawCommandOptions<'js> {
    args: Option<Value<'js>>,
    cwd: Option<Value<'js>>,
    env: Option<Value<'js>>,
    clear_env: Option<Value<'js>>,
    env_remove: Option<Value<'js>>,
    stdin: Option<Value<'js>>,
    stdout: Option<Value<'js>>,
    stderr: Option<Value<'js>>,
    timeout_ms: Option<Value<'js>>,
    timeout: Option<Value<'js>>,
    max_output_bytes: Option<Value<'js>>,
    kill_signal: Option<Value<'js>>,
    kill_grace_ms: Option<Value<'js>>,
    kill_grace: Option<Value<'js>>,
}

fn host_parse_command_options<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let mut fields = RawCommandOptions::default();
    let mut unknown = false;
    parse_options(
        &ctx,
        args.0.first(),
        "command options must be a plain object",
        |_ctx, key, val| {
            if unknown {
                return Ok(());
            }
            match key {
                "args" => fields.args = Some(val.clone()),
                "cwd" => fields.cwd = Some(val.clone()),
                "env" => fields.env = Some(val.clone()),
                "clearEnv" => fields.clear_env = Some(val.clone()),
                "envRemove" => fields.env_remove = Some(val.clone()),
                "stdin" => fields.stdin = Some(val.clone()),
                "stdout" => fields.stdout = Some(val.clone()),
                "stderr" => fields.stderr = Some(val.clone()),
                "timeoutMs" => fields.timeout_ms = Some(val.clone()),
                "timeout" => fields.timeout = Some(val.clone()),
                "maxOutputBytes" => fields.max_output_bytes = Some(val.clone()),
                "killSignal" => fields.kill_signal = Some(val.clone()),
                "killGraceMs" => fields.kill_grace_ms = Some(val.clone()),
                "killGrace" => fields.kill_grace = Some(val.clone()),
                _ => unknown = true,
            }
            Ok(())
        },
    )?;
    if unknown {
        return Err(Exception::throw_type(&ctx, "unknown command option"));
    }

    let out = ok_object(&ctx)?;

    if let Some(ref val) = fields.args
        && !val.is_undefined()
    {
        let items = match parse_string_array_inner(&ctx, val, "options.args", false)? {
            Ok(items) => items,
            Err(e) => return error_object(&ctx, &e),
        };
        let arr = Array::new(ctx.clone())?;
        for (idx, item) in items.into_iter().enumerate() {
            arr.as_object().prop(
                idx as u32,
                Property::from(item).writable().enumerable().configurable(),
            )?;
        }
        out.prop(
            "args",
            Property::from(arr).writable().enumerable().configurable(),
        )?;
    }

    if let Some(ref val) = fields.cwd
        && !val.is_undefined()
    {
        let cwd = match read_strict_string(
            &ctx,
            val,
            "options.cwd must be a string",
            "working directory contains an unpaired UTF-16 surrogate",
        )? {
            Ok(s) => s,
            Err(e) => return error_object(&ctx, &e),
        };
        if cwd.is_empty() || cwd.contains('\0') {
            return error_object(
                &ctx,
                &ExecError::InvalidArgument {
                    reason: "working directory must not be empty or contain NUL",
                },
            );
        }
        out.prop(
            "cwd",
            Property::from(cwd).writable().enumerable().configurable(),
        )?;
    }

    if let Some(ref flag_val) = fields.clear_env
        && !flag_val.is_undefined()
    {
        let Some(b) = flag_val.as_bool() else {
            return Err(Exception::throw_type(
                &ctx,
                "options.clearEnv must be a boolean",
            ));
        };
        if b {
            out.prop(
                "clearEnv",
                Property::from(true).writable().enumerable().configurable(),
            )?;
        }
    }

    if let Some(ref val) = fields.env
        && !val.is_undefined()
    {
        let pairs = match parse_env_object_inner(&ctx, val)? {
            Ok(pairs) => pairs,
            Err(e) => return error_object(&ctx, &e),
        };
        let arr = pairs_to_js_array(&ctx, pairs)?;
        out.prop(
            "env",
            Property::from(arr).writable().enumerable().configurable(),
        )?;
    }

    if let Some(ref val) = fields.env_remove
        && !val.is_undefined()
    {
        let keys = match parse_string_array_inner(&ctx, val, "options.envRemove", true)? {
            Ok(keys) => keys,
            Err(e) => return error_object(&ctx, &e),
        };
        let arr = Array::new(ctx.clone())?;
        for (idx, k) in keys.into_iter().enumerate() {
            if k.contains('=') {
                return error_object(
                    &ctx,
                    &ExecError::InvalidArgument {
                        reason: "environment variable key must not contain '='",
                    },
                );
            }
            arr.as_object().prop(
                idx as u32,
                Property::from(k).writable().enumerable().configurable(),
            )?;
        }
        out.prop(
            "envRemove",
            Property::from(arr).writable().enumerable().configurable(),
        )?;
    }

    if let Some(ref val) = fields.stdin
        && !val.is_undefined()
    {
        let stdin_obj = Object::new(ctx.clone())?;
        if val.is_string() {
            let s = match read_strict_string(
                &ctx,
                val,
                "options.stdin must be a string or Uint8Array",
                "stdin string contains an unpaired UTF-16 surrogate",
            )? {
                Ok(s) => s,
                Err(e) => return error_object(&ctx, &e),
            };
            if s == "inherit" || s == "piped" || s == "null" {
                stdin_obj.prop(
                    "kind",
                    Property::from(s).writable().enumerable().configurable(),
                )?;
            } else {
                stdin_obj.prop(
                    "kind",
                    Property::from("text")
                        .writable()
                        .enumerable()
                        .configurable(),
                )?;
                stdin_obj.prop(
                    "data",
                    Property::from(s).writable().enumerable().configurable(),
                )?;
            }
        } else {
            let bytes = with_uint8_array_bytes(
                &ctx,
                val,
                "options.stdin must be a string or Uint8Array",
                |slice| Ok(slice.to_vec()),
            )?;
            let ta = TypedArray::<u8>::new(ctx.clone(), bytes)?;
            stdin_obj.prop(
                "kind",
                Property::from("bytes")
                    .writable()
                    .enumerable()
                    .configurable(),
            )?;
            stdin_obj.prop(
                "data",
                Property::from(ta).writable().enumerable().configurable(),
            )?;
        }
        out.prop(
            "stdin",
            Property::from(stdin_obj)
                .writable()
                .enumerable()
                .configurable(),
        )?;
    }

    for (key, field_val) in [("stdout", fields.stdout), ("stderr", fields.stderr)] {
        if let Some(ref val) = field_val
            && !val.is_undefined()
        {
            let mode = match read_strict_string(
                &ctx,
                val,
                &format!("options.{key} must be a string"),
                "stdio mode contains an unpaired UTF-16 surrogate",
            )? {
                Ok(s) => s,
                Err(e) => return error_object(&ctx, &e),
            };
            if mode != "inherit" && mode != "piped" && mode != "null" {
                return error_object(
                    &ctx,
                    &ExecError::InvalidArgument {
                        reason: "stdio mode must be 'inherit', 'piped', or 'null'",
                    },
                );
            }
            out.prop(
                key,
                Property::from(mode).writable().enumerable().configurable(),
            )?;
        }
    }

    let timeout_val = match (fields.timeout_ms, fields.timeout) {
        (Some(a), Some(b)) if !a.is_undefined() && !b.is_undefined() => {
            return error_object(
                &ctx,
                &ExecError::InvalidArgument {
                    reason: "cannot specify both timeout and timeoutMs",
                },
            );
        }
        (Some(a), _) if !a.is_undefined() => Some(a),
        (_, Some(b)) if !b.is_undefined() => Some(b),
        _ => None,
    };
    if let Some(val) = timeout_val {
        if let Err(e) = parse_duration_ms(&ctx, &val, false, "timeout")? {
            return error_object(&ctx, &e);
        }
        out.prop(
            "timeoutMs",
            Property::from(val).writable().enumerable().configurable(),
        )?;
    }

    if let Some(ref val) = fields.max_output_bytes
        && !val.is_undefined()
    {
        if !val.is_number() {
            return Err(Exception::throw_type(
                &ctx,
                "options.maxOutputBytes must be a number",
            ));
        }
        let Some(num) = val.as_number() else {
            return Err(Exception::throw_type(
                &ctx,
                "options.maxOutputBytes must be a number",
            ));
        };
        if !num.is_finite() || num.fract() != 0.0 || num < 0.0 || num > MAX_OUTPUT_BYTES as f64 {
            return error_object(
                &ctx,
                &ExecError::InvalidArgument {
                    reason: "maxOutputBytes exceeds hard limit of 64 MiB",
                },
            );
        }
        out.prop(
            "maxOutputBytes",
            Property::from(num).writable().enumerable().configurable(),
        )?;
    }

    if let Some(ref val) = fields.kill_signal
        && !val.is_undefined()
    {
        let sig_str = match read_strict_string(
            &ctx,
            val,
            "options.killSignal must be a string",
            "signal contains an unpaired UTF-16 surrogate",
        )? {
            Ok(s) => s,
            Err(e) => return error_object(&ctx, &e),
        };
        let sig = match Signal::parse(&sig_str) {
            Ok(s) => s,
            Err(e) => return error_object(&ctx, &e),
        };
        out.prop(
            "killSignal",
            Property::from(sig.as_str())
                .writable()
                .enumerable()
                .configurable(),
        )?;
    }

    let kill_grace_val = match (fields.kill_grace_ms, fields.kill_grace) {
        (Some(a), Some(b)) if !a.is_undefined() && !b.is_undefined() => {
            return error_object(
                &ctx,
                &ExecError::InvalidArgument {
                    reason: "cannot specify both killGrace and killGraceMs",
                },
            );
        }
        (Some(a), _) if !a.is_undefined() => Some(a),
        (_, Some(b)) if !b.is_undefined() => Some(b),
        _ => None,
    };
    if let Some(val) = kill_grace_val {
        if let Err(e) = parse_duration_ms(&ctx, &val, true, "killGrace")? {
            return error_object(&ctx, &e);
        }
        out.prop(
            "killGraceMs",
            Property::from(val).writable().enumerable().configurable(),
        )?;
    }

    Ok(out)
}

fn build_spec_from_js<'js>(
    ctx: &Ctx<'js>,
    spec_val: &Value<'js>,
) -> Result<std::result::Result<CommandSpec, ExecError>> {
    let raw_obj = spec_val.as_raw();
    let Some(program_val) = own_data_property(ctx, raw_obj, c"program", "invalid command spec")?
    else {
        return Err(Exception::throw_type(ctx, "invalid command spec"));
    };
    let program = match read_strict_string(
        ctx,
        &program_val,
        "program must be a string",
        "program name contains an unpaired UTF-16 surrogate",
    )? {
        Ok(s) => s,
        Err(e) => return Ok(Err(e)),
    };
    let mut spec = match CommandSpec::new(program) {
        Ok(s) => s,
        Err(e) => return Ok(Err(e)),
    };

    if let Some(args_val) = own_data_property(ctx, raw_obj, c"args", "invalid command spec")? {
        let args = match parse_string_array_inner(ctx, &args_val, "args", false)? {
            Ok(a) => a,
            Err(e) => return Ok(Err(e)),
        };
        spec = match spec.args(args) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
    }

    if let Some(cwd_val) = own_data_property(ctx, raw_obj, c"cwd", "invalid command spec")?
        && !cwd_val.is_undefined()
    {
        let cwd = match read_strict_string(
            ctx,
            &cwd_val,
            "cwd must be a string",
            "working directory contains an unpaired UTF-16 surrogate",
        )? {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        spec = match spec.cwd(cwd) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
    }

    if let Some(clear_val) = own_data_property(ctx, raw_obj, c"clearEnv", "invalid command spec")?
        && clear_val.as_bool() == Some(true)
    {
        spec = spec.env_clear();
    }

    if let Some(ops_val) = own_data_property(ctx, raw_obj, c"envOps", "invalid command spec")?
        && !ops_val.is_undefined()
        && let Some(ops_arr) = ops_val.as_array()
    {
        for idx in 0..ops_arr.len() as u32 {
            let Some(op_val) = own_array_element(ctx, ops_arr, idx, "invalid env op")? else {
                continue;
            };
            let Some(op_arr) = op_val.as_array() else {
                continue;
            };
            let Some(kind_val) = own_array_element(ctx, op_arr, 0, "invalid env op")? else {
                continue;
            };
            let kind = JsString::from_value(kind_val)
                .and_then(|s| s.to_string())
                .unwrap_or_default();
            let Some(key_val) = own_array_element(ctx, op_arr, 1, "invalid env op")? else {
                continue;
            };
            let key = match read_strict_string(
                ctx,
                &key_val,
                "env key must be a string",
                "env key contains an unpaired UTF-16 surrogate",
            )? {
                Ok(k) => k,
                Err(e) => return Ok(Err(e)),
            };
            if kind == "remove" {
                spec = match spec.env_remove(key) {
                    Ok(s) => s,
                    Err(e) => return Ok(Err(e)),
                };
            } else if kind == "set" {
                let Some(val_val) = own_array_element(ctx, op_arr, 2, "invalid env op")? else {
                    continue;
                };
                let val = match read_strict_string(
                    ctx,
                    &val_val,
                    "env value must be a string",
                    "env value contains an unpaired UTF-16 surrogate",
                )? {
                    Ok(v) => v,
                    Err(e) => return Ok(Err(e)),
                };
                spec = match spec.env(key, val) {
                    Ok(s) => s,
                    Err(e) => return Ok(Err(e)),
                };
            }
        }
    }

    if let Some(stdin_val) = own_data_property(ctx, raw_obj, c"stdin", "invalid command spec")?
        && !stdin_val.is_undefined()
    {
        let stdin_raw = stdin_val.as_raw();
        let Some(kind_val) = own_data_property(ctx, stdin_raw, c"kind", "invalid stdin spec")?
        else {
            return Err(Exception::throw_type(ctx, "invalid stdin spec"));
        };
        let kind = JsString::from_value(kind_val)?.to_string()?;
        spec = match kind.as_str() {
            "inherit" => spec.stdin(StdioMode::Inherit),
            "piped" => spec.stdin(StdioMode::Piped),
            "null" => spec.stdin(StdioMode::Null),
            "text" => {
                let Some(data_val) =
                    own_data_property(ctx, stdin_raw, c"data", "invalid stdin spec")?
                else {
                    return Err(Exception::throw_type(ctx, "invalid stdin data"));
                };
                let text = match read_strict_string(
                    ctx,
                    &data_val,
                    "stdin data must be a string",
                    "stdin text contains an unpaired UTF-16 surrogate",
                )? {
                    Ok(t) => t,
                    Err(e) => return Ok(Err(e)),
                };
                spec.stdin_bytes(text.into_bytes())
            }
            "bytes" => {
                let Some(data_val) =
                    own_data_property(ctx, stdin_raw, c"data", "invalid stdin spec")?
                else {
                    return Err(Exception::throw_type(ctx, "invalid stdin data"));
                };
                let bytes = with_uint8_array_bytes(
                    ctx,
                    &data_val,
                    "stdin data must be a Uint8Array",
                    |slice| Ok(slice.to_vec()),
                )?;
                spec.stdin_bytes(bytes)
            }
            _ => {
                return Ok(Err(ExecError::InvalidArgument {
                    reason: "invalid stdin mode",
                }));
            }
        };
    }

    if let Some(stdout_val) = own_data_property(ctx, raw_obj, c"stdout", "invalid command spec")?
        && !stdout_val.is_undefined()
    {
        let mode = JsString::from_value(stdout_val)?.to_string()?;
        let stdio = match mode.as_str() {
            "inherit" => StdioMode::Inherit,
            "piped" => StdioMode::Piped,
            "null" => StdioMode::Null,
            _ => {
                return Ok(Err(ExecError::InvalidArgument {
                    reason: "stdout mode must be 'inherit', 'piped', or 'null'",
                }));
            }
        };
        spec = match spec.stdout(stdio) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
    }

    if let Some(stderr_val) = own_data_property(ctx, raw_obj, c"stderr", "invalid command spec")?
        && !stderr_val.is_undefined()
    {
        let mode = JsString::from_value(stderr_val)?.to_string()?;
        let stdio = match mode.as_str() {
            "inherit" => StdioMode::Inherit,
            "piped" => StdioMode::Piped,
            "null" => StdioMode::Null,
            _ => {
                return Ok(Err(ExecError::InvalidArgument {
                    reason: "stderr mode must be 'inherit', 'piped', or 'null'",
                }));
            }
        };
        spec = match spec.stderr(stdio) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
    }

    if let Some(timeout_val) =
        own_data_property(ctx, raw_obj, c"timeoutMs", "invalid command spec")?
        && !timeout_val.is_undefined()
    {
        let dur = match parse_duration_ms(ctx, &timeout_val, false, "timeout")? {
            Ok(d) => d,
            Err(e) => return Ok(Err(e)),
        };
        spec = match spec.timeout(dur) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
    }

    if let Some(max_val) =
        own_data_property(ctx, raw_obj, c"maxOutputBytes", "invalid command spec")?
        && !max_val.is_undefined()
    {
        let Some(num) = max_val.as_number().filter(|_| max_val.is_number()) else {
            return Err(Exception::throw_type(
                ctx,
                "maxOutputBytes must be a number",
            ));
        };
        if !num.is_finite() || num.fract() != 0.0 || num < 0.0 || num > MAX_OUTPUT_BYTES as f64 {
            return Ok(Err(ExecError::InvalidArgument {
                reason: "maxOutputBytes exceeds hard limit of 64 MiB",
            }));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            spec = match spec.max_output_bytes(num as usize) {
                Ok(s) => s,
                Err(e) => return Ok(Err(e)),
            };
        }
    }

    if let Some(sig_val) = own_data_property(ctx, raw_obj, c"killSignal", "invalid command spec")?
        && !sig_val.is_undefined()
    {
        let sig_str = match read_strict_string(
            ctx,
            &sig_val,
            "killSignal must be a string",
            "signal contains an unpaired UTF-16 surrogate",
        )? {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        let sig = match Signal::parse(&sig_str) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        spec = spec.kill_signal(sig);
    }

    if let Some(grace_val) =
        own_data_property(ctx, raw_obj, c"killGraceMs", "invalid command spec")?
        && !grace_val.is_undefined()
    {
        let dur = match parse_duration_ms(ctx, &grace_val, true, "killGraceMs")? {
            Ok(d) => d,
            Err(e) => return Ok(Err(e)),
        };
        spec = match spec.kill_grace(dur) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
    }

    Ok(Ok(spec))
}

fn host_exec_sync<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(spec_val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "execSync requires a spec"));
    };
    let spec = match build_spec_from_js(&ctx, spec_val)? {
        Ok(s) => s,
        Err(e) => return error_object(&ctx, &e),
    };
    let output = match exec_sync(&spec) {
        Ok(o) => o,
        Err(e) => return error_object(&ctx, &e),
    };
    let stdout_ta = TypedArray::<u8>::new(ctx.clone(), output.stdout)?;
    let stderr_ta = TypedArray::<u8>::new(ctx.clone(), output.stderr)?;
    let out = ok_object(&ctx)?;
    out.prop(
        "stdoutBytes",
        Property::from(stdout_ta)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "stderrBytes",
        Property::from(stderr_ta)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "code",
        Property::from(f64::from(output.code))
            .writable()
            .enumerable()
            .configurable(),
    )?;
    let sig_val = match output.signal {
        Some(s) => JsString::from_str(ctx.clone(), &s)?.into_value(),
        None => Value::new_null(ctx.clone()),
    };
    out.prop(
        "signal",
        Property::from(sig_val)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "success",
        Property::from(output.success)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "timedOut",
        Property::from(output.timed_out)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "truncated",
        Property::from(output.truncated)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    Ok(out)
}

fn host_run_sync<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(spec_val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "runSync requires a spec"));
    };
    let spec = match build_spec_from_js(&ctx, spec_val)? {
        Ok(s) => s,
        Err(e) => return error_object(&ctx, &e),
    };
    let status = match run_sync(&spec) {
        Ok(s) => s,
        Err(e) => return error_object(&ctx, &e),
    };
    let out = ok_object(&ctx)?;
    out.prop(
        "code",
        Property::from(f64::from(status.code))
            .writable()
            .enumerable()
            .configurable(),
    )?;
    let sig_val = match status.signal {
        Some(s) => JsString::from_str(ctx.clone(), &s)?.into_value(),
        None => Value::new_null(ctx.clone()),
    };
    out.prop(
        "signal",
        Property::from(sig_val)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "success",
        Property::from(status.success)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "timedOut",
        Property::from(status.timed_out)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    Ok(out)
}

fn parse_which_prepend<'js>(
    ctx: &Ctx<'js>,
    opt: Option<&Value<'js>>,
) -> Result<std::result::Result<Option<OsString>, ExecError>> {
    let Some(opt) = opt else {
        return Ok(Ok(None));
    };
    if opt.is_undefined() {
        return Ok(Ok(None));
    }
    if opt.is_string() {
        let s = match read_strict_string(
            ctx,
            opt,
            "prependPath must be a string",
            "prependPath contains an unpaired UTF-16 surrogate",
        )? {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        return Ok(Ok(Some(OsString::from(s))));
    }
    let mut prepend_val: Option<Value<'js>> = None;
    let mut unknown = false;
    parse_options(
        ctx,
        Some(opt),
        "which options must be a plain object or string",
        |_ctx, key, val| {
            if unknown {
                return Ok(());
            }
            match key {
                "prependPath" => {
                    if !val.is_undefined() {
                        prepend_val = Some(val.clone());
                    }
                }
                _ => unknown = true,
            }
            Ok(())
        },
    )?;
    if unknown {
        return Err(Exception::throw_type(ctx, "unknown which option"));
    }
    if let Some(ref val) = prepend_val {
        let s = match read_strict_string(
            ctx,
            val,
            "options.prependPath must be a string",
            "prependPath contains an unpaired UTF-16 surrogate",
        )? {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        return Ok(Ok(Some(OsString::from(s))));
    }
    Ok(Ok(None))
}

fn host_which_sync<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(name_val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "whichSync requires a name"));
    };
    let name = match read_strict_string(
        &ctx,
        name_val,
        "executable name must be a string",
        "executable name contains an unpaired UTF-16 surrogate",
    )? {
        Ok(s) => s,
        Err(e) => return error_object(&ctx, &e),
    };
    let prepend = match parse_which_prepend(&ctx, args.0.get(1))? {
        Ok(p) => p,
        Err(e) => return error_object(&ctx, &e),
    };
    let found = match which(&name, prepend.as_deref()) {
        Ok(p) => p,
        Err(e) => return error_object(&ctx, &e),
    };
    let out = ok_object(&ctx)?;
    let path_val = match found {
        Some(p) => JsString::from_str(ctx.clone(), &p.to_string_lossy())?.into_value(),
        None => Value::new_null(ctx.clone()),
    };
    out.prop(
        "path",
        Property::from(path_val)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    Ok(out)
}

fn host_which_all_sync<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(name_val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "whichAllSync requires a name"));
    };
    let name = match read_strict_string(
        &ctx,
        name_val,
        "executable name must be a string",
        "executable name contains an unpaired UTF-16 surrogate",
    )? {
        Ok(s) => s,
        Err(e) => return error_object(&ctx, &e),
    };
    let prepend = match parse_which_prepend(&ctx, args.0.get(1))? {
        Ok(p) => p,
        Err(e) => return error_object(&ctx, &e),
    };
    let found = match which_all(&name, prepend.as_deref()) {
        Ok(p) => p,
        Err(e) => return error_object(&ctx, &e),
    };
    let arr = Array::new(ctx.clone())?;
    for (idx, p) in found.into_iter().enumerate() {
        arr.as_object().prop(
            idx as u32,
            Property::from(p.to_string_lossy().into_owned())
                .writable()
                .enumerable()
                .configurable(),
        )?;
    }
    let out = ok_object(&ctx)?;
    out.prop(
        "paths",
        Property::from(arr).writable().enumerable().configurable(),
    )?;
    Ok(out)
}

fn host_lock_file_sync<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(path_val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "lockFileSync requires a path"));
    };
    let path_str = match read_strict_string(
        &ctx,
        path_val,
        "lock path must be a string",
        "lock path contains an unpaired UTF-16 surrogate",
    )? {
        Ok(s) => s,
        Err(e) => return error_object(&ctx, &e),
    };

    let mut lock_opts = LockOptions::default();
    let mut mode_val: Option<Value<'js>> = None;
    let mut create_val: Option<Value<'js>> = None;
    let mut non_blocking_val: Option<Value<'js>> = None;
    let mut timeout_ms_val: Option<Value<'js>> = None;
    let mut timeout_val: Option<Value<'js>> = None;
    let mut unknown = false;

    parse_options(
        &ctx,
        args.0.get(1),
        "lock options must be a plain object",
        |_ctx, key, val| {
            if unknown {
                return Ok(());
            }
            match key {
                "mode" => mode_val = Some(val.clone()),
                "create" => create_val = Some(val.clone()),
                "nonBlocking" => non_blocking_val = Some(val.clone()),
                "timeoutMs" => timeout_ms_val = Some(val.clone()),
                "timeout" => timeout_val = Some(val.clone()),
                _ => unknown = true,
            }
            Ok(())
        },
    )?;
    if unknown {
        return Err(Exception::throw_type(&ctx, "unknown lock option"));
    }

    if let Some(ref val) = mode_val
        && !val.is_undefined()
    {
        let mode_str = match read_strict_string(
            &ctx,
            val,
            "lock mode must be a string",
            "lock mode contains an unpaired UTF-16 surrogate",
        )? {
            Ok(s) => s,
            Err(e) => return error_object(&ctx, &e),
        };
        lock_opts.mode = match mode_str.as_str() {
            "exclusive" => LockMode::Exclusive,
            "shared" => LockMode::Shared,
            _ => {
                return error_object(
                    &ctx,
                    &ExecError::InvalidArgument {
                        reason: "lock mode must be 'exclusive' or 'shared'",
                    },
                );
            }
        };
    }

    if let Some(ref val) = create_val
        && !val.is_undefined()
    {
        let Some(b) = val.as_bool() else {
            return Err(Exception::throw_type(
                &ctx,
                "options.create must be a boolean",
            ));
        };
        lock_opts.create = b;
    }

    if let Some(ref val) = non_blocking_val
        && !val.is_undefined()
    {
        let Some(b) = val.as_bool() else {
            return Err(Exception::throw_type(
                &ctx,
                "options.nonBlocking must be a boolean",
            ));
        };
        lock_opts.non_blocking = b;
    }

    let eff_timeout = match (timeout_ms_val, timeout_val) {
        (Some(a), Some(b)) if !a.is_undefined() && !b.is_undefined() => {
            return error_object(
                &ctx,
                &ExecError::InvalidArgument {
                    reason: "cannot specify both timeout and timeoutMs",
                },
            );
        }
        (Some(a), _) if !a.is_undefined() => Some(a),
        (_, Some(b)) if !b.is_undefined() => Some(b),
        _ => None,
    };
    if let Some(val) = eff_timeout {
        let dur = match parse_duration_ms(&ctx, &val, false, "timeout")? {
            Ok(d) => d,
            Err(e) => return error_object(&ctx, &e),
        };
        lock_opts.timeout = Some(dur);
    }

    let lock_limit_err = || ExecError::Io {
        context: "file lock limit",
        source: std::io::Error::other("maximum number of open file locks exceeded (1024)"),
    };
    {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        if state.exec_locks.borrow().is_full() {
            return error_object(&ctx, &lock_limit_err());
        }
    }

    let lock = match FileLock::acquire(Path::new(&path_str), &lock_opts) {
        Ok(l) => l,
        Err(e) => {
            // `ExecError::NotFound` on a lock file path is a `CommandError`
            // (not `NotFoundOnPathError`), so omit `notFound`.
            let obj = Object::new(ctx.clone())?;
            set_null_proto(&ctx, &obj)?;
            obj.prop(
                "ok",
                Property::from(false).writable().enumerable().configurable(),
            )?;
            obj.prop(
                "code",
                Property::from(e.code().as_str())
                    .writable()
                    .enumerable()
                    .configurable(),
            )?;
            obj.prop(
                "message",
                Property::from(e.to_string())
                    .writable()
                    .enumerable()
                    .configurable(),
            )?;
            return Ok(obj);
        }
    };

    let mode_label = match lock.mode() {
        LockMode::Exclusive => "exclusive",
        LockMode::Shared => "shared",
    };

    let (id, weak_table) = {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let Some(id) = state.exec_locks.borrow_mut().insert(lock) else {
            return error_object(&ctx, &lock_limit_err());
        };
        (id, Rc::downgrade(&state.exec_locks))
    };

    let handle = Class::instance(
        ctx.clone(),
        FileLockBrand {
            id,
            table: weak_table,
        },
    )?;

    let out = ok_object(&ctx)?;
    out.prop(
        "handle",
        Property::from(handle)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    out.prop(
        "mode",
        Property::from(mode_label)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    Ok(out)
}

fn host_lock_unlock<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    let Some(handle_val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "invalid FileLock handle"));
    };
    let cls = handle_val
        .as_object()
        .and_then(Class::<FileLockBrand>::from_object)
        .ok_or_else(|| Exception::throw_type(&ctx, "invalid FileLock receiver"))?;
    let (id, table_opt) = {
        let brand = cls.borrow();
        (brand.id, brand.table.upgrade())
    };
    if let Some(table) = table_opt {
        let taken = table.borrow_mut().take(id);
        if let Some(lock) = taken
            && let Err(e) = lock.unlock()
        {
            return error_object(&ctx, &e);
        }
    }
    ok_object(&ctx)
}

fn host_lock_is_locked<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<bool> {
    let Some(handle_val) = args.0.first() else {
        return Err(Exception::throw_type(&ctx, "invalid FileLock handle"));
    };
    let cls = handle_val
        .as_object()
        .and_then(Class::<FileLockBrand>::from_object)
        .ok_or_else(|| Exception::throw_type(&ctx, "invalid FileLock receiver"))?;
    let (id, table_opt) = {
        let brand = cls.borrow();
        (brand.id, brand.table.upgrade())
    };
    Ok(table_opt.is_some_and(|t| t.borrow().contains(id)))
}

fn host_decode_utf8_lossy<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<String> {
    let Some(val) = args.0.first() else {
        return Err(Exception::throw_type(
            &ctx,
            "decodeUtf8Lossy requires a Uint8Array",
        ));
    };
    with_uint8_array_bytes(&ctx, val, "expected Uint8Array", |slice| {
        Ok(String::from_utf8_lossy(slice).into_owned())
    })
}

fn host_lines_lossy<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Array<'js>> {
    let Some(val) = args.0.first() else {
        return Err(Exception::throw_type(
            &ctx,
            "linesLossy requires a Uint8Array",
        ));
    };
    let lines = with_uint8_array_bytes(&ctx, val, "expected Uint8Array", |slice| {
        let text = String::from_utf8_lossy(slice);
        Ok(text.lines().map(str::to_owned).collect::<Vec<String>>())
    })?;
    let arr = Array::new(ctx.clone())?;
    for (idx, line) in lines.into_iter().enumerate() {
        arr.as_object().prop(
            idx as u32,
            Property::from(line).writable().enumerable().configurable(),
        )?;
    }
    Ok(arr)
}

/// The native QuickJS module served for `vael:exec`.
pub(super) struct ExecModule;

impl ModuleDef for ExecModule {
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
        let Some(exec) = state.exec.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = exec.clone().restore(ctx)?;
        for name in EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        Ok(())
    }
}

// Keep unused error code imports honest for exhaustive lint coverage.
const _: fn(ExecErrorCode) -> &'static str = ExecErrorCode::as_str;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_lock_table_enforces_max_open_locks_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut table = ExecLockTable::new();
        let mut ids = Vec::with_capacity(MAX_OPEN_LOCKS);
        for idx in 0..MAX_OPEN_LOCKS {
            let path = dir.path().join(format!("lock-{idx}.lck"));
            let lock = FileLock::acquire(
                &path,
                &LockOptions {
                    mode: LockMode::Exclusive,
                    create: true,
                    non_blocking: true,
                    timeout: None,
                },
            )
            .expect("acquire lock");
            let id = table.insert(lock).expect("lock within cap");
            ids.push(id);
        }
        assert!(table.is_full());
        let extra_path = dir.path().join("lock-overflow.lck");
        let extra = FileLock::acquire(
            &extra_path,
            &LockOptions {
                mode: LockMode::Exclusive,
                create: true,
                non_blocking: true,
                timeout: None,
            },
        )
        .expect("acquire overflow lock");
        assert!(table.insert(extra).is_none());
        let removed = ids.pop().expect("id");
        assert!(table.take(removed).is_some());
        assert!(!table.is_full());
    }
}

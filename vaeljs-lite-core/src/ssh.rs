//! Native `vael:ssh` bindings for QuickJS.
//!
//! All core SSH transport, mandatory host-key / `known_hosts` verification,
//! credential validation & `Zeroizing` wiping, bounded deadlines, handle
//! lifecycle, and `VAEL_SSH_*` error codes live in [`vaeljs_ssh_core`]. This
//! module builds one frozen registry object per context with the exports in
//! [`vaeljs_ssh_core::EXPORTS`] and shares that exact `connect` function across
//! `Vael.getBuiltInModule("vael:ssh")` and `import ... from "vael:ssh"`.

use std::time::Duration;

use rquickjs::{
    Ctx, Exception, Function, Object, Result, TypedArray, Value,
    function::Rest,
    module::ModuleDef,
    object::{Accessor, Property},
    qjs,
};
use vaeljs_ssh_core::{
    AuthConfig, ConnectOptions, DEFAULT_CONNECT_TIMEOUT_MS, DEFAULT_EXEC_TIMEOUT_MS, HostTrust,
    MAX_COMMAND_BYTES, MAX_CONNECT_TIMEOUT_MS, MAX_EXEC_TIMEOUT_MS, MAX_HOST_BYTES,
    MAX_HOST_KEY_BYTES, MAX_KNOWN_HOSTS_BYTES, MAX_OUTPUT_BYTES, MAX_PASSPHRASE_BYTES,
    MAX_PASSWORD_BYTES, MAX_PRIVATE_KEY_BYTES, MAX_USERNAME_BYTES, SshError, Zeroizing,
    validate_host, validate_username,
};

use crate::native::{freeze_native, parse_options, throw_coded_error, with_uint8_array_bytes};

/// Creates the frozen public registry object for `vael:ssh`.
pub(super) fn object<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let object = Object::new(ctx.clone())?;
    object.set(
        "connect",
        Function::new(ctx.clone(), connect)?
            .with_name("connect")?
            .with_length(1)?,
    )?;
    object.set("MAX_COMMAND_BYTES", MAX_COMMAND_BYTES as f64)?;
    object.set("MAX_OUTPUT_BYTES", MAX_OUTPUT_BYTES as f64)?;
    object.set(
        "DEFAULT_CONNECT_TIMEOUT_MS",
        DEFAULT_CONNECT_TIMEOUT_MS as f64,
    )?;
    object.set("DEFAULT_EXEC_TIMEOUT_MS", DEFAULT_EXEC_TIMEOUT_MS as f64)?;
    freeze_native(ctx, &object)?;
    Ok(object)
}

fn throw_ssh_error(ctx: &Ctx<'_>, error: SshError) -> rquickjs::Error {
    throw_coded_error(
        ctx,
        Some("SshError"),
        error.code().as_str(),
        &error.to_string(),
    )
}

fn limit_error(ctx: &Ctx<'_>) -> rquickjs::Error {
    throw_ssh_error(
        ctx,
        SshError::limit("SSH data exceeds a documented size limit"),
    )
}

fn argument_error(ctx: &Ctx<'_>) -> rquickjs::Error {
    throw_ssh_error(ctx, SshError::argument("an SSH option is invalid"))
}

struct CStringGuard(*mut qjs::JSContext, *const std::ffi::c_char);

impl Drop for CStringGuard {
    fn drop(&mut self) {
        unsafe { qjs::JS_FreeCString(self.0, self.1) };
    }
}

fn read_string_inner<'js, T>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    max: usize,
    convert: impl FnOnce(&str) -> Result<T>,
) -> Result<T> {
    if !value.is_string() {
        return Err(Exception::throw_type(ctx, type_msg));
    }
    let raw = ctx.as_raw().as_ptr();
    let mut len: qjs::size_t = 0;
    let ptr = unsafe { qjs::JS_ToCStringLen2(raw, &mut len, value.as_raw(), false) };
    if ptr.is_null() {
        if ctx.has_exception() {
            return Err(rquickjs::Error::Exception);
        }
        return Err(throw_ssh_error(ctx, SshError::internal()));
    }
    let _guard = CStringGuard(raw, ptr);
    let byte_len = len as usize;
    if byte_len > max {
        return Err(limit_error(ctx));
    }
    // SAFETY: `ptr` points to a NUL-terminated buffer of `byte_len` bytes valid
    // until `_guard` drops and calls `JS_FreeCString`.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, byte_len) };
    let text = std::str::from_utf8(bytes).map_err(|_| argument_error(ctx))?;
    convert(text)
}

fn copy_string<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    max: usize,
) -> Result<String> {
    read_string_inner(ctx, value, type_msg, max, |text| {
        let mut out = String::new();
        out.try_reserve_exact(text.len())
            .map_err(|_| limit_error(ctx))?;
        out.push_str(text);
        Ok(out)
    })
}

fn optional_string<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    type_msg: &str,
    max: usize,
) -> Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    copy_string(ctx, value, type_msg, max).map(Some)
}

fn required_string<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    type_msg: &str,
    max: usize,
) -> Result<String> {
    optional_string(ctx, value, type_msg, max)?.ok_or_else(|| Exception::throw_type(ctx, type_msg))
}

fn optional_secret_string<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    type_msg: &str,
    max: usize,
) -> Result<Option<Zeroizing<String>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    read_string_inner(ctx, value, type_msg, max, |text| {
        let mut out = Zeroizing::new(String::new());
        out.try_reserve_exact(text.len())
            .map_err(|_| limit_error(ctx))?;
        out.push_str(text);
        Ok(Some(out))
    })
}

fn copy_bytes<'js>(
    ctx: &Ctx<'js>,
    value: &Value<'js>,
    type_msg: &str,
    max: usize,
) -> Result<Vec<u8>> {
    with_uint8_array_bytes(ctx, value, type_msg, |slice| {
        if slice.len() > max {
            return Err(limit_error(ctx));
        }
        let mut out = Vec::new();
        out.try_reserve_exact(slice.len())
            .map_err(|_| limit_error(ctx))?;
        out.extend_from_slice(slice);
        Ok(out)
    })
}

fn optional_bytes<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    type_msg: &str,
    max: usize,
) -> Result<Option<Vec<u8>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    copy_bytes(ctx, value, type_msg, max).map(Some)
}

fn optional_secret_bytes<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    type_msg: &str,
    max: usize,
) -> Result<Option<Zeroizing<Vec<u8>>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_undefined() {
        return Ok(None);
    }
    with_uint8_array_bytes(ctx, value, type_msg, |slice| {
        if slice.len() > max {
            return Err(limit_error(ctx));
        }
        let mut out = Zeroizing::new(Vec::new());
        out.try_reserve_exact(slice.len())
            .map_err(|_| limit_error(ctx))?;
        out.extend_from_slice(slice);
        Ok(Some(out))
    })
}

fn optional_bool<'js>(ctx: &Ctx<'js>, value: Option<&Value<'js>>, type_msg: &str) -> Result<bool> {
    let Some(value) = value else {
        return Ok(false);
    };
    if value.is_undefined() {
        return Ok(false);
    }
    value
        .as_bool()
        .ok_or_else(|| Exception::throw_type(ctx, type_msg))
}

fn optional_int_range<'js>(
    ctx: &Ctx<'js>,
    value: Option<&Value<'js>>,
    default: u64,
    max: u64,
    name: &str,
) -> Result<u64> {
    let Some(value) = value else {
        return Ok(default);
    };
    if value.is_undefined() {
        return Ok(default);
    }
    if !value.is_number() {
        return Err(Exception::throw_type(
            ctx,
            &format!("{name} must be a number"),
        ));
    }
    let Some(num) = value.as_number() else {
        return Err(Exception::throw_type(
            ctx,
            &format!("{name} must be a number"),
        ));
    };
    if !num.is_finite() || num.fract() != 0.0 || num < 1.0 || num > max as f64 {
        return Err(Exception::throw_range(
            ctx,
            &format!("{name} must be an integer between 1 and {max}"),
        ));
    }
    Ok(num as u64)
}

#[derive(Default)]
struct RawConnectFields<'js> {
    host: Option<Value<'js>>,
    port: Option<Value<'js>>,
    username: Option<Value<'js>>,
    password: Option<Value<'js>>,
    private_key: Option<Value<'js>>,
    agent: Option<Value<'js>>,
    passphrase: Option<Value<'js>>,
    host_key: Option<Value<'js>>,
    known_hosts: Option<Value<'js>>,
    connect_timeout_ms: Option<Value<'js>>,
    exec_timeout_ms: Option<Value<'js>>,
}

fn parse_connect_options<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<ConnectOptions> {
    if value.is_undefined() {
        return Err(Exception::throw_type(
            ctx,
            "connect takes one options object",
        ));
    }
    let mut fields = RawConnectFields::default();
    parse_options(
        ctx,
        Some(value),
        "connect takes one options object",
        |ctx, key, val| {
            match key {
                "host" => fields.host = Some(val.clone()),
                "port" => fields.port = Some(val.clone()),
                "username" => fields.username = Some(val.clone()),
                "password" => fields.password = Some(val.clone()),
                "privateKey" => fields.private_key = Some(val.clone()),
                "agent" => fields.agent = Some(val.clone()),
                "passphrase" => fields.passphrase = Some(val.clone()),
                "hostKey" => fields.host_key = Some(val.clone()),
                "knownHosts" => fields.known_hosts = Some(val.clone()),
                "connectTimeoutMs" => fields.connect_timeout_ms = Some(val.clone()),
                "execTimeoutMs" => fields.exec_timeout_ms = Some(val.clone()),
                _ => {
                    return Err(Exception::throw_type(
                        ctx,
                        &format!("unknown SSH option `{key}`"),
                    ));
                }
            }
            Ok(())
        },
    )?;

    let host = required_string(
        ctx,
        fields.host.as_ref(),
        "host must be a string",
        MAX_HOST_BYTES,
    )?;
    validate_host(&host).map_err(|e| throw_ssh_error(ctx, e))?;
    let port = optional_int_range(ctx, fields.port.as_ref(), 22, 65535, "port")? as u16;
    let username = required_string(
        ctx,
        fields.username.as_ref(),
        "username must be a string",
        MAX_USERNAME_BYTES,
    )?;
    validate_username(&username).map_err(|e| throw_ssh_error(ctx, e))?;

    let password = optional_secret_string(
        ctx,
        fields.password.as_ref(),
        "password must be a string",
        MAX_PASSWORD_BYTES,
    )?;
    let private_key = optional_secret_bytes(
        ctx,
        fields.private_key.as_ref(),
        "privateKey must be a Uint8Array",
        MAX_PRIVATE_KEY_BYTES,
    )?;
    let agent = optional_bool(ctx, fields.agent.as_ref(), "agent must be a boolean")?;
    let passphrase = optional_secret_string(
        ctx,
        fields.passphrase.as_ref(),
        "passphrase must be a string",
        MAX_PASSPHRASE_BYTES,
    )?;
    let auth = AuthConfig::from_parts(password, private_key, agent, passphrase)
        .map_err(|e| throw_ssh_error(ctx, e))?;

    let host_key = optional_bytes(
        ctx,
        fields.host_key.as_ref(),
        "hostKey must be a Uint8Array",
        MAX_HOST_KEY_BYTES,
    )?;
    let known_hosts = optional_string(
        ctx,
        fields.known_hosts.as_ref(),
        "knownHosts must be a string",
        MAX_KNOWN_HOSTS_BYTES,
    )?;
    let trust = HostTrust::from_parts(host_key.as_deref(), known_hosts.as_deref())
        .map_err(|e| throw_ssh_error(ctx, e))?;

    let connect_timeout = Duration::from_millis(optional_int_range(
        ctx,
        fields.connect_timeout_ms.as_ref(),
        DEFAULT_CONNECT_TIMEOUT_MS,
        MAX_CONNECT_TIMEOUT_MS,
        "connectTimeoutMs",
    )?);
    let exec_timeout = Duration::from_millis(optional_int_range(
        ctx,
        fields.exec_timeout_ms.as_ref(),
        DEFAULT_EXEC_TIMEOUT_MS,
        MAX_EXEC_TIMEOUT_MS,
        "execTimeoutMs",
    )?);

    Ok(ConnectOptions::new(host, port, username, auth, trust)
        .with_connect_timeout(connect_timeout)
        .with_exec_timeout(exec_timeout))
}

fn connect<'js>(ctx: Ctx<'js>, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            &ctx,
            "connect takes exactly one argument",
        ));
    }
    let options = parse_connect_options(&ctx, &args.0[0])?;
    let id = {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let mut ssh_state = state.ssh_state.borrow_mut();
        ssh_state
            .connect(options)
            .map_err(|e| throw_ssh_error(&ctx, e))?
    };
    build_connection_object(&ctx, id)
}

fn build_connection_object<'js>(ctx: &Ctx<'js>, id: u32) -> Result<Object<'js>> {
    let conn = Object::new(ctx.clone())?;
    let exec_fn = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        exec_connection(&ctx, id, args)
    })?
    .with_name("exec")?
    .with_length(1)?;
    let close_fn = Function::new(ctx.clone(), move |ctx: Ctx<'js>, args: Rest<Value<'js>>| {
        close_connection(&ctx, id, args)
    })?
    .with_name("close")?
    .with_length(0)?;
    conn.prop("exec", Property::from(exec_fn).writable().configurable())?;
    conn.prop("close", Property::from(close_fn).writable().configurable())?;
    conn.prop(
        "closed",
        Accessor::from(move |ctx: Ctx<'js>| is_connection_closed(&ctx, id)).configurable(),
    )?;
    freeze_native(ctx, &conn)?;
    Ok(conn)
}

fn read_command<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> Result<Vec<u8>> {
    if value.is_string() {
        return Ok(copy_string(
            ctx,
            value,
            "command must be a string or Uint8Array",
            MAX_COMMAND_BYTES,
        )?
        .into_bytes());
    }
    copy_bytes(
        ctx,
        value,
        "command must be a string or Uint8Array",
        MAX_COMMAND_BYTES,
    )
}

fn exec_connection<'js>(ctx: &Ctx<'js>, id: u32, args: Rest<Value<'js>>) -> Result<Object<'js>> {
    if args.0.len() != 1 {
        return Err(Exception::throw_type(
            ctx,
            "exec takes exactly one argument",
        ));
    }
    {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        if !state.ssh_state.borrow().is_open(id) {
            return Err(throw_ssh_error(ctx, SshError::Closed));
        }
    }
    let command = read_command(ctx, &args.0[0])?;
    let output = {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let mut ssh_state = state.ssh_state.borrow_mut();
        ssh_state
            .exec(id, &command)
            .map_err(|e| throw_ssh_error(ctx, e))?
    };

    let stdout = match TypedArray::<u8>::new(ctx.clone(), output.stdout) {
        Ok(array) => array,
        Err(err) => {
            if let Some(state) = ctx.userdata::<super::vael::VaelState>() {
                state.ssh_state.borrow_mut().close(id);
            }
            return Err(err);
        }
    };
    let result = Object::new(ctx.clone())?;
    result.prop(
        "stdout",
        Property::from(stdout)
            .writable()
            .enumerable()
            .configurable(),
    )?;
    result.prop(
        "status",
        Property::from(f64::from(output.status))
            .writable()
            .enumerable()
            .configurable(),
    )?;
    Ok(result)
}

fn close_connection<'js>(ctx: &Ctx<'js>, id: u32, args: Rest<Value<'js>>) -> Result<()> {
    if !args.0.is_empty() {
        return Err(Exception::throw_type(ctx, "close takes no arguments"));
    }
    let state = ctx
        .userdata::<super::vael::VaelState>()
        .ok_or(rquickjs::Error::Unknown)?;
    state.ssh_state.borrow_mut().close(id);
    Ok(())
}

fn is_connection_closed(ctx: &Ctx<'_>, id: u32) -> Result<bool> {
    let state = ctx
        .userdata::<super::vael::VaelState>()
        .ok_or(rquickjs::Error::Unknown)?;
    let is_open = state.ssh_state.borrow().is_open(id);
    Ok(!is_open)
}

/// The native QuickJS module served for `vael:ssh`.
pub(super) struct SshModule;

impl ModuleDef for SshModule {
    fn declare(declarations: &rquickjs::module::Declarations<'_>) -> Result<()> {
        for name in vaeljs_ssh_core::EXPORTS {
            declarations.declare(*name)?;
        }
        Ok(())
    }

    fn evaluate<'js>(ctx: &Ctx<'js>, exports: &rquickjs::module::Exports<'js>) -> Result<()> {
        let state = ctx
            .userdata::<super::vael::VaelState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let Some(ssh) = state.ssh.as_ref() else {
            return Err(rquickjs::Error::Unknown);
        };
        let object = ssh.clone().restore(ctx)?;
        for name in vaeljs_ssh_core::EXPORTS {
            let value: Value<'js> = object.get(*name)?;
            exports.export(*name, value)?;
        }
        Ok(())
    }
}

//! V8/`deno_core` op adapter for `neo:ssh`.
//!
//! All engine-neutral SSH session management, mandatory host-key / `known_hosts`
//! verification, credential validation & `Zeroizing` wiping, bounded deadlines,
//! handle lifecycle, and `NEO_SSH_*` error codes live in [`neojs_ssh_core`].
//! This module validates V8 JavaScript arguments, translates them into
//! `neojs_ssh_core` types, manages the per-isolate [`SshState`] in `OpState`,
//! and builds null-prototype result objects for [`crate::builtins`].

use std::time::Duration;

use deno_core::OpState;
use deno_core::op2;
use deno_core::v8;
use neojs_ssh_core::{
    AuthConfig, ConnectOptions, DEFAULT_CONNECT_TIMEOUT_MS, DEFAULT_EXEC_TIMEOUT_MS, HostTrust,
    MAX_COMMAND_BYTES, MAX_CONNECT_TIMEOUT_MS, MAX_EXEC_TIMEOUT_MS, MAX_HOST_BYTES,
    MAX_HOST_KEY_BYTES, MAX_KNOWN_HOSTS_BYTES, MAX_PASSPHRASE_BYTES, MAX_PASSWORD_BYTES,
    MAX_PRIVATE_KEY_BYTES, MAX_USERNAME_BYTES, SshError, SshState, Zeroizing,
};

/// V8 binding-level or core SSH failure carried back to `neo.js` as data.
#[derive(Debug)]
enum BindingError {
    /// Value is not an accepted JavaScript type, reported as `TypeError`.
    Type,
    /// Number is outside the documented range, reported as `RangeError`.
    Range,
    /// Core SSH failure with stable `NEO_SSH_*` code.
    Ssh(SshError),
}

impl From<SshError> for BindingError {
    fn from(error: SshError) -> Self {
        Self::Ssh(error)
    }
}

fn type_error() -> BindingError {
    BindingError::Type
}

fn range_error() -> BindingError {
    BindingError::Range
}

fn limit_error() -> BindingError {
    BindingError::Ssh(SshError::limit("SSH data exceeds a documented size limit"))
}

fn argument_error() -> BindingError {
    BindingError::Ssh(SshError::argument("an SSH option is invalid"))
}

fn internal_error() -> BindingError {
    BindingError::Ssh(SshError::internal())
}

// ---------------------------------------------------------------------------
// JavaScript value handling
// ---------------------------------------------------------------------------

fn response<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
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

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: BindingError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    match error {
        BindingError::Type => {
            if let Some(name) = v8::String::new(scope, "TypeError") {
                set(scope, object, "name", name.into());
            }
            if let Some(message) = v8::String::new(scope, "an SSH option has the wrong type") {
                set(scope, object, "message", message.into());
            }
        }
        BindingError::Range => {
            if let Some(name) = v8::String::new(scope, "RangeError") {
                set(scope, object, "name", name.into());
            }
            if let Some(message) =
                v8::String::new(scope, "an SSH option is outside the documented range")
            {
                set(scope, object, "message", message.into());
            }
        }
        BindingError::Ssh(error) => {
            if let Some(name) = v8::String::new(scope, "SshError") {
                set(scope, object, "name", name.into());
            }
            if let Some(code) = v8::String::new(scope, error.code().as_str()) {
                set(scope, object, "code", code.into());
            }
            if let Some(message) = v8::String::new(scope, &error.to_string()) {
                set(scope, object, "message", message.into());
            }
        }
    }
}

fn failure<'s>(scope: &mut v8::PinScope<'s, '_>, error: BindingError) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    set_failure(scope, object, error);
    object.into()
}

fn failure_closed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: BindingError,
) -> v8::Local<'s, v8::Value> {
    let object = response(scope);
    set_failure(scope, object, error);
    set(
        scope,
        object,
        "closed",
        v8::Boolean::new(scope, true).into(),
    );
    object.into()
}

fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let length = bytes.len();
    if length == 0 {
        let buffer = v8::ArrayBuffer::new(scope, 0);
        return v8::Uint8Array::new(scope, buffer, 0, 0);
    }
    let store = v8::ArrayBuffer::new_backing_store_from_bytes(bytes.into_boxed_slice());
    let store = store.make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length)
}

/// Read an optional string and copy it, bounding it before the copy.
fn optional_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<Option<String>, BindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(string) = v8::Local::<v8::String>::try_from(value) else {
        return Err(type_error());
    };
    Ok(Some(copy_string(scope, string, max)?))
}

/// Encode one V8 string as UTF-8 without silently replacing a lone surrogate.
///
/// `to_rust_string_lossy` substitutes U+FFFD for an unpaired surrogate, which
/// would quietly change a password or an exact command. V8's UTF-8 writer is
/// asked for the original code units instead; the result is decoded as strict
/// UTF-8, so an unpaired surrogate is an argument error rather than a different
/// value.
fn copy_string(
    scope: &v8::PinScope<'_, '_>,
    string: v8::Local<'_, v8::String>,
    max: usize,
) -> Result<String, BindingError> {
    let length = string.utf8_length(scope);
    if length > max {
        return Err(limit_error());
    }
    // The extra room covers any difference between V8's length count and the
    // exact encoding, so a write is never silently truncated.
    let capacity = length.checked_add(4).ok_or_else(limit_error)?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| limit_error())?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    if written > max {
        return Err(limit_error());
    }
    buffer.truncate(written);
    String::from_utf8(buffer).map_err(|_| argument_error())
}

/// Copy an optional secret string into a zeroizing buffer.
///
/// The intermediate UTF-8 bytes and the returned string are wiped on drop.
/// Every validation return after this call therefore wipes the parsed copy.
fn optional_secret_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<Option<Zeroizing<String>>, BindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(string) = v8::Local::<v8::String>::try_from(value) else {
        return Err(type_error());
    };
    let length = string.utf8_length(scope);
    if length > max {
        return Err(limit_error());
    }
    let capacity = length.checked_add(4).ok_or_else(limit_error)?;
    let mut buffer = Zeroizing::new(Vec::new());
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| limit_error())?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    if written > max {
        return Err(limit_error());
    }
    buffer.truncate(written);
    let text = std::str::from_utf8(&buffer).map_err(|_| argument_error())?;
    // `to_owned` allocates a second buffer; the first is wiped when `buffer`
    // drops at the end of this function, and the second is wiped by its
    // `Zeroizing` wrapper.
    Ok(Some(Zeroizing::new(text.to_owned())))
}

/// Copy an optional secret `Uint8Array` into a zeroizing buffer.
fn optional_secret_bytes(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<Option<Zeroizing<Vec<u8>>>, BindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(type_error());
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(argument_error());
    };
    if buffer.was_detached() {
        return Err(argument_error());
    }
    let length = view.byte_length();
    if length > max {
        return Err(limit_error());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    bytes.try_reserve_exact(length).map_err(|_| limit_error())?;
    bytes.resize(length, 0);
    if view.copy_contents(&mut bytes) != length {
        return Err(argument_error());
    }
    Ok(Some(bytes))
}

/// Copy an optional `Uint8Array`, bounding it before the copy.
fn optional_bytes(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<Option<Vec<u8>>, BindingError> {
    if value.is_undefined() {
        return Ok(None);
    }
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(type_error());
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(argument_error());
    };
    if buffer.was_detached() {
        return Err(argument_error());
    }
    let length = view.byte_length();
    if length > max {
        return Err(limit_error());
    }
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(length).map_err(|_| limit_error())?;
    bytes.resize(length, 0);
    if view.copy_contents(&mut bytes) != length {
        return Err(argument_error());
    }
    Ok(Some(bytes))
}

fn optional_number(
    value: v8::Local<'_, v8::Value>,
    default: u64,
    max: u64,
) -> Result<u64, BindingError> {
    if value.is_undefined() {
        return Ok(default);
    }
    let Ok(number) = v8::Local::<v8::Number>::try_from(value) else {
        return Err(type_error());
    };
    let value = number.value();
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(range_error());
    }
    if value < 1.0 || value > max as f64 {
        return Err(range_error());
    }
    Ok(value as u64)
}

fn optional_port(value: v8::Local<'_, v8::Value>) -> Result<u16, BindingError> {
    if value.is_undefined() {
        return Ok(22);
    }
    let Ok(number) = v8::Local::<v8::Number>::try_from(value) else {
        return Err(type_error());
    };
    let value = number.value();
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(range_error());
    }
    if value < 1.0 || value > 65535.0 {
        return Err(range_error());
    }
    Ok(value as u16)
}

fn required_string(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
    max: usize,
) -> Result<String, BindingError> {
    optional_string(scope, value, max)?.ok_or_else(type_error)
}

// ---------------------------------------------------------------------------
// Ops
// ---------------------------------------------------------------------------

#[op2]
#[allow(clippy::too_many_arguments)]
fn op_neo_ssh_connect<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    op_state: &mut OpState,
    host: v8::Local<'s, v8::Value>,
    port: v8::Local<'s, v8::Value>,
    username: v8::Local<'s, v8::Value>,
    password: v8::Local<'s, v8::Value>,
    private_key: v8::Local<'s, v8::Value>,
    agent: v8::Local<'s, v8::Value>,
    passphrase: v8::Local<'s, v8::Value>,
    host_key: v8::Local<'s, v8::Value>,
    known_hosts: v8::Local<'s, v8::Value>,
    connect_timeout_ms: v8::Local<'s, v8::Value>,
    exec_timeout_ms: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let parsed: Result<ConnectOptions, BindingError> = (|| {
        let host = required_string(scope, host, MAX_HOST_BYTES)?;
        let port = optional_port(port)?;
        let username = required_string(scope, username, MAX_USERNAME_BYTES)?;

        let password = optional_secret_string(scope, password, MAX_PASSWORD_BYTES)?;
        let private_key = optional_secret_bytes(scope, private_key, MAX_PRIVATE_KEY_BYTES)?;
        let agent = if agent.is_undefined() {
            false
        } else {
            let Ok(flag) = v8::Local::<v8::Boolean>::try_from(agent) else {
                return Err(type_error());
            };
            flag.is_true()
        };
        let passphrase = optional_secret_string(scope, passphrase, MAX_PASSPHRASE_BYTES)?;
        let auth = AuthConfig::from_parts(password, private_key, agent, passphrase)?;

        let host_key = optional_bytes(scope, host_key, MAX_HOST_KEY_BYTES)?;
        let known_hosts = optional_string(scope, known_hosts, MAX_KNOWN_HOSTS_BYTES)?;
        let trust = HostTrust::from_parts(host_key.as_deref(), known_hosts.as_deref())?;

        let connect_timeout = Duration::from_millis(optional_number(
            connect_timeout_ms,
            DEFAULT_CONNECT_TIMEOUT_MS,
            MAX_CONNECT_TIMEOUT_MS,
        )?);
        let exec_timeout = Duration::from_millis(optional_number(
            exec_timeout_ms,
            DEFAULT_EXEC_TIMEOUT_MS,
            MAX_EXEC_TIMEOUT_MS,
        )?);

        Ok(ConnectOptions::new(host, port, username, auth, trust)
            .with_connect_timeout(connect_timeout)
            .with_exec_timeout(exec_timeout))
    })();

    let options = match parsed {
        Ok(options) => options,
        Err(error) => return failure(scope, error),
    };

    let Some(state) = op_state.try_borrow_mut::<SshState>() else {
        return failure(scope, internal_error());
    };
    let id = match state.connect(options) {
        Ok(id) => id,
        Err(error) => return failure(scope, error.into()),
    };

    let object = response(scope);
    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
    set(
        scope,
        object,
        "id",
        v8::Number::new(scope, f64::from(id)).into(),
    );
    object.into()
}

#[op2]
fn op_neo_ssh_exec<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    op_state: &mut OpState,
    #[smi] id: u32,
    command: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    let command = match read_command(scope, command) {
        Ok(command) => command,
        Err(error) => return failure(scope, error),
    };

    let Some(state) = op_state.try_borrow_mut::<SshState>() else {
        return failure_closed(scope, internal_error());
    };

    match state.exec(id, &command) {
        Ok(output) => {
            let object = response(scope);
            let Some(stdout) = bytes_to_v8(scope, output.stdout) else {
                state.close(id);
                return failure_closed(scope, limit_error());
            };
            set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            set(scope, object, "stdout", stdout.into());
            set(
                scope,
                object,
                "status",
                v8::Number::new(scope, f64::from(output.status)).into(),
            );
            object.into()
        }
        Err(error) => {
            if state.is_open(id) {
                failure(scope, error.into())
            } else {
                failure_closed(scope, error.into())
            }
        }
    }
}

fn read_command(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Vec<u8>, BindingError> {
    if let Ok(string) = v8::Local::<v8::String>::try_from(value) {
        return Ok(copy_string(scope, string, MAX_COMMAND_BYTES)?.into_bytes());
    }
    optional_bytes(scope, value, MAX_COMMAND_BYTES)?.ok_or_else(type_error)
}

#[op2]
fn op_neo_ssh_close<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    op_state: &mut OpState,
    #[smi] id: u32,
) -> v8::Local<'s, v8::Value> {
    let Some(state) = op_state.try_borrow_mut::<SshState>() else {
        return failure(scope, internal_error());
    };
    let removed = state.close(id);
    let object = response(scope);
    set(scope, object, "ok", v8::Boolean::new(scope, true).into());
    set(
        scope,
        object,
        "value",
        v8::Boolean::new(scope, removed).into(),
    );
    object.into()
}

deno_core::extension!(
    neojs_ssh,
    ops = [op_neo_ssh_connect, op_neo_ssh_exec, op_neo_ssh_close],
    state = |state| state.put(SshState::default()),
    docs = "Synchronous SSH command execution behind `Neo.getBuiltInModule(\"neo:ssh\")`."
);

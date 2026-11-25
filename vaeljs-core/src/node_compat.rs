//! V8 host bindings and native module registration for `node:*` compatibility modules.
//!
//! All pure logic resides in [`vaeljs_node_core`]; this module provides the
//! `deno_core` ops (synchronous and Tokio `spawn_blocking` async ops),
//! evaluates [`vaeljs_node_core::NODE_COMPAT_FACTORY_SOURCE`], and eagerly
//! registers the 11 canonical `node:*` modules via
//! [`JsRuntime::register_native_module`].

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use deno_core::FastStaticString;
use deno_core::JsRuntime;
use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::ascii_str;
use deno_core::op2;
use deno_core::v8;
use deno_error::AdditionalProperties;
use deno_error::JsErrorClass;
use deno_error::PropertyValue;
use vaeljs_node_core::buffer::{Encoding, decode as buf_decode, encode as buf_encode};
use vaeljs_node_core::child_process::{DEFAULT_MAX_BUFFER, SpawnOptions, spawn_sync};
use vaeljs_node_core::crypto::hash::{DigestState, HashAlgorithm, MAX_OPEN_DIGESTS};

const MAX_CHILD_ARGS: usize = 4096;
const MAX_CHILD_ENV: usize = 4096;
use serde::{Deserialize, Serialize};
use vaeljs_node_core::crypto::password::pbkdf2 as core_pbkdf2;
use vaeljs_node_core::fs::{
    CopyOptions, MkdirOptions, ReadOptions, RemoveOptions, WriteOptions, copy_file, exists, lstat,
    mkdir, read_dir, read_file, realpath, remove, rename, stat, write_file,
};
use vaeljs_node_core::helpers::{
    fs_error_code_str, mkdtemp, node_error_code_str, random_uuid, timing_safe_equal,
};
use vaeljs_node_core::os::{
    arch as os_arch, cpus_count, endianness, eol, homedir, hostname, machine, os_type,
    platform as os_platform, tmpdir,
};
use vaeljs_node_core::path::{
    ParsedPath, Style, basename, delimiter, dirname, extname, format, is_absolute, join, normalize,
    parse, relative, resolve, sep,
};
use vaeljs_node_core::process::{chdir, cwd, env_entries, env_get, hrtime_nanos, pid};
use vaeljs_node_core::url::{
    UrlComponent, file_url_to_path, parse_url, path_to_file_url, update_url,
};
use vaeljs_node_core::{NodeCoreError, NodeCoreErrorCode};

use crate::runtime::ProcessExitState;

#[derive(Debug)]
struct NodeOpError {
    class: &'static str,
    code: Option<&'static str>,
    message: String,
}

impl NodeOpError {
    fn new(class: &'static str, code: Option<&'static str>, message: impl Into<String>) -> Self {
        Self {
            class,
            code,
            message: message.into(),
        }
    }

    fn type_err(message: impl Into<String>) -> Self {
        Self::new("TypeError", None, message)
    }

    fn range_err(code: Option<&'static str>, message: impl Into<String>) -> Self {
        Self::new("RangeError", code, message)
    }

    fn from_core(err: NodeCoreError) -> Self {
        let code = node_error_code_str(&err);
        let class = match err.code() {
            NodeCoreErrorCode::InputTooLarge => "RangeError",
            NodeCoreErrorCode::InvalidEncoding => "TypeError",
            _ => "Error",
        };
        Self::new(class, Some(code), err.to_string())
    }

    fn from_fs(err: vaeljs_node_core::fs::FsError) -> Self {
        let code = fs_error_code_str(&err);
        Self::new("Error", Some(code), err.to_string())
    }
}

impl std::fmt::Display for NodeOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NodeOpError {}

impl JsErrorClass for NodeOpError {
    fn get_class(&self) -> Cow<'static, str> {
        Cow::Borrowed(self.class)
    }

    fn get_message(&self) -> Cow<'static, str> {
        Cow::Owned(self.message.clone())
    }

    fn get_additional_properties(&self) -> AdditionalProperties {
        match self.code {
            Some(code) => Box::new(std::iter::once((
                Cow::Borrowed("code"),
                PropertyValue::String(Cow::Borrowed(code)),
            ))),
            None => Box::new(std::iter::empty()),
        }
    }

    fn get_ref(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self
    }
}

struct NodeDigestResource(RefCell<Option<DigestState>>);

impl Resource for NodeDigestResource {
    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed("NodeDigestResource")
    }
}

fn parse_encoding(name: &str) -> Result<Encoding, NodeOpError> {
    match name {
        "utf8" | "utf-8" => Ok(Encoding::Utf8),
        "hex" => Ok(Encoding::Hex),
        "base64" => Ok(Encoding::Base64),
        "base64url" => Ok(Encoding::Base64Url),
        "latin1" | "binary" => Ok(Encoding::Latin1),
        "ascii" => Ok(Encoding::Ascii),
        _ => Err(NodeOpError::new(
            "TypeError",
            Some("ERR_UNKNOWN_ENCODING"),
            "unknown encoding",
        )),
    }
}

fn style(is_win: bool) -> Style {
    if is_win { Style::Win32 } else { Style::Posix }
}

fn current_cwd_lossy_safe() -> String {
    cwd()
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
        .unwrap_or_default()
}

fn path_to_string(path: PathBuf) -> Result<String, NodeOpError> {
    path.into_os_string()
        .into_string()
        .map_err(|_| NodeOpError::new("Error", Some("EINVAL"), "path is not valid UTF-8"))
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PathInfoDto {
    sep: &'static str,
    delimiter: &'static str,
    is_windows: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct ParsedPathDto {
    root: String,
    dir: String,
    base: String,
    ext: String,
    name: String,
}

impl From<ParsedPath> for ParsedPathDto {
    fn from(p: ParsedPath) -> Self {
        Self {
            root: p.root,
            dir: p.dir,
            base: p.base,
            ext: p.ext,
            name: p.name,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OsInfoDto {
    platform: &'static str,
    arch: &'static str,
    #[serde(rename = "EOL")]
    eol: &'static str,
    endianness: &'static str,
    #[serde(rename = "type")]
    os_type: &'static str,
    machine: &'static str,
    cpus_count: usize,
}

#[derive(Serialize)]
struct ProcessVersionsDto {
    node: &'static str,
    vjs: &'static str,
    v8: &'static str,
}

#[derive(Serialize)]
struct ProcessInfoDto {
    argv: Vec<String>,
    pid: u32,
    platform: &'static str,
    arch: &'static str,
    version: &'static str,
    versions: ProcessVersionsDto,
}

#[derive(Serialize)]
struct ParsedUrlDto {
    href: String,
    origin: String,
    protocol: String,
    username: String,
    password: String,
    host: String,
    hostname: String,
    port: String,
    pathname: String,
    search: String,
    hash: String,
}

impl From<vaeljs_node_core::url::ParsedUrl> for ParsedUrlDto {
    fn from(u: vaeljs_node_core::url::ParsedUrl) -> Self {
        Self {
            href: u.href,
            origin: u.origin,
            protocol: u.protocol,
            username: u.username,
            password: u.password,
            host: u.host,
            hostname: u.hostname,
            port: u.port,
            pathname: u.pathname,
            search: u.search,
            hash: u.hash,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatDto {
    is_file: bool,
    is_directory: bool,
    is_symbolic_link: bool,
    size: u64,
    mtime_ms: Option<i64>,
}

impl From<vaeljs_node_core::fs::FileInfo> for StatDto {
    fn from(info: vaeljs_node_core::fs::FileInfo) -> Self {
        Self {
            is_file: info.is_file,
            is_directory: info.is_directory,
            is_symbolic_link: info.is_symlink,
            size: info.size,
            mtime_ms: info.modified_ms,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirEntryDto {
    name: String,
    is_file: bool,
    is_directory: bool,
    is_symbolic_link: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct SpawnOptsDto {
    cwd: Option<String>,
    env: Option<Vec<(String, String)>>,
    timeout_ms: Option<u64>,
    max_buffer: Option<usize>,
}

fn read_v8_bytes<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    value: v8::Local<'s, v8::Value>,
    type_msg: &'static str,
) -> Result<Vec<u8>, NodeOpError> {
    let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) else {
        return Err(NodeOpError::type_err(type_msg));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(NodeOpError::type_err("ArrayBufferView buffer is detached"));
    };
    if buffer.was_detached() {
        return Err(NodeOpError::type_err("ArrayBufferView buffer is detached"));
    }
    let length = view.byte_length();
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| NodeOpError::new("RangeError", Some("ENOMEM"), "buffer allocation failed"))?;
    bytes.resize(length, 0);
    if length <= i32::MAX as usize {
        if view.copy_contents(&mut bytes) != length {
            return Err(NodeOpError::type_err(
                "ArrayBufferView buffer is detached or out of bounds",
            ));
        }
    } else {
        let store = buffer.get_backing_store();
        let offset = view.byte_offset();
        let end = offset.checked_add(length).ok_or_else(|| {
            NodeOpError::type_err("ArrayBufferView buffer is detached or out of bounds")
        })?;
        let slice = store.get(offset..end).ok_or_else(|| {
            NodeOpError::type_err("ArrayBufferView buffer is detached or out of bounds")
        })?;
        for (dst, src) in bytes.iter_mut().zip(slice.iter()) {
            *dst = src.get();
        }
    }
    Ok(bytes)
}

fn read_optional_v8_bytes<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    value: v8::Local<'s, v8::Value>,
    type_msg: &'static str,
) -> Result<Option<Vec<u8>>, NodeOpError> {
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    read_v8_bytes(scope, value, type_msg).map(Some)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpawnResultDto {
    status: Option<i32>,
    signal: Option<String>,
    stdout: deno_core::ToJsBuffer,
    stderr: deno_core::ToJsBuffer,
    error_code: Option<&'static str>,
    error_message: Option<String>,
    timed_out: bool,
}

// ---------------------------------------------------------------------------
// path ops
// ---------------------------------------------------------------------------

#[op2]
#[serde]
fn op_node_path_info() -> PathInfoDto {
    let native = Style::native();
    PathInfoDto {
        sep: sep(native),
        delimiter: delimiter(native),
        is_windows: native == Style::Win32,
    }
}

#[op2]
#[string]
fn op_node_path_join(is_win: bool, #[serde] parts: Vec<String>) -> Result<String, NodeOpError> {
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    join(style(is_win), &refs).map_err(NodeOpError::from_core)
}

#[op2]
#[string]
fn op_node_path_resolve(is_win: bool, #[serde] parts: Vec<String>) -> Result<String, NodeOpError> {
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    let cwd_str = current_cwd_lossy_safe();
    resolve(style(is_win), &cwd_str, &refs).map_err(NodeOpError::from_core)
}

#[op2]
#[string]
fn op_node_path_normalize(is_win: bool, #[string] p: String) -> Result<String, NodeOpError> {
    normalize(style(is_win), &p).map_err(NodeOpError::from_core)
}

#[op2]
#[string]
fn op_node_path_relative(
    is_win: bool,
    #[string] from: String,
    #[string] to: String,
) -> Result<String, NodeOpError> {
    relative(style(is_win), &from, &to).map_err(NodeOpError::from_core)
}

#[op2]
#[string]
fn op_node_path_dirname(is_win: bool, #[string] p: String) -> Result<String, NodeOpError> {
    dirname(style(is_win), &p).map_err(NodeOpError::from_core)
}

#[op2]
#[string]
fn op_node_path_basename(
    is_win: bool,
    #[string] p: String,
    #[string] suffix: Option<String>,
) -> Result<String, NodeOpError> {
    basename(style(is_win), &p, suffix.as_deref()).map_err(NodeOpError::from_core)
}

#[op2]
#[string]
fn op_node_path_extname(is_win: bool, #[string] p: String) -> Result<String, NodeOpError> {
    extname(style(is_win), &p).map_err(NodeOpError::from_core)
}

#[op2]
#[serde]
fn op_node_path_parse(is_win: bool, #[string] p: String) -> Result<ParsedPathDto, NodeOpError> {
    parse(style(is_win), &p)
        .map(ParsedPathDto::from)
        .map_err(NodeOpError::from_core)
}

#[op2]
#[string]
fn op_node_path_format(is_win: bool, #[serde] obj: ParsedPathDto) -> Result<String, NodeOpError> {
    let parsed = ParsedPath {
        root: obj.root,
        dir: obj.dir,
        base: obj.base,
        ext: obj.ext,
        name: obj.name,
    };
    format(style(is_win), &parsed).map_err(NodeOpError::from_core)
}

#[op2(fast)]
fn op_node_path_is_absolute(is_win: bool, #[string] p: String) -> Result<bool, NodeOpError> {
    is_absolute(style(is_win), &p).map_err(NodeOpError::from_core)
}

// ---------------------------------------------------------------------------
// os ops
// ---------------------------------------------------------------------------

#[op2]
#[serde]
fn op_node_os_info() -> OsInfoDto {
    OsInfoDto {
        platform: os_platform(),
        arch: os_arch(),
        eol: eol(),
        endianness: endianness(),
        os_type: os_type(),
        machine: machine(),
        cpus_count: cpus_count(),
    }
}

#[op2]
#[string]
fn op_node_os_homedir() -> Result<String, NodeOpError> {
    homedir()
        .map_err(NodeOpError::from_core)
        .and_then(path_to_string)
}

#[op2]
#[string]
fn op_node_os_tmpdir() -> Result<String, NodeOpError> {
    path_to_string(tmpdir())
}

#[op2]
#[string]
fn op_node_os_hostname() -> Result<String, NodeOpError> {
    hostname().map_err(NodeOpError::from_core)
}

// ---------------------------------------------------------------------------
// process ops
// ---------------------------------------------------------------------------

#[op2]
#[serde]
fn op_node_process_info() -> ProcessInfoDto {
    let mut argv: Vec<String> = std::env::args_os()
        .filter_map(|a| a.into_string().ok())
        .collect();
    if argv.is_empty() {
        argv.push("vjs".to_owned());
    }
    ProcessInfoDto {
        argv,
        pid: pid(),
        platform: os_platform(),
        arch: os_arch(),
        version: "v22.0.0",
        versions: ProcessVersionsDto {
            node: "22.0.0",
            vjs: env!("CARGO_PKG_VERSION"),
            v8: v8::VERSION_STRING,
        },
    }
}

#[op2]
#[string]
fn op_node_process_cwd() -> Result<String, NodeOpError> {
    cwd()
        .map_err(NodeOpError::from_core)
        .and_then(path_to_string)
}

#[op2(fast)]
fn op_node_process_chdir(#[string] dir: String) -> Result<(), NodeOpError> {
    chdir(dir).map_err(NodeOpError::from_core)
}

#[op2]
#[string]
fn op_node_process_env_get(#[string] key: String) -> Result<Option<String>, NodeOpError> {
    if key.is_empty() || key.contains('\0') || key.contains('=') {
        return Ok(None);
    }
    env_get(&key).map_err(NodeOpError::from_core)
}

#[op2]
#[serde]
fn op_node_process_env_entries() -> Vec<(String, String)> {
    env_entries()
}

#[op2]
#[string]
fn op_node_process_hrtime_nanos() -> String {
    hrtime_nanos().to_string()
}

#[op2]
fn op_node_process_get_exit_code<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
) -> v8::Local<'s, v8::Value> {
    let code = state
        .borrow::<Rc<RefCell<ProcessExitState>>>()
        .borrow()
        .exit_code;
    match code {
        Some(c) => v8::Integer::new(scope, c).into(),
        None => v8::undefined(scope).into(),
    }
}

#[op2]
fn op_node_process_set_exit_code(state: &mut OpState, #[smi] code: Option<i32>) {
    state
        .borrow::<Rc<RefCell<ProcessExitState>>>()
        .borrow_mut()
        .exit_code = code;
}

#[op2(fast)]
fn op_node_process_exit<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    state: &mut OpState,
    code: v8::Local<'s, v8::Value>,
) {
    let parsed = if code.is_undefined() || code.is_null() {
        None
    } else {
        code.int32_value(scope)
    };
    let exit_state = state.borrow::<Rc<RefCell<ProcessExitState>>>().clone();
    {
        let mut guard = exit_state.borrow_mut();
        guard.exited = true;
        if let Some(c) = parsed {
            guard.exit_code = Some(c);
        } else if guard.exit_code.is_none() {
            guard.exit_code = Some(0);
        }
    }
    scope.terminate_execution();
}

// ---------------------------------------------------------------------------
// buffer ops
// ---------------------------------------------------------------------------

#[op2]
#[string]
fn op_node_buffer_encode(
    #[buffer] bytes: &[u8],
    #[string] encoding: String,
) -> Result<String, NodeOpError> {
    let enc = parse_encoding(&encoding)?;
    Ok(buf_encode(bytes, enc))
}

#[op2]
#[buffer]
fn op_node_buffer_decode(
    #[string] input: String,
    #[string] encoding: String,
) -> Result<Vec<u8>, NodeOpError> {
    let enc = parse_encoding(&encoding)?;
    buf_decode(&input, enc).map_err(NodeOpError::from_core)
}

#[op2(fast)]
#[number]
fn op_node_buffer_byte_length(
    #[string] input: String,
    #[string] encoding: String,
) -> Result<usize, NodeOpError> {
    let enc = parse_encoding(&encoding)?;
    match enc {
        Encoding::Utf8 => Ok(input.len()),
        Encoding::Latin1 | Encoding::Ascii => Ok(input.encode_utf16().count()),
        Encoding::Hex => Ok(input.len() / 2),
        Encoding::Base64 | Encoding::Base64Url => buf_decode(&input, enc)
            .map(|v| v.len())
            .map_err(NodeOpError::from_core),
    }
}

// ---------------------------------------------------------------------------
// url ops
// ---------------------------------------------------------------------------

#[op2]
#[string]
fn op_node_url_file_to_path(#[string] url_str: String) -> Result<String, NodeOpError> {
    file_url_to_path(&url_str)
        .map_err(NodeOpError::from_core)
        .and_then(path_to_string)
}

#[op2]
#[string]
fn op_node_url_path_to_file(#[string] path_str: String) -> Result<String, NodeOpError> {
    path_to_file_url(&path_str).map_err(NodeOpError::from_core)
}

#[op2]
#[serde]
fn op_node_url_parse(
    #[string] input: String,
    #[string] base: Option<String>,
) -> Result<ParsedUrlDto, NodeOpError> {
    parse_url(&input, base.as_deref())
        .map(ParsedUrlDto::from)
        .map_err(|err| NodeOpError::new("TypeError", Some("ERR_INVALID_URL"), err.to_string()))
}

#[op2]
#[serde]
fn op_node_url_update(
    #[string] href: String,
    #[string] component: String,
    #[string] value: String,
) -> Result<ParsedUrlDto, NodeOpError> {
    let comp = UrlComponent::parse(&component)
        .ok_or_else(|| NodeOpError::type_err("invalid URL component"))?;
    update_url(&href, comp, &value)
        .map(ParsedUrlDto::from)
        .map_err(|err| NodeOpError::new("TypeError", Some("ERR_INVALID_URL"), err.to_string()))
}

// ---------------------------------------------------------------------------
// fs sync ops
// ---------------------------------------------------------------------------

fn unlink_impl(path: &str) -> Result<(), NodeOpError> {
    let info = lstat(path).map_err(NodeOpError::from_fs)?;
    if info.is_directory {
        return Err(NodeOpError::new(
            "Error",
            Some("EISDIR"),
            "illegal operation on a directory, unlink",
        ));
    }
    remove(path, RemoveOptions { recursive: false }).map_err(NodeOpError::from_fs)
}

fn rm_impl(path: &str, recursive: bool, force: bool) -> Result<(), NodeOpError> {
    match remove(path, RemoveOptions { recursive }) {
        Ok(()) => Ok(()),
        Err(err) if force && err.code() == vaeljs_node_core::fs::FsErrorCode::NotFound => Ok(()),
        Err(err) => Err(NodeOpError::from_fs(err)),
    }
}

#[op2]
#[buffer]
fn op_node_fs_read_file_sync(#[string] path: String) -> Result<Vec<u8>, NodeOpError> {
    read_file(path, ReadOptions::default()).map_err(NodeOpError::from_fs)
}

#[op2(fast)]
fn op_node_fs_write_file_sync<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    #[string] path: String,
    data: v8::Local<'s, v8::Value>,
    append: bool,
) -> Result<(), NodeOpError> {
    let bytes = read_v8_bytes(scope, data, "data must be a TypedArray or DataView")?;
    let opts = WriteOptions {
        create: true,
        create_new: false,
        append,
    };
    write_file(path, &bytes, opts).map_err(NodeOpError::from_fs)
}

#[op2(fast)]
fn op_node_fs_exists_sync(#[string] path: String) -> Result<bool, NodeOpError> {
    exists(path).map_err(NodeOpError::from_fs)
}

#[op2]
#[serde]
fn op_node_fs_stat_sync(#[string] path: String, follow: bool) -> Result<StatDto, NodeOpError> {
    let info = if follow {
        stat(path).map_err(NodeOpError::from_fs)?
    } else {
        lstat(path).map_err(NodeOpError::from_fs)?
    };
    Ok(StatDto::from(info))
}

#[op2]
#[serde]
fn op_node_fs_readdir_sync(#[string] path: String) -> Result<Vec<DirEntryDto>, NodeOpError> {
    let entries = read_dir(path).map_err(NodeOpError::from_fs)?;
    Ok(entries
        .into_iter()
        .map(|e| DirEntryDto {
            name: e.name,
            is_file: e.is_file,
            is_directory: e.is_directory,
            is_symbolic_link: e.is_symlink,
        })
        .collect())
}

#[op2(fast)]
fn op_node_fs_mkdir_sync(#[string] path: String, recursive: bool) -> Result<(), NodeOpError> {
    mkdir(path, MkdirOptions { recursive }).map_err(NodeOpError::from_fs)
}

#[op2(fast)]
fn op_node_fs_rm_sync(
    #[string] path: String,
    recursive: bool,
    force: bool,
) -> Result<(), NodeOpError> {
    rm_impl(&path, recursive, force)
}

#[op2(fast)]
fn op_node_fs_unlink_sync(#[string] path: String) -> Result<(), NodeOpError> {
    unlink_impl(&path)
}

#[op2(fast)]
fn op_node_fs_rename_sync(
    #[string] old_path: String,
    #[string] new_path: String,
) -> Result<(), NodeOpError> {
    rename(old_path, new_path).map_err(NodeOpError::from_fs)
}

#[op2(fast)]
fn op_node_fs_copy_file_sync(
    #[string] src: String,
    #[string] dest: String,
    overwrite: bool,
) -> Result<(), NodeOpError> {
    copy_file(src, dest, CopyOptions { overwrite }).map_err(NodeOpError::from_fs)
}

#[op2]
#[string]
fn op_node_fs_realpath_sync(#[string] path: String) -> Result<String, NodeOpError> {
    realpath(path)
        .map_err(NodeOpError::from_fs)
        .and_then(path_to_string)
}

#[op2]
#[string]
fn op_node_fs_mkdtemp_sync(#[string] prefix: String) -> Result<String, NodeOpError> {
    mkdtemp(&prefix)
        .map_err(NodeOpError::from_core)
        .and_then(path_to_string)
}

// ---------------------------------------------------------------------------
// fs/promises async ops (Tokio spawn_blocking)
// ---------------------------------------------------------------------------

async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, NodeOpError> + Send + 'static,
) -> Result<T, NodeOpError> {
    tokio::task::spawn_blocking(f).await.map_err(|err| {
        NodeOpError::new("Error", Some("EIO"), format!("async task failed: {err}"))
    })?
}

#[op2]
#[buffer]
async fn op_node_fs_read_file_async(#[string] path: String) -> Result<Vec<u8>, NodeOpError> {
    run_blocking(move || read_file(path, ReadOptions::default()).map_err(NodeOpError::from_fs))
        .await
}

#[op2]
async fn op_node_fs_write_file_async(
    #[string] path: String,
    #[buffer(copy)] bytes: Vec<u8>,
    append: bool,
) -> Result<(), NodeOpError> {
    run_blocking(move || {
        let opts = WriteOptions {
            create: true,
            create_new: false,
            append,
        };
        write_file(path, &bytes, opts).map_err(NodeOpError::from_fs)
    })
    .await
}

#[op2]
#[serde]
async fn op_node_fs_stat_async(
    #[string] path: String,
    follow: bool,
) -> Result<StatDto, NodeOpError> {
    run_blocking(move || {
        let info = if follow {
            stat(path).map_err(NodeOpError::from_fs)?
        } else {
            lstat(path).map_err(NodeOpError::from_fs)?
        };
        Ok(StatDto::from(info))
    })
    .await
}

#[op2]
#[serde]
async fn op_node_fs_readdir_async(#[string] path: String) -> Result<Vec<DirEntryDto>, NodeOpError> {
    run_blocking(move || {
        let entries = read_dir(path).map_err(NodeOpError::from_fs)?;
        Ok(entries
            .into_iter()
            .map(|e| DirEntryDto {
                name: e.name,
                is_file: e.is_file,
                is_directory: e.is_directory,
                is_symbolic_link: e.is_symlink,
            })
            .collect())
    })
    .await
}

#[op2]
async fn op_node_fs_mkdir_async(
    #[string] path: String,
    recursive: bool,
) -> Result<(), NodeOpError> {
    run_blocking(move || mkdir(path, MkdirOptions { recursive }).map_err(NodeOpError::from_fs))
        .await
}

#[op2]
async fn op_node_fs_rm_async(
    #[string] path: String,
    recursive: bool,
    force: bool,
) -> Result<(), NodeOpError> {
    run_blocking(move || rm_impl(&path, recursive, force)).await
}

#[op2]
async fn op_node_fs_unlink_async(#[string] path: String) -> Result<(), NodeOpError> {
    run_blocking(move || unlink_impl(&path)).await
}

#[op2]
async fn op_node_fs_rename_async(
    #[string] old_path: String,
    #[string] new_path: String,
) -> Result<(), NodeOpError> {
    run_blocking(move || rename(old_path, new_path).map_err(NodeOpError::from_fs)).await
}

#[op2]
async fn op_node_fs_copy_file_async(
    #[string] src: String,
    #[string] dest: String,
    overwrite: bool,
) -> Result<(), NodeOpError> {
    run_blocking(move || {
        copy_file(src, dest, CopyOptions { overwrite }).map_err(NodeOpError::from_fs)
    })
    .await
}

#[op2]
#[string]
async fn op_node_fs_realpath_async(#[string] path: String) -> Result<String, NodeOpError> {
    run_blocking(move || {
        realpath(path)
            .map_err(NodeOpError::from_fs)
            .and_then(path_to_string)
    })
    .await
}

#[op2]
#[string]
async fn op_node_fs_mkdtemp_async(#[string] prefix: String) -> Result<String, NodeOpError> {
    run_blocking(move || {
        mkdtemp(&prefix)
            .map_err(NodeOpError::from_core)
            .and_then(path_to_string)
    })
    .await
}

// ---------------------------------------------------------------------------
// child_process ops
// ---------------------------------------------------------------------------

fn build_spawn_options(dto: SpawnOptsDto, stdin: Option<Vec<u8>>) -> SpawnOptions {
    let has_env = dto.env.is_some();
    SpawnOptions {
        cwd: dto.cwd.map(PathBuf::from),
        env: dto.env,
        clear_env: has_env,
        stdin,
        timeout_ms: dto.timeout_ms,
        max_buffer: dto.max_buffer.unwrap_or(DEFAULT_MAX_BUFFER),
    }
}

fn validate_child_limits(args: &[String], opts: &SpawnOptsDto) -> Result<(), NodeOpError> {
    if args.len() > MAX_CHILD_ARGS {
        return Err(NodeOpError::new(
            "RangeError",
            Some("ERR_OUT_OF_RANGE"),
            format!("args length exceeds maximum ({MAX_CHILD_ARGS})"),
        ));
    }
    if let Some(env) = &opts.env
        && env.len() > MAX_CHILD_ENV
    {
        return Err(NodeOpError::new(
            "RangeError",
            Some("ERR_OUT_OF_RANGE"),
            format!("options.env exceeds maximum entries ({MAX_CHILD_ENV})"),
        ));
    }
    Ok(())
}

fn spawn_impl(
    cmd: &str,
    args: &[String],
    opts: &SpawnOptions,
) -> Result<SpawnResultDto, NodeOpError> {
    match spawn_sync(cmd, args, opts) {
        Ok(out) => Ok(SpawnResultDto {
            status: out.status,
            signal: out.signal,
            stdout: out.stdout.into(),
            stderr: out.stderr.into(),
            error_code: if out.timed_out {
                Some("ETIMEDOUT")
            } else {
                None
            },
            error_message: if out.timed_out {
                Some("Command timed out".to_owned())
            } else {
                None
            },
            timed_out: out.timed_out,
        }),
        Err(err)
            if matches!(
                err.code(),
                NodeCoreErrorCode::NotFound
                    | NodeCoreErrorCode::PermissionDenied
                    | NodeCoreErrorCode::Io
                    | NodeCoreErrorCode::ProcessError
            ) =>
        {
            Ok(SpawnResultDto {
                status: None,
                signal: None,
                stdout: Vec::new().into(),
                stderr: Vec::new().into(),
                error_code: Some(node_error_code_str(&err)),
                error_message: Some(err.to_string()),
                timed_out: false,
            })
        }
        Err(err) => Err(NodeOpError::from_core(err)),
    }
}

#[op2]
#[serde]
fn op_node_child_spawn_sync<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i>,
    #[string] command: String,
    #[serde] args: Vec<String>,
    #[serde] opts: SpawnOptsDto,
    input: v8::Local<'s, v8::Value>,
) -> Result<SpawnResultDto, NodeOpError> {
    validate_child_limits(&args, &opts)?;
    let stdin = read_optional_v8_bytes(scope, input, "input must be a TypedArray or DataView")?;
    let spawn_opts = build_spawn_options(opts, stdin);
    spawn_impl(&command, &args, &spawn_opts)
}

#[op2]
#[serde]
async fn op_node_child_exec_file_async(
    #[string] command: String,
    #[serde] args: Vec<String>,
    #[serde] opts: SpawnOptsDto,
    has_input: bool,
    #[buffer(copy)] input: Vec<u8>,
) -> Result<SpawnResultDto, NodeOpError> {
    validate_child_limits(&args, &opts)?;
    let stdin = if has_input { Some(input) } else { None };
    let spawn_opts = build_spawn_options(opts, stdin);
    run_blocking(move || spawn_impl(&command, &args, &spawn_opts)).await
}

// ---------------------------------------------------------------------------
// crypto ops
// ---------------------------------------------------------------------------

#[op2]
#[buffer]
fn op_node_crypto_random_bytes(#[number] size: usize) -> Result<Vec<u8>, NodeOpError> {
    vaeljs_node_core::crypto::envelope::random_bytes(size).map_err(|err| {
        let class = match err.code() {
            vaeljs_node_core::crypto::envelope::EnvelopeErrorCode::Limit => "RangeError",
            _ => "Error",
        };
        NodeOpError::new(class, Some(err.code().as_str()), err.to_string())
    })
}

#[op2]
#[string]
fn op_node_crypto_random_uuid() -> Result<String, NodeOpError> {
    random_uuid().map_err(NodeOpError::from_core)
}

fn ensure_digest_capacity(state: &OpState) -> Result<(), NodeOpError> {
    let open = state
        .resource_table
        .names()
        .filter(|(_, name)| *name == "NodeDigestResource")
        .count();
    if open >= MAX_OPEN_DIGESTS {
        return Err(NodeOpError::new(
            "RangeError",
            Some("ERR_CRYPTO_TOO_MANY_OBJECTS"),
            "too many open digest handles",
        ));
    }
    Ok(())
}

#[op2(fast)]
#[smi]
fn op_node_crypto_hash_create(
    state: &mut OpState,
    #[string] algorithm: String,
) -> Result<ResourceId, NodeOpError> {
    ensure_digest_capacity(state)?;
    let alg = HashAlgorithm::parse(&algorithm).ok_or_else(|| {
        NodeOpError::new(
            "Error",
            Some("ERR_CRYPTO_HASH_UNKNOWN"),
            "Digest method not supported",
        )
    })?;
    let res = NodeDigestResource(RefCell::new(Some(DigestState::hash(alg))));
    Ok(state.resource_table.add(res))
}

#[op2(fast)]
#[smi]
fn op_node_crypto_hmac_create(
    state: &mut OpState,
    #[string] algorithm: String,
    #[buffer] key: &[u8],
) -> Result<ResourceId, NodeOpError> {
    ensure_digest_capacity(state)?;
    let alg = HashAlgorithm::parse(&algorithm).ok_or_else(|| {
        NodeOpError::new(
            "Error",
            Some("ERR_CRYPTO_HASH_UNKNOWN"),
            "Digest method not supported",
        )
    })?;
    let digest = DigestState::hmac(alg, key).map_err(|err| {
        let class = if err.code() == vaeljs_node_core::crypto::hash::HashErrorCode::Limit {
            "RangeError"
        } else {
            "Error"
        };
        NodeOpError::new(class, Some(err.code().as_str()), err.to_string())
    })?;
    let res = NodeDigestResource(RefCell::new(Some(digest)));
    Ok(state.resource_table.add(res))
}

#[op2(fast)]
fn op_node_crypto_hash_update(
    state: &mut OpState,
    #[smi] id: ResourceId,
    #[buffer] data: &[u8],
) -> Result<(), NodeOpError> {
    let res = state
        .resource_table
        .get::<NodeDigestResource>(id)
        .map_err(|_| {
            NodeOpError::new(
                "Error",
                Some("ERR_CRYPTO_INVALID_STATE"),
                "Digest already called",
            )
        })?;
    let mut guard = res.0.borrow_mut();
    let Some(digest) = guard.as_mut() else {
        return Err(NodeOpError::new(
            "Error",
            Some("ERR_CRYPTO_INVALID_STATE"),
            "Digest already called",
        ));
    };
    digest
        .update(data)
        .map_err(|err| NodeOpError::new("Error", Some(err.code().as_str()), err.to_string()))
}

#[op2]
#[buffer]
fn op_node_crypto_hash_digest(
    state: &mut OpState,
    #[smi] id: ResourceId,
) -> Result<Vec<u8>, NodeOpError> {
    let res = state
        .resource_table
        .take::<NodeDigestResource>(id)
        .map_err(|_| {
            NodeOpError::new(
                "Error",
                Some("ERR_CRYPTO_INVALID_STATE"),
                "Digest already called",
            )
        })?;
    let digest = res.0.borrow_mut().take().ok_or_else(|| {
        NodeOpError::new(
            "Error",
            Some("ERR_CRYPTO_INVALID_STATE"),
            "Digest already called",
        )
    })?;
    Ok(digest.finalize())
}

#[op2]
#[buffer]
fn op_node_crypto_pbkdf2_sync(
    #[buffer] password: &[u8],
    #[buffer] salt: &[u8],
    #[smi] iterations: u32,
    #[number] keylen: usize,
    #[string] digest: String,
) -> Result<Vec<u8>, NodeOpError> {
    let alg = HashAlgorithm::parse(&digest).ok_or_else(|| {
        NodeOpError::new(
            "TypeError",
            Some("ERR_CRYPTO_INVALID_DIGEST"),
            "Invalid digest",
        )
    })?;
    if keylen == 0 {
        return Ok(Vec::new());
    }
    core_pbkdf2(alg, password, salt, iterations, keylen).map_err(|err| {
        let class = match err.code() {
            vaeljs_node_core::crypto::password::PasswordErrorCode::Range => "RangeError",
            _ => "Error",
        };
        NodeOpError::new(class, err.code().as_str(), err.to_string())
    })
}

#[op2(fast)]
fn op_node_crypto_timing_safe_equal(
    #[buffer] a: &[u8],
    #[buffer] b: &[u8],
) -> Result<bool, NodeOpError> {
    timing_safe_equal(a, b).map_err(|err| {
        NodeOpError::range_err(Some("ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH"), err.to_string())
    })
}

deno_core::extension!(
    vaeljs_node_compat,
    ops = [
        op_node_path_info,
        op_node_path_join,
        op_node_path_resolve,
        op_node_path_normalize,
        op_node_path_relative,
        op_node_path_dirname,
        op_node_path_basename,
        op_node_path_extname,
        op_node_path_parse,
        op_node_path_format,
        op_node_path_is_absolute,
        op_node_os_info,
        op_node_os_homedir,
        op_node_os_tmpdir,
        op_node_os_hostname,
        op_node_process_info,
        op_node_process_cwd,
        op_node_process_chdir,
        op_node_process_env_get,
        op_node_process_env_entries,
        op_node_process_hrtime_nanos,
        op_node_process_get_exit_code,
        op_node_process_set_exit_code,
        op_node_process_exit,
        op_node_buffer_encode,
        op_node_buffer_decode,
        op_node_buffer_byte_length,
        op_node_url_file_to_path,
        op_node_url_path_to_file,
        op_node_url_parse,
        op_node_url_update,
        op_node_fs_read_file_sync,
        op_node_fs_write_file_sync,
        op_node_fs_exists_sync,
        op_node_fs_stat_sync,
        op_node_fs_readdir_sync,
        op_node_fs_mkdir_sync,
        op_node_fs_rm_sync,
        op_node_fs_unlink_sync,
        op_node_fs_rename_sync,
        op_node_fs_copy_file_sync,
        op_node_fs_realpath_sync,
        op_node_fs_mkdtemp_sync,
        op_node_fs_read_file_async,
        op_node_fs_write_file_async,
        op_node_fs_stat_async,
        op_node_fs_readdir_async,
        op_node_fs_mkdir_async,
        op_node_fs_rm_async,
        op_node_fs_unlink_async,
        op_node_fs_rename_async,
        op_node_fs_copy_file_async,
        op_node_fs_realpath_async,
        op_node_fs_mkdtemp_async,
        op_node_child_spawn_sync,
        op_node_child_exec_file_async,
        op_node_crypto_random_bytes,
        op_node_crypto_random_uuid,
        op_node_crypto_hash_create,
        op_node_crypto_hmac_create,
        op_node_crypto_hash_update,
        op_node_crypto_hash_digest,
        op_node_crypto_pbkdf2_sync,
        op_node_crypto_timing_safe_equal,
    ],
    docs = "Private host ops for Node.js compatibility modules (`node:*`)."
);

const PATH_EXPORTS: &[FastStaticString] = &[
    ascii_str!("sep"),
    ascii_str!("delimiter"),
    ascii_str!("join"),
    ascii_str!("resolve"),
    ascii_str!("normalize"),
    ascii_str!("relative"),
    ascii_str!("dirname"),
    ascii_str!("basename"),
    ascii_str!("extname"),
    ascii_str!("parse"),
    ascii_str!("format"),
    ascii_str!("isAbsolute"),
    ascii_str!("posix"),
    ascii_str!("win32"),
];

const OS_EXPORTS: &[FastStaticString] = &[
    ascii_str!("platform"),
    ascii_str!("arch"),
    ascii_str!("EOL"),
    ascii_str!("homedir"),
    ascii_str!("tmpdir"),
    ascii_str!("hostname"),
    ascii_str!("cpus"),
    ascii_str!("endianness"),
    ascii_str!("type"),
    ascii_str!("machine"),
];

const PROCESS_EXPORTS: &[FastStaticString] = &[
    ascii_str!("argv"),
    ascii_str!("cwd"),
    ascii_str!("chdir"),
    ascii_str!("env"),
    ascii_str!("exit"),
    ascii_str!("exitCode"),
    ascii_str!("platform"),
    ascii_str!("arch"),
    ascii_str!("pid"),
    ascii_str!("version"),
    ascii_str!("versions"),
    ascii_str!("hrtime"),
];

const FS_EXPORTS: &[FastStaticString] = &[
    ascii_str!("readFileSync"),
    ascii_str!("writeFileSync"),
    ascii_str!("appendFileSync"),
    ascii_str!("existsSync"),
    ascii_str!("statSync"),
    ascii_str!("lstatSync"),
    ascii_str!("readdirSync"),
    ascii_str!("mkdirSync"),
    ascii_str!("rmSync"),
    ascii_str!("unlinkSync"),
    ascii_str!("renameSync"),
    ascii_str!("copyFileSync"),
    ascii_str!("realpathSync"),
    ascii_str!("mkdtempSync"),
    ascii_str!("constants"),
    ascii_str!("promises"),
];

const FS_PROMISES_EXPORTS: &[FastStaticString] = &[
    ascii_str!("readFile"),
    ascii_str!("writeFile"),
    ascii_str!("appendFile"),
    ascii_str!("stat"),
    ascii_str!("lstat"),
    ascii_str!("readdir"),
    ascii_str!("mkdir"),
    ascii_str!("rm"),
    ascii_str!("unlink"),
    ascii_str!("rename"),
    ascii_str!("copyFile"),
    ascii_str!("realpath"),
    ascii_str!("mkdtemp"),
    ascii_str!("constants"),
];

const CHILD_PROCESS_EXPORTS: &[FastStaticString] = &[
    ascii_str!("spawnSync"),
    ascii_str!("execFileSync"),
    ascii_str!("execFile"),
];

const BUFFER_EXPORTS: &[FastStaticString] = &[ascii_str!("Buffer")];

const URL_EXPORTS: &[FastStaticString] = &[
    ascii_str!("fileURLToPath"),
    ascii_str!("pathToFileURL"),
    ascii_str!("URL"),
    ascii_str!("URLSearchParams"),
];

const CRYPTO_EXPORTS: &[FastStaticString] = &[
    ascii_str!("randomBytes"),
    ascii_str!("randomUUID"),
    ascii_str!("createHash"),
    ascii_str!("createHmac"),
    ascii_str!("pbkdf2Sync"),
    ascii_str!("timingSafeEqual"),
];

const MODULE_SPECS: &[(&str, &[FastStaticString])] = &[
    ("node:path", PATH_EXPORTS),
    ("node:path/posix", PATH_EXPORTS),
    ("node:path/win32", PATH_EXPORTS),
    ("node:os", OS_EXPORTS),
    ("node:process", PROCESS_EXPORTS),
    ("node:fs", FS_EXPORTS),
    ("node:fs/promises", FS_PROMISES_EXPORTS),
    ("node:child_process", CHILD_PROCESS_EXPORTS),
    ("node:buffer", BUFFER_EXPORTS),
    ("node:url", URL_EXPORTS),
    ("node:crypto", CRYPTO_EXPORTS),
];

fn build_v8_node_bootstrap() -> String {
    format!(
        r#"(() => {{
    const ops = Deno.core.ops;
    const host = {{
        isV8: true,
        pathInfo: ops.op_node_path_info,
        pathJoin: ops.op_node_path_join,
        pathResolve: ops.op_node_path_resolve,
        pathNormalize: ops.op_node_path_normalize,
        pathRelative: ops.op_node_path_relative,
        pathDirname: ops.op_node_path_dirname,
        pathBasename: ops.op_node_path_basename,
        pathExtname: ops.op_node_path_extname,
        pathParse: ops.op_node_path_parse,
        pathFormat: ops.op_node_path_format,
        pathIsAbsolute: ops.op_node_path_is_absolute,
        osInfo: ops.op_node_os_info,
        osHomedir: ops.op_node_os_homedir,
        osTmpdir: ops.op_node_os_tmpdir,
        osHostname: ops.op_node_os_hostname,
        processInfo: ops.op_node_process_info,
        processCwd: ops.op_node_process_cwd,
        processChdir: ops.op_node_process_chdir,
        processEnvGet: ops.op_node_process_env_get,
        processEnvEntries: ops.op_node_process_env_entries,
        processHrtimeNanos: ops.op_node_process_hrtime_nanos,
        processGetExitCode: ops.op_node_process_get_exit_code,
        processSetExitCode: ops.op_node_process_set_exit_code,
        processExit: ops.op_node_process_exit,
        bufferEncode: ops.op_node_buffer_encode,
        bufferDecode: ops.op_node_buffer_decode,
        bufferByteLength: ops.op_node_buffer_byte_length,
        urlFileToPath: ops.op_node_url_file_to_path,
        urlPathToFile: ops.op_node_url_path_to_file,
        urlParse: ops.op_node_url_parse,
        urlUpdate: ops.op_node_url_update,
        fsReadFileSync: ops.op_node_fs_read_file_sync,
        fsWriteFileSync: ops.op_node_fs_write_file_sync,
        fsExistsSync: ops.op_node_fs_exists_sync,
        fsStatSync: ops.op_node_fs_stat_sync,
        fsReaddirSync: ops.op_node_fs_readdir_sync,
        fsMkdirSync: ops.op_node_fs_mkdir_sync,
        fsRmSync: ops.op_node_fs_rm_sync,
        fsUnlinkSync: ops.op_node_fs_unlink_sync,
        fsRenameSync: ops.op_node_fs_rename_sync,
        fsCopyFileSync: ops.op_node_fs_copy_file_sync,
        fsRealpathSync: ops.op_node_fs_realpath_sync,
        fsMkdtempSync: ops.op_node_fs_mkdtemp_sync,
        fsReadFileAsync: ops.op_node_fs_read_file_async,
        fsWriteFileAsync: ops.op_node_fs_write_file_async,
        fsStatAsync: ops.op_node_fs_stat_async,
        fsReaddirAsync: ops.op_node_fs_readdir_async,
        fsMkdirAsync: ops.op_node_fs_mkdir_async,
        fsRmAsync: ops.op_node_fs_rm_async,
        fsUnlinkAsync: ops.op_node_fs_unlink_async,
        fsRenameAsync: ops.op_node_fs_rename_async,
        fsCopyFileAsync: ops.op_node_fs_copy_file_async,
        fsRealpathAsync: ops.op_node_fs_realpath_async,
        fsMkdtempAsync: ops.op_node_fs_mkdtemp_async,
        childSpawnSync: (cmd, args, opts) => ops.op_node_child_spawn_sync(cmd, args, opts, opts?.input),
        childExecFileAsync: (cmd, args, opts) =>
            (opts?.input !== undefined && opts?.input !== null)
                ? ops.op_node_child_exec_file_async(cmd, args, opts, true, opts.input)
                : ops.op_node_child_exec_file_async(cmd, args, opts, false, new Uint8Array(0)),
        cryptoRandomBytes: ops.op_node_crypto_random_bytes,
        cryptoRandomUUID: ops.op_node_crypto_random_uuid,
        cryptoHashCreate: ops.op_node_crypto_hash_create,
        cryptoHmacCreate: ops.op_node_crypto_hmac_create,
        cryptoHashUpdate: ops.op_node_crypto_hash_update,
        cryptoHashDigest: ops.op_node_crypto_hash_digest,
        cryptoPbkdf2Sync: ops.op_node_crypto_pbkdf2_sync,
        cryptoTimingSafeEqual: ops.op_node_crypto_timing_safe_equal,
    }};
    return ({factory})(host);
}})();
"#,
        factory = vaeljs_node_core::NODE_COMPAT_FACTORY_SOURCE
    )
}

/// Installs [`ProcessExitState`] in `OpState`, constructs all 11 `node:*` module
/// objects inside an isolated closure, and registers each as a native V8 module
/// with named + default exports.
pub(crate) fn install(
    js: &mut JsRuntime,
    exit_state: Rc<RefCell<ProcessExitState>>,
) -> Result<(), String> {
    js.op_state().borrow_mut().put(exit_state);
    let modules_val = js
        .execute_script("<vaeljs:node>", build_v8_node_bootstrap())
        .map_err(|err| err.to_string())?;

    type ModuleRegistration = (&'static str, Vec<(FastStaticString, v8::Global<v8::Value>)>);

    let mut prepared: Vec<ModuleRegistration> = Vec::with_capacity(MODULE_SPECS.len());

    {
        let context = js.main_context();
        v8::scope!(let scope, js.v8_isolate());
        let context_local = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context_local);

        let root_val = v8::Local::new(scope, modules_val);
        let root_obj = v8::Local::<v8::Object>::try_from(root_val)
            .map_err(|_| "node compat bootstrap did not return an object".to_owned())?;

        for &(specifier, export_names) in MODULE_SPECS {
            let key = v8::String::new(scope, specifier)
                .ok_or_else(|| format!("cannot allocate key for {specifier}"))?;
            let mod_val = root_obj
                .get(scope, key.into())
                .ok_or_else(|| format!("missing module object for {specifier}"))?;
            let mod_obj = v8::Local::<v8::Object>::try_from(mod_val)
                .map_err(|_| format!("module {specifier} is not an object"))?;

            let mut exports = Vec::with_capacity(export_names.len() + 1);
            for &name in export_names {
                let prop_key = v8::String::new(scope, name.as_str())
                    .ok_or_else(|| format!("cannot allocate export key {}", name.as_str()))?;
                let val = mod_obj
                    .get(scope, prop_key.into())
                    .ok_or_else(|| format!("missing export {} on {specifier}", name.as_str()))?;
                exports.push((name, v8::Global::new(scope, val)));
            }
            exports.push((ascii_str!("default"), v8::Global::new(scope, mod_val)));
            prepared.push((specifier, exports));
        }
    }

    for (specifier, exports) in prepared {
        js.register_native_module(specifier, exports)
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

/// Resolves an exact canonical `node:*` specifier (lowercase `node:` prefix only).
pub(crate) fn resolve_specifier(specifier: &str) -> Option<&'static str> {
    vaeljs_node_core::canonical::SUPPORTED_NODE_MODULES
        .iter()
        .copied()
        .find(|&canonical| canonical == specifier)
}

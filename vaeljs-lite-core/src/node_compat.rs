//! Native `node:*` compatibility module bindings for QuickJS.
//!
//! All pure logic lives in [`vaeljs_node_core`]; this module wires synchronous
//! QuickJS host functions into [`vaeljs_node_core::NODE_COMPAT_FACTORY_SOURCE`],
//! stores the resulting module objects in context-local [`NodeCompatState`]
//! userdata, and serves the 10 Lite-supported `node:*` modules (`node:path`,
//! `node:path/posix`, `node:path/win32`, `node:os`, `node:process`, `node:fs`,
//! `node:child_process`, `node:buffer`, `node:url`, `node:crypto`) via
//! `Module::declare_def`. `node:fs/promises` is V8-only and excluded here.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use rquickjs::{
    Array, Ctx, Exception, Function, JsLifetime, Object, Persistent, Result, String as JsString,
    TypedArray, Value,
    function::Opt,
    module::{Declarations, Exports, ModuleDef},
    object::Property,
    qjs,
};
use vaeljs_node_core::buffer::{Encoding, decode as buf_decode, encode as buf_encode};
use vaeljs_node_core::child_process::{DEFAULT_MAX_BUFFER, SpawnOptions, spawn_sync};
use vaeljs_node_core::crypto::hash::{DigestTable, HashAlgorithm, HashErrorCode, OutputEncoding};
use vaeljs_node_core::crypto::password::pbkdf2 as core_pbkdf2;
use vaeljs_node_core::fs::{
    CopyOptions, DirEntry, FileInfo, FsError, FsErrorCode, MkdirOptions, ReadOptions,
    RemoveOptions, WriteOptions, copy_file, exists, lstat, mkdir, read_dir, read_file, realpath,
    remove, rename, stat, write_file,
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
    ParsedUrl, UrlComponent, file_url_to_path, parse_url, path_to_file_url, update_url,
};
use vaeljs_node_core::{NodeCoreError, NodeCoreErrorCode};

use crate::native::{own_array_element, throw_coded_error, with_uint8_array_bytes};
use crate::runtime::ProcessExitState;

const MAX_CHILD_ARGS: usize = 4096;
const MAX_CHILD_ENV: usize = 4096;

const PATH_EXPORTS: &[&str] = &[
    "sep",
    "delimiter",
    "join",
    "resolve",
    "normalize",
    "relative",
    "dirname",
    "basename",
    "extname",
    "parse",
    "format",
    "isAbsolute",
    "posix",
    "win32",
];

const OS_EXPORTS: &[&str] = &[
    "platform",
    "arch",
    "EOL",
    "homedir",
    "tmpdir",
    "hostname",
    "cpus",
    "endianness",
    "type",
    "machine",
];

const PROCESS_EXPORTS: &[&str] = &[
    "argv", "cwd", "chdir", "env", "exit", "exitCode", "platform", "arch", "pid", "version",
    "versions", "hrtime",
];

const FS_EXPORTS: &[&str] = &[
    "readFileSync",
    "writeFileSync",
    "appendFileSync",
    "existsSync",
    "statSync",
    "lstatSync",
    "readdirSync",
    "mkdirSync",
    "rmSync",
    "unlinkSync",
    "renameSync",
    "copyFileSync",
    "realpathSync",
    "mkdtempSync",
    "constants",
];

const CHILD_PROCESS_EXPORTS: &[&str] = &["spawnSync", "execFileSync"];
const BUFFER_EXPORTS: &[&str] = &["Buffer"];
const URL_EXPORTS: &[&str] = &["fileURLToPath", "pathToFileURL", "URL", "URLSearchParams"];
const CRYPTO_EXPORTS: &[&str] = &[
    "randomBytes",
    "randomUUID",
    "createHash",
    "createHmac",
    "pbkdf2Sync",
    "timingSafeEqual",
];

/// Context-local native state holding the cached `node:*` module objects and
/// open incremental digest table.
pub(super) struct NodeCompatState {
    pub(super) path: Persistent<Object<'static>>,
    pub(super) path_posix: Persistent<Object<'static>>,
    pub(super) path_win32: Persistent<Object<'static>>,
    pub(super) os: Persistent<Object<'static>>,
    pub(super) process: Persistent<Object<'static>>,
    pub(super) fs: Persistent<Object<'static>>,
    pub(super) child_process: Persistent<Object<'static>>,
    pub(super) buffer: Persistent<Object<'static>>,
    pub(super) url: Persistent<Object<'static>>,
    pub(super) crypto: Persistent<Object<'static>>,
    pub(super) digest_table: RefCell<DigestTable>,
}

unsafe impl<'js> JsLifetime<'js> for NodeCompatState {
    type Changed<'to> = NodeCompatState;
}

/// Resolves a Lite-supported `node:*` specifier (excluding V8-only `node:fs/promises`).
/// Resolves an exact canonical `node:*` specifier supported by QuickJS
/// (lowercase `node:` prefix only, excluding V8-only `node:fs/promises`).
pub(super) fn resolve_specifier(specifier: &str) -> Option<&'static str> {
    if specifier == "node:fs/promises" {
        return None;
    }
    vaeljs_node_core::canonical::SUPPORTED_NODE_MODULES
        .iter()
        .copied()
        .find(|&canonical| canonical == specifier)
}

fn throw_with_code<'js>(
    ctx: &Ctx<'js>,
    kind: Option<&str>,
    code: Option<&str>,
    message: &str,
) -> rquickjs::Error {
    match (kind, code) {
        (Some("RangeError"), Some(code_str)) => {
            let _ = Exception::throw_range(ctx, message);
            if ctx.has_exception() {
                let exc = ctx.catch();
                if let Some(obj) = exc.as_object() {
                    let _ = obj.prop(
                        "code",
                        Property::from(code_str)
                            .writable()
                            .enumerable()
                            .configurable(),
                    );
                }
                return ctx.throw(exc);
            }
            Exception::throw_range(ctx, message)
        }
        (Some("RangeError"), None) => Exception::throw_range(ctx, message),
        (Some("TypeError"), Some(code_str)) => {
            let _ = Exception::throw_type(ctx, message);
            if ctx.has_exception() {
                let exc = ctx.catch();
                if let Some(obj) = exc.as_object() {
                    let _ = obj.prop(
                        "code",
                        Property::from(code_str)
                            .writable()
                            .enumerable()
                            .configurable(),
                    );
                }
                return ctx.throw(exc);
            }
            Exception::throw_type(ctx, message)
        }
        (Some("TypeError"), None) => Exception::throw_type(ctx, message),
        (_, Some(code_str)) => throw_coded_error(ctx, None, code_str, message),
        (_, None) => Exception::throw_message(ctx, message),
    }
}

fn throw_core_error(ctx: &Ctx<'_>, err: NodeCoreError) -> rquickjs::Error {
    let kind = match err.code() {
        NodeCoreErrorCode::InputTooLarge => Some("RangeError"),
        NodeCoreErrorCode::InvalidEncoding => Some("TypeError"),
        _ => None,
    };
    throw_with_code(ctx, kind, Some(node_error_code_str(&err)), &err.to_string())
}

fn throw_fs_error(ctx: &Ctx<'_>, err: FsError) -> rquickjs::Error {
    throw_with_code(ctx, None, Some(fs_error_code_str(&err)), &err.to_string())
}

fn style(is_win: bool) -> Style {
    if is_win { Style::Win32 } else { Style::Posix }
}

fn path_to_string(ctx: &Ctx<'_>, path: PathBuf) -> Result<String> {
    path.into_os_string()
        .into_string()
        .map_err(|_| throw_with_code(ctx, None, Some("EINVAL"), "path is not valid UTF-8"))
}

fn current_cwd_lossy_safe() -> String {
    cwd()
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
        .unwrap_or_default()
}

fn array_length(ctx: &Ctx<'_>, array: &Array<'_>) -> Result<usize> {
    let mut len: i64 = 0;
    let rc = unsafe { qjs::JS_GetLength(ctx.as_raw().as_ptr(), array.as_raw(), &mut len) };
    if rc < 0 || len < 0 {
        return Err(if ctx.has_exception() {
            rquickjs::Error::Exception
        } else {
            Exception::throw_type(ctx, "expected an array")
        });
    }
    usize::try_from(len).map_err(|_| Exception::throw_range(ctx, "array length out of range"))
}

fn read_string_vec<'js>(ctx: &Ctx<'js>, array: Array<'js>) -> Result<Vec<String>> {
    let count = array_length(ctx, &array)?;
    if count > MAX_CHILD_ARGS {
        return Err(Exception::throw_range(
            ctx,
            &format!("args length exceeds maximum ({MAX_CHILD_ARGS})"),
        ));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(count)
        .map_err(|_| Exception::throw_range(ctx, "args allocation failed"))?;
    for idx in 0..count {
        let index = u32::try_from(idx)
            .map_err(|_| Exception::throw_range(ctx, "args index out of range"))?;
        let val = own_array_element(
            ctx,
            &array,
            index,
            "args elements must be plain data properties",
        )?
        .ok_or_else(|| Exception::throw_type(ctx, "expected array of strings"))?;
        let Some(s) = val.as_string() else {
            return Err(Exception::throw_type(ctx, "expected array of strings"));
        };
        out.push(s.to_string()?);
    }
    Ok(out)
}

fn parsed_path_to_js<'js>(ctx: &Ctx<'js>, p: ParsedPath) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("root", p.root)?;
    obj.set("dir", p.dir)?;
    obj.set("base", p.base)?;
    obj.set("ext", p.ext)?;
    obj.set("name", p.name)?;
    Ok(obj)
}

fn parsed_url_to_js<'js>(ctx: &Ctx<'js>, u: ParsedUrl) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("href", u.href)?;
    obj.set("origin", u.origin)?;
    obj.set("protocol", u.protocol)?;
    obj.set("username", u.username)?;
    obj.set("password", u.password)?;
    obj.set("host", u.host)?;
    obj.set("hostname", u.hostname)?;
    obj.set("port", u.port)?;
    obj.set("pathname", u.pathname)?;
    obj.set("search", u.search)?;
    obj.set("hash", u.hash)?;
    Ok(obj)
}

fn stat_to_js<'js>(ctx: &Ctx<'js>, info: FileInfo) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    obj.set("isFile", info.is_file)?;
    obj.set("isDirectory", info.is_directory)?;
    obj.set("isSymbolicLink", info.is_symlink)?;
    obj.set("size", info.size as f64)?;
    match info.modified_ms {
        Some(ms) => obj.set("mtimeMs", ms as f64)?,
        None => obj.set("mtimeMs", Value::new_null(ctx.clone()))?,
    }
    Ok(obj)
}

fn dir_entries_to_js<'js>(ctx: &Ctx<'js>, entries: Vec<DirEntry>) -> Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    for (idx, entry) in entries.into_iter().enumerate() {
        let item = Object::new(ctx.clone())?;
        item.set("name", entry.name)?;
        item.set("isFile", entry.is_file)?;
        item.set("isDirectory", entry.is_directory)?;
        item.set("isSymbolicLink", entry.is_symlink)?;
        arr.set(idx, item)?;
    }
    Ok(arr)
}

fn parse_encoding(ctx: &Ctx<'_>, name: &str) -> Result<Encoding> {
    match name {
        "utf8" | "utf-8" => Ok(Encoding::Utf8),
        "hex" => Ok(Encoding::Hex),
        "base64" => Ok(Encoding::Base64),
        "base64url" => Ok(Encoding::Base64Url),
        "latin1" | "binary" => Ok(Encoding::Latin1),
        "ascii" => Ok(Encoding::Ascii),
        _ => Err(throw_with_code(
            ctx,
            Some("TypeError"),
            Some("ERR_UNKNOWN_ENCODING"),
            "unknown encoding",
        )),
    }
}

fn read_bytes<'js>(ctx: &Ctx<'js>, value: &Value<'js>, msg: &str) -> Result<Vec<u8>> {
    with_uint8_array_bytes(ctx, value, msg, |slice| Ok(slice.to_vec()))
}

// ---------------------------------------------------------------------------
// Host functions
// ---------------------------------------------------------------------------

fn host_path_info<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let native = Style::native();
    let obj = Object::new(ctx)?;
    obj.set("sep", sep(native))?;
    obj.set("delimiter", delimiter(native))?;
    obj.set("isWindows", native == Style::Win32)?;
    Ok(obj)
}

fn host_path_join<'js>(ctx: Ctx<'js>, is_win: bool, parts: Array<'js>) -> Result<String> {
    let vec = read_string_vec(&ctx, parts)?;
    let refs: Vec<&str> = vec.iter().map(String::as_str).collect();
    join(style(is_win), &refs).map_err(|e| throw_core_error(&ctx, e))
}

fn host_path_resolve<'js>(ctx: Ctx<'js>, is_win: bool, parts: Array<'js>) -> Result<String> {
    let vec = read_string_vec(&ctx, parts)?;
    let refs: Vec<&str> = vec.iter().map(String::as_str).collect();
    let cwd_str = current_cwd_lossy_safe();
    resolve(style(is_win), &cwd_str, &refs).map_err(|e| throw_core_error(&ctx, e))
}

fn host_path_normalize(ctx: Ctx<'_>, is_win: bool, p: String) -> Result<String> {
    normalize(style(is_win), &p).map_err(|e| throw_core_error(&ctx, e))
}

fn host_path_relative(ctx: Ctx<'_>, is_win: bool, from: String, to: String) -> Result<String> {
    relative(style(is_win), &from, &to).map_err(|e| throw_core_error(&ctx, e))
}

fn host_path_dirname(ctx: Ctx<'_>, is_win: bool, p: String) -> Result<String> {
    dirname(style(is_win), &p).map_err(|e| throw_core_error(&ctx, e))
}

fn host_path_basename(
    ctx: Ctx<'_>,
    is_win: bool,
    p: String,
    suffix: Opt<Option<String>>,
) -> Result<String> {
    basename(style(is_win), &p, suffix.0.flatten().as_deref())
        .map_err(|e| throw_core_error(&ctx, e))
}

fn host_path_extname(ctx: Ctx<'_>, is_win: bool, p: String) -> Result<String> {
    extname(style(is_win), &p).map_err(|e| throw_core_error(&ctx, e))
}

fn host_path_parse<'js>(ctx: Ctx<'js>, is_win: bool, p: String) -> Result<Object<'js>> {
    let parsed = parse(style(is_win), &p).map_err(|e| throw_core_error(&ctx, e))?;
    parsed_path_to_js(&ctx, parsed)
}

fn host_path_format<'js>(ctx: Ctx<'js>, is_win: bool, obj: Object<'js>) -> Result<String> {
    let get_opt = |key: &str| -> Result<String> {
        let v: Value<'js> = obj.get(key)?;
        if v.is_undefined() {
            Ok(String::new())
        } else if let Some(s) = v.as_string() {
            s.to_string()
        } else {
            Err(Exception::throw_type(
                &ctx,
                "pathObject field must be a string",
            ))
        }
    };
    let parsed = ParsedPath {
        root: get_opt("root")?,
        dir: get_opt("dir")?,
        base: get_opt("base")?,
        ext: get_opt("ext")?,
        name: get_opt("name")?,
    };
    format(style(is_win), &parsed).map_err(|e| throw_core_error(&ctx, e))
}

fn host_path_is_absolute(ctx: Ctx<'_>, is_win: bool, p: String) -> Result<bool> {
    is_absolute(style(is_win), &p).map_err(|e| throw_core_error(&ctx, e))
}

fn host_os_info<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let obj = Object::new(ctx)?;
    obj.set("platform", os_platform())?;
    obj.set("arch", os_arch())?;
    obj.set("EOL", eol())?;
    obj.set("endianness", endianness())?;
    obj.set("type", os_type())?;
    obj.set("machine", machine())?;
    obj.set("cpusCount", cpus_count() as f64)?;
    Ok(obj)
}

fn host_os_homedir(ctx: Ctx<'_>) -> Result<String> {
    homedir()
        .map_err(|e| throw_core_error(&ctx, e))
        .and_then(|p| path_to_string(&ctx, p))
}

fn host_os_tmpdir(ctx: Ctx<'_>) -> Result<String> {
    path_to_string(&ctx, tmpdir())
}

fn host_os_hostname(ctx: Ctx<'_>) -> Result<String> {
    hostname().map_err(|e| throw_core_error(&ctx, e))
}

fn host_process_info<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;
    let argv = Array::new(ctx.clone())?;
    let mut args: Vec<String> = std::env::args_os()
        .filter_map(|a| a.into_string().ok())
        .collect();
    if args.is_empty() {
        args.push("vjs-lite".to_owned());
    }
    for (idx, arg) in args.into_iter().enumerate() {
        argv.set(idx, arg)?;
    }
    let versions = Object::new(ctx)?;
    versions.set("node", "22.0.0")?;
    versions.set("vaeljs", env!("CARGO_PKG_VERSION"))?;
    versions.set("quickjs", "quickjs-ng")?;

    obj.set("argv", argv)?;
    obj.set("pid", pid())?;
    obj.set("platform", os_platform())?;
    obj.set("arch", os_arch())?;
    obj.set("version", "v22.0.0")?;
    obj.set("versions", versions)?;
    Ok(obj)
}

fn host_process_cwd(ctx: Ctx<'_>) -> Result<String> {
    cwd()
        .map_err(|e| throw_core_error(&ctx, e))
        .and_then(|p| path_to_string(&ctx, p))
}

fn host_process_chdir(ctx: Ctx<'_>, dir: String) -> Result<()> {
    chdir(dir).map_err(|e| throw_core_error(&ctx, e))
}

fn host_process_env_get<'js>(ctx: Ctx<'js>, key: String) -> Result<Value<'js>> {
    if key.is_empty() || key.contains('\0') || key.contains('=') {
        return Ok(Value::new_undefined(ctx));
    }
    match env_get(&key).map_err(|e| throw_core_error(&ctx, e))? {
        Some(v) => Ok(JsString::from_str(ctx, &v)?.into_value()),
        None => Ok(Value::new_undefined(ctx)),
    }
}

fn host_process_env_entries<'js>(ctx: Ctx<'js>) -> Result<Array<'js>> {
    let arr = Array::new(ctx.clone())?;
    for (idx, (k, v)) in env_entries().into_iter().enumerate() {
        let pair = Array::new(ctx.clone())?;
        pair.set(0, k)?;
        pair.set(1, v)?;
        arr.set(idx, pair)?;
    }
    Ok(arr)
}

fn host_process_hrtime_nanos() -> String {
    hrtime_nanos().to_string()
}

fn host_buffer_encode<'js>(ctx: Ctx<'js>, bytes: Value<'js>, encoding: String) -> Result<String> {
    let enc = parse_encoding(&ctx, &encoding)?;
    with_uint8_array_bytes(&ctx, &bytes, "expected Uint8Array", |slice| {
        Ok(buf_encode(slice, enc))
    })
}

fn host_buffer_decode<'js>(
    ctx: Ctx<'js>,
    input: String,
    encoding: String,
) -> Result<TypedArray<'js, u8>> {
    let enc = parse_encoding(&ctx, &encoding)?;
    let decoded = buf_decode(&input, enc).map_err(|e| throw_core_error(&ctx, e))?;
    TypedArray::<u8>::new(ctx, decoded)
}

fn host_buffer_byte_length(ctx: Ctx<'_>, input: String, encoding: String) -> Result<usize> {
    let enc = parse_encoding(&ctx, &encoding)?;
    match enc {
        Encoding::Utf8 => Ok(input.len()),
        Encoding::Latin1 | Encoding::Ascii => Ok(input.encode_utf16().count()),
        Encoding::Hex => Ok(input.len() / 2),
        Encoding::Base64 | Encoding::Base64Url => buf_decode(&input, enc)
            .map(|v| v.len())
            .map_err(|e| throw_core_error(&ctx, e)),
    }
}

fn host_url_file_to_path(ctx: Ctx<'_>, url_str: String) -> Result<String> {
    file_url_to_path(&url_str)
        .map_err(|e| throw_core_error(&ctx, e))
        .and_then(|p| path_to_string(&ctx, p))
}

fn host_url_path_to_file(ctx: Ctx<'_>, path_str: String) -> Result<String> {
    path_to_file_url(&path_str).map_err(|e| throw_core_error(&ctx, e))
}

fn host_url_parse<'js>(
    ctx: Ctx<'js>,
    input: String,
    base: Opt<Option<String>>,
) -> Result<Object<'js>> {
    let parsed = parse_url(&input, base.0.flatten().as_deref()).map_err(|e| {
        throw_with_code(
            &ctx,
            Some("TypeError"),
            Some("ERR_INVALID_URL"),
            &e.to_string(),
        )
    })?;
    parsed_url_to_js(&ctx, parsed)
}

fn host_url_update<'js>(
    ctx: Ctx<'js>,
    href: String,
    component: String,
    value: String,
) -> Result<Object<'js>> {
    let comp = UrlComponent::parse(&component)
        .ok_or_else(|| Exception::throw_type(&ctx, "invalid URL component"))?;
    let parsed = update_url(&href, comp, &value).map_err(|e| {
        throw_with_code(
            &ctx,
            Some("TypeError"),
            Some("ERR_INVALID_URL"),
            &e.to_string(),
        )
    })?;
    parsed_url_to_js(&ctx, parsed)
}

fn host_fs_read_file_sync<'js>(ctx: Ctx<'js>, path: String) -> Result<TypedArray<'js, u8>> {
    let bytes = read_file(path, ReadOptions::default()).map_err(|e| throw_fs_error(&ctx, e))?;
    TypedArray::<u8>::new(ctx, bytes)
}

fn host_fs_write_file_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    data: Value<'js>,
    append: bool,
) -> Result<()> {
    let bytes = read_bytes(&ctx, &data, "data must be a Uint8Array")?;
    let opts = WriteOptions {
        create: true,
        create_new: false,
        append,
    };
    write_file(path, &bytes, opts).map_err(|e| throw_fs_error(&ctx, e))
}

fn host_fs_exists_sync(ctx: Ctx<'_>, path: String) -> Result<bool> {
    exists(path).map_err(|e| throw_fs_error(&ctx, e))
}

fn host_fs_stat_sync<'js>(ctx: Ctx<'js>, path: String, follow: bool) -> Result<Object<'js>> {
    let info = if follow {
        stat(path).map_err(|e| throw_fs_error(&ctx, e))?
    } else {
        lstat(path).map_err(|e| throw_fs_error(&ctx, e))?
    };
    stat_to_js(&ctx, info)
}

fn host_fs_readdir_sync<'js>(ctx: Ctx<'js>, path: String) -> Result<Array<'js>> {
    let entries = read_dir(path).map_err(|e| throw_fs_error(&ctx, e))?;
    dir_entries_to_js(&ctx, entries)
}

fn host_fs_mkdir_sync(ctx: Ctx<'_>, path: String, recursive: bool) -> Result<()> {
    mkdir(path, MkdirOptions { recursive }).map_err(|e| throw_fs_error(&ctx, e))
}

fn host_fs_rm_sync(ctx: Ctx<'_>, path: String, recursive: bool, force: bool) -> Result<()> {
    match remove(path, RemoveOptions { recursive }) {
        Ok(()) => Ok(()),
        Err(err) if force && err.code() == FsErrorCode::NotFound => Ok(()),
        Err(err) => Err(throw_fs_error(&ctx, err)),
    }
}

fn host_fs_unlink_sync(ctx: Ctx<'_>, path: String) -> Result<()> {
    let info = lstat(&path).map_err(|e| throw_fs_error(&ctx, e))?;
    if info.is_directory {
        return Err(throw_with_code(
            &ctx,
            None,
            Some("EISDIR"),
            "illegal operation on a directory, unlink",
        ));
    }
    remove(path, RemoveOptions { recursive: false }).map_err(|e| throw_fs_error(&ctx, e))
}

fn host_fs_rename_sync(ctx: Ctx<'_>, old_path: String, new_path: String) -> Result<()> {
    rename(old_path, new_path).map_err(|e| throw_fs_error(&ctx, e))
}

fn host_fs_copy_file_sync(ctx: Ctx<'_>, src: String, dest: String, overwrite: bool) -> Result<()> {
    copy_file(src, dest, CopyOptions { overwrite }).map_err(|e| throw_fs_error(&ctx, e))
}

fn host_fs_realpath_sync(ctx: Ctx<'_>, path: String) -> Result<String> {
    realpath(path)
        .map_err(|e| throw_fs_error(&ctx, e))
        .and_then(|p| path_to_string(&ctx, p))
}

fn host_fs_mkdtemp_sync(ctx: Ctx<'_>, prefix: String) -> Result<String> {
    mkdtemp(&prefix)
        .map_err(|e| throw_core_error(&ctx, e))
        .and_then(|p| path_to_string(&ctx, p))
}

fn host_child_spawn_sync<'js>(
    ctx: Ctx<'js>,
    command: String,
    args: Array<'js>,
    opts_obj: Object<'js>,
) -> Result<Object<'js>> {
    let args_vec = read_string_vec(&ctx, args)?;
    let mut spawn_opts = SpawnOptions::default();

    let cwd_val: Value<'js> = opts_obj.get("cwd")?;
    if !cwd_val.is_undefined() && !cwd_val.is_null() {
        let Some(s) = cwd_val.as_string() else {
            return Err(Exception::throw_type(&ctx, "cwd must be a string"));
        };
        spawn_opts.cwd = Some(PathBuf::from(s.to_string()?));
    }

    let env_val: Value<'js> = opts_obj.get("env")?;
    if !env_val.is_undefined() && !env_val.is_null() {
        let Some(env_arr) = env_val.into_array() else {
            return Err(Exception::throw_type(&ctx, "env must be an array"));
        };
        let count = array_length(&ctx, &env_arr)?;
        if count > MAX_CHILD_ENV {
            return Err(Exception::throw_range(
                &ctx,
                &format!("env length exceeds maximum ({MAX_CHILD_ENV})"),
            ));
        }
        let mut pairs = Vec::new();
        pairs
            .try_reserve_exact(count)
            .map_err(|_| Exception::throw_range(&ctx, "env allocation failed"))?;
        for idx in 0..count {
            let index = u32::try_from(idx)
                .map_err(|_| Exception::throw_range(&ctx, "env index out of range"))?;
            let entry = own_array_element(
                &ctx,
                &env_arr,
                index,
                "env entries must be plain data properties",
            )?
            .ok_or_else(|| Exception::throw_type(&ctx, "env entries must be [key, value] pairs"))?;
            let Some(pair) = entry.into_array() else {
                return Err(Exception::throw_type(
                    &ctx,
                    "env entries must be [key, value] pairs",
                ));
            };
            let k_val = own_array_element(
                &ctx,
                &pair,
                0,
                "env entry key must be a plain data property",
            )?
            .ok_or_else(|| Exception::throw_type(&ctx, "env entry key must be a string"))?;
            let v_val = own_array_element(
                &ctx,
                &pair,
                1,
                "env entry value must be a plain data property",
            )?
            .ok_or_else(|| Exception::throw_type(&ctx, "env entry value must be a string"))?;
            let (Some(k), Some(v)) = (k_val.as_string(), v_val.as_string()) else {
                return Err(Exception::throw_type(
                    &ctx,
                    "env entries must be [string, string] pairs",
                ));
            };
            pairs.push((k.to_string()?, v.to_string()?));
        }
        spawn_opts.clear_env = true;
        spawn_opts.env = Some(pairs);
    }

    let input_val: Value<'js> = opts_obj.get("input")?;
    if !input_val.is_undefined() && !input_val.is_null() {
        spawn_opts.stdin = Some(read_bytes(&ctx, &input_val, "input must be a Uint8Array")?);
    }

    let timeout_val: Value<'js> = opts_obj.get("timeoutMs")?;
    if !timeout_val.is_undefined() && !timeout_val.is_null() {
        let Some(ms) = timeout_val.as_number() else {
            return Err(Exception::throw_type(&ctx, "timeout must be a number"));
        };
        spawn_opts.timeout_ms = Some(ms as u64);
    }

    let max_buf_val: Value<'js> = opts_obj.get("maxBuffer")?;
    if !max_buf_val.is_undefined() && !max_buf_val.is_null() {
        let Some(n) = max_buf_val.as_number() else {
            return Err(Exception::throw_type(&ctx, "maxBuffer must be a number"));
        };
        spawn_opts.max_buffer = n as usize;
    } else {
        spawn_opts.max_buffer = DEFAULT_MAX_BUFFER;
    }

    let res_obj = Object::new(ctx.clone())?;
    match spawn_sync(&command, &args_vec, &spawn_opts) {
        Ok(out) => {
            match out.status {
                Some(code) => res_obj.set("status", code)?,
                None => res_obj.set("status", Value::new_null(ctx.clone()))?,
            }
            match out.signal {
                Some(sig) => res_obj.set("signal", sig)?,
                None => res_obj.set("signal", Value::new_null(ctx.clone()))?,
            }
            res_obj.set("stdout", TypedArray::<u8>::new(ctx.clone(), out.stdout)?)?;
            res_obj.set("stderr", TypedArray::<u8>::new(ctx.clone(), out.stderr)?)?;
            if out.timed_out {
                res_obj.set("errorCode", "ETIMEDOUT")?;
                res_obj.set("errorMessage", "Command timed out")?;
            } else {
                res_obj.set("errorCode", Value::new_null(ctx.clone()))?;
                res_obj.set("errorMessage", Value::new_null(ctx.clone()))?;
            }
            res_obj.set("timedOut", out.timed_out)?;
            Ok(res_obj)
        }
        Err(err)
            if matches!(
                err.code(),
                NodeCoreErrorCode::NotFound
                    | NodeCoreErrorCode::PermissionDenied
                    | NodeCoreErrorCode::Io
                    | NodeCoreErrorCode::ProcessError
            ) =>
        {
            res_obj.set("status", Value::new_null(ctx.clone()))?;
            res_obj.set("signal", Value::new_null(ctx.clone()))?;
            res_obj.set("stdout", TypedArray::<u8>::new(ctx.clone(), Vec::new())?)?;
            res_obj.set("stderr", TypedArray::<u8>::new(ctx.clone(), Vec::new())?)?;
            res_obj.set("errorCode", node_error_code_str(&err))?;
            res_obj.set("errorMessage", err.to_string())?;
            res_obj.set("timedOut", false)?;
            Ok(res_obj)
        }
        Err(err) => Err(throw_core_error(&ctx, err)),
    }
}

fn host_crypto_random_bytes<'js>(ctx: Ctx<'js>, size: usize) -> Result<TypedArray<'js, u8>> {
    let bytes = vaeljs_node_core::crypto::envelope::random_bytes(size).map_err(|err| {
        let kind = match err.code() {
            vaeljs_node_core::crypto::envelope::EnvelopeErrorCode::Limit => Some("RangeError"),
            _ => None,
        };
        throw_with_code(&ctx, kind, Some(err.code().as_str()), &err.to_string())
    })?;
    TypedArray::<u8>::new(ctx, bytes)
}

fn host_crypto_random_uuid(ctx: Ctx<'_>) -> Result<String> {
    random_uuid().map_err(|e| throw_core_error(&ctx, e))
}

fn host_crypto_hash_create(ctx: Ctx<'_>, algorithm: String) -> Result<u32> {
    let alg = HashAlgorithm::parse(&algorithm).ok_or_else(|| {
        throw_with_code(
            &ctx,
            None,
            Some("ERR_CRYPTO_HASH_UNKNOWN"),
            "Digest method not supported",
        )
    })?;
    let state = ctx
        .userdata::<NodeCompatState>()
        .ok_or(rquickjs::Error::Unknown)?;
    let mut table = state.digest_table.borrow_mut();
    table
        .create_hash(alg)
        .map_err(|e| throw_with_code(&ctx, None, Some(e.code().as_str()), e.message()))
}

fn host_crypto_hmac_create<'js>(ctx: Ctx<'js>, algorithm: String, key: Value<'js>) -> Result<u32> {
    let alg = HashAlgorithm::parse(&algorithm).ok_or_else(|| {
        throw_with_code(
            &ctx,
            None,
            Some("ERR_CRYPTO_HASH_UNKNOWN"),
            "Digest method not supported",
        )
    })?;
    let key_bytes = read_bytes(&ctx, &key, "key must be a Uint8Array")?;
    let state = ctx
        .userdata::<NodeCompatState>()
        .ok_or(rquickjs::Error::Unknown)?;
    let mut table = state.digest_table.borrow_mut();
    table.create_hmac(alg, &key_bytes).map_err(|e| {
        let kind = if e.code() == HashErrorCode::Limit {
            Some("RangeError")
        } else {
            None
        };
        throw_with_code(&ctx, kind, Some(e.code().as_str()), e.message())
    })
}

fn host_crypto_hash_update<'js>(ctx: Ctx<'js>, handle: u32, data: Value<'js>) -> Result<()> {
    let bytes = read_bytes(&ctx, &data, "data must be a Uint8Array")?;
    let state = ctx
        .userdata::<NodeCompatState>()
        .ok_or(rquickjs::Error::Unknown)?;
    let mut table = state.digest_table.borrow_mut();
    table
        .update(handle, &bytes)
        .map_err(|e| throw_with_code(&ctx, None, Some(e.code().as_str()), e.message()))
}

fn host_crypto_hash_digest<'js>(ctx: Ctx<'js>, handle: u32) -> Result<TypedArray<'js, u8>> {
    let bytes = {
        let state = ctx
            .userdata::<NodeCompatState>()
            .ok_or(rquickjs::Error::Unknown)?;
        let mut table = state.digest_table.borrow_mut();
        table
            .digest(handle, OutputEncoding::Binary)
            .map_err(|e| throw_with_code(&ctx, None, Some(e.code().as_str()), e.message()))?
    };
    TypedArray::<u8>::new(ctx, bytes)
}

fn host_crypto_pbkdf2_sync<'js>(
    ctx: Ctx<'js>,
    password: Value<'js>,
    salt: Value<'js>,
    iterations: u32,
    keylen: usize,
    digest: String,
) -> Result<TypedArray<'js, u8>> {
    let alg = HashAlgorithm::parse(&digest).ok_or_else(|| {
        throw_with_code(
            &ctx,
            Some("TypeError"),
            Some("ERR_CRYPTO_INVALID_DIGEST"),
            "Invalid digest",
        )
    })?;
    let pw_bytes = read_bytes(&ctx, &password, "password must be a Uint8Array")?;
    let salt_bytes = read_bytes(&ctx, &salt, "salt must be a Uint8Array")?;
    if keylen == 0 {
        return TypedArray::<u8>::new(ctx, Vec::new());
    }
    let out = core_pbkdf2(alg, &pw_bytes, &salt_bytes, iterations, keylen).map_err(|e| {
        let kind = match e.code() {
            vaeljs_node_core::crypto::password::PasswordErrorCode::Range => Some("RangeError"),
            _ => None,
        };
        throw_with_code(&ctx, kind, e.code().as_str(), e.message())
    })?;
    TypedArray::<u8>::new(ctx, out)
}

fn host_crypto_timing_safe_equal<'js>(ctx: Ctx<'js>, a: Value<'js>, b: Value<'js>) -> Result<bool> {
    let a_bytes = read_bytes(&ctx, &a, "a must be a Uint8Array")?;
    let b_bytes = read_bytes(&ctx, &b, "b must be a Uint8Array")?;
    timing_safe_equal(&a_bytes, &b_bytes).map_err(|e| {
        throw_with_code(
            &ctx,
            Some("RangeError"),
            Some("ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH"),
            &e.to_string(),
        )
    })
}

/// Evaluates [`vaeljs_node_core::NODE_COMPAT_FACTORY_SOURCE`] with QuickJS host
/// bindings and stores the resulting module objects in context userdata.
pub(super) fn install<'js>(
    ctx: &Ctx<'js>,
    exit_state: Rc<RefCell<ProcessExitState>>,
) -> Result<()> {
    let host = Object::new(ctx.clone())?;
    host.set("isV8", false)?;

    host.set("pathInfo", Function::new(ctx.clone(), host_path_info)?)?;
    host.set("pathJoin", Function::new(ctx.clone(), host_path_join)?)?;
    host.set(
        "pathResolve",
        Function::new(ctx.clone(), host_path_resolve)?,
    )?;
    host.set(
        "pathNormalize",
        Function::new(ctx.clone(), host_path_normalize)?,
    )?;
    host.set(
        "pathRelative",
        Function::new(ctx.clone(), host_path_relative)?,
    )?;
    host.set(
        "pathDirname",
        Function::new(ctx.clone(), host_path_dirname)?,
    )?;
    host.set(
        "pathBasename",
        Function::new(ctx.clone(), host_path_basename)?,
    )?;
    host.set(
        "pathExtname",
        Function::new(ctx.clone(), host_path_extname)?,
    )?;
    host.set("pathParse", Function::new(ctx.clone(), host_path_parse)?)?;
    host.set("pathFormat", Function::new(ctx.clone(), host_path_format)?)?;
    host.set(
        "pathIsAbsolute",
        Function::new(ctx.clone(), host_path_is_absolute)?,
    )?;

    host.set("osInfo", Function::new(ctx.clone(), host_os_info)?)?;
    host.set("osHomedir", Function::new(ctx.clone(), host_os_homedir)?)?;
    host.set("osTmpdir", Function::new(ctx.clone(), host_os_tmpdir)?)?;
    host.set("osHostname", Function::new(ctx.clone(), host_os_hostname)?)?;

    host.set(
        "processInfo",
        Function::new(ctx.clone(), host_process_info)?,
    )?;
    host.set("processCwd", Function::new(ctx.clone(), host_process_cwd)?)?;
    host.set(
        "processChdir",
        Function::new(ctx.clone(), host_process_chdir)?,
    )?;
    host.set(
        "processEnvGet",
        Function::new(ctx.clone(), host_process_env_get)?,
    )?;
    host.set(
        "processEnvEntries",
        Function::new(ctx.clone(), host_process_env_entries)?,
    )?;
    host.set(
        "processHrtimeNanos",
        Function::new(ctx.clone(), host_process_hrtime_nanos)?,
    )?;

    let exit_get_state = Rc::clone(&exit_state);
    host.set(
        "processGetExitCode",
        Function::new(ctx.clone(), move || -> Option<i32> {
            exit_get_state.borrow().exit_code
        })?,
    )?;

    let exit_set_state = Rc::clone(&exit_state);
    host.set(
        "processSetExitCode",
        Function::new(ctx.clone(), move |code: Opt<Option<i32>>| {
            exit_set_state.borrow_mut().exit_code = code.0.flatten();
        })?,
    )?;

    host.set(
        "processExit",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, code: Opt<Option<i32>>| -> Result<()> {
                {
                    let mut guard = exit_state.borrow_mut();
                    guard.exited = true;
                    if let Some(c) = code.0.flatten() {
                        guard.exit_code = Some(c);
                    } else if guard.exit_code.is_none() {
                        guard.exit_code = Some(0);
                    }
                }
                Err(Exception::throw_internal(&ctx, "process exited"))
            },
        )?,
    )?;

    host.set(
        "bufferEncode",
        Function::new(ctx.clone(), host_buffer_encode)?,
    )?;
    host.set(
        "bufferDecode",
        Function::new(ctx.clone(), host_buffer_decode)?,
    )?;
    host.set(
        "bufferByteLength",
        Function::new(ctx.clone(), host_buffer_byte_length)?,
    )?;

    host.set(
        "urlFileToPath",
        Function::new(ctx.clone(), host_url_file_to_path)?,
    )?;
    host.set(
        "urlPathToFile",
        Function::new(ctx.clone(), host_url_path_to_file)?,
    )?;
    host.set("urlParse", Function::new(ctx.clone(), host_url_parse)?)?;
    host.set("urlUpdate", Function::new(ctx.clone(), host_url_update)?)?;

    host.set(
        "fsReadFileSync",
        Function::new(ctx.clone(), host_fs_read_file_sync)?,
    )?;
    host.set(
        "fsWriteFileSync",
        Function::new(ctx.clone(), host_fs_write_file_sync)?,
    )?;
    host.set(
        "fsExistsSync",
        Function::new(ctx.clone(), host_fs_exists_sync)?,
    )?;
    host.set("fsStatSync", Function::new(ctx.clone(), host_fs_stat_sync)?)?;
    host.set(
        "fsReaddirSync",
        Function::new(ctx.clone(), host_fs_readdir_sync)?,
    )?;
    host.set(
        "fsMkdirSync",
        Function::new(ctx.clone(), host_fs_mkdir_sync)?,
    )?;
    host.set("fsRmSync", Function::new(ctx.clone(), host_fs_rm_sync)?)?;
    host.set(
        "fsUnlinkSync",
        Function::new(ctx.clone(), host_fs_unlink_sync)?,
    )?;
    host.set(
        "fsRenameSync",
        Function::new(ctx.clone(), host_fs_rename_sync)?,
    )?;
    host.set(
        "fsCopyFileSync",
        Function::new(ctx.clone(), host_fs_copy_file_sync)?,
    )?;
    host.set(
        "fsRealpathSync",
        Function::new(ctx.clone(), host_fs_realpath_sync)?,
    )?;
    host.set(
        "fsMkdtempSync",
        Function::new(ctx.clone(), host_fs_mkdtemp_sync)?,
    )?;

    host.set(
        "childSpawnSync",
        Function::new(ctx.clone(), host_child_spawn_sync)?,
    )?;

    host.set(
        "cryptoRandomBytes",
        Function::new(ctx.clone(), host_crypto_random_bytes)?,
    )?;
    host.set(
        "cryptoRandomUUID",
        Function::new(ctx.clone(), host_crypto_random_uuid)?,
    )?;
    host.set(
        "cryptoHashCreate",
        Function::new(ctx.clone(), host_crypto_hash_create)?,
    )?;
    host.set(
        "cryptoHmacCreate",
        Function::new(ctx.clone(), host_crypto_hmac_create)?,
    )?;
    host.set(
        "cryptoHashUpdate",
        Function::new(ctx.clone(), host_crypto_hash_update)?,
    )?;
    host.set(
        "cryptoHashDigest",
        Function::new(ctx.clone(), host_crypto_hash_digest)?,
    )?;
    host.set(
        "cryptoPbkdf2Sync",
        Function::new(ctx.clone(), host_crypto_pbkdf2_sync)?,
    )?;
    host.set(
        "cryptoTimingSafeEqual",
        Function::new(ctx.clone(), host_crypto_timing_safe_equal)?,
    )?;

    let factory_src = format!("({})", vaeljs_node_core::NODE_COMPAT_FACTORY_SOURCE);
    let factory: Function<'js> = ctx.eval(factory_src)?;
    let root: Object<'js> = factory.call((host,))?;

    let get_mod = |key: &str| -> Result<Persistent<Object<'static>>> {
        let obj: Object<'js> = root.get(key)?;
        Ok(Persistent::save(ctx, obj))
    };

    let state = NodeCompatState {
        path: get_mod("node:path")?,
        path_posix: get_mod("node:path/posix")?,
        path_win32: get_mod("node:path/win32")?,
        os: get_mod("node:os")?,
        process: get_mod("node:process")?,
        fs: get_mod("node:fs")?,
        child_process: get_mod("node:child_process")?,
        buffer: get_mod("node:buffer")?,
        url: get_mod("node:url")?,
        crypto: get_mod("node:crypto")?,
        digest_table: RefCell::new(DigestTable::default()),
    };
    let _ = ctx.store_userdata(state);
    Ok(())
}

fn export_module<'js>(
    ctx: &Ctx<'js>,
    exports: &Exports<'js>,
    names: &[&str],
    select: impl FnOnce(&NodeCompatState) -> &Persistent<Object<'static>>,
) -> Result<()> {
    let state = ctx
        .userdata::<NodeCompatState>()
        .ok_or(rquickjs::Error::Unknown)?;
    let obj = select(&state).clone().restore(ctx)?;
    for name in names {
        let value: Value<'js> = obj.get(*name)?;
        exports.export(*name, value)?;
    }
    exports.export("default", obj)?;
    Ok(())
}

macro_rules! node_module_def {
    ($struct_name:ident, $exports:expr, $field:ident) => {
        pub(super) struct $struct_name;

        impl ModuleDef for $struct_name {
            fn declare(declarations: &Declarations<'_>) -> Result<()> {
                for name in $exports {
                    declarations.declare(*name)?;
                }
                declarations.declare("default")?;
                Ok(())
            }

            fn evaluate<'js>(ctx: &Ctx<'js>, exports: &Exports<'js>) -> Result<()> {
                export_module(ctx, exports, $exports, |s| &s.$field)
            }
        }
    };
}

node_module_def!(NodePathModule, PATH_EXPORTS, path);
node_module_def!(NodePathPosixModule, PATH_EXPORTS, path_posix);
node_module_def!(NodePathWin32Module, PATH_EXPORTS, path_win32);
node_module_def!(NodeOsModule, OS_EXPORTS, os);
node_module_def!(NodeProcessModule, PROCESS_EXPORTS, process);
node_module_def!(NodeFsModule, FS_EXPORTS, fs);
node_module_def!(NodeChildProcessModule, CHILD_PROCESS_EXPORTS, child_process);
node_module_def!(NodeBufferModule, BUFFER_EXPORTS, buffer);
node_module_def!(NodeUrlModule, URL_EXPORTS, url);
node_module_def!(NodeCryptoModule, CRYPTO_EXPORTS, crypto);

pub(super) fn declare_module<'js>(
    ctx: &Ctx<'js>,
    canonical: &str,
) -> Result<rquickjs::Module<'js, rquickjs::module::Declared>> {
    use rquickjs::Module;
    match canonical {
        "node:path" => Module::declare_def::<NodePathModule, _>(ctx.clone(), canonical),
        "node:path/posix" => Module::declare_def::<NodePathPosixModule, _>(ctx.clone(), canonical),
        "node:path/win32" => Module::declare_def::<NodePathWin32Module, _>(ctx.clone(), canonical),
        "node:os" => Module::declare_def::<NodeOsModule, _>(ctx.clone(), canonical),
        "node:process" => Module::declare_def::<NodeProcessModule, _>(ctx.clone(), canonical),
        "node:fs" => Module::declare_def::<NodeFsModule, _>(ctx.clone(), canonical),
        "node:child_process" => {
            Module::declare_def::<NodeChildProcessModule, _>(ctx.clone(), canonical)
        }
        "node:buffer" => Module::declare_def::<NodeBufferModule, _>(ctx.clone(), canonical),
        "node:url" => Module::declare_def::<NodeUrlModule, _>(ctx.clone(), canonical),
        "node:crypto" => Module::declare_def::<NodeCryptoModule, _>(ctx.clone(), canonical),
        _ => Err(rquickjs::Error::new_loading_message(
            canonical,
            "Node.js built-in module is not implemented",
        )),
    }
}

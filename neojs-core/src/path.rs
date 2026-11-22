//! Private host facts for the lexical `neo:path` module.
//!
//! The JavaScript module object is installed by [`crate::builtins`] and is
//! exposed as `Neo.getBuiltInModule("path")`, `import ... from "path"`, and
//! `import ... from "neo:path"`. This file registers the two host ops it calls:
//! one reports the build target's path dialect, and one returns the current
//! process working directory as text.
//!
//! # Scope
//!
//! This is not a filesystem module. It does not read, write, stat, create, or
//! delete anything, and it does not resolve symlinks, junctions, drive
//! mappings, or short names. The working-directory op is the only host read.
//! `neo:path` is lexical string manipulation, not a containment or permission
//! boundary.
//!
//! The dialect is a build-time fact from `cfg!(windows)`, not a value supplied
//! by JavaScript. Code that needs to process Windows paths on another host uses
//! the `win32` namespace; the named exports select the build target's dialect.
//!
//! The working-directory op returns the process working directory, not a
//! runtime-local value. It uses `Path::to_str` rather than a lossy conversion,
//! so a non-UTF-8 directory is reported as unavailable instead of silently
//! changing the path. There is no per-drive working directory and no
//! environment-variable fallback. A caller that needs a stable base should pass
//! an explicit absolute path to `resolve` instead of relying on this op.

use deno_core::op2;
use deno_core::v8;

/// The private host boundary behind `neo:path`.
///
/// `opNeoPathIsWindows` returns `true` when `neojs-core` was built for Windows.
/// `opNeoPathCwd` returns the current process working directory as a V8 string,
/// or `null` when the directory cannot be read or is not valid UTF-8. Neither op
/// is a supported public API; `Deno.core` internals are reachable from
/// JavaScript and are not a sandbox.
#[op2(fast)]
fn op_neo_path_is_windows() -> bool {
    cfg!(windows)
}

/// Return the process working directory, or `null`.
///
/// The conversion is strict: `Path::to_str` rejects a non-UTF-8 path on Unix and
/// an unpaired surrogate on Windows. A lossy conversion would return a path that
/// does not name the directory the process is actually in.
#[op2]
fn op_neo_path_cwd<'s, 'i>(scope: &mut v8::PinScope<'s, 'i>) -> v8::Local<'s, v8::Value> {
    let Some(directory) = std::env::current_dir().ok() else {
        return v8::null(scope).into();
    };
    let Some(text) = directory.to_str() else {
        return v8::null(scope).into();
    };
    match v8::String::new(scope, text) {
        Some(value) => value.into(),
        None => v8::null(scope).into(),
    }
}

deno_core::extension!(
    neojs_path,
    ops = [op_neo_path_is_windows, op_neo_path_cwd],
    docs = "Private host facts for `neo:path`: the build target's path dialect and the process working directory text."
);

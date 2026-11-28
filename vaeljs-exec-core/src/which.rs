//! Cross-platform executable lookup (`which` / `which_all`) and safe bare-program resolution.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::ExecError;

#[cfg(any(windows, test))]
const DEFAULT_WINDOWS_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Finds the first executable matching `name` on `PATH` (optionally prepended by `prepend_path`).
///
/// Validates that `name` is non-empty and free of NUL bytes. Explicit paths containing
/// directory separators (or a Windows drive prefix) are checked directly without searching
/// `PATH`. On Unix, targets must be regular files with at least one execute bit (`0o111`).
/// On Windows, bare names are resolved against `PATH` + `PATHEXT` while skipping empty or
/// relative `PATH` entries (never implicitly searching the current working directory).
pub fn which(name: &str, prepend_path: Option<&OsStr>) -> Result<Option<PathBuf>, ExecError> {
    let matches = which_impl(name, prepend_path, true)?;
    Ok(matches.into_iter().next())
}

/// Finds all executables matching `name` across `prepend_path` and `PATH` in search order.
///
/// Behaves identically to [`which`] for validation, explicit path handling, regular-file
/// checks, Unix execute bits, and Windows `PATHEXT` / absolute-entry-only rules, returning
/// all unique matching paths in order.
pub fn which_all(name: &str, prepend_path: Option<&OsStr>) -> Result<Vec<PathBuf>, ExecError> {
    which_impl(name, prepend_path, false)
}

/// Returns `true` iff `path` resolves to an existing regular executable file.
///
/// Rejects empty paths and paths containing NUL bytes. Follows symlinks via
/// [`std::fs::metadata`]. On Unix, requires at least one execute permission bit
/// (`mode & 0o111 != 0`); on Windows, checks regular-file existence.
#[must_use]
pub fn is_executable_file(path: &Path) -> bool {
    let raw = path.as_os_str();
    if raw.is_empty() || raw.as_encoded_bytes().contains(&0) {
        return false;
    }
    #[cfg(not(windows))]
    {
        is_unix_executable_file(path)
    }
    #[cfg(windows)]
    {
        std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
    }
}

/// Safely reads an environment variable by `name`, returning `None` if `name` is
/// empty, contains `=` or NUL bytes, or the variable is unset / non-UTF-8.
#[must_use]
pub fn env_var(name: &str) -> Option<String> {
    if name.is_empty() || name.contains('\0') || name.contains('=') {
        return None;
    }
    std::env::var(name).ok().filter(|val| !val.contains('\0'))
}

/// Expands `${VAR}` and `%VAR%` environment-variable placeholders in `template`.
///
/// Returns `None` if `template` contains NUL bytes or if any referenced
/// placeholder environment variable is unset, empty, or invalid (preventing
/// accidental fallback to root paths like `"/.tool/bin"`). Unclosed
/// placeholders (such as trailing `"${UNCLOSED"` or `"%UNCLOSED"`) are left
/// untouched.
#[must_use]
pub fn expand_env_path(template: &str) -> Option<String> {
    if template.contains('\0') {
        return None;
    }
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
            let rest = &template[i + 2..];
            if let Some(rel_end) = rest.find('}') {
                let name = &rest[..rel_end];
                if !name.is_empty() && !name.contains('$') && !name.contains('{') {
                    let val = env_var(name)?;
                    if val.is_empty() {
                        return None;
                    }
                    out.push_str(&val);
                    i += 2 + rel_end + 1;
                    continue;
                }
            }
            out.push_str("${");
            i += 2;
            continue;
        }
        if bytes[i] == b'%' {
            let rest = &template[i + 1..];
            if let Some(rel_end) = rest.find('%') {
                let name = &rest[..rel_end];
                if !name.is_empty() {
                    let val = env_var(name)?;
                    if val.is_empty() {
                        return None;
                    }
                    out.push_str(&val);
                    i += 1 + rel_end + 1;
                    continue;
                }
                out.push_str("%%");
                i += 2;
                continue;
            }
            out.push('%');
            i += 1;
            continue;
        }
        let ch = template[i..].chars().next()?;
        out.push(ch);
        i += ch.len_utf8();
    }
    if out.contains('\0') {
        return None;
    }
    Some(out)
}

fn validate_which_inputs(name: &str, prepend_path: Option<&OsStr>) -> Result<(), ExecError> {
    if name.is_empty() {
        return Err(ExecError::InvalidArgument {
            reason: "executable name must not be empty",
        });
    }
    if name.contains('\0') {
        return Err(ExecError::InvalidArgument {
            reason: "executable name must not contain a NUL byte",
        });
    }
    if let Some(prepend) = prepend_path
        && prepend.as_encoded_bytes().contains(&0)
    {
        return Err(ExecError::InvalidArgument {
            reason: "prepend_path must not contain a NUL byte",
        });
    }
    Ok(())
}

fn is_bare_program_name(program: &str) -> bool {
    #[cfg(windows)]
    {
        is_bare_program_name_win(program)
    }
    #[cfg(not(windows))]
    {
        !program.contains('/')
    }
}

#[cfg(any(windows, test))]
pub(crate) fn is_bare_program_name_win(program: &str) -> bool {
    let bytes = program.as_bytes();
    let has_drive = bytes.get(1) == Some(&b':');
    !program.contains('/') && !program.contains('\\') && !has_drive
}

fn which_impl(
    name: &str,
    prepend_path: Option<&OsStr>,
    stop_at_first: bool,
) -> Result<Vec<PathBuf>, ExecError> {
    validate_which_inputs(name, prepend_path)?;

    if !is_bare_program_name(name) {
        return Ok(resolve_explicit_path(Path::new(name)));
    }

    let path_var = std::env::var_os("PATH");
    #[cfg(windows)]
    {
        let pathext_var = std::env::var_os("PATHEXT");
        Ok(collect_win_matches(
            name,
            prepend_path,
            path_var.as_deref(),
            pathext_var.as_deref(),
            ';',
            stop_at_first,
        ))
    }
    #[cfg(not(windows))]
    {
        Ok(collect_unix_matches(
            name,
            prepend_path,
            path_var.as_deref(),
            stop_at_first,
        ))
    }
}

fn resolve_explicit_path(path: &Path) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let pathext_var = std::env::var_os("PATHEXT");
        let pathext_list = parse_pathext(pathext_var.as_deref());
        let mut results = Vec::new();
        collect_win_candidates_in_dir(
            path.parent().unwrap_or_else(|| Path::new("")),
            path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
            path,
            &pathext_list,
            &mut results,
            true,
        );
        results
    }
    #[cfg(not(windows))]
    {
        if is_unix_executable_file(path) {
            vec![path.to_path_buf()]
        } else {
            Vec::new()
        }
    }
}

#[cfg(not(windows))]
fn is_unix_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && (meta.permissions().mode() & 0o111) != 0,
        Err(_) => false,
    }
}

#[cfg(not(windows))]
fn collect_unix_matches(
    name: &str,
    prepend_path: Option<&OsStr>,
    path_var: Option<&OsStr>,
    stop_at_first: bool,
) -> Vec<PathBuf> {
    let mut results = Vec::new();
    for raw_source in [prepend_path, path_var].into_iter().flatten() {
        let Some(source_str) = raw_source.to_str() else {
            // Fall back to `split_paths` if non-UTF-8 entries exist.
            for dir in std::env::split_paths(raw_source) {
                if dir.as_os_str().is_empty() {
                    continue;
                }
                let candidate = dir.join(name);
                if is_unix_executable_file(&candidate) && !results.contains(&candidate) {
                    results.push(candidate);
                    if stop_at_first {
                        return results;
                    }
                }
            }
            continue;
        };
        for dir_str in source_str.split(':') {
            if dir_str.is_empty() {
                continue;
            }
            let candidate = Path::new(dir_str).join(name);
            if is_unix_executable_file(&candidate) && !results.contains(&candidate) {
                results.push(candidate);
                if stop_at_first {
                    return results;
                }
            }
        }
    }
    results
}

#[cfg(any(windows, test))]
fn parse_pathext(pathext_var: Option<&OsStr>) -> Vec<String> {
    let raw = pathext_var
        .and_then(|v| v.to_str())
        .unwrap_or(DEFAULT_WINDOWS_PATHEXT);
    let parsed: Vec<String> = raw
        .split(';')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    if parsed.is_empty() {
        DEFAULT_WINDOWS_PATHEXT
            .split(';')
            .map(str::to_owned)
            .collect()
    } else {
        parsed
    }
}

#[cfg(any(windows, test))]
fn collect_win_candidates_in_dir(
    dir: &Path,
    program_name: &str,
    direct_path: &Path,
    pathext_list: &[String],
    results: &mut Vec<PathBuf>,
    stop_at_first: bool,
) {
    if program_name.is_empty() {
        return;
    }
    let program_ext = Path::new(program_name).extension().and_then(|e| e.to_str());
    let has_pathext = program_ext.is_some_and(|ext| {
        pathext_list.iter().any(|candidate| {
            let trimmed = candidate.strip_prefix('.').unwrap_or(candidate);
            trimmed.eq_ignore_ascii_case(ext)
        })
    });

    if program_ext.is_some() {
        if direct_path.is_file() && !results.contains(&direct_path.to_path_buf()) {
            results.push(direct_path.to_path_buf());
            if stop_at_first {
                return;
            }
        }
        if has_pathext {
            return;
        }
    }

    for ext in pathext_list {
        let candidate = dir.join(format!("{program_name}{ext}"));
        if candidate.is_file() && !results.contains(&candidate) {
            results.push(candidate);
            if stop_at_first {
                return;
            }
        }
    }
}

#[cfg(any(windows, test))]
pub(crate) fn collect_win_matches(
    program: &str,
    prepend_path: Option<&OsStr>,
    path_var: Option<&OsStr>,
    pathext_var: Option<&OsStr>,
    path_sep: char,
    stop_at_first: bool,
) -> Vec<PathBuf> {
    let pathext_list = parse_pathext(pathext_var);
    let mut results = Vec::new();

    for raw_source in [prepend_path, path_var].into_iter().flatten() {
        let Some(path_str) = raw_source.to_str() else {
            continue;
        };
        for dir_raw in path_str.split(path_sep) {
            let dir_trimmed = dir_raw
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(dir_raw);
            if dir_trimmed.is_empty() {
                continue;
            }
            let dir = Path::new(dir_trimmed);
            if !dir.is_absolute() {
                continue;
            }
            let direct = dir.join(program);
            collect_win_candidates_in_dir(
                dir,
                program,
                &direct,
                &pathext_list,
                &mut results,
                stop_at_first,
            );
            if stop_at_first && !results.is_empty() {
                return results;
            }
        }
    }

    results
}

#[cfg(any(windows, test))]
pub(crate) fn resolve_bare_program_in_path(
    program: &str,
    path_var: Option<&OsStr>,
    pathext_var: Option<&OsStr>,
    path_sep: char,
) -> Option<PathBuf> {
    collect_win_matches(program, None, path_var, pathext_var, path_sep, true)
        .into_iter()
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_name_and_prepend_path() {
        assert_eq!(
            which("", None).unwrap_err().code(),
            crate::ExecErrorCode::InvalidArgument
        );
        assert_eq!(
            which("foo\0bar", None).unwrap_err().code(),
            crate::ExecErrorCode::InvalidArgument
        );
        assert_eq!(
            which_all("", None).unwrap_err().code(),
            crate::ExecErrorCode::InvalidArgument
        );
        assert_eq!(
            which("ls", Some(OsStr::new("/bin\0/usr/bin")))
                .unwrap_err()
                .code(),
            crate::ExecErrorCode::InvalidArgument
        );
    }

    #[test]
    fn windows_bare_program_resolution_skips_relative_and_handles_quotes_and_dots() {
        let tmp = tempfile::tempdir().unwrap();
        let abs_bin = tmp.path().join("bin");
        let quoted_bin = tmp.path().join("quoted dir");
        std::fs::create_dir_all(&abs_bin).unwrap();
        std::fs::create_dir_all(&quoted_bin).unwrap();

        let tool_exe = abs_bin.join("mytool.exe");
        std::fs::write(&tool_exe, b"stub").unwrap();
        let dotted_exe = quoted_bin.join("my.tool.exe");
        std::fs::write(&dotted_exe, b"stub").unwrap();
        let exact_dotted = quoted_bin.join("exact.bin");
        std::fs::write(&exact_dotted, b"stub").unwrap();

        let pathext = OsStr::new(".com;.exe");
        let rel_only = OsStr::new(".;relative_dir");
        assert_eq!(
            resolve_bare_program_in_path("mytool", Some(rel_only), Some(pathext), ';'),
            None
        );

        let mixed = format!(
            ".;relative_dir;\"{}\";{}",
            quoted_bin.display(),
            abs_bin.display()
        );
        assert_eq!(
            resolve_bare_program_in_path("mytool", Some(OsStr::new(&mixed)), Some(pathext), ';'),
            Some(tool_exe.clone())
        );
        assert_eq!(
            resolve_bare_program_in_path("my.tool", Some(OsStr::new(&mixed)), Some(pathext), ';'),
            Some(dotted_exe)
        );
        assert_eq!(
            resolve_bare_program_in_path("exact.bin", Some(OsStr::new(&mixed)), Some(pathext), ';',),
            Some(exact_dotted)
        );

        assert!(is_bare_program_name_win("mytool.exe"));
        assert!(!is_bare_program_name_win("./mytool.exe"));
        assert!(!is_bare_program_name_win(r"C:\bin\mytool.exe"));
    }

    #[test]
    fn env_var_and_expand_env_path_prevent_unset_fallback_and_leave_unclosed_literal() {
        assert_eq!(env_var(""), None);
        assert_eq!(env_var("BAD=KEY"), None);
        assert_eq!(env_var("BAD\0KEY"), None);
        assert_eq!(
            expand_env_path("${VAEL_DEFINITELY_UNSET_ENV_VAR_999}/.deno/bin/deno"),
            None
        );
        assert_eq!(
            expand_env_path("%VAEL_DEFINITELY_UNSET_ENV_VAR_999%\\bin\\tool.exe"),
            None
        );
        assert_eq!(
            expand_env_path("/usr/local/${UNCLOSED"),
            Some("/usr/local/${UNCLOSED".to_owned())
        );
        assert_eq!(
            expand_env_path("/usr/local/%UNCLOSED"),
            Some("/usr/local/%UNCLOSED".to_owned())
        );
        assert_eq!(expand_env_path("bad\0path"), None);
        // `PATH` is always set and non-empty in the test environment.
        let path_val = env_var("PATH").expect("PATH must be set");
        assert_eq!(
            expand_env_path("${PATH}/extra"),
            Some(format!("{path_val}/extra"))
        );
        assert_eq!(
            expand_env_path("%PATH%\\extra"),
            Some(format!("{path_val}\\extra"))
        );
    }

    #[test]
    fn is_executable_file_checks_regular_executable_and_rejects_nul_or_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_executable_file(tmp.path()));
        assert!(!is_executable_file(Path::new("")));
        assert!(!is_executable_file(Path::new("/tmp/\0bad")));

        let file = tmp.path().join("tool");
        std::fs::write(&file, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert!(!is_executable_file(&file));
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(is_executable_file(&file));
        }
        #[cfg(windows)]
        {
            assert!(is_executable_file(&file));
        }
    }
}

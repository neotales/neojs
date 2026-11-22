//! Deterministic expansion of the files and globs given to `neojs test`.
//!
//! Each argument is either an existing regular file or a shell-style glob whose
//! metacharacters are `*`, `?`, and `[`. Globs never cross a path separator for
//! `*`/`?`, never match a leading dot implicitly, and are case-sensitive. The
//! expansion is sorted by canonical path and deduplicated, so overlapping
//! arguments run each file once regardless of their order.
//!
//! A missing literal, an unreadable directory, a malformed pattern, or a glob
//! that selects no regular files is an error. The command never turns an
//! unmatched argument into a silent empty run, because that looks exactly like
//! a passing suite.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use glob::MatchOptions;
use glob::glob_with;

/// Expands each argument and returns the canonical files in sorted order.
///
/// A file that exists literally is accepted even when its name contains a glob
/// metacharacter. Directories selected by a glob are skipped; a pattern that
/// selects only directories is an error. The caller receives no duplicate
/// paths and no path that is not a regular file.
pub(crate) fn expand(patterns: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut files = BTreeSet::new();
    for pattern in patterns {
        if pattern.is_empty() {
            return Err("a test file or glob cannot be empty".to_string());
        }
        let path = Path::new(pattern);
        if path.is_file() {
            files.insert(canonical(path, pattern)?);
            continue;
        }
        if !has_glob_metacharacter(pattern) {
            return Err(format!(
                "test file `{pattern}` does not exist or is not a regular file"
            ));
        }
        let options = MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: true,
        };
        let matches = glob_with(pattern, options).map_err(|error| {
            format!(
                "invalid test glob `{pattern}` at byte {}: {}",
                error.pos, error.msg
            )
        })?;
        let mut selected = 0usize;
        for entry in matches {
            let path = entry.map_err(|error| format!("cannot expand `{pattern}`: {error}"))?;
            if path.is_file() {
                files.insert(canonical(&path, pattern)?);
                selected += 1;
            }
        }
        if selected == 0 {
            return Err(format!("test glob `{pattern}` matched no files"));
        }
    }
    if files.is_empty() {
        return Err("no test files were selected".to_string());
    }
    Ok(files.into_iter().collect())
}

fn canonical(path: &Path, pattern: &str) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|error| {
        format!(
            "cannot resolve `{}` from `{pattern}`: {error}",
            path.display()
        )
    })
}

fn has_glob_metacharacter(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|character| matches!(character, '*' | '?' | '['))
}

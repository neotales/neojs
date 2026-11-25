//! Cross-platform lexical path manipulation matching `node:path`.
//!
//! Provides both POSIX (`/` separator, `:` delimiter) and Win32 (`\` separator,
//! `;` delimiter, drive letters, UNC shares, and device namespaces) dialects
//! explicitly via [`Style`], plus [`Style::native`] for the host build target.
//!
//! Every operation enforces single-path ([`MAX_SINGLE_PATH_BYTES`]) and
//! aggregate ([`MAX_AGGREGATE_PATH_BYTES`]) byte bounds and rejects NUL (`\0`)
//! bytes.

use crate::error::NodeCoreError;

/// Maximum byte length allowed for a single path or suffix input (`64 KiB`).
pub const MAX_SINGLE_PATH_BYTES: usize = 64 * 1024;

/// Maximum aggregate byte length allowed across multi-part inputs (`256 KiB`).
pub const MAX_AGGREGATE_PATH_BYTES: usize = 256 * 1024;

/// Path syntax dialect (`Posix` or `Win32`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    /// POSIX slash-separated paths (`node:path/posix`).
    Posix,
    /// Windows backslash/slash and drive/UNC paths (`node:path/win32`).
    Win32,
}

impl Style {
    /// Returns the path dialect of the current compilation target.
    ///
    /// # Example
    ///
    /// ```
    /// use vaeljs_node_core::path::Style;
    ///
    /// if cfg!(windows) {
    ///     assert_eq!(Style::native(), Style::Win32);
    /// } else {
    ///     assert_eq!(Style::native(), Style::Posix);
    /// }
    /// ```
    #[must_use]
    pub const fn native() -> Self {
        if cfg!(windows) {
            Self::Win32
        } else {
            Self::Posix
        }
    }
}

/// Decomposed path components returned by [`parse`] and accepted by [`format()`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParsedPath {
    /// Root prefix (such as `"/"`, `"C:\\"`, or `"\\\\server\\share\\"`).
    pub root: String,
    /// Directory portion excluding the trailing file name.
    pub dir: String,
    /// Final component including extension (for example `"file.txt"`).
    pub base: String,
    /// File extension including the leading dot (for example `".txt"`).
    pub ext: String,
    /// File name without extension (for example `"file"`).
    pub name: String,
}

/// Returns the platform path segment separator (`"/"` or `"\\"`).
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, sep};
///
/// assert_eq!(sep(Style::Posix), "/");
/// assert_eq!(sep(Style::Win32), "\\");
/// ```
#[must_use]
pub const fn sep(style: Style) -> &'static str {
    match style {
        Style::Posix => "/",
        Style::Win32 => "\\",
    }
}

/// Returns the platform `PATH` environment variable delimiter (`":"` or `";"`).
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, delimiter};
///
/// assert_eq!(delimiter(Style::Posix), ":");
/// assert_eq!(delimiter(Style::Win32), ";");
/// ```
#[must_use]
pub const fn delimiter(style: Style) -> &'static str {
    match style {
        Style::Posix => ":",
        Style::Win32 => ";",
    }
}

/// Joins all path segments with the dialect separator and normalizes the result.
///
/// Empty segments are ignored; if all segments are empty (or `parts` is empty),
/// returns `"."`.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, join};
///
/// assert_eq!(join(Style::Posix, &["/foo", "bar", "baz/asdf", ".."]).unwrap(), "/foo/bar/baz");
/// assert_eq!(join(Style::Win32, &["C:\\foo", "bar", "..", "baz"]).unwrap(), "C:\\foo\\baz");
/// assert_eq!(join(Style::Posix, &[]).unwrap(), ".");
/// ```
pub fn join(style: Style, parts: &[&str]) -> Result<String, NodeCoreError> {
    validate_parts(parts)?;
    match style {
        Style::Posix => Ok(join_posix(parts)),
        Style::Win32 => Ok(join_win32(parts)),
    }
}

/// Resolves a sequence of paths or path segments into an absolute path using `cwd`
/// as the fallback base when no segment in `parts` provides an absolute root.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, resolve};
///
/// assert_eq!(
///     resolve(Style::Posix, "/home/dev", &["src", "lib.rs"]).unwrap(),
///     "/home/dev/src/lib.rs"
/// );
/// assert_eq!(
///     resolve(Style::Posix, "/home/dev", &["a", "/etc", "hosts"]).unwrap(),
///     "/etc/hosts"
/// );
/// assert_eq!(
///     resolve(Style::Win32, "C:\\work", &["sub", "..\\out"]).unwrap(),
///     "C:\\work\\out"
/// );
/// ```
pub fn resolve(style: Style, cwd: &str, parts: &[&str]) -> Result<String, NodeCoreError> {
    validate_single(cwd)?;
    validate_parts_with_extra(cwd.len(), parts)?;
    match style {
        Style::Posix => resolve_posix(parts, || {
            if cwd.is_empty() || !is_posix_sep(cwd.as_bytes()[0]) {
                return Err(NodeCoreError::InvalidPath {
                    reason: "POSIX resolve requires an absolute cwd",
                });
            }
            Ok(cwd.to_owned())
        }),
        Style::Win32 => resolve_win32(parts, || {
            if cwd.is_empty() {
                return Err(NodeCoreError::InvalidPath {
                    reason: "Win32 resolve requires an absolute cwd",
                });
            }
            Ok(cwd.to_owned())
        }),
    }
}

/// Normalizes `path`, resolving `'.'` and `'..'` segments and collapsing repeated separators.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, normalize};
///
/// assert_eq!(normalize(Style::Posix, "/foo/bar//baz/asdf/..").unwrap(), "/foo/bar/baz");
/// assert_eq!(normalize(Style::Win32, "C:\\temp\\\\foo\\..\\bar\\").unwrap(), "C:\\temp\\bar\\");
/// ```
pub fn normalize(style: Style, path: &str) -> Result<String, NodeCoreError> {
    validate_single(path)?;
    match style {
        Style::Posix => Ok(normalize_posix(path)),
        Style::Win32 => Ok(normalize_win32(path)),
    }
}

/// Computes the relative path from `from` to `to` in `style`.
///
/// If both `from` and `to` resolve to the same location, returns `""`. When
/// `style` matches [`Style::native`], non-absolute endpoints fall back to the
/// current process working directory; for non-native styles, relative-only
/// endpoints resolve against a canonical synthetic root while mixed absolute and
/// relative endpoints fail with [`NodeCoreError::InvalidPath`].
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, relative};
///
/// assert_eq!(
///     relative(Style::Posix, "/data/orandea/test/aaa", "/data/orandea/impl/bbb").unwrap(),
///     "../../impl/bbb"
/// );
/// assert_eq!(
///     relative(Style::Win32, "C:\\orandea\\test\\aaa", "C:\\orandea\\impl\\bbb").unwrap(),
///     "..\\..\\impl\\bbb"
/// );
/// ```
pub fn relative(style: Style, from: &str, to: &str) -> Result<String, NodeCoreError> {
    validate_pair(from, to)?;
    match style {
        Style::Posix => relative_posix(from, to),
        Style::Win32 => relative_win32(from, to),
    }
}

/// Returns the directory name of `path`, similar to Unix `dirname`.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, dirname};
///
/// assert_eq!(dirname(Style::Posix, "/foo/bar/baz.js").unwrap(), "/foo/bar");
/// assert_eq!(dirname(Style::Win32, "C:\\foo\\bar").unwrap(), "C:\\foo");
/// ```
pub fn dirname(style: Style, path: &str) -> Result<String, NodeCoreError> {
    validate_single(path)?;
    match style {
        Style::Posix => Ok(dirname_posix(path)),
        Style::Win32 => Ok(dirname_win32(path)),
    }
}

/// Returns the last portion of `path`, optionally stripping a matching `suffix`.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, basename};
///
/// assert_eq!(basename(Style::Posix, "/foo/bar/baz.html", None).unwrap(), "baz.html");
/// assert_eq!(basename(Style::Posix, "/foo/bar/baz.html", Some(".html")).unwrap(), "baz");
/// assert_eq!(basename(Style::Win32, "C:\\foo\\bar.txt", Some(".txt")).unwrap(), "bar");
/// ```
pub fn basename(style: Style, path: &str, suffix: Option<&str>) -> Result<String, NodeCoreError> {
    let suffix = suffix.unwrap_or("");
    validate_pair(path, suffix)?;
    match style {
        Style::Posix => Ok(basename_posix(path, suffix)),
        Style::Win32 => Ok(basename_win32(path, suffix)),
    }
}

/// Returns the extension of `path`, from the last `.` to the end of the final component.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, extname};
///
/// assert_eq!(extname(Style::Posix, "index.html").unwrap(), ".html");
/// assert_eq!(extname(Style::Posix, ".bashrc").unwrap(), "");
/// assert_eq!(extname(Style::Win32, "C:\\dir\\archive.tar.gz").unwrap(), ".gz");
/// ```
pub fn extname(style: Style, path: &str) -> Result<String, NodeCoreError> {
    validate_single(path)?;
    match style {
        Style::Posix => Ok(extname_posix(path)),
        Style::Win32 => Ok(extname_win32(path)),
    }
}

/// Parses `path` into its [`ParsedPath`] components (`root`, `dir`, `base`, `ext`, `name`).
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{ParsedPath, Style, parse};
///
/// let parsed = parse(Style::Posix, "/home/user/dir/file.txt").unwrap();
/// assert_eq!(
///     parsed,
///     ParsedPath {
///         root: "/".to_owned(),
///         dir: "/home/user/dir".to_owned(),
///         base: "file.txt".to_owned(),
///         ext: ".txt".to_owned(),
///         name: "file".to_owned(),
///     }
/// );
/// ```
pub fn parse(style: Style, path: &str) -> Result<ParsedPath, NodeCoreError> {
    validate_single(path)?;
    match style {
        Style::Posix => Ok(parse_posix(path)),
        Style::Win32 => Ok(parse_win32(path)),
    }
}

/// Formats a [`ParsedPath`] back into a path string according to `style`.
///
/// Follows Node.js precedence rules: `dir` takes precedence over `root`, and
/// `base` takes precedence over `name` + `ext`.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{ParsedPath, Style, format};
///
/// let p = ParsedPath {
///     root: "/".to_owned(),
///     dir: "/home/user".to_owned(),
///     base: String::new(),
///     ext: ".rs".to_owned(),
///     name: "main".to_owned(),
/// };
/// assert_eq!(format(Style::Posix, &p).unwrap(), "/home/user/main.rs");
/// ```
pub fn format(style: Style, parsed: &ParsedPath) -> Result<String, NodeCoreError> {
    let fields = [
        parsed.root.as_str(),
        parsed.dir.as_str(),
        parsed.base.as_str(),
        parsed.ext.as_str(),
        parsed.name.as_str(),
    ];
    validate_parts(&fields)?;

    let dir = if !parsed.dir.is_empty() {
        parsed.dir.as_str()
    } else {
        parsed.root.as_str()
    };

    let combined_base;
    let base = if !parsed.base.is_empty() {
        parsed.base.as_str()
    } else {
        combined_base = format!("{}{}", parsed.name, parsed.ext);
        combined_base.as_str()
    };

    if dir.is_empty() {
        return Ok(base.to_owned());
    }
    if dir == parsed.root {
        return Ok(format!("{dir}{base}"));
    }
    Ok(format!("{dir}{}{base}", sep(style)))
}

/// Returns `true` if `path` is absolute in the given `style`.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::path::{Style, is_absolute};
///
/// assert!(is_absolute(Style::Posix, "/foo/bar").unwrap());
/// assert!(!is_absolute(Style::Posix, "foo/bar").unwrap());
/// assert!(is_absolute(Style::Win32, "C:\\foo").unwrap());
/// assert!(!is_absolute(Style::Win32, "C:foo").unwrap());
/// ```
pub fn is_absolute(style: Style, path: &str) -> Result<bool, NodeCoreError> {
    validate_single(path)?;
    Ok(match style {
        Style::Posix => !path.is_empty() && is_posix_sep(path.as_bytes()[0]),
        Style::Win32 => is_absolute_win32(path),
    })
}

fn validate_single(value: &str) -> Result<(), NodeCoreError> {
    if value.as_bytes().contains(&0) {
        return Err(NodeCoreError::InvalidPath {
            reason: "path must not contain a NUL byte",
        });
    }
    if value.len() > MAX_SINGLE_PATH_BYTES {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_SINGLE_PATH_BYTES,
            actual: Some(value.len()),
        });
    }
    Ok(())
}

fn validate_pair(a: &str, b: &str) -> Result<(), NodeCoreError> {
    validate_single(a)?;
    validate_single(b)?;
    let total = a.len().saturating_add(b.len());
    if total > MAX_AGGREGATE_PATH_BYTES {
        return Err(NodeCoreError::InputTooLarge {
            limit: MAX_AGGREGATE_PATH_BYTES,
            actual: Some(total),
        });
    }
    Ok(())
}

fn validate_parts(parts: &[&str]) -> Result<(), NodeCoreError> {
    validate_parts_with_extra(0, parts)
}

fn validate_parts_with_extra(initial: usize, parts: &[&str]) -> Result<(), NodeCoreError> {
    let mut total = initial;
    for &part in parts {
        validate_single(part)?;
        total = total.saturating_add(part.len());
        if total > MAX_AGGREGATE_PATH_BYTES {
            return Err(NodeCoreError::InputTooLarge {
                limit: MAX_AGGREGATE_PATH_BYTES,
                actual: Some(total),
            });
        }
    }
    Ok(())
}

#[inline]
const fn is_posix_sep(byte: u8) -> bool {
    byte == b'/'
}

#[inline]
const fn is_win32_sep(byte: u8) -> bool {
    byte == b'/' || byte == b'\\'
}

#[inline]
const fn is_device_root(byte: u8) -> bool {
    byte.is_ascii_alphabetic()
}

fn normalize_string(
    path: &str,
    allow_above_root: bool,
    separator: char,
    is_sep: fn(u8) -> bool,
) -> String {
    let bytes = path.as_bytes();
    let len = bytes.len();
    let mut result = String::new();
    let mut last_segment_len: usize = 0;
    let mut last_slash: isize = -1;
    let mut dots: i32 = 0;
    let mut code: u8 = 0;

    for i in 0..=len {
        if i < len {
            code = bytes[i];
        } else if is_sep(code) {
            break;
        } else {
            code = b'/';
        }

        if is_sep(code) {
            let i_isize = i as isize;
            if last_slash == i_isize - 1 || dots == 1 {
                // Empty segment or `.`: skip.
            } else if dots == 2 {
                if result.len() < 2 || last_segment_len != 2 || !result.ends_with("..") {
                    if result.len() > 2 {
                        if let Some(last_sep_idx) = result.rfind(separator) {
                            result.truncate(last_sep_idx);
                            last_segment_len = result
                                .rfind(separator)
                                .map_or(result.len(), |idx| result.len() - 1 - idx);
                        } else {
                            result.clear();
                            last_segment_len = 0;
                        }
                        last_slash = i_isize;
                        dots = 0;
                        continue;
                    }
                    if result.len() == 2 || result.len() == 1 {
                        result.clear();
                        last_segment_len = 0;
                        last_slash = i_isize;
                        dots = 0;
                        continue;
                    }
                }
                if allow_above_root {
                    if !result.is_empty() {
                        result.push(separator);
                    }
                    result.push_str("..");
                    last_segment_len = 2;
                }
            } else {
                let start = (last_slash + 1) as usize;
                let segment = &path[start..i];
                if !result.is_empty() {
                    result.push(separator);
                }
                result.push_str(segment);
                last_segment_len = i - start;
            }
            last_slash = i_isize;
            dots = 0;
        } else if code == b'.' && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
    }

    result
}

fn strip_trailing_separators(segment: &str, is_sep: fn(u8) -> bool, minimum: usize) -> &str {
    let bytes = segment.as_bytes();
    let mut end = bytes.len();
    while end > minimum && is_sep(bytes[end - 1]) {
        end -= 1;
    }
    &segment[..end]
}

fn strip_suffix<'a>(name: &'a str, suffix: &str) -> &'a str {
    if suffix.is_empty() || suffix.len() >= name.len() {
        return name;
    }
    name.strip_suffix(suffix).unwrap_or(name)
}

fn basename_common(
    path: &str,
    suffix: &str,
    is_sep: fn(u8) -> bool,
    has_drive_prefix: bool,
) -> String {
    let bytes = path.as_bytes();
    let mut end = bytes.len();
    while end > 0 && is_sep(bytes[end - 1]) {
        end -= 1;
    }

    let mut start = 0;
    if has_drive_prefix && end >= 2 && is_device_root(bytes[0]) && bytes[1] == b':' {
        start = 2;
    }

    if end <= start {
        return String::new();
    }

    let mut name_start = end - 1;
    while name_start > start && !is_sep(bytes[name_start - 1]) {
        name_start -= 1;
    }

    strip_suffix(&path[name_start..end], suffix).to_owned()
}

fn extname_common(path: &str, is_sep: fn(u8) -> bool, start: usize) -> String {
    let bytes = path.as_bytes();
    let mut start_dot: Option<usize> = None;
    let mut start_part = start;
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    let mut pre_dot_state: i8 = 0;

    let mut i = bytes.len();
    while i > start {
        i -= 1;
        let code = bytes[i];
        if is_sep(code) {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            continue;
        }
        if end.is_none() {
            matched_slash = false;
            end = Some(i + 1);
        }
        if code == b'.' {
            if start_dot.is_none() {
                start_dot = Some(i);
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot.is_some() {
            pre_dot_state = -1;
        }
    }

    let (Some(sd), Some(e)) = (start_dot, end) else {
        return String::new();
    };
    if pre_dot_state == 0 || (pre_dot_state == 1 && sd == e - 1 && sd == start_part + 1) {
        return String::new();
    }
    path[sd..e].to_owned()
}

fn normalize_posix(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let bytes = path.as_bytes();
    let absolute = is_posix_sep(bytes[0]);
    let trailing = is_posix_sep(bytes[bytes.len() - 1]);

    let mut tail = normalize_string(path, !absolute, '/', is_posix_sep);
    if tail.is_empty() && !absolute {
        tail.push('.');
    }
    if !tail.is_empty() && trailing {
        tail.push('/');
    }

    if absolute { format!("/{tail}") } else { tail }
}

fn join_posix(parts: &[&str]) -> String {
    let non_empty: Vec<&str> = parts.iter().copied().filter(|s| !s.is_empty()).collect();
    if non_empty.is_empty() {
        return ".".to_owned();
    }
    normalize_posix(&non_empty.join("/"))
}

fn basename_posix(path: &str, suffix: &str) -> String {
    basename_common(path, suffix, is_posix_sep, false)
}

fn dirname_posix(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let bytes = path.as_bytes();
    let mut end: Option<usize> = None;
    let mut matched_non_sep = false;

    for i in (1..bytes.len()).rev() {
        if is_posix_sep(bytes[i]) {
            if matched_non_sep {
                end = Some(i);
                break;
            }
        } else {
            matched_non_sep = true;
        }
    }

    let absolute = is_posix_sep(bytes[0]);
    let Some(end) = end else {
        return if absolute {
            "/".to_owned()
        } else {
            ".".to_owned()
        };
    };

    strip_trailing_separators(&path[..end], is_posix_sep, usize::from(absolute)).to_owned()
}

fn extname_posix(path: &str) -> String {
    extname_common(path, is_posix_sep, 0)
}

fn parse_posix(path: &str) -> ParsedPath {
    let mut result = ParsedPath::default();
    if path.is_empty() {
        return result;
    }

    let bytes = path.as_bytes();
    let absolute = is_posix_sep(bytes[0]);
    let start = if absolute {
        result.root = "/".to_owned();
        1
    } else {
        0
    };

    let mut start_dot: Option<usize> = None;
    let mut start_part = 0;
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    let mut pre_dot_state: i8 = 0;

    let mut i = bytes.len();
    while i > start {
        i -= 1;
        let code = bytes[i];
        if is_posix_sep(code) {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            continue;
        }
        if end.is_none() {
            matched_slash = false;
            end = Some(i + 1);
        }
        if code == b'.' {
            if start_dot.is_none() {
                start_dot = Some(i);
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot.is_some() {
            pre_dot_state = -1;
        }
    }

    if let Some(e) = end {
        let component_start = if start_part == 0 && absolute {
            1
        } else {
            start_part
        };
        let has_ext = start_dot.is_some_and(|sd| {
            pre_dot_state != 0 && !(pre_dot_state == 1 && sd == e - 1 && sd == component_start + 1)
        });
        if has_ext {
            let sd = start_dot.unwrap_or(e);
            result.name = path[component_start..sd].to_owned();
            result.base = path[component_start..e].to_owned();
            result.ext = path[sd..e].to_owned();
        } else {
            result.base = path[component_start..e].to_owned();
            result.name = result.base.clone();
        }
    }

    if start_part > 0 {
        result.dir =
            strip_trailing_separators(&path[..start_part - 1], is_posix_sep, usize::from(absolute))
                .to_owned();
    } else if absolute {
        result.dir = "/".to_owned();
    }

    result
}

fn is_win32_unc_root_only(path: &str) -> bool {
    let bytes = path.as_bytes();
    let len = bytes.len();
    if len < 4 || !is_win32_sep(bytes[0]) || !is_win32_sep(bytes[1]) {
        return false;
    }

    let mut idx = 2;
    let server_start = idx;
    while idx < len && !is_win32_sep(bytes[idx]) {
        idx += 1;
    }
    if idx == server_start || idx >= len {
        return false;
    }

    let sep_start = idx;
    while idx < len && is_win32_sep(bytes[idx]) {
        idx += 1;
    }
    if idx == sep_start || idx >= len {
        return false;
    }

    let share_start = idx;
    while idx < len && !is_win32_sep(bytes[idx]) {
        idx += 1;
    }
    if idx == share_start {
        return false;
    }

    while idx < len {
        if !is_win32_sep(bytes[idx]) {
            return false;
        }
        idx += 1;
    }
    true
}

fn normalize_win32(path: &str) -> String {
    let bytes = path.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return ".".to_owned();
    }

    let mut root_end = 0;
    let mut device: Option<String> = None;
    let mut absolute = false;
    let code = bytes[0];

    if len > 1 {
        if is_win32_sep(code) {
            absolute = true;
            if is_win32_sep(bytes[1]) {
                let mut j = 2;
                let mut last = j;
                while j < len && !is_win32_sep(bytes[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    let first_part = &path[last..j];
                    last = j;
                    while j < len && is_win32_sep(bytes[j]) {
                        j += 1;
                    }
                    if j < len && j != last {
                        last = j;
                        while j < len && !is_win32_sep(bytes[j]) {
                            j += 1;
                        }
                        if j == len {
                            return format!("\\\\{first_part}\\{}\\", &path[last..]);
                        }
                        if j != last {
                            device = Some(format!("\\\\{first_part}\\{}", &path[last..j]));
                            root_end = j;
                        }
                    }
                }
            } else {
                root_end = 1;
            }
        } else if is_device_root(code) && bytes[1] == b':' {
            device = Some(path[..2].to_owned());
            root_end = 2;
            if len > 2 && is_win32_sep(bytes[2]) {
                absolute = true;
                root_end = 3;
            }
        }
    } else if is_win32_sep(code) {
        return "\\".to_owned();
    }

    let mut tail = if root_end < len {
        normalize_string(&path[root_end..], !absolute, '\\', is_win32_sep)
    } else {
        String::new()
    };

    if tail.is_empty() && !absolute {
        tail.push('.');
    }
    if !tail.is_empty() && is_win32_sep(bytes[len - 1]) {
        tail.push('\\');
    }

    match device {
        None => {
            if absolute {
                if tail.is_empty() {
                    "\\".to_owned()
                } else {
                    format!("\\{tail}")
                }
            } else {
                tail
            }
        }
        Some(dev) => {
            if absolute {
                if tail.is_empty() {
                    format!("{dev}\\")
                } else {
                    format!("{dev}\\{tail}")
                }
            } else {
                format!("{dev}{tail}")
            }
        }
    }
}

fn join_win32(parts: &[&str]) -> String {
    let non_empty: Vec<&str> = parts.iter().copied().filter(|s| !s.is_empty()).collect();
    if non_empty.is_empty() {
        return ".".to_owned();
    }

    let mut needs_replace = true;
    let mut slash_count = 0;
    let first = non_empty[0].as_bytes();
    if is_win32_sep(first[0]) {
        slash_count += 1;
        if first.len() > 1 && is_win32_sep(first[1]) {
            slash_count += 1;
            if first.len() > 2 {
                if is_win32_sep(first[2]) {
                    slash_count += 1;
                } else {
                    needs_replace = false;
                }
            }
        }
    }

    let mut joined = non_empty.join("\\");
    if needs_replace {
        let joined_bytes = joined.as_bytes();
        while slash_count < joined_bytes.len() && is_win32_sep(joined_bytes[slash_count]) {
            slash_count += 1;
        }
        if slash_count >= 2 {
            joined = format!("\\{}", &joined[slash_count..]);
        }
    }

    normalize_win32(&joined)
}

fn basename_win32(path: &str, suffix: &str) -> String {
    if is_win32_unc_root_only(path) {
        return String::new();
    }
    basename_common(path, suffix, is_win32_sep, true)
}

fn dirname_win32(path: &str) -> String {
    let bytes = path.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return ".".to_owned();
    }

    let mut root_end: Option<usize> = None;
    let mut offset = 0;
    let code = bytes[0];

    if len > 1 {
        if is_win32_sep(code) {
            root_end = Some(1);
            offset = 1;
            if is_win32_sep(bytes[1]) {
                let mut j = 2;
                let mut last = j;
                while j < len && !is_win32_sep(bytes[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && is_win32_sep(bytes[j]) {
                        j += 1;
                    }
                    if j < len && j != last {
                        last = j;
                        while j < len && !is_win32_sep(bytes[j]) {
                            j += 1;
                        }
                        if j == len {
                            return path.to_owned();
                        }
                        if j != last {
                            root_end = Some(j + 1);
                            offset = j + 1;
                        }
                    }
                }
            }
        } else if is_device_root(code) && bytes[1] == b':' {
            root_end = Some(2);
            offset = 2;
            if len > 2 && is_win32_sep(bytes[2]) {
                root_end = Some(3);
                offset = 3;
            }
        }
    } else if is_win32_sep(code) {
        return path.to_owned();
    }

    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    let mut i = len;
    while i > offset {
        i -= 1;
        if is_win32_sep(bytes[i]) {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }

    let Some(end) = end else {
        return match root_end {
            None => ".".to_owned(),
            Some(re) => path[..re].to_owned(),
        };
    };

    strip_trailing_separators(&path[..end], is_win32_sep, root_end.unwrap_or(0)).to_owned()
}

fn extname_win32(path: &str) -> String {
    if is_win32_unc_root_only(path) {
        return String::new();
    }
    let bytes = path.as_bytes();
    let start = if bytes.len() >= 2 && bytes[1] == b':' && is_device_root(bytes[0]) {
        2
    } else {
        0
    };
    extname_common(path, is_win32_sep, start)
}

fn is_absolute_win32(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let code = bytes[0];
    if is_win32_sep(code) {
        return true;
    }
    is_device_root(code) && bytes.len() > 2 && bytes[1] == b':' && is_win32_sep(bytes[2])
}

fn parse_win32(path: &str) -> ParsedPath {
    let mut result = ParsedPath::default();
    let bytes = path.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return result;
    }

    let mut root_end = 0;
    let code = bytes[0];

    if len > 1 {
        if is_win32_sep(code) {
            root_end = 1;
            if is_win32_sep(bytes[1]) {
                let mut j = 2;
                let mut last = j;
                while j < len && !is_win32_sep(bytes[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && is_win32_sep(bytes[j]) {
                        j += 1;
                    }
                    if j < len && j != last {
                        last = j;
                        while j < len && !is_win32_sep(bytes[j]) {
                            j += 1;
                        }
                        if j == len {
                            root_end = j;
                        } else if j != last {
                            root_end = j + 1;
                        }
                    }
                }
            }
        } else if is_device_root(code) && bytes[1] == b':' {
            root_end = 2;
            if len > 2 {
                if is_win32_sep(bytes[2]) {
                    if len == 3 {
                        result.root = path.to_owned();
                        result.dir = path.to_owned();
                        return result;
                    }
                    root_end = 3;
                }
            } else {
                result.root = path.to_owned();
                result.dir = path.to_owned();
                return result;
            }
        }
    } else if is_win32_sep(code) {
        result.root = path.to_owned();
        result.dir = path.to_owned();
        return result;
    }

    if root_end > 0 {
        result.root = path[..root_end].to_owned();
    }

    let mut start_dot: Option<usize> = None;
    let mut start_part = root_end;
    let mut end: Option<usize> = None;
    let mut matched_slash = true;
    let mut pre_dot_state: i8 = 0;

    let mut i = len;
    while i > root_end {
        i -= 1;
        let b = bytes[i];
        if is_win32_sep(b) {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            continue;
        }
        if end.is_none() {
            matched_slash = false;
            end = Some(i + 1);
        }
        if b == b'.' {
            if start_dot.is_none() {
                start_dot = Some(i);
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot.is_some() {
            pre_dot_state = -1;
        }
    }

    if let Some(e) = end {
        let has_ext = start_dot.is_some_and(|sd| {
            pre_dot_state != 0 && !(pre_dot_state == 1 && sd == e - 1 && sd == start_part + 1)
        });
        if has_ext {
            let sd = start_dot.unwrap_or(e);
            result.name = path[start_part..sd].to_owned();
            result.base = path[start_part..e].to_owned();
            result.ext = path[sd..e].to_owned();
        } else {
            result.base = path[start_part..e].to_owned();
            result.name = result.base.clone();
        }
    }

    if start_part > 0 && start_part != root_end {
        result.dir =
            strip_trailing_separators(&path[..start_part - 1], is_win32_sep, root_end).to_owned();
    } else {
        result.dir = result.root.clone();
    }

    result
}

fn normalized_tail(mut tails: Vec<String>, separator: char, is_sep: fn(u8) -> bool) -> String {
    tails.reverse();
    let sep_str = if separator == '/' { "/" } else { "\\" };
    normalize_string(&tails.join(sep_str), false, separator, is_sep)
}

fn resolve_posix<F>(parts: &[&str], mut get_cwd: F) -> Result<String, NodeCoreError>
where
    F: FnMut() -> Result<String, NodeCoreError>,
{
    let mut tails: Vec<String> = Vec::new();
    let mut resolved_abs = false;
    for &part in parts.iter().rev() {
        if part.is_empty() {
            continue;
        }
        tails.push(part.to_owned());
        if is_posix_sep(part.as_bytes()[0]) {
            resolved_abs = true;
            break;
        }
    }
    if !resolved_abs {
        let cwd = get_cwd()?;
        if !cwd.is_empty() {
            tails.push(cwd);
        }
    }
    Ok(format!("/{}", normalized_tail(tails, '/', is_posix_sep)))
}

struct Win32ResolutionRoot {
    device: String,
    drive: String,
    absolute: bool,
    end: usize,
}

fn skip_win32_seps(bytes: &[u8], mut start: usize) -> usize {
    while start < bytes.len() && is_win32_sep(bytes[start]) {
        start += 1;
    }
    start
}

fn win32_component_end(bytes: &[u8], mut start: usize) -> usize {
    while start < bytes.len() && !is_win32_sep(bytes[start]) {
        start += 1;
    }
    start
}

fn has_namespace_prefix(bytes: &[u8]) -> bool {
    bytes.len() >= 4
        && is_win32_sep(bytes[0])
        && is_win32_sep(bytes[1])
        && (bytes[2] == b'?' || bytes[2] == b'.')
        && is_win32_sep(bytes[3])
}

fn resolution_root_win32(path: &str) -> Result<Win32ResolutionRoot, NodeCoreError> {
    let bytes = path.as_bytes();
    if bytes.is_empty() {
        return Ok(Win32ResolutionRoot {
            device: String::new(),
            drive: String::new(),
            absolute: false,
            end: 0,
        });
    }
    let first = bytes[0];
    if is_device_root(first) && bytes.len() >= 2 && bytes[1] == b':' {
        let device = path[..2].to_owned();
        let absolute = bytes.len() > 2 && is_win32_sep(bytes[2]);
        return Ok(Win32ResolutionRoot {
            drive: device.clone(),
            device,
            absolute,
            end: if absolute { 3 } else { 2 },
        });
    }
    if !is_win32_sep(first) {
        return Ok(Win32ResolutionRoot {
            device: String::new(),
            drive: String::new(),
            absolute: false,
            end: 0,
        });
    }
    if bytes.len() >= 2 && is_win32_sep(bytes[1]) {
        let server_end = win32_component_end(bytes, 2);
        let share_start = skip_win32_seps(bytes, server_end);
        let mut end = win32_component_end(bytes, share_start);
        if server_end > 2 && end > share_start {
            let server = &path[2..server_end];
            let share = &path[share_start..end];
            let mut device = format!("\\\\{server}\\{share}");
            let mut drive = String::new();
            if server == "?" && share.eq_ignore_ascii_case("unc") {
                let host_start = skip_win32_seps(bytes, end);
                let host_end = win32_component_end(bytes, host_start);
                let volume_start = skip_win32_seps(bytes, host_end);
                end = win32_component_end(bytes, volume_start);
                if host_start == host_end || volume_start == end {
                    return Err(NodeCoreError::InvalidPath {
                        reason: "namespaced UNC path requires a server and share",
                    });
                }
                device.push('\\');
                device.push_str(&path[host_start..host_end]);
                device.push('\\');
                device.push_str(&path[volume_start..end]);
            } else if server == "?"
                && share.len() == 2
                && is_device_root(share.as_bytes()[0])
                && share.as_bytes()[1] == b':'
            {
                drive = share.to_owned();
            }
            return Ok(Win32ResolutionRoot {
                device,
                drive,
                absolute: true,
                end,
            });
        }
    }
    if has_namespace_prefix(bytes) {
        return Err(NodeCoreError::InvalidPath {
            reason: "Windows namespace root is incomplete",
        });
    }
    Ok(Win32ResolutionRoot {
        device: String::new(),
        drive: String::new(),
        absolute: true,
        end: 1,
    })
}

fn resolve_win32<F>(parts: &[&str], mut get_cwd: F) -> Result<String, NodeCoreError>
where
    F: FnMut() -> Result<String, NodeCoreError>,
{
    let mut device = String::new();
    let mut drive_relative = false;
    let mut absolute = false;
    let mut tails: Vec<String> = Vec::new();

    for idx in (-1..parts.len() as isize).rev() {
        let candidate = if idx == -1 {
            get_cwd()?
        } else {
            parts[idx as usize].to_owned()
        };
        if candidate.is_empty() {
            continue;
        }
        let root = resolution_root_win32(&candidate)?;
        if !root.device.is_empty() {
            if device.is_empty() {
                device = root.device;
                drive_relative = !root.absolute;
            } else if !device.eq_ignore_ascii_case(&root.device) {
                if drive_relative && device.eq_ignore_ascii_case(&root.drive) {
                    device = root.device;
                } else {
                    continue;
                }
            }
        }
        if !absolute {
            tails.push(candidate[root.end..].to_owned());
            absolute = root.absolute;
        }
        if absolute && !device.is_empty() {
            break;
        }
    }

    if !absolute || device.is_empty() {
        return Err(NodeCoreError::InvalidPath {
            reason: "Win32 resolve requires an absolute base with a drive or share root",
        });
    }

    Ok(format!(
        "{device}\\{}",
        normalized_tail(tails, '\\', is_win32_sep)
    ))
}

fn relative_tail(from: &str, to: &str, separator: char, fold_case: bool) -> String {
    let sep_str = if separator == '/' { "/" } else { "\\" };
    let source: Vec<&str> = if from.is_empty() {
        Vec::new()
    } else {
        from.split(separator).collect()
    };
    let target: Vec<&str> = if to.is_empty() {
        Vec::new()
    } else {
        to.split(separator).collect()
    };

    let mut common = 0;
    while common < source.len() && common < target.len() {
        let same = if fold_case {
            source[common].eq_ignore_ascii_case(target[common])
        } else {
            source[common] == target[common]
        };
        if !same {
            break;
        }
        common += 1;
    }

    let mut out: Vec<&str> = Vec::with_capacity((source.len() - common) + (target.len() - common));
    out.resize(source.len() - common, "..");
    out.extend_from_slice(&target[common..]);

    let joined = out.join(sep_str);
    let bytes = joined.as_bytes();
    if fold_case && bytes.len() >= 2 && is_device_root(bytes[0]) && bytes[1] == b':' {
        format!(".\\{joined}")
    } else {
        joined
    }
}

fn native_cwd_string() -> Result<String, NodeCoreError> {
    let dir = std::env::current_dir().map_err(|_| NodeCoreError::Unavailable {
        reason: "current working directory is unavailable",
    })?;
    dir.into_os_string()
        .into_string()
        .map_err(|_| NodeCoreError::InvalidUtf8 {
            reason: "current working directory is not valid UTF-8",
        })
}

fn relative_posix(from: &str, to: &str) -> Result<String, NodeCoreError> {
    let from_abs = !from.is_empty() && is_posix_sep(from.as_bytes()[0]);
    let to_abs = !to.is_empty() && is_posix_sep(to.as_bytes()[0]);

    let mut cached_cwd: Option<String> = None;
    let mut read_cwd = || -> Result<String, NodeCoreError> {
        if let Some(ref c) = cached_cwd {
            return Ok(c.clone());
        }
        let base = if Style::native() == Style::Posix {
            native_cwd_string()?
        } else if !from_abs && !to_abs {
            "/".to_owned()
        } else {
            return Err(NodeCoreError::InvalidPath {
                reason: "non-native POSIX relative requires both paths to be absolute or both relative",
            });
        };
        cached_cwd = Some(base.clone());
        Ok(base)
    };

    let source = resolve_posix(&[from], &mut read_cwd)?;
    let target = resolve_posix(&[to], &mut read_cwd)?;
    Ok(relative_tail(&source[1..], &target[1..], '/', false))
}

fn relative_win32(from: &str, to: &str) -> Result<String, NodeCoreError> {
    let from_root = resolution_root_win32(from)?;
    let to_root = resolution_root_win32(to)?;
    let both_plain_relative = !from_root.absolute
        && from_root.device.is_empty()
        && !to_root.absolute
        && to_root.device.is_empty();

    let mut cached_cwd: Option<String> = None;
    let mut read_cwd = || -> Result<String, NodeCoreError> {
        if let Some(ref c) = cached_cwd {
            return Ok(c.clone());
        }
        let base = if Style::native() == Style::Win32 {
            native_cwd_string()?
        } else if both_plain_relative {
            "C:\\".to_owned()
        } else {
            return Err(NodeCoreError::InvalidPath {
                reason: "non-native Win32 relative requires absolute drive/UNC endpoints or plain relative paths",
            });
        };
        cached_cwd = Some(base.clone());
        Ok(base)
    };

    let source = resolve_win32(&[from], &mut read_cwd)?;
    let target = resolve_win32(&[to], &mut read_cwd)?;
    let source_root = resolution_root_win32(&source)?;
    let target_root = resolution_root_win32(&target)?;

    if !source_root.device.eq_ignore_ascii_case(&target_root.device) {
        return Ok(target);
    }

    let source_start = skip_win32_seps(source.as_bytes(), source_root.end);
    let target_start = skip_win32_seps(target.as_bytes(), target_root.end);
    Ok(relative_tail(
        &source[source_start..],
        &target[target_start..],
        '\\',
        true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::NodeCoreErrorCode;

    #[test]
    fn posix_edge_cases_match_node() {
        assert_eq!(join(Style::Posix, &["/a", "b", "../c"]).unwrap(), "/a/c");
        assert_eq!(join(Style::Posix, &["", ""]).unwrap(), ".");
        assert_eq!(normalize(Style::Posix, "/a/b/..///c/").unwrap(), "/a/c/");
        assert_eq!(normalize(Style::Posix, "../a/../b").unwrap(), "../b");
        assert_eq!(dirname(Style::Posix, "/a/b/").unwrap(), "/a");
        assert_eq!(dirname(Style::Posix, "/").unwrap(), "/");
        assert_eq!(dirname(Style::Posix, "a").unwrap(), ".");
        assert_eq!(
            basename(Style::Posix, "/a/b.txt", Some(".txt")).unwrap(),
            "b"
        );
        assert_eq!(
            basename(Style::Posix, "/a/b.txt", Some("b.txt")).unwrap(),
            "b.txt"
        );
        assert_eq!(extname(Style::Posix, ".index.html").unwrap(), ".html");
        assert_eq!(extname(Style::Posix, "index.").unwrap(), ".");
        assert_eq!(extname(Style::Posix, "..").unwrap(), "");
        assert_eq!(extname(Style::Posix, "/..").unwrap(), "");
        assert_eq!(
            parse(Style::Posix, "/..").unwrap(),
            ParsedPath {
                root: "/".to_owned(),
                dir: "/".to_owned(),
                base: "..".to_owned(),
                ext: String::new(),
                name: "..".to_owned(),
            }
        );
        assert_eq!(
            parse(Style::Posix, "/../").unwrap(),
            ParsedPath {
                root: "/".to_owned(),
                dir: "/".to_owned(),
                base: "..".to_owned(),
                ext: String::new(),
                name: "..".to_owned(),
            }
        );
        assert_eq!(relative(Style::Posix, "/a/b", "/a/b").unwrap(), "");
    }

    #[test]
    fn win32_edge_cases_match_node() {
        assert_eq!(
            join(Style::Win32, &["C:\\a", "b", "..\\c"]).unwrap(),
            "C:\\a\\c"
        );
        assert_eq!(
            normalize(Style::Win32, "\\\\server\\share\\a\\..\\b").unwrap(),
            "\\\\server\\share\\b"
        );
        assert_eq!(
            basename(Style::Win32, "\\\\server\\share.dir", None).unwrap(),
            ""
        );
        assert_eq!(extname(Style::Win32, "\\\\server\\share.dir").unwrap(), "");
        assert_eq!(
            dirname(Style::Win32, "\\\\server\\share\\").unwrap(),
            "\\\\server\\share\\"
        );
        assert_eq!(
            relative(Style::Win32, "C:\\a\\b", "D:\\a\\b").unwrap(),
            "D:\\a\\b"
        );
        assert_eq!(
            relative(Style::Win32, "foo\\bar", "foo\\baz").unwrap(),
            "..\\baz"
        );
        assert!(is_absolute(Style::Win32, "C:/foo").unwrap());
        assert!(!is_absolute(Style::Win32, "C:foo").unwrap());
    }

    #[test]
    fn enforces_nul_and_size_bounds() {
        assert_eq!(
            normalize(Style::Posix, "a\0b").unwrap_err().code(),
            NodeCoreErrorCode::InvalidPath
        );
        let huge = "a".repeat(MAX_SINGLE_PATH_BYTES + 1);
        assert_eq!(
            normalize(Style::Posix, &huge).unwrap_err().code(),
            NodeCoreErrorCode::InputTooLarge
        );
        let chunk = "a".repeat(MAX_SINGLE_PATH_BYTES);
        assert_eq!(
            join(Style::Posix, &[&chunk, &chunk, &chunk, &chunk, "x"])
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InputTooLarge
        );
    }
}

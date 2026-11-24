//! Pure-Rust Node `path` semantics for the opt-in Node layer.
//!
//! This module is engine-neutral. The V8 and QuickJS adapters validate guest
//! values, call these functions, and build native result objects. There is no
//! JavaScript path implementation and no engine-specific path logic.
//!
//! The implemented subset is deliberately finite:
//!
//! * `basename`, `dirname`, `extname`, `format`, `isAbsolute`, `join`,
//!   `normalize`, `parse`, `relative`, `resolve`, `toNamespacedPath`;
//! * `sep` and `delimiter`;
//! * `posix` and `win32` views with the same functions and constants.
//!
//! `matchesGlob` is not implemented and the adapters expose it as an explicit
//! `ERR_NODE_UNSUPPORTED` function. `_makeLong` is the deprecated alias for
//! `toNamespacedPath` and is implemented as that alias.
//!
//! The algorithms follow Node's `path` rules for the common POSIX and Win32
//! forms, including drive-relative, UNC, device-root, `.`/`..`, trailing
//! separator, and Windows reserved-device-name handling. They do not claim
//! full Node parity for every historical device-path spelling. Inputs are
//! UTF-8 strings; the adapters reject non-string values and strings above the
//! shared input bound before calling this module.

/// Maximum UTF-8 bytes accepted for one path argument.
pub const MAX_PATH_INPUT_BYTES: usize = 1_048_576;

/// The path flavor selected by a Node `path` view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathFlavor {
    Posix,
    Win32,
}

impl PathFlavor {
    /// The host default flavor, matching Node's platform default.
    pub const fn host() -> Self {
        if cfg!(windows) {
            Self::Win32
        } else {
            Self::Posix
        }
    }

    /// The separator string for this flavor.
    pub const fn sep(self) -> &'static str {
        match self {
            Self::Posix => "/",
            Self::Win32 => "\\",
        }
    }

    /// The path-list delimiter for this flavor.
    pub const fn delimiter(self) -> &'static str {
        match self {
            Self::Posix => ":",
            Self::Win32 => ";",
        }
    }
}

/// A parsed Node path object.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedPath {
    pub root: String,
    pub dir: String,
    pub base: String,
    pub ext: String,
    pub name: String,
}

/// The string fields accepted by `path.format`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathObject<'a> {
    pub root: Option<&'a str>,
    pub dir: Option<&'a str>,
    pub base: Option<&'a str>,
    pub ext: Option<&'a str>,
    pub name: Option<&'a str>,
}

/// Returns the host `path` flavor.
pub const fn host_flavor() -> PathFlavor {
    PathFlavor::host()
}

/// Returns the host working directory as a string.
///
/// `path.resolve` and `toNamespacedPath` use this the way Node uses
/// `process.cwd()`. The adapters do not read an environment variable or guest
/// value for it.
pub fn host_cwd() -> String {
    std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_owned())
}

/// Normalizes a path.
pub fn normalize(flavor: PathFlavor, path: &str) -> String {
    match flavor {
        PathFlavor::Posix => normalize_posix(path),
        PathFlavor::Win32 => normalize_win32(path),
    }
}

/// Returns whether a path is absolute for the flavor.
pub fn is_absolute(flavor: PathFlavor, path: &str) -> bool {
    match flavor {
        PathFlavor::Posix => path.starts_with('/'),
        PathFlavor::Win32 => is_absolute_win32(path),
    }
}

/// Joins path segments.
pub fn join(flavor: PathFlavor, paths: &[&str]) -> String {
    match flavor {
        PathFlavor::Posix => join_posix(paths),
        PathFlavor::Win32 => join_win32(paths),
    }
}

/// Resolves path segments against `cwd`.
pub fn resolve(flavor: PathFlavor, cwd: &str, paths: &[&str]) -> String {
    match flavor {
        PathFlavor::Posix => resolve_posix(cwd, paths),
        PathFlavor::Win32 => resolve_win32(cwd, paths),
    }
}

/// Returns a relative path from `from` to `to`, resolved against `cwd`.
pub fn relative(flavor: PathFlavor, cwd: &str, from: &str, to: &str) -> String {
    match flavor {
        PathFlavor::Posix => relative_posix(cwd, from, to),
        PathFlavor::Win32 => relative_win32(cwd, from, to),
    }
}

/// Returns the directory portion of a path.
pub fn dirname(flavor: PathFlavor, path: &str) -> String {
    match flavor {
        PathFlavor::Posix => dirname_posix(path),
        PathFlavor::Win32 => dirname_win32(path),
    }
}

/// Returns the final component of a path, optionally removing `suffix`.
pub fn basename(flavor: PathFlavor, path: &str, suffix: Option<&str>) -> String {
    match flavor {
        PathFlavor::Posix => basename_posix(path, suffix),
        PathFlavor::Win32 => basename_win32(path, suffix),
    }
}

/// Returns the extension of the final component.
pub fn extname(flavor: PathFlavor, path: &str) -> String {
    match flavor {
        PathFlavor::Posix => extname_posix(path),
        PathFlavor::Win32 => extname_win32(path),
    }
}

/// Parses a path into root, dir, base, ext, and name.
pub fn parse(flavor: PathFlavor, path: &str) -> ParsedPath {
    match flavor {
        PathFlavor::Posix => parse_posix(path),
        PathFlavor::Win32 => parse_win32(path),
    }
}

/// Formats a path object.
pub fn format(flavor: PathFlavor, object: &PathObject<'_>) -> String {
    let sep = flavor.sep();
    let base = object
        .base
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{}{}",
                object.name.unwrap_or_default(),
                object.ext.unwrap_or_default()
            )
        });
    if let Some(dir) = object.dir.filter(|value| !value.is_empty()) {
        return format!("{dir}{sep}{base}");
    }
    if let Some(root) = object.root.filter(|value| !value.is_empty()) {
        return format!("{root}{base}");
    }
    base
}

/// Returns a namespaced path for the flavor.
///
/// POSIX returns the input unchanged. Win32 follows Node's `\\?\` and
/// `\\?\UNC\` transformation for the resolved path.
pub fn to_namespaced_path(flavor: PathFlavor, cwd: &str, path: &str) -> String {
    match flavor {
        PathFlavor::Posix => path.to_owned(),
        PathFlavor::Win32 => to_namespaced_path_win32(cwd, path),
    }
}

fn chars(path: &str) -> Vec<char> {
    path.chars().collect()
}

fn string(chars: &[char]) -> String {
    chars.iter().collect()
}

fn is_posix_sep(character: char) -> bool {
    character == '/'
}

fn is_win_sep(character: char) -> bool {
    character == '/' || character == '\\'
}

fn is_drive_root(character: char) -> bool {
    character.is_ascii_alphabetic()
}

fn is_windows_reserved_name(path: &[char], colon_index: isize) -> bool {
    if colon_index < 0 {
        return false;
    }
    let end = colon_index as usize;
    if end == 0 {
        return false;
    }
    let start = path[..end]
        .iter()
        .rposition(|&character| is_win_sep(character))
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut name: String = path[start..end].iter().collect();
    name = name.trim_end_matches([' ', '.']).to_ascii_uppercase();
    if name.is_empty() {
        return false;
    }
    matches!(
        name.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn normalize_string(
    path: &[char],
    allow_above_root: bool,
    separator: char,
    is_separator: impl Fn(char) -> bool,
) -> String {
    let mut result: Vec<char> = Vec::new();
    let mut last_segment_length = 0usize;
    let mut last_slash: isize = -1;
    let mut dots = 0i32;
    let mut code = '\0';

    for index in 0..=path.len() {
        if index < path.len() {
            code = path[index];
        } else if is_separator(code) {
            break;
        } else {
            code = '/';
        }

        if is_separator(code) {
            if last_slash == index as isize - 1 || dots == 1 {
                // A separator immediately after another separator or a single
                // dot contributes nothing.
            } else if dots == 2 {
                let length = result.len();
                let is_dot_dot = length >= 2
                    && last_segment_length == 2
                    && result[length - 1] == '.'
                    && result[length - 2] == '.';
                if !is_dot_dot {
                    if length > 2 {
                        if let Some(last_separator) =
                            result.iter().rposition(|&character| character == separator)
                        {
                            result.truncate(last_separator);
                            last_segment_length = result.len().saturating_sub(
                                result
                                    .iter()
                                    .rposition(|&character| character == separator)
                                    .map(|position| position + 1)
                                    .unwrap_or(0),
                            );
                        } else {
                            result.clear();
                            last_segment_length = 0;
                        }
                        last_slash = index as isize;
                        dots = 0;
                        continue;
                    }
                    if length == 1 || length == 2 {
                        result.clear();
                        last_segment_length = 0;
                        last_slash = index as isize;
                        dots = 0;
                        continue;
                    }
                }
                if allow_above_root {
                    if !result.is_empty() {
                        result.push(separator);
                    }
                    result.push('.');
                    result.push('.');
                    last_segment_length = 2;
                }
            } else {
                if !result.is_empty() {
                    result.push(separator);
                }
                let start = if last_slash < 0 {
                    0
                } else {
                    (last_slash + 1) as usize
                };
                result.extend_from_slice(&path[start..index]);
                last_segment_length = index - start;
            }
            last_slash = index as isize;
            dots = 0;
        } else if code == '.' && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
    }

    string(&result)
}

fn normalize_posix(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let value = chars(path);
    let is_absolute = value[0] == '/';
    let trailing_separator = value.last() == Some(&'/');
    let normalized = normalize_string(&value, !is_absolute, '/', is_posix_sep);
    if normalized.is_empty() {
        if is_absolute {
            return "/".to_owned();
        }
        return if trailing_separator {
            "./".to_owned()
        } else {
            ".".to_owned()
        };
    }
    let mut result = if is_absolute {
        format!("/{normalized}")
    } else {
        normalized
    };
    if trailing_separator {
        result.push('/');
    }
    result
}

fn join_posix(paths: &[&str]) -> String {
    let mut joined = String::new();
    for path in paths.iter().filter(|path| !path.is_empty()) {
        if !joined.is_empty() {
            joined.push('/');
        }
        joined.push_str(path);
    }
    if joined.is_empty() {
        ".".to_owned()
    } else {
        normalize_posix(&joined)
    }
}

fn resolve_posix(cwd: &str, paths: &[&str]) -> String {
    let mut resolved = String::new();
    let mut absolute = false;
    for path in paths.iter().rev() {
        if path.is_empty() {
            continue;
        }
        if resolved.is_empty() {
            resolved.push_str(path);
        } else {
            resolved = format!("{path}/{resolved}");
        }
        if path.starts_with('/') {
            absolute = true;
            break;
        }
    }
    if !absolute {
        if resolved.is_empty() {
            resolved.push_str(cwd);
        } else {
            resolved = format!("{cwd}/{resolved}");
        }
        absolute = cwd.starts_with('/');
    }
    let normalized = normalize_string(&chars(&resolved), !absolute, '/', is_posix_sep);
    if absolute {
        format!("/{normalized}")
    } else if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    }
}

fn relative_posix(cwd: &str, from: &str, to: &str) -> String {
    if from == to {
        return String::new();
    }
    let from = resolve_posix(cwd, &[from]);
    let to = resolve_posix(cwd, &[to]);
    if from == to {
        return String::new();
    }
    let from_value = chars(&from);
    let to_value = chars(&to);
    let from_start = 1usize;
    let from_end = from_value.len();
    let to_start = 1usize;
    let to_end = to_value.len();
    let from_len = from_end.saturating_sub(from_start);
    let to_len = to_end.saturating_sub(to_start);
    let length = from_len.min(to_len);
    let mut last_common_separator: isize = -1;
    let mut index = 0usize;
    while index < length {
        let from_code = from_value[from_start + index];
        if from_code != to_value[to_start + index] {
            break;
        }
        if from_code == '/' {
            last_common_separator = index as isize;
        }
        index += 1;
    }
    if index == length {
        if to_len > length {
            if to_value.get(to_start + index) == Some(&'/') {
                return string(&to_value[to_start + index + 1..to_end]);
            }
            if index == 0 {
                return string(&to_value[to_start + index..to_end]);
            }
        } else if from_len > length {
            if from_value.get(from_start + index) == Some(&'/') {
                last_common_separator = index as isize;
            } else if index == 0 {
                last_common_separator = 0;
            }
        }
    }
    let mut output = String::new();
    let mut position = from_start + last_common_separator.max(0) as usize + 1;
    while position <= from_end {
        if position == from_end || from_value[position] == '/' {
            if output.is_empty() {
                output.push_str("..");
            } else {
                output.push_str("/..");
            }
        }
        position += 1;
    }
    let tail_start = to_start + last_common_separator.max(0) as usize;
    output.push_str(&string(&to_value[tail_start..to_end]));
    output
}

fn dirname_posix(path: &str) -> String {
    let value = chars(path);
    if value.is_empty() {
        return ".".to_owned();
    }
    let has_root = value[0] == '/';
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut index = value.len() as isize - 1;
    while index >= 1 {
        if value[index as usize] == '/' {
            if !matched_slash {
                end = index;
                break;
            }
        } else {
            matched_slash = false;
        }
        index -= 1;
    }
    if end == -1 {
        return if has_root { "/" } else { "." }.to_owned();
    }
    if has_root && end == 1 {
        return "//".to_owned();
    }
    string(&value[..end as usize])
}

fn basename_posix(path: &str, suffix: Option<&str>) -> String {
    basename_with(path, suffix, is_posix_sep, 0)
}

fn extname_posix(path: &str) -> String {
    extname_with(path, is_posix_sep, 0)
}

fn parse_posix(path: &str) -> ParsedPath {
    parse_with(path, is_posix_sep, true, 0)
}

fn is_absolute_win32(path: &str) -> bool {
    let value = chars(path);
    if value.is_empty() {
        return false;
    }
    is_win_sep(value[0])
        || (value.len() > 2 && is_drive_root(value[0]) && value[1] == ':' && is_win_sep(value[2]))
}

fn normalize_win32(path: &str) -> String {
    let value = chars(path);
    let length = value.len();
    if length == 0 {
        return ".".to_owned();
    }
    let mut root_end = 0usize;
    let mut device: Option<String> = None;
    let mut absolute = false;
    let first = value[0];
    if length == 1 {
        return if is_win_sep(first) {
            "\\".to_owned()
        } else {
            path.to_owned()
        };
    }
    if is_win_sep(first) {
        absolute = true;
        if is_win_sep(value[1]) {
            let mut index = 2usize;
            let mut last = index;
            while index < length && !is_win_sep(value[index]) {
                index += 1;
            }
            if index < length && index != last {
                let first_part = string(&value[last..index]);
                last = index;
                while index < length && is_win_sep(value[index]) {
                    index += 1;
                }
                if index < length && index != last {
                    last = index;
                    while index < length && !is_win_sep(value[index]) {
                        index += 1;
                    }
                    if index == length || index != last {
                        if first_part == "." || first_part == "?" {
                            device = Some(format!("\\\\{first_part}"));
                            root_end = 4;
                            let colon_index = value.iter().position(|&character| character == ':');
                            if let Some(colon_index) = colon_index {
                                let possible_device = string(&value[4..colon_index + 1]);
                                if is_windows_reserved_name(
                                    &chars(&possible_device),
                                    possible_device.chars().count() as isize - 1,
                                ) {
                                    device = Some(format!("\\\\?\\{possible_device}"));
                                    root_end = 4 + possible_device.chars().count();
                                }
                            }
                        } else if index == length {
                            return format!("\\\\{first_part}\\{}\\", string(&value[last..]));
                        } else {
                            device =
                                Some(format!("\\\\{first_part}\\{}", string(&value[last..index])));
                            root_end = index;
                        }
                    }
                }
            }
        } else {
            root_end = 1;
        }
    } else {
        let colon_index = value.iter().position(|&character| character == ':');
        if let Some(colon_index) = colon_index
            && colon_index > 0
        {
            if is_drive_root(first) && colon_index == 1 {
                device = Some(string(&value[..2]));
                root_end = 2;
                if length > 2 && is_win_sep(value[2]) {
                    absolute = true;
                    root_end = 3;
                }
            } else if is_windows_reserved_name(&value, colon_index as isize) {
                device = Some(string(&value[..colon_index + 1]));
                root_end = colon_index + 1;
            }
        }
    }

    let tail = if root_end < length {
        normalize_string(&value[root_end..], !absolute, '\\', is_win_sep)
    } else {
        String::new()
    };
    let mut tail = tail;
    if tail.is_empty() && !absolute {
        tail = ".".to_owned();
    }
    if !tail.is_empty() && is_win_sep(value[length - 1]) {
        tail.push('\\');
    }
    if !absolute && device.is_none() && path.contains(':') {
        if tail.len() >= 2
            && is_drive_root(tail.chars().next().unwrap_or('\0'))
            && tail.chars().nth(1) == Some(':')
        {
            return format!(".\\{tail}");
        }
        let mut search = path.find(':');
        while let Some(index) = search {
            if index + 1 == path.len() || path.chars().nth(index + 1).is_some_and(is_win_sep) {
                return format!(".\\{tail}");
            }
            search = path[index + 1..].find(':').map(|next| index + 1 + next);
        }
    }
    let colon_index = value.iter().position(|&character| character == ':');
    if is_windows_reserved_name(
        &value,
        colon_index.map(|index| index as isize).unwrap_or(-1),
    ) {
        return format!(".\\{}{tail}", device.unwrap_or_default());
    }
    match device {
        None => {
            if absolute {
                format!("\\{tail}")
            } else {
                tail
            }
        }
        Some(device) => {
            if absolute {
                format!("{device}\\{tail}")
            } else {
                format!("{device}{tail}")
            }
        }
    }
}

fn join_win32(paths: &[&str]) -> String {
    let parts: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|path| !path.is_empty())
        .collect();
    if parts.is_empty() {
        return ".".to_owned();
    }
    let first = parts[0];
    let mut joined = parts.join("\\");
    let mut needs_replace = true;
    let mut slash_count = 0usize;
    let first_chars = chars(first);
    if first_chars
        .first()
        .is_some_and(|&character| is_win_sep(character))
    {
        slash_count += 1;
        if first_chars.len() > 1 && is_win_sep(first_chars[1]) {
            slash_count += 1;
            if first_chars.len() > 2 {
                if is_win_sep(first_chars[2]) {
                    slash_count += 1;
                } else {
                    needs_replace = false;
                }
            }
        }
    }
    if needs_replace {
        let joined_chars = chars(&joined);
        while slash_count < joined_chars.len() && is_win_sep(joined_chars[slash_count]) {
            slash_count += 1;
        }
        if slash_count >= 2 {
            joined = format!("\\{}", string(&joined_chars[slash_count..]));
        }
    }
    let joined_chars = chars(&joined);
    let mut components: Vec<String> = Vec::new();
    let mut component = String::new();
    let mut index = 0usize;
    while index < joined_chars.len() {
        if joined_chars[index] == '\\' {
            if !component.is_empty() {
                components.push(component.clone());
                component.clear();
            }
            while index + 1 < joined_chars.len() && joined_chars[index + 1] == '\\' {
                index += 1;
            }
        } else {
            component.push(joined_chars[index]);
        }
        index += 1;
    }
    if !component.is_empty() {
        components.push(component);
    }
    if components.iter().any(|component| {
        component
            .find(':')
            .is_some_and(|colon| is_windows_reserved_name(&chars(component), colon as isize))
    }) {
        return joined
            .chars()
            .map(|character| if character == '/' { '\\' } else { character })
            .collect();
    }
    normalize_win32(&joined)
}

fn resolve_win32(cwd: &str, paths: &[&str]) -> String {
    let mut resolved_device = String::new();
    let mut resolved_tail = String::new();
    let mut resolved_absolute = false;
    let mut index = paths.len() as isize - 1;
    while index >= -1 {
        let path_owned;
        let path: &str = if index >= 0 {
            let value = paths[index as usize];
            if value.is_empty() {
                index -= 1;
                continue;
            }
            value
        } else if resolved_device.is_empty() {
            cwd
        } else {
            let device = resolved_device.to_ascii_lowercase();
            let cwd_chars = chars(cwd);
            if cwd_chars.len() >= 3
                && is_drive_root(cwd_chars[0])
                && cwd_chars[1] == ':'
                && cwd_chars[2] == '\\'
                && cwd_chars[..2]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase()
                    != device
            {
                path_owned = format!("{resolved_device}\\");
                &path_owned
            } else {
                cwd
            }
        };
        let value = chars(path);
        let length = value.len();
        let mut root_end = 0usize;
        let mut device = String::new();
        let mut absolute = false;
        if length == 1 {
            if is_win_sep(value[0]) {
                root_end = 1;
                absolute = true;
            }
        } else if is_win_sep(value[0]) {
            absolute = true;
            if is_win_sep(value[1]) {
                let mut position = 2usize;
                let mut last = position;
                while position < length && !is_win_sep(value[position]) {
                    position += 1;
                }
                if position < length && position != last {
                    let first_part = string(&value[last..position]);
                    last = position;
                    while position < length && is_win_sep(value[position]) {
                        position += 1;
                    }
                    if position < length && position != last {
                        last = position;
                        while position < length && !is_win_sep(value[position]) {
                            position += 1;
                        }
                        if position == length || position != last {
                            if first_part != "." && first_part != "?" {
                                device =
                                    format!("\\\\{first_part}\\{}", string(&value[last..position]));
                                root_end = position;
                            } else {
                                device = format!("\\\\{first_part}");
                                root_end = 4;
                            }
                        }
                    }
                }
            } else {
                root_end = 1;
            }
        } else if length > 2 && is_drive_root(value[0]) && value[1] == ':' {
            device = string(&value[..2]);
            root_end = 2;
            if is_win_sep(value[2]) {
                absolute = true;
                root_end = 3;
            }
        }

        if !device.is_empty() {
            if !resolved_device.is_empty() {
                if !device.eq_ignore_ascii_case(&resolved_device) {
                    index -= 1;
                    continue;
                }
            } else {
                resolved_device = device;
            }
        }

        if resolved_absolute {
            if !resolved_device.is_empty() {
                break;
            }
        } else {
            resolved_tail = format!("{}\\{resolved_tail}", string(&value[root_end..]));
            resolved_absolute = absolute;
            if absolute && !resolved_device.is_empty() {
                break;
            }
        }
        index -= 1;
    }

    let tail = normalize_string(&chars(&resolved_tail), !resolved_absolute, '\\', is_win_sep);
    if resolved_absolute {
        format!("{resolved_device}\\{tail}")
    } else {
        let result = format!("{resolved_device}{tail}");
        if result.is_empty() {
            ".".to_owned()
        } else {
            result
        }
    }
}

fn relative_win32(cwd: &str, from: &str, to: &str) -> String {
    if from == to {
        return String::new();
    }
    let from_original = resolve_win32(cwd, &[from]);
    let to_original = resolve_win32(cwd, &[to]);
    if from_original == to_original {
        return String::new();
    }
    let from_lower = from_original.to_ascii_lowercase();
    let to_lower = to_original.to_ascii_lowercase();
    if from_lower == to_lower {
        return String::new();
    }
    if from_original.chars().count() != from_lower.chars().count()
        || to_original.chars().count() != to_lower.chars().count()
    {
        let mut from_split: Vec<String> = from_original.split('\\').map(str::to_owned).collect();
        let mut to_split: Vec<String> = to_original.split('\\').map(str::to_owned).collect();
        if from_split.last().is_some_and(|value| value.is_empty()) {
            from_split.pop();
        }
        if to_split.last().is_some_and(|value| value.is_empty()) {
            to_split.pop();
        }
        let from_len = from_split.len();
        let to_len = to_split.len();
        let length = from_len.min(to_len);
        let mut index = 0usize;
        while index < length && from_split[index].eq_ignore_ascii_case(&to_split[index]) {
            index += 1;
        }
        if index == 0 {
            return to_original;
        }
        if index == length {
            if to_len > length {
                return to_split[index..].join("\\");
            }
            if from_len > length {
                return format!("{}{}", "..\\".repeat(from_len - 1 - index), "..");
            }
            return String::new();
        }
        return format!(
            "{}{}",
            "..\\".repeat(from_len - index),
            to_split[index..].join("\\")
        );
    }

    let mut from_start = 0usize;
    let from_value = chars(&from_lower);
    while from_start < from_value.len() && from_value[from_start] == '\\' {
        from_start += 1;
    }
    let mut from_end = from_value.len();
    while from_end > from_start + 1 && from_value[from_end - 1] == '\\' {
        from_end -= 1;
    }
    let from_len = from_end - from_start;
    let mut to_start = 0usize;
    let to_value = chars(&to_lower);
    while to_start < to_value.len() && to_value[to_start] == '\\' {
        to_start += 1;
    }
    let mut to_end = to_value.len();
    while to_end > to_start + 1 && to_value[to_end - 1] == '\\' {
        to_end -= 1;
    }
    let to_len = to_end - to_start;
    let length = from_len.min(to_len);
    let mut last_common_separator: isize = -1;
    let mut index = 0usize;
    while index < length {
        let from_code = from_value[from_start + index];
        if from_code != to_value[to_start + index] {
            break;
        }
        if from_code == '\\' {
            last_common_separator = index as isize;
        }
        index += 1;
    }
    if index != length {
        if last_common_separator == -1 {
            return to_original;
        }
    } else {
        if to_len > length {
            if to_value.get(to_start + index) == Some(&'\\') {
                return string(&to_value[to_start + index + 1..to_end]);
            }
            if index == 2 {
                return string(&to_value[to_start + index..to_end]);
            }
        }
        if from_len > length {
            if from_value.get(from_start + index) == Some(&'\\') {
                last_common_separator = index as isize;
            } else if index == 2 {
                last_common_separator = 3;
            }
        }
        if last_common_separator == -1 {
            last_common_separator = 0;
        }
    }
    let mut output = String::new();
    let mut position = from_start + last_common_separator.max(0) as usize + 1;
    while position <= from_end {
        if position == from_end || from_value[position] == '\\' {
            if output.is_empty() {
                output.push_str("..");
            } else {
                output.push_str("\\..");
            }
        }
        position += 1;
    }
    let to_original_value = chars(&to_original);
    let tail_start = to_start + last_common_separator.max(0) as usize;
    if !output.is_empty() {
        output.push_str(&string(&to_original_value[tail_start..to_end]));
        return output;
    }
    let mut start = tail_start;
    if to_original_value.get(start) == Some(&'\\') {
        start += 1;
    }
    string(&to_original_value[start..to_end])
}

fn dirname_win32(path: &str) -> String {
    let value = chars(path);
    let length = value.len();
    if length == 0 {
        return ".".to_owned();
    }
    let first = value[0];
    if length == 1 {
        return if is_win_sep(first) {
            path.to_owned()
        } else {
            ".".to_owned()
        };
    }
    let mut root_end: isize = -1;
    let mut offset = 0usize;
    if is_win_sep(first) {
        root_end = 1;
        offset = 1;
        if is_win_sep(value[1]) {
            let mut index = 2usize;
            let mut last = index;
            while index < length && !is_win_sep(value[index]) {
                index += 1;
            }
            if index < length && index != last {
                last = index;
                while index < length && is_win_sep(value[index]) {
                    index += 1;
                }
                if index < length && index != last {
                    last = index;
                    while index < length && !is_win_sep(value[index]) {
                        index += 1;
                    }
                    if index == length {
                        return path.to_owned();
                    }
                    if index != last {
                        root_end = index as isize + 1;
                        offset = index + 1;
                    }
                }
            }
        }
    } else if is_drive_root(first) && value.get(1) == Some(&':') {
        root_end = if length > 2 && is_win_sep(value[2]) {
            3
        } else {
            2
        };
        offset = root_end as usize;
    }
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut index = length as isize - 1;
    while index >= offset as isize {
        if is_win_sep(value[index as usize]) {
            if !matched_slash {
                end = index;
                break;
            }
        } else {
            matched_slash = false;
        }
        index -= 1;
    }
    if end == -1 {
        if root_end == -1 {
            return ".".to_owned();
        }
        end = root_end;
    }
    string(&value[..end as usize])
}

fn basename_win32(path: &str, suffix: Option<&str>) -> String {
    let value = chars(path);
    let start = if value.len() >= 2 && is_drive_root(value[0]) && value[1] == ':' {
        2
    } else {
        0
    };
    basename_with(path, suffix, is_win_sep, start)
}

fn extname_win32(path: &str) -> String {
    let value = chars(path);
    let start = if value.len() >= 2 && is_drive_root(value[0]) && value[1] == ':' {
        2
    } else {
        0
    };
    extname_with(path, is_win_sep, start)
}

fn parse_win32(path: &str) -> ParsedPath {
    let value = chars(path);
    let length = value.len();
    if length == 0 {
        return ParsedPath::default();
    }
    let mut result = ParsedPath::default();
    if length == 1 {
        if is_win_sep(value[0]) {
            result.root = path.to_owned();
            result.dir = path.to_owned();
        } else {
            result.base = path.to_owned();
            result.name = path.to_owned();
        }
        return result;
    }
    let mut root_end = 0usize;
    let first = value[0];
    if is_win_sep(first) {
        root_end = 1;
        if is_win_sep(value[1]) {
            let mut index = 2usize;
            let mut last = index;
            while index < length && !is_win_sep(value[index]) {
                index += 1;
            }
            if index < length && index != last {
                last = index;
                while index < length && is_win_sep(value[index]) {
                    index += 1;
                }
                if index < length && index != last {
                    last = index;
                    while index < length && !is_win_sep(value[index]) {
                        index += 1;
                    }
                    if index == length {
                        root_end = index;
                    } else if index != last {
                        root_end = index + 1;
                    }
                }
            }
        }
    } else if is_drive_root(first) && value.get(1) == Some(&':') {
        if length <= 2 {
            result.root = path.to_owned();
            result.dir = path.to_owned();
            return result;
        }
        root_end = 2;
        if is_win_sep(value[2]) {
            if length == 3 {
                result.root = path.to_owned();
                result.dir = path.to_owned();
                return result;
            }
            root_end = 3;
        }
    }
    if root_end > 0 {
        result.root = string(&value[..root_end]);
    }
    let mut start_dot: isize = -1;
    let mut start_part = root_end;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state = 0i32;
    let mut index = length as isize - 1;
    while index >= root_end as isize {
        let character = value[index as usize];
        if is_win_sep(character) {
            if !matched_slash {
                start_part = index as usize + 1;
                break;
            }
        } else {
            if end == -1 {
                matched_slash = false;
                end = index + 1;
            }
            if character == '.' {
                if start_dot == -1 {
                    start_dot = index;
                } else if pre_dot_state != 1 {
                    pre_dot_state = 1;
                }
            } else if start_dot != -1 {
                pre_dot_state = -1;
            }
        }
        index -= 1;
    }
    if end != -1 {
        if start_dot == -1
            || pre_dot_state == 0
            || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part as isize + 1)
        {
            result.base = string(&value[start_part..end as usize]);
            result.name = result.base.clone();
        } else {
            result.name = string(&value[start_part..start_dot as usize]);
            result.base = string(&value[start_part..end as usize]);
            result.ext = string(&value[start_dot as usize..end as usize]);
        }
    }
    if start_part > 0 && start_part != root_end {
        result.dir = string(&value[..start_part - 1]);
    } else {
        result.dir = result.root.clone();
    }
    result
}

fn basename_with(
    path: &str,
    suffix: Option<&str>,
    is_separator: impl Fn(char) -> bool,
    start_offset: usize,
) -> String {
    let value = chars(path);
    let mut start = start_offset;
    let mut end: isize = -1;
    let mut matched_slash = true;
    if let Some(suffix) = suffix {
        let suffix_value = chars(suffix);
        if !suffix_value.is_empty() && suffix_value.len() <= value.len() {
            if suffix_value == value {
                return String::new();
            }
            let mut ext_index = suffix_value.len() as isize - 1;
            let mut first_non_slash_end: isize = -1;
            let mut index = value.len() as isize - 1;
            while index >= start as isize {
                let character = value[index as usize];
                if is_separator(character) {
                    if !matched_slash {
                        start = index as usize + 1;
                        break;
                    }
                } else {
                    if first_non_slash_end == -1 {
                        matched_slash = false;
                        first_non_slash_end = index + 1;
                    }
                    if ext_index >= 0 {
                        if character == suffix_value[ext_index as usize] {
                            ext_index -= 1;
                            if ext_index == -1 {
                                end = index;
                            }
                        } else {
                            ext_index = -1;
                            end = first_non_slash_end;
                        }
                    }
                }
                index -= 1;
            }
            if start as isize == end {
                end = first_non_slash_end;
            } else if end == -1 {
                end = value.len() as isize;
            }
            return string(&value[start..end as usize]);
        }
    }
    let mut index = value.len() as isize - 1;
    while index >= start as isize {
        if is_separator(value[index as usize]) {
            if !matched_slash {
                start = index as usize + 1;
                break;
            }
        } else if end == -1 {
            matched_slash = false;
            end = index + 1;
        }
        index -= 1;
    }
    if end == -1 {
        return String::new();
    }
    string(&value[start..end as usize])
}

fn extname_with(path: &str, is_separator: impl Fn(char) -> bool, start_offset: usize) -> String {
    let value = chars(path);
    let start = start_offset;
    let mut start_dot: isize = -1;
    let mut start_part = start_offset;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state = 0i32;
    let mut index = value.len() as isize - 1;
    while index >= start as isize {
        let character = value[index as usize];
        if is_separator(character) {
            if !matched_slash {
                start_part = index as usize + 1;
                break;
            }
        } else {
            if end == -1 {
                matched_slash = false;
                end = index + 1;
            }
            if character == '.' {
                if start_dot == -1 {
                    start_dot = index;
                } else if pre_dot_state != 1 {
                    pre_dot_state = 1;
                }
            } else if start_dot != -1 {
                pre_dot_state = -1;
            }
        }
        index -= 1;
    }
    if start_dot == -1
        || end == -1
        || pre_dot_state == 0
        || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part as isize + 1)
    {
        String::new()
    } else {
        string(&value[start_dot as usize..end as usize])
    }
}

fn parse_with(
    path: &str,
    is_separator: impl Fn(char) -> bool,
    is_posix: bool,
    _start_offset: usize,
) -> ParsedPath {
    let value = chars(path);
    let mut result = ParsedPath::default();
    if value.is_empty() {
        return result;
    }
    let is_absolute = is_posix && value[0] == '/';
    let mut start = 0usize;
    if is_absolute {
        result.root = "/".to_owned();
        start = 1;
    }
    let mut start_dot: isize = -1;
    let mut start_part = 0usize;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state = 0i32;
    let mut index = value.len() as isize - 1;
    while index >= start as isize {
        let character = value[index as usize];
        if is_separator(character) {
            if !matched_slash {
                start_part = index as usize + 1;
                break;
            }
        } else {
            if end == -1 {
                matched_slash = false;
                end = index + 1;
            }
            if character == '.' {
                if start_dot == -1 {
                    start_dot = index;
                } else if pre_dot_state != 1 {
                    pre_dot_state = 1;
                }
            } else if start_dot != -1 {
                pre_dot_state = -1;
            }
        }
        index -= 1;
    }
    if end != -1 {
        let base_start = if start_part == 0 && is_absolute {
            1
        } else {
            start_part
        };
        if start_dot == -1
            || pre_dot_state == 0
            || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part as isize + 1)
        {
            result.base = string(&value[base_start..end as usize]);
            result.name = result.base.clone();
        } else {
            result.name = string(&value[base_start..start_dot as usize]);
            result.base = string(&value[base_start..end as usize]);
            result.ext = string(&value[start_dot as usize..end as usize]);
        }
    }
    if start_part > 0 {
        result.dir = string(&value[..start_part - 1]);
    } else if is_absolute {
        result.dir = "/".to_owned();
    }
    result
}

fn to_namespaced_path_win32(cwd: &str, path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let resolved = resolve_win32(cwd, &[path]);
    let value = chars(&resolved);
    if value.len() <= 2 {
        return path.to_owned();
    }
    if value[0] == '\\' && value[1] == '\\' {
        let third = value.get(2).copied().unwrap_or('\0');
        if third != '?' && third != '.' {
            return format!("\\\\?\\UNC\\{}", string(&value[2..]));
        }
    } else if value.len() >= 3 && is_drive_root(value[0]) && value[1] == ':' && value[2] == '\\' {
        return format!("\\\\?\\{resolved}");
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(root: &str, dir: &str, base: &str, ext: &str, name: &str) -> ParsedPath {
        ParsedPath {
            root: root.to_owned(),
            dir: dir.to_owned(),
            base: base.to_owned(),
            ext: ext.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn posix_vectors_match_node() {
        let cases = [
            ("", ".", ".", "", "", false, parsed("", "", "", "", "")),
            (".", ".", ".", ".", "", false, parsed("", "", ".", "", ".")),
            (
                "..",
                "..",
                ".",
                "..",
                "",
                false,
                parsed("", "", "..", "", ".."),
            ),
            ("/", "/", "/", "", "", true, parsed("/", "/", "", "", "")),
            ("//", "/", "/", "", "", true, parsed("/", "/", "", "", "")),
            (
                "/foo/bar",
                "/foo/bar",
                "/foo",
                "bar",
                "",
                true,
                parsed("/", "/foo", "bar", "", "bar"),
            ),
            (
                "/foo/bar/",
                "/foo/bar/",
                "/foo",
                "bar",
                "",
                true,
                parsed("/", "/foo", "bar", "", "bar"),
            ),
            (
                "foo/bar",
                "foo/bar",
                "foo",
                "bar",
                "",
                false,
                parsed("", "foo", "bar", "", "bar"),
            ),
            (
                "foo/bar/",
                "foo/bar/",
                "foo",
                "bar",
                "",
                false,
                parsed("", "foo", "bar", "", "bar"),
            ),
            (
                "foo//bar",
                "foo/bar",
                "foo/",
                "bar",
                "",
                false,
                parsed("", "foo/", "bar", "", "bar"),
            ),
            (
                "foo/./bar",
                "foo/bar",
                "foo/.",
                "bar",
                "",
                false,
                parsed("", "foo/.", "bar", "", "bar"),
            ),
            (
                "foo/../bar",
                "bar",
                "foo/..",
                "bar",
                "",
                false,
                parsed("", "foo/..", "bar", "", "bar"),
            ),
            (
                "../foo",
                "../foo",
                "..",
                "foo",
                "",
                false,
                parsed("", "..", "foo", "", "foo"),
            ),
            (
                "../../foo",
                "../../foo",
                "../..",
                "foo",
                "",
                false,
                parsed("", "../..", "foo", "", "foo"),
            ),
            (
                "foo/../../bar",
                "../bar",
                "foo/../..",
                "bar",
                "",
                false,
                parsed("", "foo/../..", "bar", "", "bar"),
            ),
            (
                "/foo/../..",
                "/",
                "/foo/..",
                "..",
                "",
                true,
                parsed("/", "/foo/..", "..", "", ".."),
            ),
            (
                "/../foo",
                "/foo",
                "/..",
                "foo",
                "",
                true,
                parsed("/", "/..", "foo", "", "foo"),
            ),
            (
                "a/b/c/..",
                "a/b",
                "a/b/c",
                "..",
                "",
                false,
                parsed("", "a/b/c", "..", "", ".."),
            ),
            (
                "a/b/c/../",
                "a/b/",
                "a/b/c",
                "..",
                "",
                false,
                parsed("", "a/b/c", "..", "", ".."),
            ),
            (
                "C:\\foo\\bar",
                "C:\\foo\\bar",
                ".",
                "C:\\foo\\bar",
                "",
                false,
                parsed("", "", "C:\\foo\\bar", "", "C:\\foo\\bar"),
            ),
            (
                "C:/foo/bar",
                "C:/foo/bar",
                "C:/foo",
                "bar",
                "",
                false,
                parsed("", "C:/foo", "bar", "", "bar"),
            ),
            (
                "/foo\\bar",
                "/foo\\bar",
                "/",
                "foo\\bar",
                "",
                true,
                parsed("/", "/", "foo\\bar", "", "foo\\bar"),
            ),
        ];
        for (path, norm, dir, base, ext, absolute, parse_expected) in cases {
            assert_eq!(
                normalize(PathFlavor::Posix, path),
                norm,
                "normalize {path:?}"
            );
            assert_eq!(dirname(PathFlavor::Posix, path), dir, "dirname {path:?}");
            assert_eq!(
                basename(PathFlavor::Posix, path, None),
                base,
                "basename {path:?}"
            );
            assert_eq!(extname(PathFlavor::Posix, path), ext, "extname {path:?}");
            assert_eq!(
                is_absolute(PathFlavor::Posix, path),
                absolute,
                "isAbsolute {path:?}"
            );
            assert_eq!(
                parse(PathFlavor::Posix, path),
                parse_expected,
                "parse {path:?}"
            );
        }
    }

    #[test]
    fn win32_vectors_match_node() {
        let cases = [
            ("", ".", ".", "", "", false, parsed("", "", "", "", "")),
            (".", ".", ".", ".", "", false, parsed("", "", ".", "", ".")),
            (
                "..",
                "..",
                ".",
                "..",
                "",
                false,
                parsed("", "", "..", "", ".."),
            ),
            ("/", "\\", "/", "", "", true, parsed("/", "/", "", "", "")),
            ("//", "\\", "/", "", "", true, parsed("/", "/", "", "", "")),
            (
                "/foo/bar",
                "\\foo\\bar",
                "/foo",
                "bar",
                "",
                true,
                parsed("/", "/foo", "bar", "", "bar"),
            ),
            (
                "C:\\foo\\bar",
                "C:\\foo\\bar",
                "C:\\foo",
                "bar",
                "",
                true,
                parsed("C:\\", "C:\\foo", "bar", "", "bar"),
            ),
            (
                "C:/foo/bar",
                "C:\\foo\\bar",
                "C:/foo",
                "bar",
                "",
                true,
                parsed("C:/", "C:/foo", "bar", "", "bar"),
            ),
            (
                "C:\\foo\\..\\bar",
                "C:\\bar",
                "C:\\foo\\..",
                "bar",
                "",
                true,
                parsed("C:\\", "C:\\foo\\..", "bar", "", "bar"),
            ),
            (
                "\\\\server\\share\\foo",
                "\\\\server\\share\\foo",
                "\\\\server\\share\\",
                "foo",
                "",
                true,
                parsed(
                    "\\\\server\\share\\",
                    "\\\\server\\share\\",
                    "foo",
                    "",
                    "foo",
                ),
            ),
            (
                "\\\\?\\C:\\foo\\bar",
                "\\\\?\\C:\\foo\\bar",
                "\\\\?\\C:\\foo",
                "bar",
                "",
                true,
                parsed("\\\\?\\C:\\", "\\\\?\\C:\\foo", "bar", "", "bar"),
            ),
            (
                "\\\\.\\C:\\foo",
                "\\\\.\\C:\\foo",
                "\\\\.\\C:\\",
                "foo",
                "",
                true,
                parsed("\\\\.\\C:\\", "\\\\.\\C:\\", "foo", "", "foo"),
            ),
            (
                "C:foo",
                "C:foo",
                "C:",
                "foo",
                "",
                false,
                parsed("C:", "C:", "foo", "", "foo"),
            ),
            (
                "C:foo\\bar",
                "C:foo\\bar",
                "C:foo",
                "bar",
                "",
                false,
                parsed("C:", "C:foo", "bar", "", "bar"),
            ),
            (
                "\\foo\\bar",
                "\\foo\\bar",
                "\\foo",
                "bar",
                "",
                true,
                parsed("\\", "\\foo", "bar", "", "bar"),
            ),
            (
                "/foo\\bar",
                "\\foo\\bar",
                "/foo",
                "bar",
                "",
                true,
                parsed("/", "/foo", "bar", "", "bar"),
            ),
        ];
        for (path, norm, dir, base, ext, absolute, parse_expected) in cases {
            assert_eq!(
                normalize(PathFlavor::Win32, path),
                norm,
                "normalize {path:?}"
            );
            assert_eq!(dirname(PathFlavor::Win32, path), dir, "dirname {path:?}");
            assert_eq!(
                basename(PathFlavor::Win32, path, None),
                base,
                "basename {path:?}"
            );
            assert_eq!(extname(PathFlavor::Win32, path), ext, "extname {path:?}");
            assert_eq!(
                is_absolute(PathFlavor::Win32, path),
                absolute,
                "isAbsolute {path:?}"
            );
            assert_eq!(
                parse(PathFlavor::Win32, path),
                parse_expected,
                "parse {path:?}"
            );
        }
    }

    #[test]
    fn join_and_resolve_vectors_match_node() {
        assert_eq!(join(PathFlavor::Posix, &[]), ".");
        assert_eq!(join(PathFlavor::Posix, &["a", "b"]), "a/b");
        assert_eq!(join(PathFlavor::Posix, &["a", "/b"]), "a/b");
        assert_eq!(join(PathFlavor::Posix, &["/a", "..", "..", "b"]), "/b");
        assert_eq!(join(PathFlavor::Win32, &["a", "b"]), "a\\b");
        assert_eq!(join(PathFlavor::Win32, &["/a", "/b"]), "\\a\\b");
        assert_eq!(join(PathFlavor::Win32, &["C:\\a", "b"]), "C:\\a\\b");
        assert_eq!(
            join(PathFlavor::Win32, &["\\\\server\\share", "a"]),
            "\\\\server\\share\\a"
        );

        assert_eq!(resolve(PathFlavor::Posix, "/tmp", &[]), "/tmp");
        assert_eq!(resolve(PathFlavor::Posix, "/tmp", &["a"]), "/tmp/a");
        assert_eq!(resolve(PathFlavor::Posix, "/tmp", &["/a"]), "/a");
        assert_eq!(
            resolve(PathFlavor::Posix, "/tmp", &["a", "..", "b"]),
            "/tmp/b"
        );
        assert_eq!(resolve(PathFlavor::Win32, "/tmp", &[]), "\\tmp");
        assert_eq!(resolve(PathFlavor::Win32, "/tmp", &["a"]), "\\tmp\\a");
        assert_eq!(resolve(PathFlavor::Win32, "/tmp", &["/a"]), "\\a");
        assert_eq!(
            resolve(PathFlavor::Win32, "/tmp", &["C:foo"]),
            "C:\\tmp\\foo"
        );
        assert_eq!(
            resolve(PathFlavor::Win32, "/tmp", &["\\\\server\\share", "a"]),
            "\\\\server\\share\\a"
        );
    }

    #[test]
    fn relative_vectors_match_node() {
        assert_eq!(relative(PathFlavor::Posix, "/tmp", "/a/b", "/a/b/c"), "c");
        assert_eq!(relative(PathFlavor::Posix, "/tmp", "/a/b/c", "/a/b"), "..");
        assert_eq!(relative(PathFlavor::Posix, "/tmp", "/a/b", "/a/c"), "../c");
        assert_eq!(relative(PathFlavor::Posix, "/tmp", "/", "/a"), "a");
        assert_eq!(relative(PathFlavor::Posix, "/tmp", "/a", "/"), "..");
        assert_eq!(
            relative(PathFlavor::Win32, "/tmp", "C:\\a\\b", "C:\\a\\b\\c"),
            "c"
        );
        assert_eq!(
            relative(PathFlavor::Win32, "/tmp", "C:\\a\\b", "c:\\a\\c"),
            "..\\c"
        );
        assert_eq!(
            relative(PathFlavor::Win32, "/tmp", "C:\\a", "D:\\b"),
            "D:\\b"
        );
        assert_eq!(
            relative(
                PathFlavor::Win32,
                "/tmp",
                "\\\\server\\share\\a",
                "\\\\server\\share\\a\\b"
            ),
            "b"
        );
    }

    #[test]
    fn format_vectors_match_node() {
        let cases = [
            (PathObject::default(), ""),
            (
                PathObject {
                    root: Some("/"),
                    ..PathObject::default()
                },
                "/",
            ),
            (
                PathObject {
                    root: Some("/"),
                    name: Some("a"),
                    ..PathObject::default()
                },
                "/a",
            ),
            (
                PathObject {
                    dir: Some("/a"),
                    base: Some("b"),
                    ..PathObject::default()
                },
                "/a/b",
            ),
            (
                PathObject {
                    dir: Some("/a/"),
                    base: Some("b"),
                    ..PathObject::default()
                },
                "/a//b",
            ),
            (
                PathObject {
                    dir: Some(""),
                    base: Some("b"),
                    ..PathObject::default()
                },
                "b",
            ),
            (
                PathObject {
                    base: Some("b"),
                    name: Some("c"),
                    ext: Some(".js"),
                    ..PathObject::default()
                },
                "b",
            ),
        ];
        for (object, expected) in cases {
            assert_eq!(format(PathFlavor::Posix, &object), expected);
        }
        assert_eq!(
            format(
                PathFlavor::Win32,
                &PathObject {
                    dir: Some("C:\\a"),
                    name: Some("b"),
                    ..PathObject::default()
                }
            ),
            "C:\\a\\b"
        );
    }

    #[test]
    fn to_namespaced_path_vectors_match_node() {
        assert_eq!(
            to_namespaced_path(PathFlavor::Posix, "/tmp", "foo/bar"),
            "foo/bar"
        );
        assert_eq!(
            to_namespaced_path(PathFlavor::Win32, "/tmp", "C:\\foo\\bar"),
            "\\\\?\\C:\\foo\\bar"
        );
        assert_eq!(
            to_namespaced_path(PathFlavor::Win32, "/tmp", "\\\\server\\share\\foo"),
            "\\\\?\\UNC\\server\\share\\foo"
        );
        assert_eq!(
            to_namespaced_path(PathFlavor::Win32, "/tmp", "\\\\?\\C:\\foo"),
            "\\\\?\\C:\\foo"
        );
    }

    #[test]
    fn win32_reserved_name_vectors() {
        assert_eq!(normalize(PathFlavor::Win32, "CON"), "CON");
        assert_eq!(normalize(PathFlavor::Win32, "CON:"), ".\\CON:.");
        assert_eq!(normalize(PathFlavor::Win32, "foo\\CON"), "foo\\CON");
        assert_eq!(normalize(PathFlavor::Win32, "C:\\CON"), "C:\\CON");
    }

    #[test]
    fn posix_basename_and_extname_edges_match_node() {
        assert_eq!(basename(PathFlavor::Posix, "/foo/bar/", Some("bar")), "bar");
        assert_eq!(basename(PathFlavor::Posix, "/foo/bar", Some("bar")), "bar");
        assert_eq!(basename(PathFlavor::Posix, "foo.js", Some("js")), "foo.");
        assert_eq!(basename(PathFlavor::Posix, "foo", Some("foo")), "");
        assert_eq!(basename(PathFlavor::Posix, "", Some("")), "");
        assert_eq!(extname(PathFlavor::Posix, "foo."), ".");
        assert_eq!(extname(PathFlavor::Posix, "foo..bar"), ".bar");
        assert_eq!(extname(PathFlavor::Posix, ".foo"), "");
        assert_eq!(normalize(PathFlavor::Posix, "foo/"), "foo/");
        assert_eq!(normalize(PathFlavor::Posix, "./"), "./");
        assert_eq!(normalize(PathFlavor::Posix, "../"), "../");
        assert_eq!(
            parse(PathFlavor::Posix, "foo."),
            parsed("", "", "foo.", ".", "foo")
        );
        assert_eq!(
            parse(PathFlavor::Posix, "foo.tar.gz"),
            parsed("", "", "foo.tar.gz", ".gz", "foo.tar")
        );
    }
}

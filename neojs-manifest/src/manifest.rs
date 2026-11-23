//! Validated `neo.jsonc` manifest types.
//!
//! The manifest is a project file. It is not an npm `package.json`, a Deno
//! `deno.json` verbatim, or a publishing contract. This module validates the
//! fields listed on [`Manifest`] and rejects every other key.
//!
//! Parsing is pure except for [`Manifest::from_path`], which reads the one path
//! it is given. Nothing here discovers a manifest, resolves a workspace, applies
//! `include` or `exclude`, installs an import map, or fetches a package.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use neojs_module_policy::module_format;
use semver::Version;
use serde_json::Value;

use crate::Error;
use crate::jsonc;
use crate::specifier::NormalizedSpecifier;
use crate::specifier::SpecifierKind;
use crate::specifier::Support;
use crate::specifier::is_windows_drive_path;
use crate::specifier::normalize_specifier;

/// The conventional manifest file name.
///
/// This crate never looks for the file on its own; the caller supplies a path.
pub const MANIFEST_FILE_NAME: &str = "neo.jsonc";

/// Maximum manifest bytes accepted by [`Manifest::parse`].
pub const MAX_MANIFEST_BYTES: usize = jsonc::MAX_JSONC_BYTES;

const MAX_DESCRIPTION_BYTES: usize = 4096;
const MAX_AUTHORS: usize = 64;
const MAX_AUTHOR_BYTES: usize = 256;
const MAX_EXPORT_ENTRIES: usize = 1024;
const MAX_EXPORT_KEY_BYTES: usize = 512;
const MAX_EXPORT_TARGET_BYTES: usize = 1024;
const MAX_PATTERNS: usize = 256;
const MAX_PATTERN_BYTES: usize = 1024;
const MAX_IMPORTS: usize = 1024;
const MAX_IMPORT_KEY_BYTES: usize = 1024;

/// Fields a future version of this project may use, rejected with a reason so a
/// user does not mistake a reserved name for working configuration.
const RESERVED_FIELDS: &[(&str, &str)] = &[
    (
        "node",
        "the Node compatibility transform is planned but not implemented",
    ),
    ("nodeModulesDir", "a node_modules layout is not implemented"),
    (
        "type",
        "CommonJS and package type selection are not implemented; exports are always ECMAScript modules",
    ),
    (
        "main",
        "use `exports`; a `main` fallback is not implemented",
    ),
    (
        "workspace",
        "use `workspaces`; Deno's singular key is not accepted by this foundation",
    ),
    (
        "importMap",
        "use `imports`; import-map application is not implemented",
    ),
    ("tasks", "task execution is not implemented"),
    ("compilerOptions", "compiler options are not implemented"),
    ("lint", "lint configuration is not implemented"),
    ("fmt", "format configuration is not implemented"),
    ("lock", "lockfile configuration is not implemented"),
];

/// A validated `neo.jsonc` manifest.
///
/// Only `name` is required. Every other field is omitted or empty when absent.
/// `version` is an exact SemVer version, not a range. `exports` targets must be
/// `./`-relative and end in `.ts`, `.mts`, `.js`, or `.mjs`; `.tsx`, `.jsx`,
/// `.cts`, `.cjs`, declarations, JSON, and Wasm targets are rejected.
///
/// `include`, `exclude`, and `workspaces` accept relative patterns containing
/// `*`, `**`, `?`, `[`, `]`, `{`, or `}`, but this crate does not interpret
/// them. They are syntax-checked metadata only. A pattern may not be absolute,
/// may not contain `..`, and may not start with `!`.
///
/// `imports` is a map from a specifier key to a normalized target. npm, JSR,
/// GitHub, and Node targets are accepted as metadata and report
/// [`Support::Unsupported`]; local and HTTPS targets report
/// [`Support::NormalizedOnly`] because the map is not installed in a runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The scoped package name, such as `@scope/name`.
    pub name: PackageName,
    /// An exact SemVer version, when the manifest declares one.
    pub version: Option<Version>,
    /// A human-readable summary.
    pub description: Option<String>,
    /// Author names or contact strings, in declaration order.
    pub authors: Vec<String>,
    /// Public entrypoints, when the manifest declares any.
    pub exports: Option<Exports>,
    /// Files or patterns a package would include. Not interpreted here.
    pub include: Vec<Pattern>,
    /// Files or patterns a package would exclude. Not interpreted here.
    pub exclude: Vec<Pattern>,
    /// Normalized import-map entries. Not installed in a runtime.
    pub imports: ImportMap,
    /// Workspace directory patterns. Not expanded here.
    pub workspaces: Vec<Pattern>,
}

impl Manifest {
    /// Parses and validates one manifest.
    ///
    /// The input is JSONC: comments and one trailing comma per object or array
    /// are accepted, and duplicate keys are rejected. Unknown fields are
    /// rejected, including the reserved names in the module documentation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Jsonc`] or [`Error::DuplicateKey`] for malformed input,
    /// [`Error::MissingField`] when `name` is absent, and
    /// [`Error::InvalidField`], [`Error::UnsupportedField`], or
    /// [`Error::UnknownField`] for schema failures. A target that names an
    /// unsupported transport such as `http:` returns
    /// [`Error::UnsupportedTransport`].
    pub fn parse(text: &str) -> Result<Self, Error> {
        let value = jsonc::parse(text)?;
        let Value::Object(object) = value else {
            return Err(invalid_field(
                "manifest",
                "the top-level value must be an object",
            ));
        };

        let mut name = None;
        let mut version = None;
        let mut description = None;
        let mut authors = Vec::new();
        let mut exports = None;
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        let mut imports = ImportMap::default();
        let mut workspaces = Vec::new();

        for (key, value) in &object {
            match key.as_str() {
                "name" => name = Some(PackageName::parse(string(value, "name")?)?),
                "version" => version = Some(parse_version(value)?),
                "description" => description = Some(parse_description(value)?),
                "authors" => authors = parse_authors(value)?,
                "exports" => exports = Some(parse_exports(value)?),
                "include" => include = parse_patterns(value, "include")?,
                "exclude" => exclude = parse_patterns(value, "exclude")?,
                "imports" => imports = parse_imports(value)?,
                "workspaces" => workspaces = parse_patterns(value, "workspaces")?,
                other => return Err(unknown_field(other)),
            }
        }

        let name = name.ok_or(Error::MissingField { field: "name" })?;
        Ok(Self {
            name,
            version,
            description,
            authors,
            exports,
            include,
            exclude,
            imports,
            workspaces,
        })
    }

    /// Reads and validates a manifest from a caller-supplied path.
    ///
    /// This does not search parent directories and does not default to
    /// [`MANIFEST_FILE_NAME`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] when the file cannot be read, or any error from
    /// [`Self::parse`].
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&text)
    }
}

/// A scoped package name.
///
/// The accepted form is `@scope/name`. Both parts are 1 to 213 bytes, use only
/// lowercase ASCII letters, digits, `-`, `_`, and `.`, and start with a letter
/// or digit. The complete spelling is at most 214 bytes. This is deliberately
/// narrower than npm's historical name rules and is not a registry identity.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackageName {
    full: String,
    scope: String,
    name: String,
}

impl PackageName {
    /// Parses a scoped package name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidField`] with field `name` when the spelling is
    /// not `@scope/name` or a part is outside the documented character set.
    pub fn parse(raw: &str) -> Result<Self, Error> {
        Self::parse_in(raw, "name")
    }

    fn parse_in(raw: &str, field: &'static str) -> Result<Self, Error> {
        if raw.len() > 214 {
            return Err(invalid_field(field, "the package name is over 214 bytes"));
        }
        let Some(rest) = raw.strip_prefix('@') else {
            return Err(invalid_field(
                field,
                "the package name must be scoped, as `@scope/name`",
            ));
        };
        let Some((scope, name)) = rest.split_once('/') else {
            return Err(invalid_field(
                field,
                "the package name must include both a scope and a name",
            ));
        };
        if name.contains('/') {
            return Err(invalid_field(
                field,
                "the package name must contain exactly one `/`",
            ));
        }
        validate_package_part(scope, field, "scope")?;
        validate_package_part(name, field, "name")?;
        Ok(Self {
            full: raw.to_owned(),
            scope: scope.to_owned(),
            name: name.to_owned(),
        })
    }

    /// Returns the scope without its leading `@`.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Returns the package name without the scope.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the complete `@scope/name` spelling.
    pub fn as_str(&self) -> &str {
        &self.full
    }
}

impl fmt::Display for PackageName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.full)
    }
}

fn validate_package_part(part: &str, field: &'static str, subject: &str) -> Result<(), Error> {
    if part.is_empty() {
        return Err(invalid_field(field, format!("the {subject} is empty")));
    }
    if part.len() > 213 {
        return Err(invalid_field(
            field,
            format!("the {subject} is over 213 bytes"),
        ));
    }
    if part == "." || part == ".." {
        return Err(invalid_field(
            field,
            format!("the {subject} cannot be `.` or `..`"),
        ));
    }
    if !part.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
    }) {
        return Err(invalid_field(
            field,
            format!("the {subject} may contain only lowercase letters, digits, `-`, `_`, and `.`"),
        ));
    }
    if !part
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(invalid_field(
            field,
            format!("the {subject} must start with a letter or digit"),
        ));
    }
    Ok(())
}

/// One public export entrypoint, either a single root target or a keyed map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exports {
    /// A single target for the package root.
    Single(ExportTarget),
    /// A map from `.` or `./subpath` keys to targets.
    Map(BTreeMap<ExportKey, ExportTarget>),
}

/// A validated export key: `.` or a `./`-relative subpath.
///
/// Wildcard keys are not implemented. The key is not a filesystem path; it is
/// the name an importer writes after the package name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExportKey(String);

impl ExportKey {
    /// Parses a `.` or `./subpath` export key.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidField`] with field `exports` for a key that is
    /// absolute, contains `.` or `..` segments, or uses a wildcard.
    pub fn parse(raw: &str) -> Result<Self, Error> {
        if raw == "." {
            return Ok(Self(raw.to_owned()));
        }
        let Some(path) = raw.strip_prefix("./") else {
            return Err(invalid_field(
                "exports",
                "an export key must be `.` or start with `./`",
            ));
        };
        if path.is_empty() {
            return Err(invalid_field("exports", "an export key names no subpath"));
        }
        if raw.len() > MAX_EXPORT_KEY_BYTES {
            return Err(invalid_field(
                "exports",
                format!("an export key is over the {MAX_EXPORT_KEY_BYTES}-byte limit"),
            ));
        }
        if raw.contains('*') || raw.contains('?') {
            return Err(Error::UnsupportedField {
                field: "exports".to_owned(),
                reason: "wildcard export keys are not implemented",
            });
        }

        validate_safe_segments(path, "exports", "an export key")?;
        Ok(Self(raw.to_owned()))
    }

    /// Returns the key exactly as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExportKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A validated export target: a `./`-relative TypeScript or JavaScript module.
///
/// The target must end in `.ts`, `.mts`, `.js`, or `.mjs`. Conditional objects,
/// null exclusions, wildcard targets, CommonJS, JSX/TSX, declarations, JSON, and
/// Wasm are not accepted.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExportTarget(String);

impl ExportTarget {
    /// Parses an export target.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidField`] with field `exports` when the target is
    /// not a `./`-relative module with an accepted extension.
    pub fn parse(raw: &str) -> Result<Self, Error> {
        let Some(path) = raw.strip_prefix("./") else {
            return Err(invalid_field(
                "exports",
                "an export target must start with `./`",
            ));
        };
        if path.is_empty() {
            return Err(invalid_field("exports", "an export target names no file"));
        }
        if raw.len() > MAX_EXPORT_TARGET_BYTES {
            return Err(invalid_field(
                "exports",
                format!("an export target is over the {MAX_EXPORT_TARGET_BYTES}-byte limit"),
            ));
        }
        if path.contains('*') || path.contains('?') {
            return Err(Error::UnsupportedField {
                field: "exports".to_owned(),
                reason: "wildcard export targets are not implemented",
            });
        }
        validate_safe_segments(path, "exports", "an export target")?;
        match module_format(Path::new(raw)) {
            Ok(_) => {
                let extension = Path::new(raw)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.to_ascii_lowercase());
                if !matches!(extension.as_deref(), Some("js" | "mjs" | "ts" | "mts")) {
                    return Err(invalid_field(
                        "exports",
                        "an export target must end in .ts, .mts, .js, or .mjs",
                    ));
                }
            }
            Err(reason) => return Err(invalid_field("exports", reason)),
        }
        Ok(Self(raw.to_owned()))
    }

    /// Returns the target exactly as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExportTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A validated relative glob pattern used by `include`, `exclude`, or
/// `workspaces`.
///
/// This crate checks the spelling only. It does not expand or match globs.
/// Patterns are 1 to 1024 bytes, are relative, and contain no empty, `.`, or
/// `..` segment and no backslash. A leading `!` is rejected because negation is
/// not implemented.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pattern(String);

impl Pattern {
    /// Parses a relative pattern.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidField`] with field `pattern` for an absolute or
    /// escaping pattern, or one that starts with `!`.
    pub fn parse(raw: &str) -> Result<Self, Error> {
        Self::parse_in(raw, "pattern")
    }

    fn parse_in(raw: &str, field: &'static str) -> Result<Self, Error> {
        if raw.is_empty() {
            return Err(invalid_field(field, "a pattern is empty"));
        }
        if raw.len() > MAX_PATTERN_BYTES {
            return Err(invalid_field(
                field,
                format!("a pattern is over the {MAX_PATTERN_BYTES}-byte limit"),
            ));
        }
        if raw.trim() != raw {
            return Err(invalid_field(
                field,
                "a pattern may not start or end with whitespace",
            ));
        }
        if raw.chars().any(char::is_control) {
            return Err(invalid_field(
                field,
                "a pattern contains a control character",
            ));
        }
        if raw.starts_with('!') {
            return Err(Error::UnsupportedField {
                field: field.to_owned(),
                reason: "negated glob patterns are not implemented",
            });
        }
        if raw.starts_with('/')
            || raw.starts_with('\\')
            || is_windows_drive_path(raw)
            || raw.contains('\\')
        {
            return Err(invalid_field(
                field,
                "a pattern must be relative and use `/` as its separator",
            ));
        }
        for segment in raw.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." {
                return Err(invalid_field(
                    field,
                    "a pattern has an empty, `.`, or `..` segment",
                ));
            }
        }
        Ok(Self(raw.to_owned()))
    }

    /// Returns the pattern exactly as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Pattern {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A validated import map.
///
/// Keys are preserved exactly. Values are normalized by
/// [`normalize_specifier`]. Iteration is sorted by key.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportMap {
    entries: BTreeMap<String, NormalizedSpecifier>,
}

impl ImportMap {
    /// Returns the target for a key, or `None` when the key is absent.
    pub fn get(&self, key: &str) -> Option<&NormalizedSpecifier> {
        self.entries.get(key)
    }

    /// Iterates entries in ascending key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &NormalizedSpecifier)> {
        self.entries
            .iter()
            .map(|(key, specifier)| (key.as_str(), specifier))
    }

    /// Returns the number of mappings.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns whether the map has no mappings.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the first entry, in key order, whose transport is not
    /// implemented.
    ///
    /// A caller that intends to install this map must reject the returned
    /// entry. This crate never installs it, so the method is a guard for a
    /// future integration rather than a load path.
    pub fn first_unsupported(&self) -> Option<(&str, Support)> {
        self.entries.iter().find_map(|(key, specifier)| {
            let support = specifier.support();
            match support {
                Support::NormalizedOnly => None,
                Support::Unsupported(_) => Some((key.as_str(), support)),
            }
        })
    }
}

fn parse_version(value: &Value) -> Result<Version, Error> {
    let raw = string(value, "version")?;
    Version::parse(raw).map_err(|_| {
        invalid_field(
            "version",
            "the version must be an exact SemVer version such as `1.2.3`",
        )
    })
}

fn parse_description(value: &Value) -> Result<String, Error> {
    let raw = string(value, "description")?;
    if raw.len() > MAX_DESCRIPTION_BYTES {
        return Err(invalid_field(
            "description",
            format!("the description is over the {MAX_DESCRIPTION_BYTES}-byte limit"),
        ));
    }
    if raw
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(invalid_field(
            "description",
            "the description contains a control character",
        ));
    }
    Ok(raw.to_owned())
}

fn parse_authors(value: &Value) -> Result<Vec<String>, Error> {
    let items = array(value, "authors")?;
    if items.len() > MAX_AUTHORS {
        return Err(invalid_field(
            "authors",
            format!("there are more than {MAX_AUTHORS} authors"),
        ));
    }
    let mut authors = Vec::with_capacity(items.len());
    for item in items {
        let raw = string(item, "authors")?;
        if raw.is_empty() || raw.trim() != raw {
            return Err(invalid_field(
                "authors",
                "an author must be non-empty and may not start or end with whitespace",
            ));
        }
        if raw.len() > MAX_AUTHOR_BYTES {
            return Err(invalid_field(
                "authors",
                format!("an author is over the {MAX_AUTHOR_BYTES}-byte limit"),
            ));
        }
        if raw
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            return Err(invalid_field(
                "authors",
                "an author contains a control character",
            ));
        }
        authors.push(raw.to_owned());
    }
    Ok(authors)
}

fn parse_exports(value: &Value) -> Result<Exports, Error> {
    match value {
        Value::String(target) => Ok(Exports::Single(ExportTarget::parse(target)?)),
        Value::Object(object) => {
            if object.is_empty() {
                return Err(invalid_field("exports", "the export map is empty"));
            }
            if object.len() > MAX_EXPORT_ENTRIES {
                return Err(invalid_field(
                    "exports",
                    format!("there are more than {MAX_EXPORT_ENTRIES} export entries"),
                ));
            }
            let mut entries = BTreeMap::new();
            for (key, value) in object {
                let key = ExportKey::parse(key)?;
                let target = match value {
                    Value::String(target) => ExportTarget::parse(target)?,
                    Value::Object(_) => {
                        return Err(Error::UnsupportedField {
                            field: "exports".to_owned(),
                            reason: "conditional export objects are not implemented",
                        });
                    }
                    Value::Null => {
                        return Err(Error::UnsupportedField {
                            field: "exports".to_owned(),
                            reason: "null export exclusions are not implemented",
                        });
                    }
                    _ => {
                        return Err(invalid_field(
                            "exports",
                            "an export target must be a string",
                        ));
                    }
                };
                entries.insert(key, target);
            }
            Ok(Exports::Map(entries))
        }
        _ => Err(invalid_field(
            "exports",
            "exports must be a string or an object of string targets",
        )),
    }
}

fn parse_patterns(value: &Value, field: &'static str) -> Result<Vec<Pattern>, Error> {
    let items = array(value, field)?;
    if items.len() > MAX_PATTERNS {
        return Err(invalid_field(
            field,
            format!("there are more than {MAX_PATTERNS} patterns"),
        ));
    }
    let mut patterns = Vec::with_capacity(items.len());
    for item in items {
        let raw = string(item, field)?;
        patterns.push(Pattern::parse_in(raw, field)?);
    }
    Ok(patterns)
}

fn parse_imports(value: &Value) -> Result<ImportMap, Error> {
    let object = match value {
        Value::Object(object) => object,
        _ => return Err(invalid_field("imports", "imports must be an object")),
    };
    if object.len() > MAX_IMPORTS {
        return Err(invalid_field(
            "imports",
            format!("there are more than {MAX_IMPORTS} import entries"),
        ));
    }
    let mut entries = BTreeMap::new();
    for (key, value) in object {
        validate_import_key(key)?;
        let raw = string(value, "imports")?;
        let specifier = normalize_specifier(raw)?;
        if matches!(specifier.kind(), SpecifierKind::Bare) {
            return Err(invalid_field(
                "imports",
                "an import-map target must be a relative path, URL, or package specifier",
            ));
        }
        entries.insert(key.clone(), specifier);
    }
    Ok(ImportMap { entries })
}

fn validate_import_key(key: &str) -> Result<(), Error> {
    if key.is_empty() {
        return Err(invalid_field("imports", "an import-map key is empty"));
    }
    if key.len() > MAX_IMPORT_KEY_BYTES {
        return Err(invalid_field(
            "imports",
            format!("an import-map key is over the {MAX_IMPORT_KEY_BYTES}-byte limit"),
        ));
    }
    if key.chars().any(char::is_control) {
        return Err(invalid_field(
            "imports",
            "an import-map key contains a control character",
        ));
    }
    if key.chars().any(char::is_whitespace) {
        return Err(invalid_field(
            "imports",
            "an import-map key contains whitespace",
        ));
    }
    if key.contains('\\') {
        return Err(invalid_field(
            "imports",
            "an import-map key may not contain `\\`",
        ));
    }
    Ok(())
}

fn validate_safe_segments(path: &str, field: &'static str, subject: &str) -> Result<(), Error> {
    if path.contains('\\') || path.contains('%') || path.contains('?') || path.contains('#') {
        return Err(invalid_field(
            field,
            format!("{subject} contains `\\`, `%`, `?`, or `#`"),
        ));
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(invalid_field(
                field,
                format!("{subject} has an empty, `.`, or `..` segment"),
            ));
        }
        if !segment.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'.' | b'_'
                        | b'-'
                        | b'~'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b':'
                        | b'@'
                )
        }) {
            return Err(invalid_field(
                field,
                format!("{subject} contains a character outside the safe path set"),
            ));
        }
    }
    Ok(())
}

fn string<'a>(value: &'a Value, field: &'static str) -> Result<&'a str, Error> {
    value.as_str().ok_or_else(|| {
        invalid_field(
            field,
            format!("`{field}` must be a string, not {}", kind_name(value)),
        )
    })
}

fn array<'a>(value: &'a Value, field: &'static str) -> Result<&'a [Value], Error> {
    value.as_array().map(Vec::as_slice).ok_or_else(|| {
        invalid_field(
            field,
            format!("`{field}` must be an array, not {}", kind_name(value)),
        )
    })
}

fn kind_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn invalid_field(field: &'static str, reason: impl Into<String>) -> Error {
    Error::InvalidField {
        field,
        reason: reason.into(),
    }
}

fn unknown_field(field: &str) -> Error {
    if let Some((_, reason)) = RESERVED_FIELDS
        .iter()
        .find(|(reserved, _)| *reserved == field)
    {
        Error::UnsupportedField {
            field: field.to_owned(),
            reason,
        }
    } else {
        Error::UnknownField {
            field: field.to_owned(),
        }
    }
}

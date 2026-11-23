//! Errors returned while reading, parsing, or validating a manifest.
//!
//! Every variant is a distinct programmatic cause. Messages name the field or
//! limit that failed, but never echo field values, URLs, or file contents: a
//! manifest can carry a signed query, an author name, or other text that should
//! not appear in a log line.

use std::path::PathBuf;

use thiserror::Error;

/// A failure while reading or interpreting a `neo.jsonc` manifest.
#[derive(Debug, Error)]
pub enum Error {
    /// The manifest file could not be read.
    #[error("could not read `{path}`: {source}")]
    Io {
        /// The path supplied by the caller.
        path: PathBuf,
        /// The operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The document is not valid JSONC.
    #[error("JSONC syntax error at line {line}, column {column}: {message}")]
    Jsonc {
        /// One-based line number.
        line: usize,
        /// One-based byte column within the line.
        column: usize,
        /// Why the document is invalid.
        message: String,
    },
    /// The same key appears twice in one object.
    ///
    /// The key itself is not included: JSONC parsing is a generic operation,
    /// and a key can contain text a caller should not find in a log line. The
    /// line and column identify the duplicate.
    #[error("duplicate object key at line {line}, column {column}")]
    DuplicateKey {
        /// One-based line number.
        line: usize,
        /// One-based byte column within the line.
        column: usize,
    },
    /// A required field is absent.
    #[error("manifest field `{field}` is required")]
    MissingField {
        /// The absent field.
        field: &'static str,
    },
    /// A present field has the wrong type or a value outside its contract.
    #[error("manifest field `{field}` is invalid: {reason}")]
    InvalidField {
        /// The field that failed validation.
        field: &'static str,
        /// What the field contract requires.
        reason: String,
    },
    /// A field is recognised but deliberately not implemented.
    #[error("manifest field `{field}` is not implemented: {reason}")]
    UnsupportedField {
        /// The field that is reserved for a future change.
        field: String,
        /// The feature that would have to exist first.
        reason: &'static str,
    },
    /// A field is not part of the accepted manifest.
    #[error("unknown manifest field `{field}`")]
    UnknownField {
        /// The unrecognised key.
        field: String,
    },
    /// A specifier is syntactically invalid.
    #[error("specifier is invalid: {reason}")]
    InvalidSpecifier {
        /// Which part of the grammar failed.
        reason: String,
    },
    /// The specifier names a transport that this project does not implement.
    #[error("the `{scheme}:` transport is not implemented: {reason}")]
    UnsupportedTransport {
        /// The URL scheme, without its colon.
        scheme: String,
        /// What would have to exist before the scheme could be loaded.
        reason: &'static str,
    },
}

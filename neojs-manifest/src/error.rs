//! Errors returned while reading, parsing, or validating a manifest.
//!
//! Every variant is a distinct programmatic cause. Messages name static fields
//! and limits, but never echo document text, field values, URLs, filesystem
//! paths, unknown keys, or schemes: a manifest can carry a signed query, an
//! author name, or another value that should not appear in a log line.

use thiserror::Error;

/// A failure while reading or interpreting a `neo.jsonc` manifest.
#[derive(Debug, Error)]
pub enum Error {
    /// The manifest file could not be read.
    ///
    /// The supplied path is not included in the message. The caller knows which
    /// path it asked for, and a path can identify an internal location.
    #[error("could not read the manifest: {source}")]
    Io {
        /// The operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// The manifest is larger than the accepted limit.
    #[error("the manifest is over the {limit}-byte limit")]
    ManifestTooLarge {
        /// The maximum number of bytes [`crate::Manifest::parse`] accepts.
        limit: usize,
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
        /// A reserved name from this crate's fixed schema.
        field: &'static str,
        /// The feature that would have to exist first.
        reason: &'static str,
    },
    /// A field is not part of the accepted manifest.
    ///
    /// The key is not included. The manifest boundary rejects a typo without
    /// turning an arbitrary key into a log entry.
    #[error(
        "unknown manifest field; accepted fields are name, version, description, authors, exports, include, exclude, imports, and workspaces"
    )]
    UnknownField,
    /// A specifier is syntactically invalid.
    #[error("specifier is invalid: {reason}")]
    InvalidSpecifier {
        /// Which part of the grammar failed.
        reason: String,
    },
    /// The specifier names a transport that this project does not implement.
    ///
    /// The scheme is not included. An unrecognised scheme is caller text, not
    /// a schema name.
    #[error("the specifier uses an unsupported transport: {reason}")]
    UnsupportedTransport {
        /// What would have to exist before the transport could be loaded.
        reason: &'static str,
    },
}

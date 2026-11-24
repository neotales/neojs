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
    /// The key and its value are not included. The line and byte column point
    /// at the opening quote of the top-level key. The manifest boundary rejects
    /// a typo without turning arbitrary text into a log entry.
    #[error("unknown manifest field at line {line}, byte column {column}")]
    UnknownField {
        /// One-based line number of the key.
        line: usize,
        /// One-based byte column of the key's opening quote.
        column: usize,
    },
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
    /// The workspace expands to more members than the accepted limit.
    #[error("the workspace has more than {limit} members")]
    WorkspaceTooManyMembers {
        /// The maximum number of workspace members accepted.
        limit: usize,
    },
    /// Workspace glob traversal visited more filesystem entries than the
    /// accepted limit.
    #[error("workspace glob traversal visited more than {limit} filesystem entries")]
    WorkspaceGlobLimitExceeded {
        /// The maximum number of filesystem entries inspected across workspace globs.
        limit: usize,
    },
    /// A workspace member directory or manifest resolves outside the workspace
    /// root, or points back to the workspace root itself.
    #[error("a workspace member path escapes or points back to the workspace root")]
    WorkspaceEscape,
    /// A workspace member directory does not exist, is not a directory, or
    /// contains no manifest file.
    #[error("a workspace member directory is missing its manifest")]
    MissingMemberManifest,
    /// Two packages in the workspace (root or members) declare the same
    /// `@scope/name` package name.
    #[error("duplicate package name in workspace")]
    DuplicateWorkspacePackage,
    /// A workspace member declares its own `workspaces` patterns.
    #[error("nested workspaces are not supported")]
    NestedWorkspace,
    /// The requested active member name does not belong to the workspace.
    #[error("the requested package is not a member of the workspace")]
    UnknownWorkspaceMember,
    /// A workspace package import targets a subpath not declared in the
    /// package's `exports`.
    #[error("the requested subpath is not exported by the workspace package")]
    UnexportedSubpath,
}

//! Errors returned by the embedding API.
//!
//! Match on the variants to decide how to react. The message text is for people
//! and may change, so it is not part of the contract.

use std::io;
use std::path::PathBuf;

/// A failure while preparing or running a JavaScript program.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The entry file could not be made absolute or read as UTF-8 text.
    ///
    /// The common causes are a missing file, a directory in place of a file, and
    /// a file that is not valid UTF-8 (reported as
    /// [`std::io::ErrorKind::InvalidData`]).
    #[error("cannot read `{}`: {source}", path.display())]
    Entry {
        /// The path the caller passed to [`crate::Runtime::run_file`].
        path: PathBuf,
        /// The operating-system error.
        #[source]
        source: io::Error,
    },

    /// The entry file is not a format this runtime executes.
    ///
    /// neojs runs ECMAScript modules and erasable TypeScript. JSX and TSX,
    /// CommonJS (`.cjs` and `.cts`), declaration files, JSON, WebAssembly, and
    /// native-addon entry files are rejected with a reason rather than parsed as
    /// JavaScript.
    #[error("cannot run `{}`: {reason}", path.display())]
    UnsupportedEntry {
        /// The path the caller passed to [`crate::Runtime::run_file`].
        path: PathBuf,
        /// Why the entry file is not executed.
        reason: String,
    },

    /// The entry file is TypeScript that cannot be reduced to ECMAScript.
    ///
    /// The erasable subset is supported. Invalid TypeScript, and valid TypeScript
    /// that needs a transform (for example an `enum`, a `namespace`, or a
    /// parameter property), is rejected with the stripper's reason. This is not a
    /// type error: type stripping does not type check.
    ///
    /// The failure happens before V8 loads the module, so the runtime remains
    /// unused and can run another entry module. A `.ts` file imported by a
    /// running module fails as [`Error::Core`] instead, and that load consumes the
    /// runtime.
    #[error("cannot compile `{}`: {reason}", path.display())]
    TypeScript {
        /// The path the caller passed to [`crate::Runtime::run_file`].
        path: PathBuf,
        /// Why the file could not be reduced to ECMAScript.
        reason: String,
    },

    /// A remote entry module was requested from a runtime without remote
    /// loading enabled.
    ///
    /// Remote loading is opt-in: create the runtime with
    /// [`crate::Runtime::with_remote`] and an explicit
    /// [`crate::RemoteOptions`]. A remote import from an already-running module
    /// fails as [`Error::Core`] with a message instead.
    #[error(
        "remote module loading is disabled; create the Runtime with Runtime::with_remote to enable it"
    )]
    RemoteDisabled,

    /// The remote entry module could not be fetched or is not a format this
    /// runtime executes.
    ///
    /// This covers an invalid or non-HTTPS URL, a URL with credentials, a
    /// network or TLS failure, an HTTP error status, a redirect that violates
    /// the policy, a body above the configured limit, and an unsupported module
    /// extension. The failure happens before V8 loads the module, so the
    /// runtime remains unused. A remote module imported by a running module
    /// fails as [`Error::Core`] instead.
    #[error("cannot load `{url}`: {reason}")]
    RemoteEntry {
        /// The requested URL without credentials, a query, or a fragment.
        ///
        /// A query can contain a signed token, so it is not included in an
        /// error message.
        url: String,
        /// Why the remote entry module is not executed.
        reason: String,
    },

    /// The remote entry module is TypeScript that cannot be reduced to
    /// ECMAScript.
    ///
    /// This has the same meaning as [`Error::TypeScript`] for a URL entry. The
    /// runtime remains unused, because the failure happens before V8 loads the
    /// module.
    #[error("cannot compile `{url}`: {reason}")]
    RemoteTypeScript {
        /// The requested URL without credentials, a query, or a fragment.
        url: String,
        /// Why the module could not be reduced to ECMAScript.
        reason: String,
    },

    /// The native built-in surface could not be installed.
    ///
    /// This is an internal construction failure (for example a V8 allocation
    /// failure while creating the native `neo:chars` functions), not a problem
    /// in the program being run. The runtime cannot be used.
    #[error("cannot install the native built-in surface: {0}")]
    NativeBuiltins(String),

    /// V8 rejected the module while loading, linking, or evaluating it.
    ///
    /// This covers module resolution, import loading, syntax errors, uncaught
    /// exceptions, and rejected top-level promises. The display text carries
    /// V8's stack when one is available.
    #[error(transparent)]
    Core(#[from] deno_core::error::CoreError),

    /// The internal Tokio driver runtime could not be created.
    ///
    /// [`crate::Runtime::new`] starts one worker thread to service V8's delayed
    /// foreground tasks. This error means the operating system refused to start
    /// it, so no isolate was created.
    #[error("cannot start the async executor: {0}")]
    Executor(#[source] io::Error),

    /// [`crate::Runtime::run_file`] was called from inside a Tokio runtime.
    ///
    /// Blocking on a second runtime from an asynchronous context would panic, so
    /// the call fails instead. Use [`crate::Runtime::run_file_async`] there.
    #[error("run_file cannot block inside a Tokio runtime; call run_file_async instead")]
    BlockingInAsyncContext,

    /// The runtime already loaded its one entry module.
    ///
    /// A [`crate::Runtime`] runs at most one entry module. Loading consumes the
    /// runtime, and a failed or cancelled load consumes it too because V8's module
    /// map can hold partial state. Create a new runtime for the next entry module.
    #[error(
        "this Runtime has already loaded an entry module; create a new Runtime for each entry module"
    )]
    RuntimeConsumed,

    /// The `test` runner returned an unusable report or failed unexpectedly.
    ///
    /// This is a runner fault, not a test failure. Test failures are reported in
    /// [`crate::TestReport`] and make the command exit non-zero; this variant
    /// means the counts could not be trusted, so they are not returned.
    #[error("test runner failed: {0}")]
    TestRunner(String),
}

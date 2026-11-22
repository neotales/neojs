//! The embeddable runtime, its single-entry rule, and its error type.

use std::io;
use std::path::{Path, PathBuf};

use neojs_module_policy::{ModuleFormat, module_format};
use rquickjs::{CatchResultExt, Context, Function, Module};

use crate::loader::{FileLoader, FileResolver};

/// Loads the `console` object and the one host function it calls.
///
/// The shim stringifies arguments with JavaScript's `String`, so numbers,
/// booleans, `null`, and objects all print something. It is not a full console:
/// there is no `debug`, `warn`, `info`, grouping, formatting placeholders, or
/// inspection of object contents. `console.log` writes to standard output and
/// `console.error` to standard error.
const CONSOLE_SHIM: &str = r#"
globalThis.console = Object.freeze({
    log: (...args) => globalThis.__neoltjsWrite("out", args.map((value) => String(value)).join(" ")),
    error: (...args) => globalThis.__neoltjsWrite("err", args.map((value) => String(value)).join(" ")),
});
"#;

/// A failure while constructing or running a QuickJS program.
///
/// The message text is for people and may change; match on the variants when a
/// caller needs to react differently.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// QuickJS could not create a runtime, context, or console binding.
    #[error("cannot initialize the QuickJS runtime: {message}")]
    Init {
        /// The binding's error message.
        message: String,
    },

    /// The entry file could not be canonicalized or read.
    #[error("cannot read `{}`: {source}", path.display())]
    Entry {
        /// The path the caller passed to [`Runtime::run_file`].
        path: PathBuf,
        /// The operating-system error.
        source: io::Error,
    },

    /// The entry file is not a format this runtime executes.
    #[error("cannot run `{}`: {reason}", path.display())]
    UnsupportedEntry {
        /// The path the caller passed to [`Runtime::run_file`].
        path: PathBuf,
        /// Why the entry file is not executed.
        reason: String,
    },

    /// The module could not be resolved, loaded, linked, or evaluated.
    ///
    /// This covers a failed import, a syntax error, an uncaught exception, a
    /// rejected top-level promise, and a top-level `await` that cannot settle.
    #[error("cannot evaluate `{module}`: {message}")]
    Module {
        /// The entry module identity.
        module: String,
        /// The QuickJS or loader message.
        message: String,
    },

    /// [`Runtime::run_file`] or [`Runtime::run_source`] was called after an
    /// entry module had already started.
    #[error("the runtime has already run an entry module; create a new runtime")]
    RuntimeConsumed,
}

/// An embeddable QuickJS runtime that executes one ECMAScript module entry.
///
/// Module identity is the entry file's canonical absolute path. Relative imports
/// resolve from the importing module, not from the process working directory.
/// One runtime evaluates one entry; create another for later work. The loader is
/// installed on a private QuickJS runtime, so multiple [`Runtime`] values do not
/// share modules or globals.
pub struct Runtime {
    context: Context,
    _runtime: rquickjs::Runtime,
    consumed: bool,
}

impl Runtime {
    /// Creates a QuickJS runtime, installs the file loader, and installs
    /// `console`.
    ///
    /// This performs no filesystem access. An error means QuickJS could not
    /// allocate its runtime or context, or the console binding could not be
    /// installed.
    pub fn new() -> Result<Self, Error> {
        let runtime = rquickjs::Runtime::new().map_err(|error| Error::Init {
            message: error.to_string(),
        })?;
        runtime.set_loader(FileResolver, FileLoader);
        let context = Context::full(&runtime).map_err(|error| Error::Init {
            message: error.to_string(),
        })?;
        context
            .with(|ctx| -> rquickjs::Result<()> {
                let write =
                    Function::new(ctx.clone(), |stream: String, text: String| {
                        match stream.as_str() {
                            "err" => eprintln!("{text}"),
                            _ => println!("{text}"),
                        }
                    })?;
                ctx.globals().set("__neoltjsWrite", write)?;
                ctx.eval::<(), _>(CONSOLE_SHIM)?;
                Ok(())
            })
            .map_err(|error| Error::Init {
                message: error.to_string(),
            })?;
        Ok(Self {
            context,
            _runtime: runtime,
            consumed: false,
        })
    }

    /// Runs one entry module file to completion.
    ///
    /// The runtime checks the name against the shared module policy before
    /// reading anything. `.js`, `.mjs`, and extensionless names run; `.ts` and
    /// `.mts` are rejected because this runtime has no type stripper, with the
    /// same extension policy the V8 runtime uses. The path is canonicalized
    /// before use so imports and error messages refer to one identity.
    ///
    /// This method blocks while it drives QuickJS's job queue. It writes nothing
    /// itself, but the module's `console` calls write to the process streams.
    ///
    /// A failure before evaluation (unsupported format, unreadable file) leaves
    /// the runtime unused. Once evaluation starts the runtime is consumed even
    /// if the module throws. A top-level `await` that needs an external event
    /// fails with [`Error::Module`] because this runtime has no host asynchrony.
    pub fn run_file(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        if self.consumed {
            return Err(Error::RuntimeConsumed);
        }
        let path = path.as_ref();
        let format = module_format(path).map_err(|reason| Error::UnsupportedEntry {
            path: path.to_path_buf(),
            reason,
        })?;
        if format == ModuleFormat::TypeScript {
            return Err(Error::UnsupportedEntry {
                path: path.to_path_buf(),
                reason: "TypeScript is not implemented in neoltjs yet; use a .js or .mjs file"
                    .to_string(),
            });
        }
        let canonical = std::fs::canonicalize(path).map_err(|source| Error::Entry {
            path: path.to_path_buf(),
            source,
        })?;
        let source = std::fs::read(&canonical).map_err(|source| Error::Entry {
            path: path.to_path_buf(),
            source,
        })?;
        let module = canonical
            .to_str()
            .ok_or_else(|| Error::UnsupportedEntry {
                path: path.to_path_buf(),
                reason: "the canonical path is not valid UTF-8".to_string(),
            })?
            .to_owned();

        self.consumed = true;
        self.evaluate(module, source)
    }

    /// Runs one ECMAScript module from memory.
    ///
    /// `name` is the module identity and the base for relative imports. Use an
    /// absolute path when the source may import a file; a relative name is
    /// resolved by the loader against the process working directory, which
    /// makes the import depend on where the host runs. This method does not read
    /// `name` and does not apply the extension policy to it; it does apply the
    /// policy to imported files.
    ///
    /// The single-entry rule and the top-level `await` limitation described on
    /// [`Runtime::run_file`] apply here too.
    pub fn run_source(&mut self, name: &str, source: &str) -> Result<(), Error> {
        if self.consumed {
            return Err(Error::RuntimeConsumed);
        }
        self.consumed = true;
        self.evaluate(name.to_owned(), source.as_bytes().to_vec())
    }

    /// Declares and evaluates one module, then drives its completion promise.
    fn evaluate(&self, name: String, source: Vec<u8>) -> Result<(), Error> {
        self.context.with(|ctx| {
            let promise = Module::evaluate(ctx.clone(), name.as_str(), source).map_err(|error| {
                Error::Module {
                    module: name.clone(),
                    message: caught_message(&ctx, error),
                }
            })?;
            match promise.finish::<()>() {
                Ok(()) => Ok(()),
                Err(rquickjs::Error::WouldBlock) => Err(Error::Module {
                    module: name.clone(),
                    message: "top-level await could not settle; host asynchronous work is not implemented in neoltjs yet".to_string(),
                }),
                Err(error) => Err(Error::Module {
                    module: name.clone(),
                    message: caught_message(&ctx, error),
                }),
            }
        })
    }
}

/// Formats a QuickJS error, retrieving and clearing any pending exception.
fn caught_message(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> String {
    match Err::<(), _>(error).catch(ctx) {
        Ok(()) => "unknown QuickJS error".to_string(),
        Err(caught) => caught.to_string(),
    }
}

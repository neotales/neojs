//! The embeddable runtime, its single-entry rule, and its error type.

use std::cell::RefCell;
use std::io;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neojs_module_policy::{ModuleFormat, module_format};
use rquickjs::{CatchResultExt, Context, Ctx, Function, Module, Value, qjs};

use crate::loader::{FileLoader, FileResolver};

/// The most QuickJS jobs one entry may execute.
///
/// QuickJS has no cancellation or fairness hook exposed by this binding. A
/// promise chain that keeps queueing work would otherwise spin forever inside
/// `run_file`. The budget covers the entry's top-level `await` and the drain
/// after the entry settles. It is deliberately generous for normal module
/// startup and still fails closed on a nonconverging loop.
const JOB_BUDGET: usize = 100_000;

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

/// Unhandled promise rejections recorded by QuickJS's host rejection tracker.
#[derive(Default)]
struct RejectionState {
    /// Rejections not yet marked handled, keyed by promise identity.
    unhandled: Vec<UnhandledRejection>,
}

struct UnhandledRejection {
    /// The promise object's address. It is used only as an identity token and
    /// is never dereferenced.
    promise: usize,
    message: String,
}

impl RejectionState {
    /// Records QuickJS's unhandled/handled notification.
    ///
    /// QuickJS calls the tracker with `handled == false` when a promise rejects
    /// without a handler and with `handled == true` if a handler is attached
    /// later. The pointer is stable while the promise is alive, which is when
    /// the second notification can occur.
    fn record(&mut self, promise: Value<'_>, handled: bool, message: String) {
        let promise = promise_identity(&promise);
        if handled {
            self.unhandled.retain(|entry| entry.promise != promise);
        } else if !self.unhandled.iter().any(|entry| entry.promise == promise) {
            self.unhandled.push(UnhandledRejection { promise, message });
        }
    }

    fn take(&mut self) -> Vec<UnhandledRejection> {
        std::mem::take(&mut self.unhandled)
    }
}

/// Returns a pointer-shaped identity for a promise without retaining it.
///
/// QuickJS promises are objects, so the object pointer is the identity QuickJS
/// itself uses in the two tracker notifications. The value is never
/// dereferenced, so a stale identity cannot cause an invalid access; a new
/// promise reusing an address can only be compared after the old promise is
/// gone and its notification is no longer pending.
fn promise_identity(promise: &Value<'_>) -> usize {
    debug_assert!(promise.is_object());
    // SAFETY: reading the pointer field of an object-tagged `JSValue` is what
    // QuickJS itself does. The pointer is kept only as a `usize` token.
    unsafe { promise.as_raw().u.ptr as usize }
}

/// Formats a rejection reason into a message suitable for an error.
///
/// An Error object's `stack` carries the message and source position. Other
/// values go through JavaScript `String`. Both can throw or be absent, so the
/// fallback is the binding's debug representation.
fn rejection_message<'js>(ctx: &Ctx<'js>, reason: &Value<'js>) -> String {
    if let Some(object) = reason.as_object() {
        let name = object.get::<_, String>("name").ok();
        let message = object.get::<_, String>("message").ok();
        if name.is_some() || message.is_some() {
            let head = match (name.as_deref(), message.as_deref()) {
                (Some(name), Some(message)) if !name.is_empty() => format!("{name}: {message}"),
                (_, Some(message)) => message.to_string(),
                (Some(name), None) => name.to_string(),
                (None, None) => String::new(),
            };
            if let Ok(stack) = object.get::<_, String>("stack") {
                if head.is_empty() || stack.contains(&head) {
                    return stack;
                }
                return format!("{head}\n{stack}");
            }
            if !head.is_empty() {
                return head;
            }
        }
    }
    if let Ok(string) = ctx.globals().get::<_, Function>("String")
        && let Ok(text) = string.call::<_, String>((reason.clone(),))
    {
        return text;
    }
    format!("{reason:?}")
}

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

    /// One or more promises rejected without a handler by the time the entry
    /// and its queued jobs settled.
    ///
    /// The details contain each remaining reason, in rejection order. A
    /// rejection that gains a handler later is removed and is not reported.
    #[error("cannot evaluate `{module}`: {count} unhandled promise rejection(s): {details}")]
    UnhandledRejection {
        /// The entry module identity.
        module: String,
        /// How many rejections remained unhandled.
        count: usize,
        /// Formatted reasons, one per line.
        details: String,
    },

    /// The entry and its queued jobs kept producing work past the job budget.
    ///
    /// The runtime is abandoned; create a new one. This is a fail-closed guard
    /// against a nonconverging promise loop, not a time or memory limit.
    #[error("cannot evaluate `{module}`: job queue did not settle after {jobs} jobs; aborting")]
    NonConvergingJobs {
        /// The entry module identity.
        module: String,
        /// Jobs executed when the budget was exceeded.
        jobs: usize,
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
    rejections: Rc<RefCell<RejectionState>>,
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
        let rejections = Rc::new(RefCell::new(RejectionState::default()));
        let tracker_rejections = Rc::clone(&rejections);
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            move |ctx: Ctx<'_>, promise: Value<'_>, reason: Value<'_>, handled: bool| {
                let message = if handled {
                    String::new()
                } else {
                    rejection_message(&ctx, &reason)
                };
                tracker_rejections
                    .borrow_mut()
                    .record(promise, handled, message);
            },
        )));
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
            rejections,
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
    /// After the entry promise settles it keeps running queued jobs until the
    /// queue is empty, so a trailing `.then`, an unawaited dynamic import, or a
    /// module side effect is not silently lost. At most 100,000 jobs are
    /// executed; a chain that keeps queueing work fails with
    /// [`Error::NonConvergingJobs`] instead of spinning forever.
    ///
    /// A promise that rejects without a handler by the time the queue is empty
    /// fails with [`Error::UnhandledRejection`], including the reason and stack
    /// when QuickJS supplies them. A rejection that gains a handler before the
    /// drain finishes is not reported.
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
    /// `name` is the module identity and the base for relative imports. The
    /// entry source is not read from disk and its own extension is not checked.
    /// If `name` is an absolute path, imports resolve from its parent directory
    /// even when the process working directory changes. A relative `name` has
    /// no filesystem identity: relative imports are joined to its parent and
    /// therefore depend on the process working directory at load time. Pass an
    /// absolute path when the source may import another file; a name with no
    /// directory (for example `"memory.js"`) resolves imports from the process
    /// working directory.
    ///
    /// Imported files still go through the loader and the shared extension
    /// policy. Preflight does not apply to the in-memory entry because there is
    /// no file to check. Once this method begins evaluation the runtime is
    /// consumed, including when evaluation, the job drain, or an unhandled
    /// rejection fails. The top-level `await`, job-drain, and rejection rules
    /// described on [`Runtime::run_file`] apply here too.
    pub fn run_source(&mut self, name: &str, source: &str) -> Result<(), Error> {
        if self.consumed {
            return Err(Error::RuntimeConsumed);
        }
        self.consumed = true;
        self.evaluate(name.to_owned(), source.as_bytes().to_vec())
    }

    /// Declares and evaluates one module, drives its top-level `await`, drains
    /// the remaining job queue, and reports unhandled rejections.
    fn evaluate(&self, name: String, source: Vec<u8>) -> Result<(), Error> {
        let result = self.context.with(|ctx| {
            let promise =
                Module::evaluate(ctx.clone(), name.as_str(), source).map_err(|error| {
                    Error::Module {
                        module: name.clone(),
                        message: caught_message(&ctx, error),
                    }
                })?;
            drive_entry(&ctx, &name, &promise)
        });
        result?;
        self.report_unhandled_rejections(name)
    }

    /// Turns rejections still unhandled after the drain into one error.
    fn report_unhandled_rejections(&self, module: String) -> Result<(), Error> {
        let unhandled = self.rejections.borrow_mut().take();
        if unhandled.is_empty() {
            return Ok(());
        }
        let count = unhandled.len();
        let details = unhandled
            .iter()
            .enumerate()
            .map(|(index, rejection)| format!("{}) {}", index + 1, rejection.message.trim()))
            .collect::<Vec<_>>()
            .join("\n");
        Err(Error::UnhandledRejection {
            module,
            count,
            details,
        })
    }
}

/// One step of the QuickJS job queue.
enum JobStep {
    /// No job was queued.
    Empty,
    /// One job ran successfully.
    Ran,
    /// One job threw; the message is already retrieved and cleared.
    Threw(String),
}

/// Executes one QuickJS job and reports whether it ran or threw.
///
/// `Ctx::execute_pending_job` clears a thrown job's exception before returning,
/// so it cannot distinguish a successful job from a failed one. That would make
/// `queueMicrotask(() => { throw ... })` exit successfully. This calls the same
/// QuickJS function directly and retrieves the exception itself.
///
/// # Safety
///
/// The raw call is safe here because the caller holds the runtime lock through
/// `Context::with` and this runtime has exactly one context, so the context
/// QuickJS reports for the job is the context that owns `ctx`.
fn execute_job(ctx: &Ctx<'_>) -> JobStep {
    let runtime = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let mut job_context = MaybeUninit::uninit();
    // SAFETY: the runtime is locked and valid for the duration of the call.
    let result = unsafe { qjs::JS_ExecutePendingJob(runtime, job_context.as_mut_ptr()) };
    if result == 0 {
        return JobStep::Empty;
    }
    if result > 0 {
        return JobStep::Ran;
    }

    // SAFETY: QuickJS writes the job's context when it returns a nonzero
    // result. The debug assertion documents the single-context invariant.
    let job_context = unsafe { job_context.assume_init() };
    debug_assert_eq!(
        job_context,
        ctx.as_raw().as_ptr(),
        "the runtime must have exactly one context"
    );
    let reason = ctx.catch();
    JobStep::Threw(rejection_message(ctx, &reason))
}

/// Drives the entry promise and then the jobs it left behind.
///
/// QuickJS's `Promise::finish` stops as soon as the entry promise settles. A
/// `.then` callback, a dynamic import, or a module side effect queued just
/// before settlement would then be silently dropped. This loop continues until
/// the queue is empty, which is the only completion point QuickJS exposes
/// without host asynchrony.
///
/// The same budget bounds both phases. If the queue still produces a job after
/// the budget is exhausted, the call fails closed instead of spinning. The
/// runtime is single-entry, so the caller cannot meaningfully continue it.
fn drive_entry(ctx: &Ctx<'_>, module: &str, promise: &rquickjs::Promise<'_>) -> Result<(), Error> {
    let mut jobs = 0usize;
    loop {
        if let Some(result) = promise.result::<()>() {
            result.map_err(|error| Error::Module {
                module: module.to_owned(),
                message: caught_message(ctx, error),
            })?;
            break;
        }
        match execute_job(ctx) {
            JobStep::Empty => {
                return Err(Error::Module {
                    module: module.to_owned(),
                    message: "top-level await could not settle; host asynchronous work is not implemented in neoltjs yet".to_string(),
                });
            }
            JobStep::Ran => {
                jobs += 1;
                if jobs > JOB_BUDGET {
                    return Err(Error::NonConvergingJobs {
                        module: module.to_owned(),
                        jobs,
                    });
                }
            }
            JobStep::Threw(message) => {
                return Err(Error::Module {
                    module: module.to_owned(),
                    message: format!("uncaught exception in queued job: {message}"),
                });
            }
        }
    }

    loop {
        match execute_job(ctx) {
            JobStep::Empty => break,
            JobStep::Ran => {
                jobs += 1;
                if jobs > JOB_BUDGET {
                    return Err(Error::NonConvergingJobs {
                        module: module.to_owned(),
                        jobs,
                    });
                }
            }
            JobStep::Threw(message) => {
                return Err(Error::Module {
                    module: module.to_owned(),
                    message: format!("uncaught exception in queued job: {message}"),
                });
            }
        }
    }
    Ok(())
}

/// Formats a QuickJS error, retrieving and clearing any pending exception.
fn caught_message(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> String {
    match Err::<(), _>(error).catch(ctx) {
        Ok(()) => "unknown QuickJS error".to_string(),
        Err(caught) => caught.to_string(),
    }
}

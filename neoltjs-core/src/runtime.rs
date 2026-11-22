//! The embeddable runtime, its single-entry rule, and its error type.

use std::cell::RefCell;
use std::io;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neojs_module_policy::{ModuleFormat, module_format};
use rquickjs::{CatchResultExt, Context, Ctx, Function, Module, Persistent, Value, qjs};

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
    /// Rejections not yet marked handled. The promise and reason are rooted, so
    /// a handled notification cannot be confused with a later promise that
    /// reuses a freed address.
    unhandled: Vec<RetainedRejection>,
}

/// A rejection whose promise and reason stay alive until it is handled or
/// reported.
struct RetainedRejection {
    /// The rooted promise's object address. It is stable while the promise is
    /// retained and is used only as an identity token.
    key: usize,
    /// Keeps the promise alive and gives it a stable address.
    _promise: Persistent<Value<'static>>,
    /// Keeps the reason alive so formatting can be deferred without invoking
    /// user code in the tracker callback.
    reason: Persistent<Value<'static>>,
}

impl RejectionState {
    /// Records QuickJS's unhandled/handled notification.
    ///
    /// This runs inside the engine's reject/handle path, so it deliberately does
    /// not read properties, call `String`, or otherwise execute user code. It
    /// only roots the two values and records their identity. QuickJS calls the
    /// tracker with `handled == false` when a promise rejects without a handler
    /// and with `handled == true` if a handler is attached later.
    fn record<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        promise: Value<'js>,
        reason: Value<'js>,
        handled: bool,
    ) {
        let key = promise_identity(&promise);
        if handled {
            self.unhandled.retain(|entry| entry.key != key);
        } else if !self.unhandled.iter().any(|entry| entry.key == key) {
            self.unhandled.push(RetainedRejection {
                key,
                _promise: Persistent::save(ctx, promise),
                reason: Persistent::save(ctx, reason),
            });
        }
    }

    fn take(&mut self) -> Vec<RetainedRejection> {
        std::mem::take(&mut self.unhandled)
    }

    /// Drops every root while the context is still alive.
    fn clear(&mut self) {
        self.unhandled.clear();
    }
}

/// Returns a pointer-shaped identity for a promise.
///
/// The value is retained by [`RejectionState`] for as long as the entry exists,
/// so the address cannot be reused by another promise before the handled
/// notification is processed. The value is never dereferenced.
fn promise_identity(promise: &Value<'_>) -> usize {
    debug_assert!(promise.is_object());
    // SAFETY: reading the pointer field of an object-tagged `JSValue` is what
    // QuickJS itself does. The pointer is kept only as a `usize` token.
    unsafe { promise.as_raw().u.ptr as usize }
}

/// Clears a pending JavaScript exception, if any.
///
/// Property getters and `String` can throw while formatting. The exception must
/// not be left pending for later host work to mistake for a fresh failure.
fn clear_pending_exception(ctx: &Ctx<'_>) {
    if ctx.has_exception() {
        ctx.catch();
    }
}

/// Reads a string property for diagnostics, clearing a getter's exception.
fn diagnostic_string<'js>(
    ctx: &Ctx<'js>,
    object: &rquickjs::Object<'js>,
    key: &str,
) -> Option<String> {
    match object.get::<_, String>(key) {
        Ok(value) => Some(value),
        Err(_) => {
            clear_pending_exception(ctx);
            None
        }
    }
}

/// Formats a rejection reason after the run has finished.
///
/// This is deliberately deferred: reading `name`, `message`, or `stack`, or
/// calling `String`, can execute user code. By the time it runs, the unhandled
/// set has already been snapshotted, so a getter that attaches a handler cannot
/// retroactively change the report. Every failure is cleared and falls back to a
/// type-only description that executes no user code.
fn rejection_message<'js>(ctx: &Ctx<'js>, reason: &Value<'js>) -> String {
    if let Some(object) = reason.as_object() {
        let name = diagnostic_string(ctx, object, "name");
        let message = diagnostic_string(ctx, object, "message");
        if name.is_some() || message.is_some() {
            let head = match (name.as_deref(), message.as_deref()) {
                (Some(name), Some(message)) if !name.is_empty() => format!("{name}: {message}"),
                (_, Some(message)) => message.to_string(),
                (Some(name), None) => name.to_string(),
                (None, None) => String::new(),
            };
            if let Some(stack) = diagnostic_string(ctx, object, "stack") {
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
    if let Ok(string) = ctx.globals().get::<_, Function>("String") {
        match string.call::<_, String>((reason.clone(),)) {
            Ok(text) => return text,
            Err(_) => clear_pending_exception(ctx),
        }
    } else {
        clear_pending_exception(ctx);
    }
    format!("<unformattable rejection reason: {:?}>", reason.type_of())
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

    /// The entry and its queued jobs kept producing work at the job budget.
    ///
    /// The runtime is abandoned; create a new one. This is a fail-closed guard
    /// against a nonconverging promise loop, not a time or memory limit. The
    /// budget is checked before the next job, so no over-budget side effect is
    /// executed.
    #[error("cannot evaluate `{module}`: job queue did not settle after {jobs} jobs; aborting")]
    NonConvergingJobs {
        /// The entry module identity.
        module: String,
        /// Jobs executed when another job was still pending.
        jobs: usize,
    },

    /// [`Runtime::run_file`] or [`Runtime::run_source`] was called after an
    /// entry module had already started.
    #[error("the runtime has already run an entry module; create a new runtime")]
    RuntimeConsumed,
}

/// Installs a test-only way to force QuickJS garbage collection.
///
/// Address-reuse regressions need GC at a deterministic point inside a script.
/// This global exists only when the crate's own unit tests are compiled; it is
/// absent from normal builds and from integration tests.
#[cfg(test)]
fn install_test_gc(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let runtime = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) } as usize;
    let collect = Function::new(ctx.clone(), move || {
        // SAFETY: the runtime stays alive while this test-only global is
        // reachable, and QuickJS allows collection from a host callback.
        unsafe { qjs::JS_RunGC(runtime as *mut qjs::JSRuntime) };
    })?;
    ctx.globals().set("__neoltjsGc", collect)
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
                tracker_rejections
                    .borrow_mut()
                    .record(&ctx, promise, reason, handled);
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
                #[cfg(test)]
                install_test_gc(&ctx)?;
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
        let details = self.context.with(move |ctx| {
            clear_pending_exception(&ctx);
            let details = unhandled
                .iter()
                .enumerate()
                .map(|(index, rejection)| {
                    let reason = match rejection.reason.clone().restore(&ctx) {
                        Ok(reason) => rejection_message(&ctx, &reason),
                        Err(_) => "<reason unavailable>".to_string(),
                    };
                    clear_pending_exception(&ctx);
                    format!("{}) {}", index + 1, reason.trim())
                })
                .collect::<Vec<_>>()
                .join("\n");
            // Drop the roots while the context and runtime are alive.
            drop(unhandled);
            details
        });
        Err(Error::UnhandledRejection {
            module,
            count,
            details,
        })
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // The tracker closure holds another `Rc` to this state, so clearing it
        // here is what releases rooted promises and reasons while the context
        // is still alive. Dropping them later with the runtime would call into
        // a freed context.
        self.rejections.borrow_mut().clear();
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

/// Returns the runtime pointer for operations that need the raw C API.
fn runtime_ptr(ctx: &Ctx<'_>) -> *mut qjs::JSRuntime {
    // SAFETY: `ctx` is valid for the duration of the call.
    unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) }
}

/// Returns whether QuickJS has at least one queued job.
///
/// Checking before execution is what lets an entry that uses exactly the job
/// budget succeed: the budget is only enforced when another job is known to be
/// waiting, so no extra job runs merely to discover the queue is empty.
fn is_job_pending(ctx: &Ctx<'_>) -> bool {
    // SAFETY: the runtime pointer comes from a live context.
    unsafe { qjs::JS_IsJobPending(runtime_ptr(ctx)) }
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
    let runtime = runtime_ptr(ctx);
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
        if !is_job_pending(ctx) {
            return Err(Error::Module {
                module: module.to_owned(),
                message: "top-level await could not settle; host asynchronous work is not implemented in neoltjs yet".to_string(),
            });
        }
        if jobs >= JOB_BUDGET {
            return Err(Error::NonConvergingJobs {
                module: module.to_owned(),
                jobs,
            });
        }
        match execute_job(ctx) {
            JobStep::Ran => jobs += 1,
            JobStep::Threw(message) => {
                return Err(Error::Module {
                    module: module.to_owned(),
                    message: format!("uncaught exception in queued job: {message}"),
                });
            }
            JobStep::Empty => {
                return Err(Error::Module {
                    module: module.to_owned(),
                    message: "job queue reported pending work but no job ran".to_string(),
                });
            }
        }
    }

    loop {
        if !is_job_pending(ctx) {
            break;
        }
        if jobs >= JOB_BUDGET {
            return Err(Error::NonConvergingJobs {
                module: module.to_owned(),
                jobs,
            });
        }
        match execute_job(ctx) {
            JobStep::Ran => jobs += 1,
            JobStep::Threw(message) => {
                return Err(Error::Module {
                    module: module.to_owned(),
                    message: format!("uncaught exception in queued job: {message}"),
                });
            }
            JobStep::Empty => {
                return Err(Error::Module {
                    module: module.to_owned(),
                    message: "job queue reported pending work but no job ran".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freed_promise_address_reuse_does_not_remove_a_retained_rejection() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                r#"
const count = 32;
for (let i = 0; i < count; i++) {
    Promise.reject(new Error(`unhandled ${i}`));
}
__neoltjsGc();
for (let i = 0; i < count; i++) {
    Promise.reject(new Error(`handled ${i}`)).catch(() => {});
}
"#,
            )
            .expect_err("unhandled rejections");
        let Error::UnhandledRejection { count, details, .. } = error else {
            panic!("expected an unhandled rejection, got {error:?}");
        };
        assert_eq!(count, 32);
        assert!(details.contains("unhandled 0"), "{details}");
        assert!(details.contains("unhandled 31"), "{details}");
        assert!(!details.contains("Error: handled"), "{details}");
    }

    #[test]
    fn formatting_a_reason_that_attaches_a_handler_keeps_the_report() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                r#"
const reason = {
    get name() {
        promise.catch(() => {});
        return "LateHandled";
    },
    get message() {
        return "attached during formatting";
    },
};
const promise = Promise.reject(reason);
"#,
            )
            .expect_err("unhandled rejection");
        let Error::UnhandledRejection { count, details, .. } = error else {
            panic!("expected an unhandled rejection, got {error:?}");
        };
        assert_eq!(count, 1);
        assert!(details.contains("LateHandled"), "{details}");
        assert!(details.contains("attached during formatting"), "{details}");
    }

    #[test]
    fn formatting_clears_throwing_getters_and_string() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                r#"
const reason = {};
Object.defineProperty(reason, "name", {
    get() { throw new Error("name getter"); },
});
Object.defineProperty(reason, "message", {
    get() { throw new Error("message getter"); },
});
Object.defineProperty(reason, "stack", {
    get() { throw new Error("stack getter"); },
});
globalThis.String = () => { throw new Error("String replaced"); };
Promise.reject(reason);
"#,
            )
            .expect_err("unhandled rejection");
        assert!(
            matches!(error, Error::UnhandledRejection { count: 1, .. }),
            "{error:?}"
        );
        let pending = runtime.context.with(|ctx| ctx.has_exception());
        assert!(!pending, "formatting left an exception pending");
    }

    #[test]
    fn exactly_the_job_budget_succeeds_when_the_queue_is_empty() {
        let mut runtime = Runtime::new().expect("runtime");
        let source = format!(
            "for (let i = 0; i < {JOB_BUDGET}; i++) {{ Promise.resolve().then(() => {{}}); }}"
        );
        runtime
            .run_source("memory.js", &source)
            .expect("exactly the budget");
    }

    #[test]
    fn the_job_budget_fails_before_an_extra_job_runs() {
        let mut runtime = Runtime::new().expect("runtime");
        let source = format!(
            r#"
for (let i = 0; i < {JOB_BUDGET}; i++) {{
    Promise.resolve().then(() => {{}});
}}
Promise.resolve().then(() => {{ globalThis.__extra = 1; }});
"#
        );
        let error = runtime
            .run_source("memory.js", &source)
            .expect_err("over budget");
        assert!(
            matches!(error, Error::NonConvergingJobs { jobs, .. } if jobs == JOB_BUDGET),
            "{error:?}"
        );
        let extra_ran = runtime
            .context
            .with(|ctx| ctx.eval::<bool, _>("globalThis.__extra !== undefined"))
            .expect("inspect marker");
        assert!(!extra_ran, "the over-budget job ran before failing");
    }
}

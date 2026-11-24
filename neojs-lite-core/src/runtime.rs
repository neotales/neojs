//! The embeddable runtime, its single-entry rule, and its error type.

use std::cell::RefCell;
use std::io;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use neojs_module_policy::{ModuleFormat, module_format};
use rquickjs::{
    CatchResultExt, CaughtError, Context, Ctx, Function, Module, Persistent, Value, qjs,
};
#[cfg(feature = "typescript")]
use url::Url;

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
    log: (...args) => globalThis.__neojsLiteWrite("out", args.map((value) => String(value)).join(" ")),
    error: (...args) => globalThis.__neojsLiteWrite("err", args.map((value) => String(value)).join(" ")),
});
"#;

/// Unhandled promise rejections recorded by QuickJS's host rejection tracker.
#[derive(Default)]
pub(super) struct RejectionState {
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
    #[cfg_attr(not(feature = "neo-test"), allow(dead_code))]
    promise: Persistent<Value<'static>>,
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
                promise: Persistent::save(ctx, promise),
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

/// Reads an own data property that is already a string primitive.
///
/// This uses `JS_GetOwnProperty`, which returns a descriptor without invoking a
/// getter. Only an ordinary Error object is inspected, so no proxy trap runs.
/// An accessor (including a user-defined `message` or `stack`) is ignored, and
/// any descriptor value or exception is freed/cleared before returning.
fn own_string_property(
    ctx: &Ctx<'_>,
    object: qjs::JSValue,
    name: &std::ffi::CStr,
) -> Option<String> {
    let raw = ctx.as_raw().as_ptr();
    let atom = unsafe { qjs::JS_NewAtom(raw, name.as_ptr()) };
    if atom == qjs::JS_ATOM_NULL {
        return None;
    }
    let mut descriptor = MaybeUninit::<qjs::JSPropertyDescriptor>::uninit();
    let found = unsafe { qjs::JS_GetOwnProperty(raw, descriptor.as_mut_ptr(), object, atom) };
    unsafe { qjs::JS_FreeAtom(raw, atom) };
    if found < 0 {
        clear_pending_exception(ctx);
        return None;
    }
    if found == 0 {
        return None;
    }

    let descriptor = unsafe { descriptor.assume_init() };
    let is_data = (descriptor.flags as u32 & qjs::JS_PROP_GETSET) == 0;
    let text = if is_data && unsafe { qjs::JS_IsString(descriptor.value) } {
        string_primitive(ctx, descriptor.value)
    } else {
        None
    };
    unsafe {
        qjs::JS_FreeValue(raw, descriptor.value);
        qjs::JS_FreeValue(raw, descriptor.getter);
        qjs::JS_FreeValue(raw, descriptor.setter);
    }
    text
}

/// Converts a string primitive to Rust without invoking conversion hooks.
fn string_primitive(ctx: &Ctx<'_>, value: qjs::JSValue) -> Option<String> {
    let mut len = 0;
    let raw = ctx.as_raw().as_ptr();
    let ptr = unsafe { qjs::JS_ToCStringLen2(raw, &mut len, value, false) };
    if ptr.is_null() {
        clear_pending_exception(ctx);
        return None;
    }
    // SAFETY: QuickJS returns a NUL-terminated buffer of `len` bytes, valid
    // until JS_FreeCString. The bytes may contain arbitrary UTF-8 input.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let text = String::from_utf8_lossy(bytes).into_owned();
    unsafe { qjs::JS_FreeCString(raw, ptr) };
    Some(text)
}

/// Formats a primitive rejection reason using only intrinsic value access.
fn primitive_reason(ctx: &Ctx<'_>, reason: &Value<'_>) -> Option<String> {
    match reason.type_of() {
        rquickjs::Type::String => string_primitive(ctx, reason.as_raw()),
        rquickjs::Type::Int => reason.as_int().map(|value| value.to_string()),
        rquickjs::Type::Float => reason.as_float().map(|value| value.to_string()),
        rquickjs::Type::Bool => reason.as_bool().map(|value| value.to_string()),
        rquickjs::Type::Null => Some("null".to_string()),
        rquickjs::Type::Undefined => Some("undefined".to_string()),
        _ => None,
    }
}

/// Formats an Error object from own data properties only.
///
/// `JS_IsError` is a class-id check and does not call user code. Error objects
/// are ordinary objects in QuickJS, so the descriptor lookup cannot run a proxy
/// trap or getter. The result is `Error: <message>` plus stack when both are
/// plain string data.
fn error_reason(ctx: &Ctx<'_>, reason: &Value<'_>) -> Option<String> {
    if !reason.is_error() || reason.is_proxy() {
        return None;
    }
    let message = own_string_property(ctx, reason.as_raw(), c"message")?;
    let head = if message.is_empty() {
        "Error".to_string()
    } else {
        format!("Error: {message}")
    };
    match own_string_property(ctx, reason.as_raw(), c"stack") {
        Some(stack) if message.is_empty() || stack.contains(&message) => Some(stack),
        Some(stack) => Some(format!("{head}\n{stack}")),
        None => Some(head),
    }
}

/// Formats a rejection reason after the run has finished.
///
/// This never invokes arbitrary JavaScript: no getters, proxy traps,
/// `toString`, or `String` conversion. Primitive values are read directly, an
/// ordinary Error's own data `message`/`stack` strings are used when present,
/// and everything else becomes a type-only description. Any exception raised
/// by QuickJS while inspecting a descriptor is cleared.
fn rejection_message(ctx: &Ctx<'_>, reason: &Value<'_>) -> String {
    if let Some(text) = primitive_reason(ctx, reason) {
        return text;
    }
    if let Some(text) = error_reason(ctx, reason) {
        return text;
    }
    if reason.is_error() {
        "<rejected Error with no readable message>".to_string()
    } else {
        format!("<rejected {:?}>", reason.type_of())
    }
}

/// A failure while constructing or running a QuickJS program.
///
/// The message text is for people and may change; match on the variants when a
/// caller needs to react differently.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// QuickJS could not create a runtime, context, or console/Neo binding.
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

    /// The entry file is TypeScript that cannot be reduced to ECMAScript.
    ///
    /// The erasable subset is supported when the `typescript` feature is
    /// enabled. Invalid TypeScript, and valid TypeScript that needs a transform
    /// (for example an `enum`, a `namespace`, or a parameter property), is
    /// rejected with the stripper's reason. This is not a type error: type
    /// stripping does not type check.
    ///
    /// The failure happens before QuickJS loads the module, so the runtime
    /// remains unused and can run another entry module. An imported `.ts` file
    /// that fails to strip returns [`Error::Module`] instead, and that load
    /// consumes the runtime.
    #[error("cannot compile `{}`: {reason}", path.display())]
    TypeScript {
        /// The path the caller passed to [`Runtime::run_file`].
        path: PathBuf,
        /// Why the file could not be reduced to ECMAScript.
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

    /// The `test` runner returned an unusable report or failed unexpectedly.
    ///
    /// This is a runner fault, not a test failure. Test failures are reported in
    /// [`TestReport`] and make the command exit non-zero; this variant means
    /// the counts could not be trusted, so they are not returned.
    #[error("test runner failed: {0}")]
    TestRunner(String),
}

/// Cumulative counts from the `neo:test` built-in in one runtime.
///
/// `total` counts executed tests, including pending ones. `passed`, `failed`,
/// and `pending` partition it. `unhandled` is separate: it counts unhandled
/// Promise rejections observed while the test module was active. A run is
/// successful only when [`TestReport::all_passed`] is true, which requires no
/// failure, no pending test, no unhandled rejection, and no unsettled work.
///
/// The counts are cumulative across explicit `run()` calls and the host's
/// final call, so catching a failure from `run()` cannot hide it from the host.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TestReport {
    /// Tests executed so far, including pending tests.
    pub total: u32,
    /// Tests that returned without throwing or rejecting.
    pub passed: u32,
    /// Tests that threw, rejected, or returned an unsupported value.
    pub failed: u32,
    /// Tests registered without a callback.
    pub pending: u32,
    /// Unhandled Promise rejections observed while the test module was active.
    pub unhandled: u32,
    /// 1 when the job queue still reported pending work after the bounded
    /// settle rounds, otherwise 0. An unsettled verdict is a failure.
    pub unsettled: u32,
    /// One summary line per failure, in execution order.
    pub failures: Vec<String>,
}

impl TestReport {
    /// True when no test failed, none is pending, no rejection was unhandled,
    /// and no job-queue work was left unsettled.
    pub fn all_passed(&self) -> bool {
        self.failed == 0 && self.pending == 0 && self.unhandled == 0 && self.unsettled == 0
    }
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
    ctx.globals().set("__neojsLiteGc", collect)
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
    /// `console` and the native `Neo` introspection global.
    ///
    /// `Neo.engine` is `"neo-lite"`. With the default `neo-assert`, `neo-chars`,
    /// `neo-fs`, `neo-os`, `neo-strings`, `neo-test`, and `neo-uuid` features
    /// the registry contains the canonical modules `neo:assert`, `neo:chars`,
    /// `neo:fs`, `neo:os`, `neo:strings`, `neo:test`, `neo:uuid`, `neo:uuid/v4`,
    /// and `neo:uuid/v7`.
    /// `Neo.getBuiltInModulesList()` returns one frozen array of canonical
    /// specifiers, and `Neo.getBuiltInModule(specifier)` returns the frozen
    /// registry object whose named exports share identity with the module
    /// namespace. The namespace itself is a distinct object. A bare name, another
    /// scheme, or a non-string argument does not return a module; a non-string
    /// name throws `TypeError`. With default features off the registry and list
    /// are empty. It does not claim the V8 runtime's full built-in surface.
    /// This performs no filesystem access. An error means QuickJS could not
    /// allocate its runtime or context, or the native surface could not be
    /// installed.
    pub fn new() -> Result<Self, Error> {
        Self::build(FileResolver::new())
    }

    /// Creates a QuickJS runtime with an explicit `neo.jsonc`
    /// [`neojs_manifest::ImportResolver`].
    ///
    /// [`Runtime::new`] performs zero implicit manifest discovery. Embedders
    /// opt in by enabling the `manifest` feature, depending on
    /// `neojs-manifest`, calling `neojs_manifest::discover_manifest` (and
    /// `Workspace::load` when `workspaces` is non-empty), and passing
    /// `ImportResolver::from_manifest` or `ImportResolver::from_workspace`
    /// here. Local `imports` and workspace `@scope/pkg` references resolve to
    /// local `.js`/`.mjs`/`.ts`/`.mts` modules (with `.ts`/`.mts` stripped when
    /// the `typescript` feature is enabled); remote targets (`https:`, `gh:`,
    /// `jsr:`, `npm:`) are rejected because remote loading is not supported in
    /// `neojs-lite-core`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use neojs_lite_core::{ImportResolver, Runtime};
    /// use neojs_manifest::{Workspace, discover_manifest};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// if let Some(found) = discover_manifest(".", None)? {
    ///     let resolver = if found.manifest.workspaces.is_empty() {
    ///         ImportResolver::from_manifest(&found)
    ///     } else {
    ///         let ws = Workspace::load(&found.path)?;
    ///         ImportResolver::from_workspace(&ws, None)?
    ///     };
    ///     Runtime::with_import_resolver(resolver)?.run_file("main.js")?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "manifest")]
    pub fn with_import_resolver(resolver: neojs_manifest::ImportResolver) -> Result<Self, Error> {
        Self::build(FileResolver::with_import_resolver(resolver))
    }

    fn build(resolver: FileResolver) -> Result<Self, Error> {
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
        runtime.set_loader(resolver, FileLoader);
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
                ctx.globals().set("__neojsLiteWrite", write)?;
                ctx.eval::<(), _>(CONSOLE_SHIM)?;
                crate::neo::install(
                    &ctx,
                    #[cfg(feature = "neo-test")]
                    &rejections,
                )?;
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
    /// `.mts` also run when the `typescript` feature is enabled (and are
    /// rejected with [`Error::UnsupportedEntry`] when it is disabled), with the
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
    /// fails with [`Error::UnhandledRejection`]. The report formats primitive
    /// values directly and an Error's own `message`/`stack` data properties when
    /// present; it never invokes getters, proxy traps, `toString`, or `String`.
    /// A rejection that gains a handler before the drain finishes is not
    /// reported. An entry or queued job that throws uses the same formatter;
    /// native loader and engine errors keep their own messages.
    ///
    /// A failure before evaluation (unsupported format, unreadable file, or
    /// TypeScript strip failure) leaves the runtime unused. Once evaluation
    /// starts the runtime is consumed even if the module throws. A top-level
    /// `await` that needs an external event fails with [`Error::Module`]
    /// because this runtime has no host asynchrony.
    pub fn run_file(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        if self.consumed {
            return Err(Error::RuntimeConsumed);
        }
        let path = path.as_ref();
        let format = module_format(path).map_err(|reason| Error::UnsupportedEntry {
            path: path.to_path_buf(),
            reason,
        })?;
        #[cfg(not(feature = "typescript"))]
        if format == ModuleFormat::TypeScript {
            return Err(Error::UnsupportedEntry {
                path: path.to_path_buf(),
                reason: "TypeScript is not enabled in this build; enable the `typescript` feature or use a .js or .mjs file"
                    .to_string(),
            });
        }
        let canonical = std::fs::canonicalize(path).map_err(|source| Error::Entry {
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
        let source = match format {
            ModuleFormat::JavaScript => {
                std::fs::read(&canonical).map_err(|source| Error::Entry {
                    path: path.to_path_buf(),
                    source,
                })?
            }
            ModuleFormat::TypeScript => {
                #[cfg(feature = "typescript")]
                {
                    let specifier =
                        Url::from_file_path(&canonical).map_err(|()| Error::UnsupportedEntry {
                            path: path.to_path_buf(),
                            reason:
                                "the path cannot be represented as a file URL; use a UTF-8 path"
                                    .to_string(),
                        })?;
                    let text =
                        std::fs::read_to_string(&canonical).map_err(|source| Error::Entry {
                            path: path.to_path_buf(),
                            source,
                        })?;
                    neojs_module_policy::typescript::strip(&specifier, text)
                        .map_err(|reason| Error::TypeScript {
                            path: path.to_path_buf(),
                            reason,
                        })?
                        .into_bytes()
                }
                #[cfg(not(feature = "typescript"))]
                unreachable!("rejected before canonicalization when typescript is disabled")
            }
        };

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
            drive_promise::<()>(
                &ctx,
                &name,
                &promise,
                "top-level await could not settle; host asynchronous work is not implemented in neojs-lite yet",
                true,
            )?;
            #[cfg(feature = "neo-test")]
            flush_unhandled_rejections(&ctx, &self.rejections).map_err(|error| Error::Module {
                module: name.clone(),
                message: caught_message(&ctx, error),
            })?;
            Ok(())
        });
        match result {
            Ok(()) => self.report_unhandled_rejections(name),
            Err(error) => {
                self.clear_rejection_roots();
                Err(error)
            }
        }
    }

    /// Runs the tests registered with `neo:test` and returns their cumulative report.
    ///
    /// Tests may be registered at top level or after a top-level await. A failing
    /// test, a pending test, an unhandled rejection, or remaining unsettled job
    /// queue work is recorded in the returned [`TestReport`], not as an `Err`.
    /// `Err` means the runner itself failed, could not settle a test Promise, or
    /// returned an inconsistent report.
    pub fn run_tests(&mut self) -> Result<TestReport, Error> {
        #[cfg(not(feature = "neo-test"))]
        {
            Err(Error::TestRunner(
                "the `neo-test` feature is not enabled in this build".to_string(),
            ))
        }
        #[cfg(feature = "neo-test")]
        {
            const TEST_RUN_SCRIPT: &str =
                r#"Neo.getBuiltInModule("neo:test").run({ throwOnFailure: false, settle: true })"#;

            let result = self.context.with(|ctx| {
                let value: Value<'_> = ctx
                    .eval(TEST_RUN_SCRIPT)
                    .map_err(|error| Error::TestRunner(caught_message(&ctx, error)))?;
                let promise = value.into_promise().ok_or_else(|| {
                    Error::TestRunner("the test runner did not return a Promise".to_string())
                })?;
                // Stop driving as soon as `run()` settles so a bounded-unsettled
                // verdict (`unsettled: 1` after 100 rounds) is returned in
                // `TestReport` rather than turned into `NonConvergingJobs`.
                let report_value = drive_promise::<Value<'_>>(
                    &ctx,
                    "<neojs:test>",
                    &promise,
                    "test runner did not settle; a test returned a Promise that never resolved",
                    false,
                )
                .map_err(|error| match error {
                    Error::Module { message, .. } => Error::TestRunner(message),
                    other => other,
                })?;
                if let Err(error) = flush_unhandled_rejections(&ctx, &self.rejections) {
                    return Err(Error::TestRunner(caught_message(&ctx, error)));
                }
                read_test_report(&ctx, report_value)
            });
            self.clear_rejection_roots();
            result
        }
    }

    /// Releases every retained promise and reason while the context is alive.
    fn clear_rejection_roots(&self) {
        self.context
            .with(|_ctx| self.rejections.borrow_mut().clear());
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

/// Delivers any remaining unhandled rejections to the `neo:test` handler, if
/// installed.
///
/// The pending vector is taken before invoking the JS callback so a re-entrant
/// rejection inside the callback cannot conflict with a `RefCell` borrow.
#[cfg(feature = "neo-test")]
pub(super) fn flush_unhandled_rejections<'js>(
    ctx: &Ctx<'js>,
    rejections: &Rc<RefCell<RejectionState>>,
) -> rquickjs::Result<()> {
    let handler = {
        let Some(state) = ctx.userdata::<crate::neo::NeoState>() else {
            return Ok(());
        };
        state.rejection_handler.borrow().clone()
    };
    let Some(handler) = handler else {
        return Ok(());
    };
    let handler = handler.restore(ctx)?;
    let pending = rejections.borrow_mut().take();
    for rejection in pending {
        let promise = rejection.promise.clone().restore(ctx)?;
        let reason = rejection.reason.clone().restore(ctx)?;
        let _ = handler.call::<_, Value<'js>>((promise, reason));
        clear_pending_exception(ctx);
    }
    Ok(())
}

/// Validates and reads the summary object returned by `neo:test`'s `run()`.
#[cfg(feature = "neo-test")]
fn read_test_report(ctx: &Ctx<'_>, value: Value<'_>) -> Result<TestReport, Error> {
    let Some(object) = value.into_object() else {
        return Err(Error::TestRunner(
            "the test report is not an object".to_string(),
        ));
    };

    let read_u32 = |name: &str| -> Result<u32, Error> {
        let field: Value<'_> = object
            .get(name)
            .map_err(|_| Error::TestRunner(format!("the test report has no `{name}` field")))?;
        if let Some(int) = field.as_int() {
            return u32::try_from(int)
                .map_err(|_| Error::TestRunner(format!("the `{name}` field is not a u32")));
        }
        if let Some(float) = field.as_float()
            && float.is_finite()
            && float >= 0.0
            && float <= f64::from(u32::MAX)
            && float.fract() == 0.0
        {
            return Ok(float as u32);
        }
        Err(Error::TestRunner(format!(
            "the `{name}` field is not a u32"
        )))
    };

    let total = read_u32("total")?;
    let passed = read_u32("passed")?;
    let failed = read_u32("failed")?;
    let pending = read_u32("pending")?;
    let unhandled = read_u32("unhandled")?;
    let unsettled = read_u32("unsettled")?;
    if passed
        .checked_add(failed)
        .and_then(|count| count.checked_add(pending))
        != Some(total)
    {
        return Err(Error::TestRunner(
            "the test counts do not add up".to_string(),
        ));
    }
    if unsettled > 1 {
        return Err(Error::TestRunner(
            "the `unsettled` field must be 0 or 1".to_string(),
        ));
    }

    let failures_value: Value<'_> = object
        .get("failures")
        .map_err(|_| Error::TestRunner("the test report has no `failures` array".to_string()))?;
    let Some(failures) = failures_value.into_array() else {
        return Err(Error::TestRunner(
            "the test report has no `failures` array".to_string(),
        ));
    };
    let mut lines = Vec::with_capacity(failures.len());
    for index in 0..failures.len() {
        let entry: Value<'_> = failures
            .get(index)
            .map_err(|_| Error::TestRunner("a test failure entry is missing".to_string()))?;
        if !entry.is_string() {
            return Err(Error::TestRunner(
                "a test failure entry is not a string".to_string(),
            ));
        }
        let text = string_primitive(ctx, entry.as_raw()).ok_or_else(|| {
            Error::TestRunner("a test failure entry could not be read".to_string())
        })?;
        lines.push(text);
    }

    let expected_failures = failed
        .checked_add(pending)
        .and_then(|count| count.checked_add(unhandled))
        .and_then(|count| count.checked_add(unsettled))
        .ok_or_else(|| Error::TestRunner("the test failure count overflowed".to_string()))?;
    if lines.len() as u64 != u64::from(expected_failures) {
        return Err(Error::TestRunner(format!(
            "the test report has {} failure line(s) for {} failed, pending, unhandled, or unsettled result(s)",
            lines.len(),
            expected_failures
        )));
    }

    Ok(TestReport {
        total,
        passed,
        failed,
        pending,
        unhandled,
        unsettled,
        failures: lines,
    })
}

/// Returns whether QuickJS has at least one queued job.
///
/// Checking before execution is what lets an entry that uses exactly the job
/// budget succeed: the budget is only enforced when another job is known to be
/// waiting, so no extra job runs merely to discover the queue is empty.
pub(super) fn is_job_pending(ctx: &Ctx<'_>) -> bool {
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

fn step_job(ctx: &Ctx<'_>, module: &str, jobs: &mut usize) -> Result<(), Error> {
    if *jobs >= JOB_BUDGET {
        return Err(Error::NonConvergingJobs {
            module: module.to_owned(),
            jobs: *jobs,
        });
    }
    match execute_job(ctx) {
        JobStep::Ran => {
            *jobs += 1;
            Ok(())
        }
        JobStep::Threw(message) => Err(Error::Module {
            module: module.to_owned(),
            message: format!("uncaught exception in queued job: {message}"),
        }),
        JobStep::Empty => Err(Error::Module {
            module: module.to_owned(),
            message: "job queue reported pending work but no job ran".to_string(),
        }),
    }
}

/// Drives `promise` to settlement, optionally draining trailing queued jobs.
///
/// QuickJS's `Promise::finish` stops as soon as the entry promise settles. For
/// module entries (`drain_trailing == true`), a `.then` callback, dynamic import,
/// or module side effect queued just before settlement would then be dropped,
/// so the loop continues until the queue is empty. For `run_tests`
/// (`drain_trailing == false`), the test runner has its own bounded settle
/// window (`TEST_SETTLE_ROUNDS`), so driving stops once the runner promise
/// resolves with its `TestReport`.
///
/// The same budget bounds both phases so a nonconverging queue fails closed.
fn drive_promise<'js, T: rquickjs::FromJs<'js>>(
    ctx: &Ctx<'js>,
    module: &str,
    promise: &rquickjs::Promise<'js>,
    unsettled_message: &str,
    drain_trailing: bool,
) -> Result<T, Error> {
    let mut jobs = 0usize;
    let output;
    loop {
        if let Some(result) = promise.result::<T>() {
            output = result.map_err(|error| Error::Module {
                module: module.to_owned(),
                message: caught_message(ctx, error),
            })?;
            break;
        }
        if !is_job_pending(ctx) {
            return Err(Error::Module {
                module: module.to_owned(),
                message: unsettled_message.to_owned(),
            });
        }
        step_job(ctx, module, &mut jobs)?;
    }

    if drain_trailing {
        while is_job_pending(ctx) {
            step_job(ctx, module, &mut jobs)?;
        }
    }
    Ok(output)
}

/// Formats a QuickJS error, retrieving and clearing any pending exception.
///
/// Native binding, loader, and engine errors keep their own message. A caught
/// JavaScript exception goes through the same non-executing formatter as an
/// unhandled rejection: no getters, proxy traps, `toString`, or `String`
/// conversion run while an entry error is being reported.
fn caught_message(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    let message = match Err::<(), _>(error).catch(ctx) {
        Ok(()) => "unknown QuickJS error".to_string(),
        Err(CaughtError::Error(error)) => error.to_string(),
        Err(CaughtError::Exception(exception)) => rejection_message(ctx, &exception.into_value()),
        Err(CaughtError::Value(value)) => rejection_message(ctx, &value),
    };
    clear_pending_exception(ctx);
    message
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
__neojsLiteGc();
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
    fn formatting_never_invokes_reason_getters_or_string() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                r#"
globalThis.__calls = { getters: 0, strings: 0, jobs: 0, rejections: 0 };
const reason = {};
Object.defineProperty(reason, "name", {
    get() {
        __calls.getters++;
        Promise.resolve().then(() => { __calls.jobs++; });
        return "getter name";
    },
});
Object.defineProperty(reason, "message", {
    get() {
        __calls.getters++;
        Promise.reject(new Error("getter rejection"));
        __calls.rejections++;
        return "getter message";
    },
});
Object.defineProperty(reason, "stack", {
    get() {
        __calls.getters++;
        for (let i = 0; i < 10000; i++) { __calls.loop = i; }
        return "getter stack";
    },
});
Object.defineProperty(reason, "toString", {
    get() {
        __calls.getters++;
        return () => "getter string";
    },
});
globalThis.String = () => {
    __calls.strings++;
    return "String called";
};
Promise.reject(reason);
"#,
            )
            .expect_err("unhandled rejection");
        let Error::UnhandledRejection { details, .. } = error else {
            panic!("expected an unhandled rejection, got {error:?}");
        };
        assert!(details.contains("<rejected Object>"), "{details}");
        assert!(!details.contains("getter"), "{details}");
        let calls = runtime
            .context
            .with(|ctx| {
                let getters = ctx.eval::<i32, _>("__calls.getters")?;
                let strings = ctx.eval::<i32, _>("__calls.strings")?;
                let jobs = ctx.eval::<i32, _>("__calls.jobs")?;
                let rejections = ctx.eval::<i32, _>("__calls.rejections")?;
                rquickjs::Result::Ok((getters, strings, jobs, rejections))
            })
            .expect("inspect getter counters");
        assert_eq!(calls, (0, 0, 0, 0), "formatting invoked user code");
        assert!(!runtime.context.with(|ctx| ctx.has_exception()));
    }

    #[test]
    fn error_reasons_use_only_own_data_message() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                r#"
globalThis.__calls = 0;
const error = new Error("safe message");
Object.defineProperty(error, "message", {
    get() {
        __calls++;
        throw new Error("getter must not run");
    },
});
Promise.reject(error);
"#,
            )
            .expect_err("unhandled rejection");
        let Error::UnhandledRejection { details, .. } = error else {
            panic!("expected an unhandled rejection, got {error:?}");
        };
        assert!(
            details.contains("<rejected Error with no readable message>"),
            "{details}"
        );
        let calls = runtime
            .context
            .with(|ctx| ctx.eval::<i32, _>("__calls"))
            .expect("inspect getter counter");
        assert_eq!(calls, 0, "the message getter ran");
        assert!(!runtime.context.with(|ctx| ctx.has_exception()));
    }

    #[test]
    fn error_reasons_are_useful_without_user_code() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source("memory.js", "Promise.reject(new Error('safe message'));")
            .expect_err("unhandled rejection");
        let Error::UnhandledRejection { details, .. } = error else {
            panic!("expected an unhandled rejection, got {error:?}");
        };
        assert!(details.contains("Error: safe message"), "{details}");
        assert!(!runtime.context.with(|ctx| ctx.has_exception()));
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

    #[test]
    fn entry_error_never_invokes_error_accessors() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                r#"
globalThis.__calls = 0;
const error = new Error("original");
Object.defineProperty(error, "message", {
    get() {
        __calls++;
        Promise.resolve().then(() => { __calls++; });
        Promise.reject(new Error("getter rejection"));
        throw new Error("getter threw");
    },
});
Object.defineProperty(error, "stack", {
    get() {
        __calls++;
        for (let i = 0; i < 10000; i++) {}
        return "getter stack";
    },
});
throw error;
"#,
            )
            .expect_err("entry error");
        let Error::Module { message, .. } = error else {
            panic!("expected a module error, got {error:?}");
        };
        assert!(
            message.contains("<rejected Error with no readable message>"),
            "{message}"
        );
        assert!(!message.contains("getter threw"), "{message}");
        let calls = runtime
            .context
            .with(|ctx| ctx.eval::<i32, _>("__calls"))
            .expect("inspect getter counter");
        assert_eq!(calls, 0, "entry diagnostics invoked a getter");
        assert!(!runtime.context.with(|ctx| ctx.has_exception()));
    }

    #[test]
    fn unhandled_proxy_rejection_does_not_run_traps() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                r#"
globalThis.__traps = 0;
const reason = new Proxy({}, {
    get() {
        __traps++;
        throw new Error("get trap");
    },
    getOwnPropertyDescriptor() {
        __traps++;
        throw new Error("descriptor trap");
    },
    has() {
        __traps++;
        return false;
    },
    ownKeys() {
        __traps++;
        return [];
    },
});
Promise.reject(reason);
"#,
            )
            .expect_err("unhandled rejection");
        let Error::UnhandledRejection { details, .. } = error else {
            panic!("expected an unhandled rejection, got {error:?}");
        };
        assert!(details.contains("<rejected Proxy>"), "{details}");
        let traps = runtime
            .context
            .with(|ctx| ctx.eval::<i32, _>("__traps"))
            .expect("inspect proxy trap counter");
        assert_eq!(traps, 0, "diagnostics invoked a proxy trap");
        assert!(!runtime.context.with(|ctx| ctx.has_exception()));
    }

    fn assert_roots_cleared(runtime: &Runtime) {
        assert_eq!(runtime.rejections.borrow().unhandled.len(), 0);
        runtime
            .context
            .with(|ctx| ctx.eval::<(), _>("__neojsLiteGc()"))
            .expect("collect after terminal error");
    }

    #[test]
    fn terminal_entry_error_clears_retained_rejections() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                "Promise.reject(new Error('pending')); throw new Error('terminal');",
            )
            .expect_err("entry error");
        assert!(matches!(error, Error::Module { .. }), "{error:?}");
        assert_roots_cleared(&runtime);
    }

    #[test]
    fn terminal_job_error_clears_retained_rejections() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                "Promise.reject(new Error('pending')); queueMicrotask(() => { throw new Error('job terminal'); });",
            )
            .expect_err("job error");
        assert!(matches!(error, Error::Module { .. }), "{error:?}");
        assert_roots_cleared(&runtime);
    }

    #[test]
    fn terminal_budget_error_clears_retained_rejections() {
        let mut runtime = Runtime::new().expect("runtime");
        let error = runtime
            .run_source(
                "memory.js",
                "Promise.reject(new Error('pending')); function again() { Promise.resolve().then(again); } again();",
            )
            .expect_err("budget error");
        assert!(
            matches!(error, Error::NonConvergingJobs { jobs, .. } if jobs == JOB_BUDGET),
            "{error:?}"
        );
        assert_roots_cleared(&runtime);
    }
}

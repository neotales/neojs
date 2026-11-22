//! The embeddable runtime.
//!
//! Each [`Runtime`] owns one V8 isolate and runs at most one entry module. It
//! owns two Tokio runtimes. A multi-thread `driver` with one worker thread is
//! entered while the isolate is created, because deno_core records the Tokio
//! handle present at that point and uses it to schedule V8's delayed foreground
//! tasks; it also supplies the bounded blocking pool for `crypto`'s
//! asynchronous hashing. A current-thread `ops` runtime drives the isolate in
//! [`Runtime::run_file`], because deno_core spawns its async-op driver with
//! `deno_unsync::spawn`, which requires a current-thread runtime on the isolate
//! thread. The runtimes stay alive until the isolate is gone.
//!
//! [`Runtime::run_file_async`] does not drive `ops`; an asynchronous `crypto`
//! call made there needs an ambient Tokio current-thread runtime and fails with
//! `NEO_CRYPTO_NO_EXECUTOR` otherwise.
//!
//! The type is not `Send` and must be used from the thread that created it.

use std::path::Path;
use std::rc::Rc;

use deno_core::JsRuntime;
use deno_core::PollEventLoopOptions;
use deno_core::RuntimeOptions;
use deno_core::url::Url;

use crate::Error;
use crate::age::neojs_age;
use crate::builtins::NEO_SHIM;
use crate::crypto::neojs_crypto;
use crate::crypto_cipher::neojs_crypto_cipher;
use crate::crypto_hash::neojs_crypto_hash;
use crate::loader::FileModuleLoader;
use crate::loader::ModuleFormat;
use crate::loader::module_format;
use crate::path::neojs_path;
use crate::ssh::neojs_ssh;
use crate::stdio::neojs_stdio;
use crate::typescript;

/// Installs the five console methods neojs supports.
///
/// deno_core's built-in console sends every method to stdout, drops
/// `console.info`, and joins `null` and `undefined` as empty strings. This shim
/// gives all five methods one formatter: strings print unchanged, objects print
/// as JSON, typed arrays print their type and first ten elements, and `warn` and
/// `error` write to stderr. Only the displayed elements are read from a typed
/// array.
const CONSOLE_SHIM: &str = r#"
(() => {
    const format = (value) => {
        if (typeof value === "string") return value;
        if (typeof value === "bigint") return `${value}n`;
        if (typeof value === "symbol" || typeof value === "function") return String(value);
        if (ArrayBuffer.isView(value) && !(value instanceof DataView)) {
            const count = Math.min(value.length, 10);
            const elements = [];
            for (let index = 0; index < count; index += 1) {
                elements.push(value[index]);
            }
            return `${value.constructor.name}(${value.length}) [${elements.join(", ")}]`;
        }
        try {
            const json = JSON.stringify(value, null, 2);
            return json === undefined ? String(value) : json;
        } catch {
            return String(value);
        }
    };
    const writer = (isError) => (...args) => {
        Deno.core.print(`${args.map(format).join(" ")}\n`, isError);
    };
    globalThis.console.log = writer(false);
    globalThis.console.debug = writer(false);
    globalThis.console.info = writer(false);
    globalThis.console.warn = writer(true);
    globalThis.console.error = writer(true);
})();
"#;

/// An ECMAScript runtime with a local-file module loader.
///
/// A runtime runs at most one entry module. Create a new [`Runtime`] for each
/// entry module. `.ts` and `.mts` entry and imported files are type-stripped
/// before V8 sees them. The loader reads any path the process can read and does
/// not check permissions, so this type is not a sandbox.
pub struct Runtime {
    /// The isolate. It is an `Option` so `Drop` can destroy it before the
    /// drivers are stopped; it is `None` only while dropping.
    js: Option<JsRuntime>,
    /// Whether entry-module loading has started. It is set before V8 sees the
    /// module, because a failed load can leave the module map in a partial state.
    consumed: bool,
    /// Services V8's delayed foreground tasks (GC memory-reducer work and other
    /// timer tasks) and supplies the bounded blocking pool for asynchronous
    /// hashing. deno_core records the Tokio handle that is current when the
    /// isolate is created and aborts the process when a delayed task is posted
    /// without one, so `new` enters this runtime while creating the isolate and
    /// keeps it alive. It has one worker thread, which is enough to honor delays
    /// while the isolate itself is advanced on the owning thread.
    ///
    /// It is an `Option` so `Drop` can shut it down without blocking; see the
    /// `Drop` implementation.
    driver: Option<tokio::runtime::Runtime>,
    /// A current-thread runtime that drives the isolate for
    /// [`Runtime::run_file`] and hosts deno_core's async-op driver.
    ///
    /// deno_core spawns the op driver with `deno_unsync::spawn`, which requires
    /// a current-thread Tokio runtime. Driving the isolate inside this runtime
    /// keeps the isolate, the op futures, and their non-`Send` state on one
    /// thread. [`Runtime::run_file_async`] uses the caller's executor instead;
    /// see that method's documentation for the asynchronous-hashing condition.
    ops: Option<tokio::runtime::Runtime>,
}

impl Runtime {
    /// Creates a runtime and installs the console and built-in module shims.
    ///
    /// The runtime has one realm. The `age`, `chars`, `console`, `crypto`,
    /// `path`, `ssh`, and `stdio` built-in modules are cached in that realm, and
    /// both bare and `neo:`-prefixed import specifiers resolve to them; a new
    /// `Runtime` creates a new realm, new functions, and new module instances.
    /// The `age`, `crypto`, `path`, `ssh`, and `stdio` modules register their
    /// private host ops in that realm before user code runs. The `console`
    /// module's object is the console object the runtime installed: its default
    /// export's value is that object, and its
    /// named exports are that object's method values when the module is
    /// evaluated. Importing it shares function identity and stream behaviour at
    /// that point, and does not follow a replacement of the whole global
    /// property.
    ///
    /// This creates the V8 isolate inside a Tokio driver runtime with one worker
    /// thread, which stays alive for the life of the `Runtime` and services V8's
    /// delayed foreground tasks. That is required: without a Tokio handle, V8's
    /// delayed GC tasks abort the process. It also creates a current-thread
    /// runtime for [`Runtime::run_file`] and deno_core's async-op driver. Neither
    /// runtime depends on whether the caller is inside Tokio.
    ///
    /// Fails if a runtime cannot be started, or if a shim cannot be installed,
    /// which indicates a broken embedded script. V8 creates its isolate on a
    /// single thread; create one `Runtime` per thread and per entry module.
    ///
    /// With the off-by-default `webcrypto-proto` feature, this also registers
    /// the `deno_crypto`, `deno_web`, and `deno_webidl` extensions and installs
    /// the Web Crypto globals. The feature is an evaluation surface and does
    /// not make this type a sandbox.
    pub fn new() -> Result<Self, Error> {
        #[cfg_attr(not(feature = "webcrypto-proto"), allow(unused_mut))]
        let mut runtime = Self::with_shims(CONSOLE_SHIM, NEO_SHIM)?;
        #[cfg(feature = "webcrypto-proto")]
        {
            let js = runtime
                .js
                .as_mut()
                .expect("the isolate is present until Drop");
            js.execute_script("<neojs:webcrypto>", crate::webcrypto_proto::WEBCRYPTO_SHIM)
                .map_err(|error| Error::Core(error.into()))?;
        }
        Ok(runtime)
    }

    /// Creates a runtime and installs the given shim scripts.
    ///
    /// [`Runtime::new`] passes the production shims. The parameters exist so a
    /// unit test can hand in a failing script and prove that the shim error is
    /// returned unchanged, without panicking while the driver is torn down
    /// inside a Tokio context.
    ///
    /// The isolate is adopted by a `Runtime` before the shims run, so every
    /// error path drops the isolate first and then stops the driver through
    /// [`Runtime`]'s `Drop`, which does not block. Building the runtimes is the
    /// only failure before a `Runtime` exists.
    fn with_shims(console: &'static str, builtins: &'static str) -> Result<Self, Error> {
        let driver = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            // Bounded pool for `neo:crypto` asynchronous hashing and password
            // work. Four threads
            // is a deliberate cap: queued work waits instead of spawning
            // unbounded blocking threads.
            .max_blocking_threads(4)
            .enable_time()
            .thread_name("neojs-v8")
            .build()
            .map_err(Error::Executor)?;
        let ops = match tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .thread_name("neojs-ops")
            .build()
        {
            Ok(ops) => ops,
            Err(error) => {
                // The driver is already built. Shut it down without blocking so
                // an error during `Runtime::new` inside a Tokio context does not
                // drop a runtime from an async context.
                driver.shutdown_background();
                return Err(Error::Executor(error));
            }
        };
        #[allow(unused_mut)]
        let mut extensions = vec![
            neojs_age::init(),
            neojs_crypto::init(),
            neojs_crypto_cipher::init(),
            neojs_crypto_hash::init(),
            neojs_path::init(),
            neojs_ssh::init(),
            neojs_stdio::init(),
        ];
        #[cfg(feature = "webcrypto-proto")]
        extensions.extend(crate::webcrypto_proto::extensions());
        #[cfg(feature = "test-support")]
        extensions.push(crate::crypto_hash::test_support_extension());
        let js = {
            let _guard = driver.enter();
            JsRuntime::new(RuntimeOptions {
                module_loader: Some(Rc::new(FileModuleLoader)),
                extensions,
                ..Default::default()
            })
        };
        let mut runtime = Self {
            js: Some(js),
            consumed: false,
            driver: Some(driver),
            ops: Some(ops),
        };
        let js = runtime
            .js
            .as_mut()
            .expect("the isolate is present until Drop");
        js.execute_script("<neojs:console>", console)
            .map_err(|error| Error::Core(error.into()))?;
        js.execute_script("<neojs:builtins>", builtins)
            .map_err(|error| Error::Core(error.into()))?;
        Ok(runtime)
    }

    /// Runs one entry module to completion and blocks until it finishes.
    ///
    /// A `Runtime` runs at most one entry module. Once loading starts, later
    /// calls return [`Error::RuntimeConsumed`], including after a load or
    /// evaluation failure. Failures before loading — an unsupported format, an
    /// unreadable path, or TypeScript that cannot be type-stripped — leave the
    /// runtime unused.
    ///
    /// Relative imports resolve against the file's URL, not the process working
    /// directory. An uncaught exception or rejected top-level promise is
    /// returned as [`Error::Core`]. This method itself writes nothing, but the
    /// module's console calls write to the process streams.
    ///
    /// A consumed runtime returns [`Error::RuntimeConsumed`] first. Otherwise,
    /// calling from inside a Tokio runtime returns
    /// [`Error::BlockingInAsyncContext`]; use [`Runtime::run_file_async`] there.
    /// This blocks the calling thread by driving the runtime's own current-thread
    /// Tokio runtime; it does not require the caller to be outside Tokio for any
    /// other reason, and a runtime created inside a Tokio context can be run
    /// outside it. Asynchronous `crypto` calls work here, because this method
    /// drives the async-op driver.
    pub fn run_file(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        if self.consumed {
            return Err(Error::RuntimeConsumed);
        }
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(Error::BlockingInAsyncContext);
        }
        let Self {
            js,
            consumed,
            driver: _,
            ops,
        } = self;
        let js = js.as_mut().expect("the isolate is present until Drop");
        let ops = ops.as_ref().expect("the ops runtime is present until Drop");
        ops.block_on(run_entry(js, consumed, path.as_ref()))
    }

    /// Runs one entry module to completion on the caller's executor.
    ///
    /// This is the asynchronous form of [`Runtime::run_file`]. It has the same
    /// single-entry and printing behaviour, and it can be polled on any executor;
    /// it neither needs nor uses an ambient Tokio runtime. V8's delayed
    /// foreground tasks are serviced by the runtime's own driver thread.
    ///
    /// Asynchronous `crypto` calls (`hashAsync`, `hmacAsync`, `pbkdf2Async`,
    /// `bcryptHashAsync`, and `bcryptVerifyAsync`) need a Tokio current-thread
    /// runtime to host deno_core's async-op driver. They work when this future
    /// is polled inside such a runtime, as it is under [`Runtime::run_file`],
    /// and otherwise fail with `NEO_CRYPTO_NO_EXECUTOR`. The synchronous hash,
    /// HMAC, PBKDF2, and bcrypt API does not depend on the executor.
    ///
    /// With the off-by-default `webcrypto-proto` feature, a Web Crypto call
    /// made through this method is stricter: `deno_crypto` uses
    /// `tokio::task::spawn_blocking` directly, so polling this future without
    /// an ambient Tokio runtime aborts the process instead of returning an
    /// error. Wrap the future in a Tokio runtime, or use [`Runtime::run_file`],
    /// when Web Crypto may run.
    ///
    /// Entry preparation is not offloaded: the file read and any TypeScript type
    /// stripping run synchronously on the thread that polls this future, and
    /// imported modules are read and stripped synchronously during module
    /// loading. Module compilation and evaluation also run on that thread. Use a
    /// runtime with a blocking-friendly polling thread, or keep entry files
    /// modest, when that matters.
    ///
    /// The returned future does work only when polled. Dropping it before the
    /// first poll leaves the runtime unused. A preflight failure (unsupported
    /// format, unreadable path, or TypeScript that cannot be type-stripped) also
    /// leaves it unused. Once preflight succeeds, the runtime is consumed before
    /// loading starts. Cancelling the future after that point leaves the runtime
    /// consumed even if evaluation has not finished, and the module may already
    /// have run and produced console output or other observable effects. The
    /// partially loaded module graph is abandoned with the future; nothing retries
    /// it. Create a new runtime for any later work. Cancelling the future stops
    /// polling but does not drop the isolate's pending op futures or resources;
    /// a queued `crypto` job is aborted only when the [`Runtime`] is dropped,
    /// and a job already running on the blocking pool cannot be stopped.
    pub async fn run_file_async(&mut self, path: impl AsRef<Path>) -> Result<(), Error> {
        let js = self.js.as_mut().expect("the isolate is present until Drop");
        run_entry(js, &mut self.consumed, path.as_ref()).await
    }
}

impl Drop for Runtime {
    /// Stops the isolate and then the driver without blocking.
    ///
    /// The isolate goes first, so it unregisters itself and V8 cannot post new
    /// delayed tasks while the drivers are stopping. Dropping a Tokio runtime
    /// waits for its worker threads, and Tokio panics when that happens inside an
    /// asynchronous context; a `Runtime` may be dropped from one, so both
    /// runtimes are shut down in the background instead.
    fn drop(&mut self) {
        drop(self.js.take());
        if let Some(ops) = self.ops.take() {
            ops.shutdown_background();
        }
        if let Some(driver) = self.driver.take() {
            driver.shutdown_background();
        }
    }
}

/// Loads, evaluates, and runs one entry module on an existing isolate.
///
/// The caller decides how to drive the returned future: [`Runtime::run_file`]
/// blocks on the runtime's current-thread `ops` runtime, while
/// [`Runtime::run_file_async`] returns it to the caller's executor. Preparation
/// happens synchronously when the future is first polled, before the module is
/// marked as loaded.
async fn run_entry(js: &mut JsRuntime, consumed: &mut bool, path: &Path) -> Result<(), Error> {
    if *consumed {
        return Err(Error::RuntimeConsumed);
    }
    let (specifier, source) = prepare_entry(path)?;
    *consumed = true;
    let module = js.load_main_es_module_from_code(&specifier, source).await?;
    let evaluation = js.mod_evaluate(module);
    js.run_event_loop(PollEventLoopOptions::default()).await?;
    Ok(evaluation.await?)
}

fn prepare_entry(path: &Path) -> Result<(Url, String), Error> {
    let format = module_format(path).map_err(|reason| Error::UnsupportedEntry {
        path: path.to_path_buf(),
        reason,
    })?;

    let absolute = std::path::absolute(path).map_err(|source| Error::Entry {
        path: path.to_path_buf(),
        source,
    })?;
    let specifier = Url::from_file_path(&absolute).map_err(|()| Error::UnsupportedEntry {
        path: path.to_path_buf(),
        reason: "the path cannot be represented as a file URL; use a UTF-8 path".to_string(),
    })?;
    let source = std::fs::read_to_string(&absolute).map_err(|source| Error::Entry {
        path: path.to_path_buf(),
        source,
    })?;
    let source = match format {
        ModuleFormat::JavaScript => source,
        ModuleFormat::TypeScript => {
            typescript::strip(&specifier, source).map_err(|reason| Error::TypeScript {
                path: path.to_path_buf(),
                reason,
            })?
        }
    };
    Ok((specifier, source))
}

#[cfg(test)]
mod tests {
    use super::CONSOLE_SHIM;
    use super::Runtime;
    use crate::Error;
    use crate::builtins::NEO_SHIM;

    /// A failing shim must return its own error, even inside a Tokio context.
    ///
    /// `Runtime::new` used to install the shims before adopting the isolate, so
    /// an error returned with `?` dropped the local Tokio driver inside the
    /// caller's async context. Tokio panics when a runtime is dropped there,
    /// which replaced the shim error. Both shim positions are exercised.
    #[test]
    fn failed_shim_installation_returns_its_error_inside_tokio() {
        let caller = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("caller runtime");
        caller.block_on(async {
            let cases = [
                (
                    "throw new Error(\"console shim failed\");",
                    NEO_SHIM,
                    "console shim failed",
                ),
                (
                    CONSOLE_SHIM,
                    "throw new Error(\"builtin shim failed\");",
                    "builtin shim failed",
                ),
            ];
            for (console, builtins, expected) in cases {
                let error = Runtime::with_shims(console, builtins)
                    .err()
                    .expect("a broken shim must fail");
                match error {
                    Error::Core(error) => {
                        let message = error.to_string();
                        assert!(message.contains(expected), "{message}");
                    }
                    other => panic!("unexpected error: {other:?}"),
                }
            }
        });
    }
}

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
use deno_core::v8;

use crate::Error;
use crate::age::neojs_age;
use crate::assert::neojs_assert;
use crate::builtins::neo_shim;
use crate::cron::neojs_cron;
use crate::crypto::neojs_crypto;
use crate::crypto_cipher::neojs_crypto_cipher;
use crate::crypto_hash::neojs_crypto_hash;
use crate::crypto_shamir::neojs_crypto_shamir;
use crate::jsr_loader::JsrModuleLoader;
use crate::jsr_loader::Limits as JsrLimits;
use crate::jsr_loader::RemoteRegistry;
use crate::loader::FileModuleLoader;
use crate::loader::ModuleFormat;
use crate::loader::PreparedEntry;
use crate::loader::module_format;
use crate::path::neojs_path;
use crate::remote::Redactions;
use crate::remote::RemoteClient;
use crate::remote::RemoteOptions;
use crate::remote::diagnostic_url;
use crate::remote::display_url;
use crate::remote::redact_core_error;
use crate::remote::validate_url;
use crate::ssh::neojs_ssh;
use crate::stdio::neojs_stdio;
use crate::typescript;
use crate::uuid::neojs_uuid;

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

/// An ECMAScript runtime with a local-file module loader and optional HTTPS
/// and JSR module loading.
///
/// A runtime runs at most one entry module. Create a new [`Runtime`] for each
/// entry module. `.ts` and `.mts` entry and imported files are type-stripped
/// before V8 sees them. The loader reads any path the process can read and, when
/// remote loading is enabled, fetches `https:` URLs and resolves `jsr:` imports.
/// It does not check permissions, so this type is not a sandbox.
pub struct Runtime {
    /// The isolate. It is an `Option` so `Drop` can destroy it before the
    /// drivers are stopped; it is `None` only while dropping.
    js: Option<JsRuntime>,
    /// Whether entry-module loading has started. It is set before V8 sees the
    /// module, because a failed load can leave the module map in a partial state.
    consumed: bool,
    /// The remote client, when [`Runtime::with_remote`] enabled HTTPS loading.
    /// The loader holds a clone; entry loading uses this copy so both paths
    /// apply the same limits.
    remote: Option<RemoteClient>,
    /// Module URLs whose queries must be redacted from generated V8
    /// diagnostics. It is shared with the loader's remote client.
    redactions: Option<Redactions>,
    /// The file module loader, kept so a remote entry can be handed to it
    /// before loading starts. It is `None` for the crate's test-only JSR
    /// runtime, which installs a different loader.
    loader: Option<Rc<FileModuleLoader>>,
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

/// Cumulative counts from the `test` built-in in one runtime.
///
/// `total` counts executed tests, including pending ones. `passed`, `failed`,
/// and `pending` partition it. `unhandled` is separate: it counts unhandled
/// Promise rejections observed while the test module was active. A run is
/// successful only when [`TestReport::all_passed`] is true, which requires no
/// failure, no pending test, and no unhandled rejection.
///
/// The counts are cumulative across explicit `run()` calls and the host's
/// final call, so catching a failure from `run()` cannot hide it from the host.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TestReport {
    /// Tests executed so far, including pending tests.
    pub total: u32,
    /// Tests that returned without throwing or rejecting.
    pub passed: u32,
    /// Tests that threw, rejected, timed out, or returned an unsupported value.
    pub failed: u32,
    /// Tests registered without a callback.
    pub pending: u32,
    /// Unhandled Promise rejections observed while the test module was active.
    pub unhandled: u32,
    /// 1 when the event loop still reported pending work after the bounded
    /// settle window, otherwise 0. An unsettled verdict is a failure.
    pub unsettled: u32,
    /// One summary line per failure, in execution order.
    pub failures: Vec<String>,
}

impl TestReport {
    /// True when no test failed, none is pending, no rejection was unhandled,
    /// and no event-loop work was left unsettled.
    pub fn all_passed(&self) -> bool {
        self.failed == 0 && self.pending == 0 && self.unhandled == 0 && self.unsettled == 0
    }
}

impl Runtime {
    /// Creates a runtime and installs the console and built-in module shims.
    ///
    /// The runtime has one realm. The `age`, `assert`, `chars`, `console`,
    /// `cron`, `crypto`, `os`, `path`, `slices`, `ssh`, `stdio`, `strings`,
    /// `test`, and `uuid`
    /// built-in modules are cached in that realm, and only their canonical
    /// `neo:` import specifiers resolve to them; a bare name such as `age` is
    /// not a built-in module specifier. A new `Runtime` creates a new realm,
    /// new functions, and new module instances. The global `Neo` reports
    /// `engine` `"neo"`, and `Neo.getBuiltInModulesList` returns one frozen
    /// array of the canonical `neo:` specifiers in registration order. The
    /// `age`, `cron`, `crypto`, `path`, `ssh`, `stdio`, and `uuid` modules
    /// register their private host ops in that realm before user code runs;
    /// `chars`, `os`, `slices`, `strings`, and `test` are plain ECMAScript, and
    /// `assert` calls one private Promise-observation host op. The `cron` ops
    /// take the runtime's own Tokio timer and resource table, so an undisposed
    /// job keeps the event loop alive. The `console`
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
    pub fn new() -> Result<Self, Error> {
        Self::with_shims(CONSOLE_SHIM, neo_shim())
    }

    /// Creates a runtime that can also load modules from `https:` URLs.
    ///
    /// This is the explicit opt-in for remote loading. [`Runtime::new`] never
    /// performs a network request, and a URL entry or HTTPS import there fails
    /// with [`Error::RemoteDisabled`] or [`Error::Core`]. The returned runtime
    /// accepts an HTTPS entry through [`Runtime::run_url`] or
    /// [`Runtime::run_url_async`], and any running module may import an
    /// absolute `https:` URL. Relative imports inside a remote module resolve
    /// against that module's URL and therefore stay remote.
    ///
    /// Only `https:` is accepted. `http:`, `data:`, `blob:`, and `file:` are
    /// not remote transports, and a remote module cannot import `file:`.
    ///
    /// This opt-in also enables `jsr:@scope/name[@constraint][/subpath]`
    /// imports. They resolve through the public JSR registry: package metadata
    /// selects an exact version, version metadata resolves the entrypoint, and
    /// each file's length and SHA-256 are verified against the version manifest
    /// before it reaches TypeScript stripping or V8. That proves consistency
    /// with the retrieved metadata, not publisher identity, and the cache is
    /// mutable local state rather than an authentication boundary. Metadata and
    /// files use this runtime's HTTPS client, redirect policy, deadlines,
    /// request/byte budgets, and user cache. Resolution is synchronous, so the
    /// first request for a package or version blocks the isolate thread. A
    /// direct `https://jsr.io/...` import from a local or ordinary remote module
    /// is an ordinary HTTPS module, not a `jsr:` import; an explicit jsr.io URL
    /// imported inside a JSR graph is verified against the manifest. If the same
    /// canonical URL is already pending from a `jsr:` resolution (including a
    /// bare `import.meta.resolve` or a failed verified load), a later direct
    /// load of that URL takes the verified path because the original specifier
    /// is not recoverable at load time; that is conservative verification, not
    /// a bypass. There is no
    /// lockfile, publisher signature, dist-tag, private registry, `npm:`,
    /// `node:`, `gh:`, or import-map support.
    ///
    /// Redirects are followed one hop at a time: the target must be
    /// credential-free HTTPS and within `options.max_redirects`; a downgrade to
    /// HTTP or another scheme is refused before the target is requested.
    /// Cross-origin HTTPS redirects are allowed because no cookie,
    /// authorization header, or referrer is sent.
    ///
    /// When a fetch follows redirects, the module map registers the module
    /// under the final URL and aliases the requested URL to it, as a browser
    /// does. `import.meta.url` is the final URL, relative imports resolve
    /// against it, and a later import of either spelling reaches the same
    /// instance. Two URLs that differ only by query are two modules.
    ///
    /// The transport is the blocking `ureq` client over `rustls` with the
    /// bundled Mozilla root store. It sends no ambient credentials, does not
    /// read proxy settings from the environment, and does not accept a URL with
    /// a username or password. Idle connection pooling is disabled so ureq cannot
    /// hide a retry of a request whose reused connection was closed. Downloaded
    /// modules are cached in one
    /// user-profile package cache, `$XDG_CACHE_HOME/neojs/packages` or
    /// `$HOME/.cache/neojs/packages`, keyed by the SHA-256 of the canonical URL
    /// including its query. A fresh `Cache-Control: max-age` entry is reused;
    /// freshness includes the upstream `Age` and `Date`; a stale entry is
    /// revalidated with `If-None-Match` and `If-Modified-Since`; `no-store` does
    /// not store the body and leaves an absent-entry tombstone. Writes are atomic
    /// and concurrent processes cannot publish a
    /// partial body. Each entry records and verifies the body length and SHA-256
    /// digest, so corruption is a cache miss. A per-key lock and persistent
    /// monotonic revision files make a `200` lose to a newer entry or no-store
    /// tombstone, make a `304` use its request revision and not a later
    /// completion token, and compare a representation (validator plus actual
    /// body digest) separately from publication revisions, so a newer `304` for
    /// the same bytes can supersede an older one while a weak ETag shared by
    /// different bytes cannot. A newer `304` merges only headers it actually
    /// supplied into the current representation, so it cannot undo an earlier
    /// `no-cache` by omitting Cache-Control. A `304` without usable Date/Age
    /// derives age from the current entry under the lock. Ordering is independent of the wall
    /// clock. Lock waits are capped at 250 ms and the remaining fetch deadline.
    /// With a longer deadline an unavailable lock skips the cache operation and
    /// the fetch continues to the network; with a deadline at or below the cap,
    /// the lock may consume it and the fetch times out before transport. There is
    /// no signature or content-addressed store: the
    /// cache is not integrity protection against someone who can write it and
    /// pins no version. See [`RemoteOptions`] to change the directory or disable
    /// it.
    ///
    /// One runtime's remote graph is bounded by per-module body, aggregate
    /// byte, and module-count limits. A module and its bytes are admitted before
    /// the network or cache is consulted, a body is read only up to the
    /// remaining aggregate allowance, and a rejected module is neither fetched
    /// nor published to the cache. Each distinct requested URL consumes one
    /// module slot even when its fetch fails; retrying the same URL reuses that
    /// slot but repeats the I/O. A separate request limit counts every network
    /// request, including retries and redirect hops. A failed entry preflight
    /// refunds module, byte, and redaction charges but not the request already
    /// spent; create a new `Runtime` for a fresh request budget. The deadline is
    /// per module, not for the whole wall-clock execution of a program. These are
    /// resource bounds, not a sandbox or a CPU or V8 heap bound.
    ///
    /// Generated V8 errors redact known remote module URLs from frame locations
    /// and only from messages whose name and prefix match a generated
    /// diagnostic. Module identity and `import.meta.url` are unchanged, and a
    /// script-supplied exception message, including text shaped like "Cannot
    /// find module" or ` at <URL>:`, and a custom stack are preserved, so a
    /// program can still expose a URL it puts in its own error. A user-thrown
    /// `SyntaxError`/`TypeError` that exactly copies a generated prefix and
    /// location shape is indistinguishable from V8's text and may be redacted.
    /// Do not treat a module specifier as secret storage.
    ///
    /// Remote loading is not SSRF protection: there is no host allowlist,
    /// private-address rejection, or DNS pinning, so the program can fetch any
    /// HTTPS URL the process's network allows. Remote code runs with the same
    /// authority as local code; this is not a sandbox.
    ///
    /// The fetch and its limits are applied synchronously on the isolate thread
    /// when the entry future is polled or an imported module is loaded.
    /// Cancelling [`Runtime::run_url_async`] before its first poll performs no
    /// I/O and leaves the runtime unused; a fetch already in progress cannot be
    /// interrupted, and cancelling after loading starts leaves the runtime
    /// consumed.
    pub fn with_remote(options: RemoteOptions) -> Result<Self, Error> {
        Self::build(CONSOLE_SHIM, neo_shim(), Some(RemoteClient::new(options)))
    }

    /// Creates a runtime over an injected remote transport.
    ///
    /// This exists only with the off-by-default `test-support` feature, so a
    /// test can drive remote loading without a network or a real host. It is not
    /// part of the stable embedding API.
    #[cfg(feature = "test-support")]
    #[doc(hidden)]
    pub fn with_remote_transport<F>(options: RemoteOptions, fetch: F) -> Result<Self, Error>
    where
        F: Fn(
                &str,
                std::time::Duration,
                Option<&str>,
                Option<&str>,
            ) -> Result<crate::remote::TestRemoteResponse, String>
            + Send
            + Sync
            + 'static,
    {
        let transport = crate::remote::test_transport(fetch);
        Self::build(
            CONSOLE_SHIM,
            neo_shim(),
            Some(RemoteClient::with_transport(options, transport)),
        )
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
    fn with_shims(console: &'static str, builtins: String) -> Result<Self, Error> {
        Self::with_module_loader(console, builtins, Rc::new(FileModuleLoader::new(None)))
    }

    /// Creates a runtime with an explicit module loader.
    ///
    /// [`Runtime::new`] installs [`FileModuleLoader`]. The crate's JSR tests use
    /// this to install a loader backed by an in-memory registry. This method is
    /// crate-private; [`Runtime::with_remote`] is the public constructor that
    /// enables remote and `jsr:` loading.
    fn with_module_loader(
        console: &'static str,
        builtins: String,
        module_loader: Rc<dyn deno_core::ModuleLoader>,
    ) -> Result<Self, Error> {
        Self::assemble(console, builtins, module_loader, None, None)
    }

    /// Creates a runtime with an optional remote client.
    ///
    /// The remote client's file loader is kept separately so a prepared remote
    /// entry can be handed to it before loading starts. When remote loading is
    /// enabled, the loader is a [`JsrModuleLoader`] so `jsr:` imports resolve
    /// through the same transport, cache, and limits.
    fn build(
        console: &'static str,
        builtins: String,
        remote: Option<RemoteClient>,
    ) -> Result<Self, Error> {
        let files = Rc::new(FileModuleLoader::new(remote.clone()));
        let module_loader: Rc<dyn deno_core::ModuleLoader> = match remote.clone() {
            Some(remote_client) => Rc::new(JsrModuleLoader::new(
                Rc::new(RemoteRegistry::new(remote_client)),
                files.clone(),
                JsrLimits::default(),
            )),
            None => files.clone(),
        };
        Self::assemble(console, builtins, module_loader, Some(files), remote)
    }

    /// Builds the runtimes, isolate, extensions, and shims.
    fn assemble(
        console: &'static str,
        builtins: String,
        module_loader: Rc<dyn deno_core::ModuleLoader>,
        loader: Option<Rc<FileModuleLoader>>,
        remote: Option<RemoteClient>,
    ) -> Result<Self, Error> {
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
            neojs_assert::init(),
            neojs_cron::init(),
            neojs_crypto::init(),
            neojs_crypto_cipher::init(),
            neojs_crypto_hash::init(),
            neojs_crypto_shamir::init(),
            neojs_path::init(),
            neojs_ssh::init(),
            neojs_stdio::init(),
            neojs_uuid::init(),
        ];
        #[cfg(feature = "test-support")]
        extensions.push(crate::cron::test_support_extension());
        #[cfg(feature = "test-support")]
        extensions.push(crate::crypto_hash::test_support_extension());
        let redactions = remote.as_ref().map(RemoteClient::redactions);
        let js = {
            let _guard = driver.enter();
            JsRuntime::new(RuntimeOptions {
                module_loader: Some(module_loader),
                extensions,
                ..Default::default()
            })
        };
        let mut runtime = Self {
            js: Some(js),
            consumed: false,
            remote,
            redactions,
            loader,
            driver: Some(driver),
            ops: Some(ops),
        };
        let js = runtime
            .js
            .as_mut()
            .expect("the isolate is present until Drop");
        #[cfg(feature = "neo-chars")]
        crate::chars::install(js).map_err(Error::NativeBuiltins)?;
        js.execute_script("<neojs:console>", console)
            .map_err(|error| Error::Core(error.into()))?;
        js.execute_script("<neojs:builtins>", builtins)
            .map_err(|error| Error::Core(error.into()))?;
        #[cfg(feature = "neo-chars")]
        crate::chars::remove_entries_global(js).map_err(Error::NativeBuiltins)?;
        Ok(runtime)
    }

    /// Creates a JSR test runtime with explicit resource limits.
    ///
    /// The module-graph tests use small limits to cross a bound through real
    /// V8 loading. No production build compiles this constructor.
    #[cfg(test)]
    pub(crate) fn with_jsr_registry_limits(
        registry: Rc<dyn crate::jsr_loader::Registry>,
        limits: crate::jsr_loader::Limits,
    ) -> Result<Self, Error> {
        let loader = crate::jsr_loader::JsrModuleLoader::with_limits(registry, limits);
        Self::with_module_loader(CONSOLE_SHIM, neo_shim(), Rc::new(loader))
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
        let redactions = self.redactions.clone();
        let Self {
            js,
            consumed,
            remote: _,
            redactions: _,
            loader: _,
            driver: _,
            ops,
        } = self;
        let js = js.as_mut().expect("the isolate is present until Drop");
        let ops = ops.as_ref().expect("the ops runtime is present until Drop");
        ops.block_on(run_entry(js, consumed, path.as_ref()))
            .map_err(|error| redact_error(error, redactions.as_ref()))
    }

    /// Runs one remote entry module to completion and blocks until it finishes.
    ///
    /// This is [`Runtime::run_file`] for a URL. Remote loading must have been
    /// enabled with [`Runtime::with_remote`]; otherwise it returns
    /// [`Error::RemoteDisabled`] without a network request. The URL must be
    /// `https:`, must have a host, and must not contain a username or password.
    ///
    /// Entry preparation follows the same single-use rule as [`Runtime::run_file`]:
    /// a parse, scheme, fetch, limit, UTF-8, or unsupported-format failure
    /// happens before V8 loads the module and leaves the runtime unused. A
    /// TypeScript stripping failure is [`Error::RemoteTypeScript`]. Once loading
    /// starts, the runtime is consumed.
    ///
    /// The fetch happens synchronously on the calling thread. If the response
    /// redirects, the final URL becomes the module's identity and the requested
    /// URL becomes an alias for it; relative imports resolve against the final
    /// URL.
    ///
    /// This method blocks; calling it from inside a Tokio runtime returns
    /// [`Error::BlockingInAsyncContext`]. Use [`Runtime::run_url_async`] there.
    pub fn run_url(&mut self, url: &str) -> Result<(), Error> {
        if self.consumed {
            return Err(Error::RuntimeConsumed);
        }
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(Error::BlockingInAsyncContext);
        }
        let remote = self.remote.clone().ok_or(Error::RemoteDisabled)?;
        let loader = self.loader.clone().ok_or(Error::RemoteDisabled)?;
        let redactions = self.redactions.clone();
        let Self {
            js,
            consumed,
            remote: _,
            redactions: _,
            loader: _,
            driver: _,
            ops,
        } = self;
        let js = js.as_mut().expect("the isolate is present until Drop");
        let ops = ops.as_ref().expect("the ops runtime is present until Drop");
        ops.block_on(run_remote_entry(
            js,
            consumed,
            loader.as_ref(),
            &remote,
            url,
        ))
        .map_err(|error| redact_error(error, redactions.as_ref()))
    }

    /// Runs one remote entry module to completion on the caller's executor.
    ///
    /// This is the asynchronous form of [`Runtime::run_url`], with the same URL
    /// policy, single-use rule, and identity rule. It can be polled on any
    /// executor and does not need an ambient Tokio runtime.
    ///
    /// The fetch is not offloaded: it runs synchronously on the thread that
    /// polls the future, before the runtime is marked consumed. Dropping the
    /// future before its first poll performs no I/O and leaves the runtime
    /// unused. A fetch already in progress cannot be cancelled. After loading
    /// starts, dropping the future leaves the runtime consumed and the module
    /// may already have run.
    pub async fn run_url_async(&mut self, url: &str) -> Result<(), Error> {
        let remote = self.remote.clone().ok_or(Error::RemoteDisabled)?;
        let loader = self.loader.clone().ok_or(Error::RemoteDisabled)?;
        let redactions = self.redactions.clone();
        let js = self.js.as_mut().expect("the isolate is present until Drop");
        run_remote_entry(js, &mut self.consumed, loader.as_ref(), &remote, url)
            .await
            .map_err(|error| redact_error(error, redactions.as_ref()))
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
        let redactions = self.redactions.clone();
        let js = self.js.as_mut().expect("the isolate is present until Drop");
        run_entry(js, &mut self.consumed, path.as_ref())
            .await
            .map_err(|error| redact_error(error, redactions.as_ref()))
    }

    /// Runs the tests registered through the `test` built-in and returns their
    /// cumulative report.
    ///
    /// Call this after [`Runtime::run_file`] so tests registered after a
    /// top-level await are included. It can also be called on a fresh runtime;
    /// the report is then empty. The method blocks on this runtime's own
    /// current-thread executor, so it must not be called from inside a Tokio
    /// runtime; use [`Runtime::run_tests_async`] there.
    ///
    /// A failing test, a pending test, an unhandled rejection, or a bounded
    /// settle window that still reports event-loop work is recorded in the
    /// returned [`TestReport`], not as an `Err`. `Err` means the runner itself
    /// failed or its counts were inconsistent. The test module prints
    /// `ok`/`not ok` lines and a summary to the process's standard output; a
    /// test can replace `Deno.core`, so that output is observable but not a
    /// security boundary. The runtime is not a sandbox and a synchronous
    /// callback that never returns still hangs the process.
    ///
    /// The runner waits up to about 100 ms of event-loop turns after the tests.
    /// If the runtime still reports pending timers, async ops, imports, or
    /// rejection events, the verdict is marked unsettled and fails. Unrefed
    /// work that the event loop does not report is outside this check. The
    /// report does not cancel outstanding work; dropping the runtime discards
    /// it.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use neojs_core::Runtime;
    ///
    /// # fn main() -> Result<(), neojs_core::Error> {
    /// let mut runtime = Runtime::new()?;
    /// runtime.run_file("example.test.js")?;
    /// let report = runtime.run_tests()?;
    /// assert!(report.all_passed(), "{:?}", report.failures);
    /// # Ok(())
    /// # }
    /// ```
    pub fn run_tests(&mut self) -> Result<TestReport, Error> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(Error::BlockingInAsyncContext);
        }
        let Self { js, ops, .. } = self;
        let js = js.as_mut().expect("the isolate is present until Drop");
        let ops = ops.as_ref().expect("the ops runtime is present until Drop");
        ops.block_on(run_tests(js))
    }

    /// Runs the registered tests on the caller's executor.
    ///
    /// This is the asynchronous form of [`Runtime::run_tests`]. It has the same
    /// reporting contract and can be polled on any executor; the runtime's own
    /// driver thread services V8's delayed foreground tasks. A callback that
    /// never settles is bounded by its per-test timeout (30 seconds by default),
    /// but a callback that blocks the isolate synchronously is not interruptible.
    pub async fn run_tests_async(&mut self) -> Result<TestReport, Error> {
        let js = self.js.as_mut().expect("the isolate is present until Drop");
        run_tests(js).await
    }
}

/// Redacts remote module URLs from a V8 error when remote loading is enabled.
fn redact_error(error: Error, redactions: Option<&Redactions>) -> Error {
    match (error, redactions) {
        (Error::Core(error), Some(redactions)) => Error::Core(redact_core_error(error, redactions)),
        (other, _) => other,
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
    evaluate_entry(js, consumed, specifier, source).await
}

/// Loads and runs one remote entry module on an existing isolate.
async fn run_remote_entry(
    js: &mut JsRuntime,
    consumed: &mut bool,
    loader: &FileModuleLoader,
    remote: &RemoteClient,
    url: &str,
) -> Result<(), Error> {
    if *consumed {
        return Err(Error::RuntimeConsumed);
    }
    let entry = prepare_url_entry(remote, url)?;
    let specifier = entry.requested.clone();
    loader.set_entry(entry);
    *consumed = true;
    let module = js.load_main_es_module(&specifier).await?;
    finish_entry(js, module).await
}

/// Marks the runtime consumed, then loads, evaluates, and drains one module.
///
/// Preparation has already succeeded, so every failure from here on leaves the
/// runtime consumed.
async fn evaluate_entry(
    js: &mut JsRuntime,
    consumed: &mut bool,
    specifier: Url,
    source: String,
) -> Result<(), Error> {
    *consumed = true;
    let module = js.load_main_es_module_from_code(&specifier, source).await?;
    finish_entry(js, module).await
}

/// Evaluates a loaded main module and drains the event loop.
async fn finish_entry(js: &mut JsRuntime, module: deno_core::ModuleId) -> Result<(), Error> {
    let evaluation = js.mod_evaluate(module);
    js.run_event_loop(PollEventLoopOptions::default()).await?;
    Ok(evaluation.await?)
}

/// The JavaScript expression the host evaluates to run the queued tests.
///
/// `throwOnFailure: false` makes the runner return its report even when tests
/// failed; the host decides the exit status. `Neo` and its module registry are
/// installed as non-writable globals before user code runs.
const TEST_RUN_SCRIPT: &str =
    r#"Neo.getBuiltInModule("neo:test").run({ throwOnFailure: false, settle: true })"#;

/// Runs the `test` built-in and reads its cumulative report.
async fn run_tests(js: &mut JsRuntime) -> Result<TestReport, Error> {
    let value = js
        .execute_script("<neojs:test>", TEST_RUN_SCRIPT)
        .map_err(|error| Error::Core(deno_core::error::CoreErrorKind::Js(error).into_box()))?;
    // `resolve` watches the promise but does not advance the event loop by
    // itself; a timer or async op inside the runner would never settle. Drive
    // both until the report resolves.
    let resolution = js.resolve(value);
    let value = js
        .with_event_loop_promise(resolution, PollEventLoopOptions::default())
        .await
        .map_err(Error::Core)?;
    read_test_report(js, value)
}

/// Reads the primitives from the null-prototype summary object.
///
/// The object is created by the runner, so every field is an own data property.
/// A missing or non-numeric field means the runner did not finish correctly and
/// its counts cannot be trusted; that is a runner failure, never a pass.
fn read_test_report(js: &mut JsRuntime, value: v8::Global<v8::Value>) -> Result<TestReport, Error> {
    deno_core::scope!(scope, js);
    let value = v8::Local::new(scope, value);
    let object = value
        .to_object(scope)
        .ok_or_else(|| Error::TestRunner("the test report is not an object".to_string()))?;

    let number = |name: &str| -> Result<u32, Error> {
        let key = v8::String::new(scope, name)
            .ok_or_else(|| Error::TestRunner(format!("the field `{name}` is too large")))?;
        let value = object
            .get(scope, key.into())
            .ok_or_else(|| Error::TestRunner(format!("the test report has no `{name}` field")))?;
        value
            .uint32_value(scope)
            .ok_or_else(|| Error::TestRunner(format!("the `{name}` field is not a u32")))
    };

    let total = number("total")?;
    let passed = number("passed")?;
    let failed = number("failed")?;
    let pending = number("pending")?;
    let unhandled = number("unhandled")?;
    let unsettled = number("unsettled")?;
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

    let key = v8::String::new(scope, "failures")
        .ok_or_else(|| Error::TestRunner("the `failures` field is too large".to_string()))?;
    let failures = object
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or_else(|| Error::TestRunner("the test report has no `failures` array".to_string()))?;
    let mut lines = Vec::with_capacity(failures.length() as usize);
    for index in 0..failures.length() {
        let Some(value) = failures.get_index(scope, index) else {
            return Err(Error::TestRunner(
                "a test failure entry is missing".to_string(),
            ));
        };
        if !value.is_string() {
            return Err(Error::TestRunner(
                "a test failure entry is not a string".to_string(),
            ));
        }
        lines.push(value.to_rust_string_lossy(scope));
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

/// Fetches and prepares a remote entry module.
///
/// The returned [`PreparedEntry`] keeps the requested URL as the map alias and
/// the final URL as the module's identity. Relative imports resolve against the
/// final URL, and `import.meta.url` is the final URL. Errors describe a
/// sanitized URL, so a password in a malformed input is not echoed.
fn prepare_url_entry(remote: &RemoteClient, url: &str) -> Result<PreparedEntry, Error> {
    let requested = Url::parse(url).map_err(|error| Error::RemoteEntry {
        url: "<invalid URL>".to_string(),
        reason: format!("invalid URL: {error}"),
    })?;
    validate_url(&requested).map_err(|reason| Error::RemoteEntry {
        url: display_url(&requested),
        reason,
    })?;

    let fetched = remote
        .fetch_entry(&requested)
        .map_err(|reason| Error::RemoteEntry {
            url: display_url(&requested),
            reason,
        })?;
    let body_len = fetched.body.len();
    let module_charged = fetched.module_charged;
    // A redirect can change the extension. The format follows the final URL,
    // just as the module identity and relative-import base do.
    let format = match module_format(Path::new(fetched.final_url.path())) {
        Ok(format) => format,
        Err(reason) => {
            remote.refund_entry(&requested, body_len, module_charged);
            return Err(Error::RemoteEntry {
                url: display_url(&requested),
                reason,
            });
        }
    };
    let code = match String::from_utf8(fetched.body) {
        Ok(code) => code,
        Err(_) => {
            remote.refund_entry(&requested, body_len, module_charged);
            return Err(Error::RemoteEntry {
                url: display_url(&requested),
                reason: "the module is not valid UTF-8".to_string(),
            });
        }
    };
    let source = match format {
        ModuleFormat::JavaScript => code,
        ModuleFormat::TypeScript => {
            match typescript::strip(&diagnostic_url(&fetched.final_url), code) {
                Ok(source) => source,
                Err(reason) => {
                    remote.refund_entry(&requested, body_len, module_charged);
                    return Err(Error::RemoteTypeScript {
                        url: display_url(&requested),
                        reason,
                    });
                }
            }
        }
    };
    Ok(PreparedEntry {
        requested,
        found: fetched.final_url,
        code: source,
    })
}

#[cfg(test)]
mod tests {
    use super::CONSOLE_SHIM;
    use super::Runtime;
    use crate::Error;
    use crate::builtins::neo_shim;
    use std::rc::Rc;

    struct IdentityLoader;

    impl deno_core::ModuleLoader for IdentityLoader {
        fn resolve(
            &self,
            specifier: &str,
            _referrer: &str,
            _kind: deno_core::ResolutionKind,
        ) -> deno_core::ModuleResolveResponse {
            deno_core::ModuleSpecifier::parse(specifier)
                .map_err(|error| deno_error::JsErrorBox::generic(error.to_string()))
        }

        fn load(
            &self,
            _specifier: &deno_core::ModuleSpecifier,
            _maybe_referrer: Option<&deno_core::ModuleLoadReferrer>,
            _options: deno_core::ModuleLoadOptions,
        ) -> deno_core::ModuleLoadResponse {
            deno_core::ModuleLoadResponse::Sync(Err(deno_error::JsErrorBox::generic(
                "the collision test does not expect a file load",
            )))
        }
    }

    #[cfg(feature = "neo-chars")]
    fn noop<'s>(
        _scope: &mut deno_core::v8::PinScope<'s, '_>,
        _args: deno_core::v8::FunctionCallbackArguments<'s>,
        _rv: deno_core::v8::ReturnValue<'s, deno_core::v8::Value>,
    ) {
    }

    #[cfg(feature = "neo-chars")]
    fn native_export(
        js: &mut deno_core::JsRuntime,
        name: deno_core::FastStaticString,
    ) -> (
        deno_core::FastStaticString,
        deno_core::v8::Global<deno_core::v8::Value>,
    ) {
        let context = js.main_context();
        deno_core::v8::scope!(let scope, js.v8_isolate());
        let context = deno_core::v8::Local::new(scope, context);
        let scope = &mut deno_core::v8::ContextScope::new(scope, context);
        let function = deno_core::v8::Function::new(scope, noop).expect("native function");
        let value: deno_core::v8::Local<deno_core::v8::Value> = function.into();
        (name, deno_core::v8::Global::new(scope, value))
    }

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
                    neo_shim(),
                    "console shim failed",
                ),
                (
                    CONSOLE_SHIM,
                    "throw new Error(\"builtin shim failed\");".to_string(),
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

    #[cfg(feature = "neo-chars")]
    #[test]
    fn duplicate_native_module_registration_keeps_the_original_chars_module() {
        let mut runtime = Runtime::new().expect("runtime");
        let js = runtime.js.as_mut().expect("the isolate is present");
        let replacement = native_export(js, deno_core::ascii_str!("isLetter"));
        assert!(
            js.register_native_module("neo:chars", vec![replacement])
                .is_err(),
            "a duplicate specifier must be rejected"
        );

        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.js");
        std::fs::write(
            &file,
            r#"
import * as chars from "neo:chars";
const registry = Neo.getBuiltInModule("neo:chars");
if (chars.isLetter !== registry.isLetter) {
    throw new Error("the namespace and registry no longer share a function");
}
if (chars.isLetter(0x41) !== true) {
    throw new Error("the original native function was replaced");
}
const dynamic = await import("neo:chars");
if (dynamic.isLetter !== chars.isLetter) {
    throw new Error("the original module identity was replaced");
}
"#,
        )
        .expect("write source");
        runtime
            .run_file(&file)
            .expect("the original chars module remains intact");
    }

    #[cfg(feature = "neo-chars")]
    #[test]
    fn duplicate_native_export_names_are_rejected_before_registration() {
        let mut runtime = Runtime::new().expect("runtime");
        let js = runtime.js.as_mut().expect("the isolate is present");
        let first = native_export(js, deno_core::ascii_str!("x"));
        let second = native_export(js, deno_core::ascii_str!("x"));
        assert!(
            js.register_native_module("neo:probe-native", vec![first, second])
                .is_err(),
            "duplicate export names must be rejected"
        );

        // The rejected call must not have inserted the specifier. If it had,
        // this registration would fail as a duplicate and the map would be
        // permanently poisoned.
        let third = native_export(js, deno_core::ascii_str!("x"));
        assert!(
            js.register_native_module("neo:probe-native", vec![third])
                .is_ok(),
            "the rejected registration mutated the module map"
        );
        let fourth = native_export(js, deno_core::ascii_str!("y"));
        assert!(
            js.register_native_module("neo:probe-native", vec![fourth])
                .is_err(),
            "a second registration of the same specifier must be rejected"
        );
    }

    #[test]
    fn native_module_registration_rejects_a_lazy_esm_collision() {
        let mut runtime = Runtime::new().expect("runtime");
        let js = runtime.js.as_mut().expect("the isolate is present");
        let specifier = "ext:test/native-lazy-collision.js";
        let source = "export const value = 7;";
        js.add_lazy_loaded_esm_source_for_test(specifier, source);

        let export = js
            .execute_script("<native-export>", "(function value() { return 7; })")
            .expect("export value");
        let result =
            js.register_native_module(specifier, vec![(deno_core::ascii_str!("value"), export)]);
        assert!(
            result.is_err(),
            "a native registration must reject a known lazy ESM collision"
        );

        // Load through the ordinary lazy-module path, not with source bytes
        // supplied by the test. Two independent lazy loaders must return the
        // one original namespace, with the original export.
        js.execute_script(
            "<lazy-collision>",
            format!(
                r#"
const specifier = "{specifier}";
const loadA = Deno.core.createLazyLoader(specifier);
const loadB = Deno.core.createLazyLoader(specifier);
const first = loadA();
const again = loadA();
const other = loadB();
if (first !== again || first !== other) {{
    throw new Error("the lazy module identity changed");
}}
if (first.value !== 7) {{
    throw new Error("the original lazy export changed");
}}
"#
            ),
        )
        .expect("the remaining lazy source loads and evaluates");
    }

    #[test]
    fn native_module_registration_rejects_typed_and_alias_collisions() {
        let mut js = deno_core::JsRuntime::new(deno_core::RuntimeOptions {
            module_loader: Some(Rc::new(IdentityLoader)),
            ..Default::default()
        });

        // A JSON module lives under RequestedModuleType::Json, not None.
        js.add_json_module_for_test("https://test/typed.json", r#"{"ok":true}"#);
        let export = js
            .execute_script("<native-export>", "(function value() { return 7; })")
            .expect("export value");
        assert!(
            js.register_native_module(
                "https://test/typed.json",
                vec![(deno_core::ascii_str!("value"), export)],
            )
            .is_err(),
            "a native registration must reject a typed module name"
        );

        // The alias is also under RequestedModuleType::Json, so a None lookup
        // alone would miss it; has_name_any_type must catch it.
        js.add_json_alias_for_test("https://test/alias.json", "https://test/typed.json");
        let export = js
            .execute_script("<native-export>", "(function value() { return 7; })")
            .expect("export value");
        assert!(
            js.register_native_module(
                "https://test/alias.json",
                vec![(deno_core::ascii_str!("value"), export)],
            )
            .is_err(),
            "a native registration must reject a typed alias name"
        );

        // Resolve and import both names through the real module graph. The
        // JSON module and its typed alias must still exist, carry the original
        // value, and resolve to one module identity after both rejections.
        let main =
            deno_core::ModuleSpecifier::parse("https://test/main.js").expect("main specifier");
        let main_id = deno_core::futures::executor::block_on(js.load_main_es_module_from_code(
            &main,
            r#"
import typed from "https://test/typed.json" with { type: "json" };
import alias from "https://test/alias.json" with { type: "json" };
if (typed.ok !== true) throw new Error("the typed JSON value changed");
if (typed !== alias) throw new Error("the typed alias lost module identity");
"#,
        ))
        .expect("the typed module and alias resolve and load");
        // `mod_evaluate` alone can return Ok while a top-level rejection is
        // thrown into V8. Drive the event loop, as the runtime does for a real
        // entry, and assert both the event-loop result and the evaluation
        // result.
        let evaluation = js.mod_evaluate(main_id);
        deno_core::futures::executor::block_on(
            js.run_event_loop(deno_core::PollEventLoopOptions::default()),
        )
        .expect("the typed import graph's event loop completes");
        deno_core::futures::executor::block_on(evaluation)
            .expect("the typed import graph evaluates");
    }
}

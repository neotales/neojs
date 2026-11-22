//! Experimental Web Crypto bootstrap, enabled by the `webcrypto-proto` feature.
//!
//! This module exists to evaluate `deno_crypto` in neojs. It is not a stable
//! surface, it is not part of the default build, and its JavaScript contract
//! may change or be removed. Nothing here is claimed to be secure or complete.
//!
//! The Web Crypto implementation lives in the `deno_crypto` crate and is
//! exposed as the `Crypto`, `CryptoKey`, and `SubtleCrypto` cppgc classes plus
//! a lazily evaluated `ext:deno_crypto/00_crypto.js` script. `deno_crypto`
//! declares `deno_webidl` and `deno_web` as extension dependencies, so all
//! three extensions are registered even though only the crypto classes and the
//! DOM exception/console scripts are used. The rest of `deno_web`'s JavaScript
//! (events, streams, blobs, URL, timers, compression) is not evaluated and its
//! globals are not installed, but its host ops are registered by the extension
//! and remain reachable through `Deno.core.ops`.
//!
//! `deno_crypto` expects its embedder to install the four Web Crypto globals and
//! to register the `DOMException*` error builders that turn Rust error classes
//! into `DOMException` values. Without those builders, a rejected Web Crypto
//! promise rejects with `undefined`, so [`WEBCRYPTO_SHIM`] mirrors the
//! registration in Deno's `runtime/js/99_main.js`. Only the five error classes
//! `deno_crypto` uses are registered.
//!
//! The algorithm work runs through `deno_core::unsync::spawn_blocking`, which
//! is `tokio::task::spawn_blocking`. It therefore needs a Tokio runtime on the
//! polling thread, and it is not cancellable from JavaScript.

use deno_core::Extension;

/// Builds the three extensions Web Crypto needs, in dependency order.
///
/// `deno_web` requires a blob store and a broadcast channel even though none of
/// its scripts are evaluated here. The location and CSS parser options are
/// disabled because they would add ambient capabilities that Web Crypto does
/// not need.
pub(crate) fn extensions() -> Vec<Extension> {
    vec![
        deno_webidl::deno_webidl::init(),
        deno_web::deno_web::init(
            deno_web::BlobStore::default_arc(),
            None,
            false,
            deno_web::InMemoryBroadcastChannel::default(),
        ),
        deno_crypto::deno_crypto::init(None),
    ]
}

/// Installs the Web Crypto globals and the DOM exception error builders.
///
/// Evaluation happens once per realm before user code runs. The `crypto`
/// property is the non-writable singleton `deno_crypto` builds on first access;
/// `Crypto`, `CryptoKey`, `SubtleCrypto`, and `DOMException` are configurable
/// globals like their browser counterparts. No other `deno_web` script is
/// loaded, so `Event`, `TextEncoder`, `URL`, and the stream globals do not
/// exist merely because Web Crypto does.
///
/// The `DOMException*` builders must run before the singleton is created,
/// because creating or using a class can reject a promise. This shim registers
/// them first, then installs `DOMException` and the four Web Crypto globals.
pub(crate) const WEBCRYPTO_SHIM: &str = r#"
(() => {
    "use strict";

    const core = Deno.core;

    // The DOMException implementation is not a cppgc class; it is plain
    // JavaScript from `deno_web`. Loading the script returns its exports and
    // also makes `webidl` available to it. `01_console.js` is pulled in lazily
    // by `00_crypto.js` for its custom-inspect helper.
    const {
        DOMException,
        QuotaExceededError,
    } = core.loadExtScript("ext:deno_web/01_dom_exception.js");

    // These are the five `DOMException*` classes `deno_crypto` can raise.
    // Registering a builder twice throws, so this runs exactly once per realm.
    core.registerErrorBuilder(
        "DOMExceptionOperationError",
        (message) => new DOMException(message, "OperationError"),
    );
    core.registerErrorBuilder(
        "DOMExceptionQuotaExceededError",
        (message) => new QuotaExceededError(message),
    );
    core.registerErrorBuilder(
        "DOMExceptionNotSupportedError",
        (message) => new DOMException(message, "NotSupportedError"),
    );
    core.registerErrorBuilder(
        "DOMExceptionDataError",
        (message) => new DOMException(message, "DataError"),
    );
    core.registerErrorBuilder(
        "DOMExceptionTypeMismatchError",
        (message) => new DOMException(message, "TypeMismatchError"),
    );

    const webcrypto = core.loadExtScript("ext:deno_crypto/00_crypto.js");

    const define = (name, value) => {
        Object.defineProperty(globalThis, name, {
            __proto__: null,
            value,
            writable: true,
            enumerable: false,
            configurable: true,
        });
    };
    define("DOMException", DOMException);
    define("CryptoKey", webcrypto.CryptoKey);
    define("Crypto", webcrypto.Crypto);
    define("SubtleCrypto", webcrypto.SubtleCrypto);
    // The singleton is writable: false like Deno's global. `webcrypto.crypto`
    // mints the `Crypto` instance (and its `SubtleCrypto`) on first read.
    Object.defineProperty(globalThis, "crypto", {
        __proto__: null,
        value: webcrypto.crypto,
        writable: false,
        enumerable: false,
        configurable: true,
    });
})();
"#;

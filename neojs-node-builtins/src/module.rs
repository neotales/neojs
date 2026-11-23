//! The Node module registry owned by this opt-in layer.
//!
//! Node modules are not NeoJS built-ins. They are never placed in the `Neo`
//! registry, and they have no bare or `neo:` alias. A Node module is addressed
//! only by its exact `node:` specifier.
//!
//! Resolution for an unsupported Node module fails with a reason. It never
//! falls through to a NeoJS module of a similar name, because that would run a
//! different API under a Node specifier.

/// The one implemented Node module specifier.
pub const CHILD_PROCESS_SPECIFIER: &str = "node:child_process";

/// The exact Node module specifiers implemented by this crate.
pub const SUPPORTED_MODULES: &[&str] = &[CHILD_PROCESS_SPECIFIER];

const CHILD_PROCESS_SOURCE: &str = include_str!("child_process.js");

/// Returns the ECMAScript module source for a supported Node specifier.
///
/// `None` means the module is not implemented. Callers must report
/// [`unsupported_module_reason`] rather than trying another registry.
pub fn module_source(specifier: &str) -> Option<&'static str> {
    match specifier {
        CHILD_PROCESS_SPECIFIER => Some(CHILD_PROCESS_SOURCE),
        _ => None,
    }
}

/// Returns a reason for an unsupported Node specifier.
///
/// The message names the requested module and the supported set. It never
/// substitutes a different module.
pub fn unsupported_module_reason(specifier: &str) -> String {
    format!(
        "the Node module `{specifier}` is not implemented; supported Node modules: {}",
        SUPPORTED_MODULES.join(", ")
    )
}

/// Returns whether `name` is a familiar bare Node module name.
///
/// This is used only to give a precise "use the `node:` prefix" error. It is
/// not a resolution table: a known bare name still resolves to nothing.
pub fn is_known_node_bare_name(name: &str) -> bool {
    matches!(
        name,
        "assert"
            | "async_hooks"
            | "buffer"
            | "child_process"
            | "cluster"
            | "console"
            | "constants"
            | "crypto"
            | "dgram"
            | "diagnostics_channel"
            | "dns"
            | "domain"
            | "events"
            | "fs"
            | "http"
            | "http2"
            | "https"
            | "inspector"
            | "module"
            | "net"
            | "os"
            | "path"
            | "perf_hooks"
            | "process"
            | "punycode"
            | "querystring"
            | "readline"
            | "repl"
            | "stream"
            | "string_decoder"
            | "sys"
            | "timers"
            | "tls"
            | "trace_events"
            | "tty"
            | "url"
            | "util"
            | "v8"
            | "vm"
            | "wasi"
            | "worker_threads"
            | "zlib"
    )
}

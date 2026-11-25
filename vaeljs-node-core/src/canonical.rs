//! Canonical `node:*` module registry and bare-specifier classification.
//!
//! Runtimes use [`SUPPORTED_NODE_MODULES`] to route canonical `node:*` imports
//! and [`is_known_bare_node_module`] to guarantee that bare Node module names
//! (such as `"fs"` or `"path"`) are never silently aliased to `vael:*` modules.

/// Canonical `node:*` module specifiers backed by `vaeljs-node-core`.
pub const SUPPORTED_NODE_MODULES: &[&str] = &[
    "node:path",
    "node:path/posix",
    "node:path/win32",
    "node:os",
    "node:fs",
    "node:fs/promises",
    "node:process",
    "node:child_process",
    "node:url",
    "node:buffer",
    "node:crypto",
];

/// Bare Node.js built-in module names (sorted for binary search).
///
/// Any specifier matching one of these bare names must be rejected or resolved
/// as a bare/package import according to runtime policy—never silently
/// rewritten to `vael:*`.
const KNOWN_BARE_NODE_MODULES: &[&str] = &[
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "sqlite",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "test",
    "test/reporters",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
];

/// Returns `true` if `specifier` is in [`SUPPORTED_NODE_MODULES`].
///
/// # Example
///
/// ```
/// use vaeljs_node_core::canonical::is_supported_node_module;
///
/// assert!(is_supported_node_module("node:path"));
/// assert!(is_supported_node_module("node:fs/promises"));
/// assert!(!is_supported_node_module("path"));
/// assert!(!is_supported_node_module("vael:fs"));
/// ```
#[must_use]
pub fn is_supported_node_module(specifier: &str) -> bool {
    SUPPORTED_NODE_MODULES.contains(&specifier)
}

/// Returns `true` when `specifier` is a known bare Node.js built-in module
/// name (for example `"fs"`, `"path"`, `"child_process"`, or `"fs/promises"`).
///
/// Prefixed specifiers such as `"node:fs"`, `"vael:fs"`, or `"./fs"` return
/// `false`. Runtimes use this helper to ensure bare Node names are never
/// silently aliased to `vael:` built-ins.
///
/// # Example
///
/// ```
/// use vaeljs_node_core::canonical::is_known_bare_node_module;
///
/// assert!(is_known_bare_node_module("fs"));
/// assert!(is_known_bare_node_module("fs/promises"));
/// assert!(is_known_bare_node_module("child_process"));
/// assert!(!is_known_bare_node_module("node:fs"));
/// assert!(!is_known_bare_node_module("vael:fs"));
/// assert!(!is_known_bare_node_module("./fs"));
/// ```
#[must_use]
pub fn is_known_bare_node_module(specifier: &str) -> bool {
    KNOWN_BARE_NODE_MODULES.binary_search(&specifier).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_bare_modules_list_is_sorted_and_unique() {
        for pair in KNOWN_BARE_NODE_MODULES.windows(2) {
            assert!(
                pair[0] < pair[1],
                "KNOWN_BARE_NODE_MODULES must be strictly sorted: {:?} >= {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn every_supported_node_module_has_matching_bare_name() {
        for spec in SUPPORTED_NODE_MODULES {
            let bare = spec
                .strip_prefix("node:")
                .expect("supported specifier must start with node:");
            assert!(
                is_known_bare_node_module(bare),
                "bare name {bare:?} must be classified"
            );
            assert!(is_supported_node_module(spec));
            assert!(!is_known_bare_node_module(spec));
        }
    }
}

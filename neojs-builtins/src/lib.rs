//! Engine-independent Neo built-in module implementations.
//!
//! This crate holds the ECMAScript source and metadata for built-in modules
//! that both the V8 runtime (`neojs-core`) and the QuickJS runtime
//! (`neojs-lite-core`) implement. It deliberately depends on neither
//! `deno_core` nor `rquickjs`, so it can sit in both dependency graphs without
//! moving an engine across the separation boundary. It also performs no I/O and
//! has no dependencies of its own.
//!
//! # What is shared
//!
//! A module contributes three pieces:
//!
//! * an **implementation** expression that evaluates to the frozen public
//!   module object and captures the intrinsics it uses;
//! * a **canonical module wrapper** that re-exports that object from
//!   `Neo.getBuiltInModule("<canonical specifier>")`;
//! * a [`BuiltinModule`] descriptor with the canonical specifier, the exact
//!   export names, and both sources.
//!
//! Each engine keeps its own realm shim and loader/registry adapter. The engine
//! decides where to splice the implementation and how to accept a specifier;
//! the implementation and wrapper text are the same in both.
//!
//! # Module selection
//!
//! Every shared module has its own Cargo feature. `chars` is the first. The
//! feature is off by default; a dependent that wants the module enables it.
//! `neojs-core` enables `chars` so a default V8 build keeps the existing
//! surface, and `neojs-lite-core` enables it explicitly. Cargo features are
//! additive and unify across the dependency graph, so a build that another
//! dependent enables a module for contains that module. Selecting modules is
//! compile-time surface selection, not a sandbox.
//!
//! # Precise limits
//!
//! * The shared source is ECMAScript, not Rust. It uses Unicode property
//!   escapes (`\p{...}` with the `u` flag), full case mapping through
//!   `String.prototype.toLowerCase`/`toUpperCase`, and bound intrinsics. Both
//!   engines implement these, but their Unicode tables can be from different
//!   releases. A code point assigned in a newer release can classify
//!   differently between V8 and QuickJS; the contract tests use long-assigned
//!   code points and do not claim a fixed Unicode version.
//! * The implementation captures intrinsics when the realm shim is evaluated.
//!   Replacing those intrinsics afterwards does not affect the module. Code
//!   that replaced them before the shim ran is not defended against.
//! * The shared text is not a permission, capability, or isolation boundary.

/// One shared built-in module, described without reference to an engine.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinModule {
    /// The canonical `neo:` import specifier and the registry key.
    pub specifier: &'static str,
    /// The named exports, in the implementation's property order.
    pub exports: &'static [&'static str],
    /// An ECMAScript expression that evaluates to the frozen module object.
    pub implementation: &'static str,
    /// The canonical ECMAScript module wrapper served by a loader.
    pub source: &'static str,
}

#[cfg(feature = "chars")]
pub mod chars;

/// Every shared module this build selected, in registration order.
#[cfg(feature = "chars")]
pub static MODULES: &[BuiltinModule] = &[chars::MODULE];

/// Every shared module this build selected. Empty when no module feature is on.
#[cfg(not(feature = "chars"))]
pub static MODULES: &[BuiltinModule] = &[];

/// Returns the shared module for a canonical specifier, or `None`.
///
/// The lookup is exact: a bare name, a different scheme, a query, or a fragment
/// does not match. Each engine's loader decides whether it also accepts a
/// legacy spelling and normalizes it before calling this function.
pub fn module(specifier: &str) -> Option<&'static BuiltinModule> {
    MODULES
        .iter()
        .find(|builtin| builtin.specifier == specifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_is_canonical_and_exact() {
        #[cfg(feature = "chars")]
        {
            assert_eq!(MODULES.len(), 1);
            assert_eq!(
                module("neo:chars").map(|builtin| builtin.specifier),
                Some("neo:chars")
            );
            assert!(module("chars").is_none());
            assert!(module("NEO:chars").is_none());
            assert!(module("neo:chars?x").is_none());
            assert!(module("neo:chars#x").is_none());
            assert!(module("neo:strings").is_none());
            assert_eq!(chars::MODULE.specifier, "neo:chars");
        }
        #[cfg(not(feature = "chars"))]
        {
            assert!(MODULES.is_empty());
            assert!(module("neo:chars").is_none());
        }
    }

    #[test]
    fn every_descriptor_has_source_and_exports() {
        for builtin in MODULES {
            assert!(builtin.specifier.starts_with("neo:"));
            assert!(!builtin.exports.is_empty());
            assert!(!builtin.implementation.trim().is_empty());
            assert!(!builtin.source.trim().is_empty());
        }
    }
}

//! Engine-neutral Neo built-in module implementations.
//!
//! This crate holds Rust implementations and metadata for built-in modules that
//! both the V8 runtime (`neojs-core`) and the QuickJS runtime
//! (`neojs-lite-core`) expose. It deliberately depends on neither `deno_core`
//! nor `rquickjs`, so it can sit in both dependency graphs without moving an
//! engine across the separation boundary. It performs no I/O.
//!
//! # What is shared
//!
//! A module contributes:
//!
//! * a Rust implementation with total predicates and explicit error results;
//! * a [`BuiltinModule`] descriptor with the canonical specifier and the exact
//!   export names.
//!
//! Each engine keeps its own binding. The V8 runtime creates native V8 function
//! objects and the QuickJS runtime exports native QuickJS functions. There is no
//! ECMAScript implementation source and no ECMAScript module wrapper in this
//! crate.
//!
//! # Module selection
//!
//! Every shared module has its own Cargo feature. `chars` is the first. The
//! feature is off by default; a dependent that wants the module enables it.
//! `neojs-core` and `neojs-lite-core` enable it in their default feature sets.
//! Cargo features are additive and unify across the dependency graph, so a build
//! that another dependent enables a module for contains that module. Selecting
//! modules is compile-time surface selection, not a sandbox.
//!
//! # Precise limits
//!
//! * Character classification uses the Unicode General_Category tables in the
//!   [`unicode_general_category`] crate, fixed at Unicode 16.0. Case mapping
//!   uses the Rust compiler's `char::to_lowercase` and `char::to_uppercase`
//!   tables. Those tables are not tied to the category table's release: a
//!   compiler upgrade can change a case mapping without changing
//!   classification, and either can differ from the engine tables used by the
//!   previous JavaScript implementation.
//! * The module has no host op, no capability, and no I/O. It is not a sandbox.
//! * The Rust functions are not a stable ABI. The public contract is the
//!   JavaScript module behavior; this crate's exact Rust signatures are
//!   internal to the two engines.

/// The backend that implements a shared module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Implemented by Rust functions in this crate and bound natively by each
    /// engine. There is no ECMAScript source.
    Rust,
}

/// One shared built-in module, described without reference to an engine.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinModule {
    /// The canonical `neo:` import specifier and the registry key.
    pub specifier: &'static str,
    /// The named exports, in the implementation's property order.
    pub exports: &'static [&'static str],
    /// How the module is implemented.
    pub backend: Backend,
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
            assert_eq!(chars::MODULE.backend, Backend::Rust);
        }
        #[cfg(not(feature = "chars"))]
        {
            assert!(MODULES.is_empty());
            assert!(module("neo:chars").is_none());
        }
    }

    #[test]
    fn every_descriptor_has_exports_and_a_rust_backend() {
        for builtin in MODULES {
            assert!(builtin.specifier.starts_with("neo:"));
            assert!(!builtin.exports.is_empty());
            assert_eq!(builtin.backend, Backend::Rust);
        }
    }
}

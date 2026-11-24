//! Engine-neutral Neo built-in module implementations.
//!
//! This crate holds Rust and shared ECMAScript implementations and metadata for
//! built-in modules that both the V8 runtime (`neojs-core`) and the QuickJS
//! runtime (`neojs-lite-core`) expose. It deliberately depends on neither
//! `deno_core` nor `rquickjs`, so it can sit in both dependency graphs without
//! moving an engine across the separation boundary. It performs no I/O.
//!
//! # What is shared
//!
//! A module contributes:
//!
//! * either a Rust implementation (`Backend::Rust`, e.g. `chars` and `strings`)
//!   with total predicates and explicit error results, or a shared ECMAScript
//!   factory and ESM wrapper (`Backend::Ecmascript`, e.g. `assert` and `test`)
//!   parameterized by engine host hooks;
//! * a [`BuiltinModule`] descriptor with the canonical specifier and the exact
//!   export names.
//!
//! Each engine keeps its own binding. For Rust modules, V8 creates native V8
//! function objects and QuickJS exports native QuickJS functions. For shared
//! ECMAScript modules, each engine evaluates the shared factory once per realm
//! with its native host hooks and serves the shared `.esm.js` wrapper.
//!
//! # Module selection
//!
//! Every shared module has its own Cargo feature. `assert`, `chars`, `strings`,
//! and `test` are the current modules. The features are off by default; a
//! dependent that wants a module enables it. `neojs-core` enables `assert` and
//! `test` unconditionally and `chars`/`strings` in its default feature set;
//! `neojs-lite-core` enables all four in its default feature set. Cargo features
//! are additive and unify across the dependency graph, so a build that another
//! dependent enables a module for contains that module. Selecting modules is
//! compile-time surface selection, not a sandbox.
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
    /// Implemented by shared ECMAScript source in this crate with engine-provided
    /// host hooks.
    Ecmascript,
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

#[cfg(feature = "assert")]
pub mod assert;
#[cfg(feature = "chars")]
pub mod chars;
#[cfg(feature = "strings")]
pub mod strings;
#[cfg(feature = "test")]
pub mod test;

/// Every shared module this build selected, in registration order.
pub static MODULES: &[BuiltinModule] = &[
    #[cfg(feature = "assert")]
    assert::MODULE,
    #[cfg(feature = "chars")]
    chars::MODULE,
    #[cfg(feature = "strings")]
    strings::MODULE,
    #[cfg(feature = "test")]
    test::MODULE,
];

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
        #[cfg(feature = "assert")]
        {
            assert_eq!(
                module("neo:assert").map(|builtin| builtin.specifier),
                Some("neo:assert")
            );
            assert!(module("assert").is_none());
            assert!(module("NEO:assert").is_none());
            assert!(module("neo:assert?x").is_none());
            assert!(module("neo:assert#x").is_none());
            assert_eq!(assert::MODULE.specifier, "neo:assert");
            assert_eq!(assert::MODULE.backend, Backend::Ecmascript);
        }
        #[cfg(not(feature = "assert"))]
        {
            assert!(module("neo:assert").is_none());
        }

        #[cfg(feature = "chars")]
        {
            assert_eq!(
                module("neo:chars").map(|builtin| builtin.specifier),
                Some("neo:chars")
            );
            assert!(module("chars").is_none());
            assert!(module("NEO:chars").is_none());
            assert!(module("neo:chars?x").is_none());
            assert!(module("neo:chars#x").is_none());
            assert_eq!(chars::MODULE.specifier, "neo:chars");
            assert_eq!(chars::MODULE.backend, Backend::Rust);
        }
        #[cfg(not(feature = "chars"))]
        {
            assert!(module("neo:chars").is_none());
        }

        #[cfg(feature = "strings")]
        {
            assert_eq!(
                module("neo:strings").map(|builtin| builtin.specifier),
                Some("neo:strings")
            );
            assert!(module("strings").is_none());
            assert!(module("NEO:strings").is_none());
            assert!(module("neo:strings?x").is_none());
            assert!(module("neo:strings#x").is_none());
            assert_eq!(strings::MODULE.specifier, "neo:strings");
            assert_eq!(strings::MODULE.backend, Backend::Rust);
        }
        #[cfg(not(feature = "strings"))]
        {
            assert!(module("neo:strings").is_none());
        }

        #[cfg(feature = "test")]
        {
            assert_eq!(
                module("neo:test").map(|builtin| builtin.specifier),
                Some("neo:test")
            );
            assert!(module("test").is_none());
            assert!(module("NEO:test").is_none());
            assert!(module("neo:test?x").is_none());
            assert!(module("neo:test#x").is_none());
            assert_eq!(test::MODULE.specifier, "neo:test");
            assert_eq!(test::MODULE.backend, Backend::Ecmascript);
        }
        #[cfg(not(feature = "test"))]
        {
            assert!(module("neo:test").is_none());
        }

        #[cfg(not(any(
            feature = "assert",
            feature = "chars",
            feature = "strings",
            feature = "test"
        )))]
        assert!(MODULES.is_empty());
    }

    #[test]
    fn every_descriptor_has_exports_and_expected_backend() {
        for builtin in MODULES {
            assert!(builtin.specifier.starts_with("neo:"));
            assert!(!builtin.exports.is_empty());
            match builtin.specifier {
                "neo:chars" | "neo:strings" => assert_eq!(builtin.backend, Backend::Rust),
                "neo:assert" | "neo:test" => assert_eq!(builtin.backend, Backend::Ecmascript),
                other => panic!("unexpected built-in module: {other}"),
            }
        }
    }
}

//! Engine-neutral Vael built-in module implementations.
//!
//! This crate holds Rust and shared ECMAScript implementations and metadata for
//! built-in modules that both the V8 runtime (`vaeljs-core`) and the QuickJS
//! runtime (`vaeljs-lite-core`) expose. It deliberately depends on neither
//! `deno_core` nor `rquickjs`, so it can sit in both dependency graphs without
//! moving an engine across the separation boundary. It performs no I/O.
//!
//! # What is shared
//!
//! A module contributes:
//!
//! * either a Rust implementation (`Backend::Rust`, e.g. `chars`, `os`,
//!   `strings`, and `uuid`) with total predicates and explicit error results,
//!   or a shared ECMAScript factory and ESM wrapper (`Backend::Ecmascript`,
//!   e.g. `assert` and `test`) parameterized by engine host hooks;
//! * a [`BuiltinModule`] descriptor with the canonical specifier and the exact
//!   export names.
//!
//! Each engine keeps its own binding. For Rust modules, V8 creates native V8
//! function objects or host ops and QuickJS exports native QuickJS values and
//! functions. For shared ECMAScript modules, each engine evaluates the shared
//! factory once per realm with its native host hooks and serves the shared
//! `.esm.js` wrapper.
//!
//! # Module selection
//!
//! Every shared module has its own Cargo feature. `assert`, `chars`, `os`,
//! `strings`, `test`, and `uuid` are the current modules (`uuid` registers
//! `vael:uuid`, `vael:uuid/v4`, and `vael:uuid/v7`). The features are off by
//! default; a dependent that wants a module enables it. `vaeljs-core` enables
//! `assert`, `os`, `test`, and `uuid` unconditionally and `chars`/`strings` in
//! its default feature set; `vaeljs-lite-core` enables all six in its default
//! feature set. Cargo features are additive and unify across the dependency
//! graph, so a build that another dependent enables a module for contains that
//! module. Selecting modules is compile-time surface selection, not a sandbox.
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
    /// The canonical `vael:` import specifier and the registry key.
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
#[cfg(feature = "console")]
pub mod console;
#[cfg(feature = "os")]
pub mod os;
#[cfg(feature = "path")]
pub mod path;
#[cfg(feature = "slices")]
pub mod slices;
#[cfg(feature = "strings")]
pub mod strings;
#[cfg(feature = "test")]
pub mod test;
#[cfg(feature = "uuid")]
pub mod uuid;

/// Every shared module this build selected, in registration order.
pub static MODULES: &[BuiltinModule] = &[
    #[cfg(feature = "assert")]
    assert::MODULE,
    #[cfg(feature = "chars")]
    chars::MODULE,
    #[cfg(feature = "console")]
    console::MODULE,
    #[cfg(feature = "os")]
    os::MODULE,
    #[cfg(feature = "path")]
    path::MODULE,
    #[cfg(feature = "slices")]
    slices::MODULE,
    #[cfg(feature = "strings")]
    strings::MODULE,
    #[cfg(feature = "test")]
    test::MODULE,
    #[cfg(feature = "uuid")]
    uuid::MODULE,
    #[cfg(feature = "uuid")]
    uuid::V4_MODULE,
    #[cfg(feature = "uuid")]
    uuid::V7_MODULE,
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
                module("vael:assert").map(|builtin| builtin.specifier),
                Some("vael:assert")
            );
            assert!(module("assert").is_none());
            assert!(module("VAEL:assert").is_none());
            assert!(module("vael:assert?x").is_none());
            assert!(module("vael:assert#x").is_none());
            assert_eq!(assert::MODULE.specifier, "vael:assert");
            assert_eq!(assert::MODULE.backend, Backend::Ecmascript);
        }
        #[cfg(not(feature = "assert"))]
        {
            assert!(module("vael:assert").is_none());
        }

        #[cfg(feature = "chars")]
        {
            assert_eq!(
                module("vael:chars").map(|builtin| builtin.specifier),
                Some("vael:chars")
            );
            assert!(module("chars").is_none());
            assert!(module("VAEL:chars").is_none());
            assert!(module("vael:chars?x").is_none());
            assert!(module("vael:chars#x").is_none());
            assert_eq!(chars::MODULE.specifier, "vael:chars");
            assert_eq!(chars::MODULE.backend, Backend::Rust);
        }
        #[cfg(not(feature = "chars"))]
        {
            assert!(module("vael:chars").is_none());
        }

        #[cfg(feature = "console")]
        {
            assert_eq!(
                module("vael:console").map(|builtin| builtin.specifier),
                Some("vael:console")
            );
            assert!(module("console").is_none());
            assert!(module("VAEL:console").is_none());
            assert!(module("vael:console?x").is_none());
            assert!(module("vael:console#x").is_none());
            assert_eq!(console::MODULE.specifier, "vael:console");
            assert_eq!(console::MODULE.backend, Backend::Ecmascript);
        }
        #[cfg(not(feature = "console"))]
        {
            assert!(module("vael:console").is_none());
        }

        #[cfg(feature = "os")]
        {
            assert_eq!(
                module("vael:os").map(|builtin| builtin.specifier),
                Some("vael:os")
            );
            assert!(module("os").is_none());
            assert!(module("VAEL:os").is_none());
            assert!(module("vael:os?x").is_none());
            assert!(module("vael:os#x").is_none());
            assert_eq!(os::MODULE.specifier, "vael:os");
            assert_eq!(os::MODULE.backend, Backend::Rust);
        }
        #[cfg(not(feature = "os"))]
        {
            assert!(module("vael:os").is_none());
        }

        #[cfg(feature = "path")]
        {
            assert_eq!(
                module("vael:path").map(|builtin| builtin.specifier),
                Some("vael:path")
            );
            assert!(module("path").is_none());
            assert!(module("VAEL:path").is_none());
            assert!(module("vael:path?x").is_none());
            assert!(module("vael:path#x").is_none());
            assert_eq!(path::MODULE.specifier, "vael:path");
            assert_eq!(path::MODULE.backend, Backend::Ecmascript);
        }
        #[cfg(not(feature = "path"))]
        {
            assert!(module("vael:path").is_none());
        }

        #[cfg(feature = "slices")]
        {
            assert_eq!(
                module("vael:slices").map(|builtin| builtin.specifier),
                Some("vael:slices")
            );
            assert!(module("slices").is_none());
            assert!(module("VAEL:slices").is_none());
            assert!(module("vael:slices?x").is_none());
            assert!(module("vael:slices#x").is_none());
            assert_eq!(slices::MODULE.specifier, "vael:slices");
            assert_eq!(slices::MODULE.backend, Backend::Ecmascript);
        }
        #[cfg(not(feature = "slices"))]
        {
            assert!(module("vael:slices").is_none());
        }

        #[cfg(feature = "strings")]
        {
            assert_eq!(
                module("vael:strings").map(|builtin| builtin.specifier),
                Some("vael:strings")
            );
            assert!(module("strings").is_none());
            assert!(module("VAEL:strings").is_none());
            assert!(module("vael:strings?x").is_none());
            assert!(module("vael:strings#x").is_none());
            assert_eq!(strings::MODULE.specifier, "vael:strings");
            assert_eq!(strings::MODULE.backend, Backend::Rust);
        }
        #[cfg(not(feature = "strings"))]
        {
            assert!(module("vael:strings").is_none());
        }

        #[cfg(feature = "test")]
        {
            assert_eq!(
                module("vael:test").map(|builtin| builtin.specifier),
                Some("vael:test")
            );
            assert!(module("test").is_none());
            assert!(module("VAEL:test").is_none());
            assert!(module("vael:test?x").is_none());
            assert!(module("vael:test#x").is_none());
            assert_eq!(test::MODULE.specifier, "vael:test");
            assert_eq!(test::MODULE.backend, Backend::Ecmascript);
        }
        #[cfg(not(feature = "test"))]
        {
            assert!(module("vael:test").is_none());
        }

        #[cfg(feature = "uuid")]
        {
            for specifier in ["vael:uuid", "vael:uuid/v4", "vael:uuid/v7"] {
                assert_eq!(
                    module(specifier).map(|builtin| builtin.specifier),
                    Some(specifier)
                );
            }
            for invalid in [
                "uuid",
                "uuid/v4",
                "uuid/v7",
                "VAEL:uuid",
                "vael:uuid?x",
                "vael:uuid#x",
            ] {
                assert!(module(invalid).is_none(), "{invalid}");
            }
            assert_eq!(uuid::MODULE.specifier, "vael:uuid");
            assert_eq!(uuid::MODULE.backend, Backend::Rust);
            assert_eq!(uuid::V4_MODULE.specifier, "vael:uuid/v4");
            assert_eq!(uuid::V4_MODULE.backend, Backend::Rust);
            assert_eq!(uuid::V7_MODULE.specifier, "vael:uuid/v7");
            assert_eq!(uuid::V7_MODULE.backend, Backend::Rust);
        }
        #[cfg(not(feature = "uuid"))]
        {
            assert!(module("vael:uuid").is_none());
            assert!(module("vael:uuid/v4").is_none());
            assert!(module("vael:uuid/v7").is_none());
        }

        #[cfg(not(any(
            feature = "assert",
            feature = "chars",
            feature = "console",
            feature = "os",
            feature = "path",
            feature = "slices",
            feature = "strings",
            feature = "test",
            feature = "uuid"
        )))]
        assert!(MODULES.is_empty());
    }

    #[test]
    fn every_descriptor_has_exports_and_expected_backend() {
        for builtin in MODULES {
            assert!(builtin.specifier.starts_with("vael:"));
            assert!(!builtin.exports.is_empty());
            match builtin.specifier {
                "vael:chars" | "vael:os" | "vael:strings" | "vael:uuid" | "vael:uuid/v4"
                | "vael:uuid/v7" => {
                    assert_eq!(builtin.backend, Backend::Rust)
                }
                "vael:assert" | "vael:console" | "vael:path" | "vael:slices" | "vael:test" => {
                    assert_eq!(builtin.backend, Backend::Ecmascript)
                }
                other => panic!("unexpected built-in module: {other}"),
            }
        }
    }
}

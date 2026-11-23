//! The shared `neo:chars` implementation and canonical module wrapper.
//!
//! `IMPLEMENTATION` is an ECMAScript expression. A realm shim evaluates it to
//! get the frozen module object; the V8 shim declares it as `const chars`, and
//! the QuickJS shim does the same before adding it to its registry. `SOURCE` is
//! the canonical wrapper served for `neo:chars`; it reads that registry object
//! so imports and registry lookups share function identity.
//!
//! The module has no host op, no capability, and no I/O. It classifies Unicode
//! scalar values and returns one-code-point case mappings. It is not a sandbox
//! and does not defend against intrinsics replaced before the shim ran.

use crate::BuiltinModule;

/// The canonical import specifier and registry key for `chars`.
pub const SPECIFIER: &str = "neo:chars";

/// The named exports, in the implementation's property order.
pub const EXPORTS: &[&str] = &[
    "isChar",
    "isAscii",
    "isLatin1",
    "isControl",
    "isDigit",
    "isLetter",
    "isLetterOrDigit",
    "isLower",
    "isUpper",
    "isPunc",
    "isSpace",
    "isSymbol",
    "toLower",
    "toUpper",
];

/// An ECMAScript expression that returns the frozen `chars` module object.
///
/// It captures every intrinsic it uses when it is evaluated. See the source
/// file for the exact classification and case-mapping contract.
pub const IMPLEMENTATION: &str = include_str!("js/chars.js");

/// The canonical `neo:chars` ECMAScript module wrapper.
///
/// It reads `Neo.getBuiltInModule("neo:chars")` and re-exports the function
/// objects, so the module namespace and the registry object share identity.
pub const SOURCE: &str = include_str!("js/chars.esm.js");

/// The shared module descriptor.
pub const MODULE: BuiltinModule = BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    implementation: IMPLEMENTATION,
    source: SOURCE,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_list_matches_the_wrapper_and_the_object() {
        for name in EXPORTS {
            assert!(
                SOURCE.contains(&format!("export const {name} = chars.{name};")),
                "the wrapper is missing the `{name}` export"
            );
            assert!(
                IMPLEMENTATION.contains(&format!("        {name},\n")),
                "the implementation is missing the `{name}` property"
            );
        }
        assert!(!EXPORTS.contains(&"default"));
        assert!(!SOURCE.contains("export default"));
    }

    #[test]
    fn the_wrapper_uses_the_canonical_registry_key_only() {
        assert!(SOURCE.contains(r#"Neo.getBuiltInModule("neo:chars")"#));
        assert!(!SOURCE.contains(r#"Neo.getBuiltInModule("chars")"#));
        assert!(!SOURCE.contains(r#"from "chars""#));
    }

    #[test]
    fn the_implementation_is_an_expression_and_captures_its_intrinsics() {
        assert!(!IMPLEMENTATION.contains("\nimport "));
        assert!(!IMPLEMENTATION.contains("\nexport "));
        for captured in [
            "const isInteger = Number.isInteger;",
            "const fromCodePoint = String.fromCodePoint;",
            "const regexpExec = Function.prototype.call.bind(RegExp.prototype.exec);",
            "const lowerCase = Function.prototype.call.bind(String.prototype.toLowerCase);",
            "const upperCase = Function.prototype.call.bind(String.prototype.toUpperCase);",
            "const freeze = Object.freeze;",
        ] {
            assert!(
                IMPLEMENTATION.contains(captured),
                "missing captured intrinsic: {captured}"
            );
        }
        assert!(IMPLEMENTATION.contains("\\p{L}"));
        assert!(IMPLEMENTATION.contains("return freeze({"));
    }
}

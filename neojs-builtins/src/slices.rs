//! Shared metadata and ECMAScript source for the `neo:slices` built-in module.
//!
//! The runtime logic lives in `slices.js` as a self-contained factory function
//! parameterized by `chars` case-mapping hooks (`{ toUpper, toLower }`), plus
//! `slices.esm.js` for the ES module wrapper. Neither file depends on V8 or
//! QuickJS.

use crate::{Backend, BuiltinModule};

/// The canonical `neo:slices` module specifier and registry key.
pub const SPECIFIER: &str = "neo:slices";

/// The named exports of `neo:slices`, matching `slices.esm.js`.
pub const EXPORTS: &[&str] = &[
    "CharSlice",
    "ReadOnlyCharSlice",
    "ReadOnlySlice",
    "Slice",
    "charSlice",
    "readonlyCharSlice",
    "readonlySlice",
    "slice",
];

/// Factory expression `(function createNeoSlices(host) { ... })` returning the
/// frozen `neo:slices` module object.
pub const FACTORY_SOURCE: &str = include_str!("slices.js");

/// ECMAScript module source re-exporting `Neo.getBuiltInModule("neo:slices")`.
pub const ESM_SOURCE: &str = include_str!("slices.esm.js");

/// Shared descriptor for `neo:slices`.
pub const MODULE: BuiltinModule = BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: Backend::Ecmascript,
};

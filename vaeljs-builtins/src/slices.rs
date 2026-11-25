//! Shared metadata and ECMAScript source for the `vael:slices` built-in module.
//!
//! The runtime logic lives in `slices.js` as a self-contained factory function
//! parameterized by `chars` case-mapping hooks (`{ toUpper, toLower }`), plus
//! `slices.esm.js` for the ES module wrapper. Neither file depends on V8 or
//! QuickJS.

use crate::{Backend, BuiltinModule};

/// The canonical `vael:slices` module specifier and registry key.
pub const SPECIFIER: &str = "vael:slices";

/// The named exports of `vael:slices`, matching `slices.esm.js`.
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

/// Factory expression `(function createVaelSlices(host) { ... })` returning the
/// frozen `vael:slices` module object.
pub const FACTORY_SOURCE: &str = include_str!("slices.js");

/// ECMAScript module source re-exporting `Vael.getBuiltInModule("vael:slices")`.
pub const ESM_SOURCE: &str = include_str!("slices.esm.js");

/// Shared descriptor for `vael:slices`.
pub const MODULE: BuiltinModule = BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: Backend::Ecmascript,
};

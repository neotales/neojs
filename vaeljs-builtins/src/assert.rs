//! Shared metadata and ECMAScript source for the `vael:assert` built-in module.
//!
//! The runtime logic lives in `assert.js` as a self-contained factory function
//! parameterized by two engine-provided Promise observation hooks
//! (`isNativePromise` and `observePromise`), plus `assert.esm.js` for the ES
//! module wrapper. Neither file depends on V8 or QuickJS.

use crate::{Backend, BuiltinModule};

/// The canonical `vael:assert` module specifier and registry key.
pub const SPECIFIER: &str = "vael:assert";

/// The named exports of `vael:assert`, matching `assert.esm.js`.
pub const EXPORTS: &[&str] = &[
    "default",
    "assert",
    "AssertionError",
    "ok",
    "equals",
    "notEquals",
    "strictEquals",
    "notStrictEquals",
    "fail",
    "throws",
    "doesNotThrow",
    "rejects",
    "doesNotReject",
];

/// Factory expression `(function createVaelAssert(host) { ... })` returning the
/// frozen callable `assert` module object.
pub const FACTORY_SOURCE: &str = include_str!("assert.js");

/// ECMAScript module source re-exporting `Vael.getBuiltInModule("vael:assert")`.
pub const ESM_SOURCE: &str = include_str!("assert.esm.js");

/// Shared descriptor for `vael:assert`.
pub const MODULE: BuiltinModule = BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: Backend::Ecmascript,
};

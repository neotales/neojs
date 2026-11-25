//! TypeScript entry and module files, reduced to ECMAScript by erasing type
//! syntax.
//!
//! This module delegates to [`vaeljs_module_policy::typescript`], which is
//! shared with `vaeljs-lite-core` so V8 and QuickJS accept and reject the exact
//! same erasable TypeScript subset and emit identical diagnostics.

use deno_core::url::Url;

/// Erases TypeScript syntax, returning ECMAScript that keeps source positions.
///
/// Delegates directly to [`vaeljs_module_policy::typescript::strip`].
pub(crate) fn strip(specifier: &Url, source: String) -> Result<String, String> {
    vaeljs_module_policy::typescript::strip(specifier, source)
}

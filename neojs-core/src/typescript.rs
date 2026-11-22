//! TypeScript entry and module files, reduced to ECMAScript by erasing type
//! syntax.
//!
//! neojs runs the *erasable* subset of TypeScript: annotations, interfaces, type
//! aliases, generics, `as`/`satisfies`, `import type`, and the other constructs
//! that can be deleted without changing what the program does. Constructs that
//! need a transform are rejected with the stripper's reason instead of being
//! approximated.
//!
//! Type stripping does not type check. A type error is not reported, and the
//! program runs with whatever values it has at runtime. Erased text is replaced
//! by spaces, so line and column positions are preserved and a V8 stack trace
//! points into the original `.ts` source.
//!
//! `.tsx` and `.jsx` are not handled here: JSX needs parsing and code
//! generation, not erasure, and is not implemented. `.cts` is CommonJS
//! TypeScript and is not implemented either.

use deno_ast::MediaType;
use deno_ast::ParseParams;
use deno_ast::TypeStripOptions;
use deno_ast::parse_module;
use deno_ast::type_strip;
use deno_core::url::Url;

/// Erases TypeScript syntax, returning ECMAScript that keeps source positions.
///
/// `specifier` is the module's `file:` URL; the stripper uses it to name the
/// file in its diagnostics. The source must already be text, so a file that is
/// not valid UTF-8 fails before this function is reached.
///
/// The result is parsed as an ECMAScript module, which is what neojs executes.
/// Any error is a rejection, not a type error, because no type checking happens:
/// an unsupported construct names itself and its position, while invalid
/// TypeScript is reported with the parser's position and source excerpt. The
/// stripper consumes the source, so it is copied to locate a syntax error after
/// the fact.
pub(crate) fn strip(specifier: &Url, source: String) -> Result<String, String> {
    let original = source.clone();
    type_strip(specifier, source, TypeStripOptions { module: Some(true) })
        .map_err(|error| describe_error(specifier, &original, error.to_string()))
}

/// Returns a message for a failed strip, adding a position when it has none.
///
/// The stripper reports a position for the constructs it rejects, but a parse
/// failure is only `[InvalidSyntax] Syntax error`. Parsing the same source gives
/// a line, a column, and an excerpt. If the parser accepts the file, the syntax
/// is valid TypeScript that needs a transform, so the message says that instead
/// of leaving a bare "syntax error".
fn describe_error(specifier: &Url, source: &str, message: String) -> String {
    let positioned = format!(" at {specifier}:");
    if message.contains(&positioned) {
        return message;
    }
    let params = ParseParams {
        specifier: specifier.clone(),
        text: source.into(),
        media_type: MediaType::TypeScript,
        capture_tokens: false,
        scope_analysis: false,
        maybe_syntax: None,
    };
    match parse_module(params) {
        Err(diagnostic) => diagnostic.to_string(),
        Ok(_) => format!(
            "{message}; the file is valid TypeScript, so it uses syntax that type stripping cannot erase"
        ),
    }
}

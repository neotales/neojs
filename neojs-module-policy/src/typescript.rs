//! TypeScript entry and module files, reduced to ECMAScript by erasing type
//! syntax.
//!
//! Both `neojs-core` (V8) and `neojs-lite-core` (QuickJS) share this stripper
//! behind the opt-in `typescript` feature so they accept and reject the exact
//! same TypeScript syntax with identical diagnostics.
//!
//! neojs runs the *erasable* subset of TypeScript: annotations, interfaces, type
//! aliases, generics, `as`/`satisfies`, `import type`, and the other constructs
//! that can be deleted without changing what the program does. Constructs that
//! need a transform (such as `enum`, `namespace`, parameter properties,
//! `import = require(...)`, `export = ...`, or angle-bracket assertions) are
//! rejected with the stripper's reason instead of being approximated.
//!
//! Type stripping does not type check. A type error is not reported, and the
//! program runs with whatever values it has at runtime. Erased text is replaced
//! by spaces, so line and column positions are preserved and engine stack
//! traces point into the original `.ts` source.
//!
//! `.tsx` and `.jsx` are not handled here: JSX needs parsing and code
//! generation, not erasure, and is not implemented. `.cts` is CommonJS
//! TypeScript and is not implemented either.

use deno_ast::MediaType;
use deno_ast::ParseParams;
use deno_ast::TypeStripOptions;
use deno_ast::parse_module;
use deno_ast::type_strip;
use url::Url;

/// Erases TypeScript syntax, returning ECMAScript that keeps source positions.
///
/// `specifier` is the module's URL (typically a `file:` URL); the stripper uses
/// it to name the file in its diagnostics. The source must already be UTF-8
/// text, so a file that is not valid UTF-8 fails before this function is
/// reached.
///
/// The result is parsed as an ECMAScript module, which is what both neojs
/// runtimes execute. Any error is a rejection, not a type error, because no
/// type checking happens: an unsupported construct names itself and its
/// position, while invalid TypeScript is reported with the parser's position
/// and source excerpt. The stripper consumes the source, so it is copied to
/// locate a syntax error after the fact.
pub fn strip(specifier: &Url, source: String) -> Result<String, String> {
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

#[cfg(test)]
mod tests {
    use super::strip;
    use url::Url;

    fn file_url(name: &str) -> Url {
        Url::parse(&format!("file:///work/{name}")).expect("valid test URL")
    }

    #[test]
    fn strips_erasable_typescript_and_preserves_line_and_column_positions() {
        let source = concat!(
            "import type { Foo } from \"./never-loaded.d.ts\";\n",
            "interface Box<T> { value: T }\n",
            "type Pair<T> = [T, T];\n",
            "const identity = <T,>(value: T): T => value;\n",
            "const boxed: Box<number> = { value: 42 as number satisfies number };\n",
            "const pair: Pair<number> = [boxed.value, identity(7)];\n",
            "export const total: number = pair[0]! + pair[1];\n",
        );
        let original_lines: Vec<&str> = source.lines().collect();
        let stripped = strip(&file_url("main.ts"), source.to_owned()).expect("strip erasable TS");
        let stripped_lines: Vec<&str> = stripped.lines().collect();

        assert_eq!(stripped_lines.len(), original_lines.len());
        // `import type` and `interface` and `type` are erased to spaces.
        assert!(
            stripped_lines[0].trim().is_empty(),
            "{:?}",
            stripped_lines[0]
        );
        assert!(
            stripped_lines[1].trim().is_empty(),
            "{:?}",
            stripped_lines[1]
        );
        assert!(
            stripped_lines[2].trim().is_empty(),
            "{:?}",
            stripped_lines[2]
        );
        // Column positions of retained runtime tokens stay aligned.
        let orig_col = original_lines[4].find("{ value:").expect("orig col");
        let strip_col = stripped_lines[4].find("{ value:").expect("strip col");
        assert_eq!(strip_col, orig_col);
        assert!(stripped_lines[6].starts_with("export const total"));
        assert!(!stripped_lines[6].contains(": number"));
    }

    #[test]
    fn reports_syntax_error_with_file_position_and_excerpt() {
        let url = file_url("broken.ts");
        let error = strip(&url, "const value: = 1;\n".to_owned()).expect_err("syntax error");
        assert!(error.contains("SyntaxError"), "{error}");
        assert!(error.contains("file:///work/broken.ts:1:"), "{error}");
        assert!(error.contains("const value: = 1;"), "{error}");
    }

    #[test]
    fn rejects_non_erasable_typescript_constructs() {
        let cases = [
            (
                "enum.ts",
                "enum Color { Red }\n",
                "enum is not supported in strip-only mode",
            ),
            (
                "namespace.ts",
                "namespace N { export const x = 1; }\n",
                "namespace declaration is not supported",
            ),
            (
                "parameter.ts",
                "class A { constructor(private x: number) {} }\n",
                "parameter property is not supported",
            ),
            (
                "equals.ts",
                "import value = require(\"./value.js\");\n",
                "import equals declaration is not supported",
            ),
            (
                "assignment.ts",
                "export = 1;\n",
                "export assignment is not supported",
            ),
            (
                "angle.ts",
                "const value = <number>1;\n",
                "angle-bracket syntax",
            ),
            (
                "decorator.ts",
                "function dec() {}\n@dec class A {}\n",
                "valid TypeScript",
            ),
        ];

        for (name, source, expected) in cases {
            let error = strip(&file_url(name), source.to_owned()).expect_err(name);
            assert!(error.contains(expected), "{name}: {error}");
        }
    }
}

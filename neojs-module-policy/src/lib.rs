//! Engine-independent module policy shared by the neojs runtimes.
//!
//! This crate holds the decisions that do not depend on V8, QuickJS, or any
//! other JavaScript engine: which file extensions are executable modules, which
//! extensions are recognised but unsupported, and which specifiers are file
//! paths rather than bare module names. Both the V8 runtime (`neojs-core`) and
//! the QuickJS runtime (`neojs-lite-core`) depend on it, so an extension that one
//! engine rejects is rejected with the same reason by the other.
//!
//! The crate has no dependencies and performs no I/O. Engine-specific work —
//! reading files, type stripping, loading modules, and reporting JavaScript
//! errors — stays in each runtime.
//!
//! This is deliberately a small shared surface. It does not claim that the two
//! runtimes implement the same JavaScript surface: the QuickJS runtime currently
//! executes ECMAScript modules without TypeScript or host asynchrony.

use std::path::Path;

/// How a file would be executed, decided by its name alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModuleFormat {
    /// ECMAScript, passed to the engine unchanged.
    JavaScript,
    /// TypeScript, which a runtime must reduce to ECMAScript before evaluating.
    TypeScript,
}

/// The module type declared by the nearest `package.json` `"type"` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageType {
    /// `"type": "module"` — `.js` and `.ts` files are treated as ECMAScript/TypeScript modules.
    Module,
    /// `"type": "commonjs"` — `.js` files are CommonJS and rejected.
    CommonJs,
    /// `"type"` is omitted — `.js` defaults to CommonJS in npm packages and is rejected.
    Unspecified,
}

const JSX_REASON: &str =
    "JSX and TSX are not implemented; use .ts or .mts, or transpile the JSX first";
const CTS_REASON: &str = "CommonJS TypeScript (.cts) is not supported; use .ts or .mts instead";
const DECLARATION_REASON: &str =
    "declaration files (.d.ts, .d.mts, and .d.cts) describe types and contain no runnable code";
const COMMONJS_REASON: &str = "the .cjs extension is not supported; neojs runs ECMAScript modules";
const PACKAGE_COMMONJS_REASON: &str = "CommonJS modules (.js without \"type\": \"module\") are not supported; neojs runs ECMAScript modules";
const PACKAGE_EXPLICIT_COMMONJS_REASON: &str = "CommonJS modules (.js under \"type\": \"commonjs\") are not supported; neojs runs ECMAScript modules";
const PACKAGE_TS_TYPE_REASON: &str =
    "TypeScript (.ts) inside a package requires \"type\": \"module\" in the nearest package.json";
const PACKAGE_EXTENSIONLESS_REASON: &str = "extensionless files inside a package are not supported; use .mjs, .mts, or .js/.ts under \"type\": \"module\"";
const JSON_REASON: &str = "JSON modules are not implemented; only ECMAScript modules are loaded";
const WASM_REASON: &str =
    "WebAssembly modules are not implemented; only ECMAScript modules are loaded";
const NATIVE_REASON: &str = "native addons are not implemented";

/// Returns the format of a file, or why it is not an executable module.
///
/// `.js` and `.mjs` are JavaScript. `.ts` and `.mts` are TypeScript and must be
/// type-stripped by the calling runtime before evaluation. An absent extension
/// is accepted and treated as JavaScript. Declaration files, JSX and TSX,
/// CommonJS, JSON, WebAssembly, native addons, and unknown extensions are
/// rejected with a reason. The check is by name only; the file is not opened.
///
/// A runtime that does not implement TypeScript must reject
/// [`ModuleFormat::TypeScript`] explicitly rather than passing the source to an
/// engine. This function only classifies the name.
pub fn module_format(path: &Path) -> Result<ModuleFormat, String> {
    if is_declaration_file(path) {
        return Err(DECLARATION_REASON.to_string());
    }
    let Some(extension) = path.extension() else {
        return Ok(ModuleFormat::JavaScript);
    };
    let extension = match extension.to_str() {
        Some(extension) => extension.to_ascii_lowercase(),
        None => return Err("the file extension is not valid UTF-8".to_string()),
    };

    let reason = match extension.as_str() {
        "js" | "mjs" => return Ok(ModuleFormat::JavaScript),
        "ts" | "mts" => return Ok(ModuleFormat::TypeScript),
        "tsx" | "jsx" => JSX_REASON,
        "cts" => CTS_REASON,
        "cjs" => COMMONJS_REASON,
        "json" => JSON_REASON,
        "wasm" => WASM_REASON,
        "node" => NATIVE_REASON,
        other => {
            return Err(format!(
                "unknown module extension `.{other}`; neojs loads ECMAScript modules (.js or .mjs) and erasable TypeScript (.ts or .mts)"
            ));
        }
    };
    Err(reason.to_string())
}

/// Returns the format of a file inside a package whose nearest `package.json`
/// declares `nearest_type`, or why it is not an executable module.
///
/// Unlike [`module_format`], this applies strict package-boundary rules:
/// - `.mjs` is always [`ModuleFormat::JavaScript`].
/// - `.mts` is always [`ModuleFormat::TypeScript`].
/// - `.js` is [`ModuleFormat::JavaScript`] only when `nearest_type` is
///   [`PackageType::Module`]; [`PackageType::CommonJs`] and
///   [`PackageType::Unspecified`] fail closed as unsupported CommonJS.
/// - `.ts` is [`ModuleFormat::TypeScript`] only when `nearest_type` is
///   [`PackageType::Module`].
/// - `.cjs`, `.cts`, `.d.ts`, `.d.mts`, `.d.cts`, `.tsx`, `.jsx`, `.json`,
///   `.wasm`, `.node`, extensionless files, and unknown extensions fail closed.
pub fn package_module_format(
    path: &Path,
    nearest_type: PackageType,
) -> Result<ModuleFormat, String> {
    if is_declaration_file(path) {
        return Err(DECLARATION_REASON.to_string());
    }
    let Some(extension) = path.extension() else {
        return Err(PACKAGE_EXTENSIONLESS_REASON.to_string());
    };
    let extension = match extension.to_str() {
        Some(extension) => extension.to_ascii_lowercase(),
        None => return Err("the file extension is not valid UTF-8".to_string()),
    };

    let reason = match extension.as_str() {
        "mjs" => return Ok(ModuleFormat::JavaScript),
        "mts" => return Ok(ModuleFormat::TypeScript),
        "js" => match nearest_type {
            PackageType::Module => return Ok(ModuleFormat::JavaScript),
            PackageType::CommonJs => PACKAGE_EXPLICIT_COMMONJS_REASON,
            PackageType::Unspecified => PACKAGE_COMMONJS_REASON,
        },
        "ts" => match nearest_type {
            PackageType::Module => return Ok(ModuleFormat::TypeScript),
            PackageType::CommonJs | PackageType::Unspecified => PACKAGE_TS_TYPE_REASON,
        },
        "tsx" | "jsx" => JSX_REASON,
        "cts" => CTS_REASON,
        "cjs" => COMMONJS_REASON,
        "json" => JSON_REASON,
        "wasm" => WASM_REASON,
        "node" => NATIVE_REASON,
        other => {
            return Err(format!(
                "unknown module extension `.{other}`; neojs loads ECMAScript modules (.mjs, or .js under \"type\": \"module\") and erasable TypeScript (.mts, or .ts under \"type\": \"module\")"
            ));
        }
    };
    Err(reason.to_string())
}

/// Returns whether a file name is a declaration file.
///
/// `.d.ts`, `.d.mts`, and `.d.cts` are recognised for identity only; they
/// contain no runnable code.
fn is_declaration_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".d.ts") || lower.ends_with(".d.mts") || lower.ends_with(".d.cts")
}

/// Returns whether a specifier is written as a filesystem path rather than a
/// bare module name.
///
/// This recognises POSIX absolute paths, POSIX relative paths, Windows relative
/// paths written with backslashes, and Windows UNC paths. It does not decide
/// whether a file exists and does not normalise or canonicalise the path.
pub fn is_path_specifier(specifier: &str) -> bool {
    specifier.starts_with('/')
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier.starts_with(".\\")
        || specifier.starts_with("..\\")
        || specifier.starts_with("\\\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javascript_extensions_are_executable() {
        assert_eq!(
            module_format(Path::new("main.js")),
            Ok(ModuleFormat::JavaScript)
        );
        assert_eq!(
            module_format(Path::new("MAIN.MJS")),
            Ok(ModuleFormat::JavaScript)
        );
        assert_eq!(
            module_format(Path::new("main")),
            Ok(ModuleFormat::JavaScript)
        );
    }

    #[test]
    fn typescript_extensions_are_classified_for_stripping() {
        assert_eq!(
            module_format(Path::new("main.ts")),
            Ok(ModuleFormat::TypeScript)
        );
        assert_eq!(
            module_format(Path::new("main.MTS")),
            Ok(ModuleFormat::TypeScript)
        );
    }

    #[test]
    fn unsupported_extensions_name_the_reason() {
        let cases = [
            ("old.cjs", ".cjs extension is not supported"),
            ("view.tsx", "JSX and TSX are not implemented"),
            ("types.d.ts", "declaration files"),
            ("data.json", "JSON modules are not implemented"),
            ("module.wasm", "WebAssembly modules are not implemented"),
            ("addon.node", "native addons are not implemented"),
        ];
        for (name, expected) in cases {
            let reason = module_format(Path::new(name)).expect_err(name);
            assert!(reason.contains(expected), "`{name}`: {reason}");
        }
    }

    #[test]
    fn unknown_extensions_are_rejected() {
        let reason = module_format(Path::new("main.txt")).expect_err("unknown extension");
        assert!(
            reason.contains("unknown module extension `.txt`"),
            "{reason}"
        );
    }

    #[test]
    fn path_specifiers_cover_both_separators() {
        for specifier in [
            "/tmp/main.js",
            "./main.js",
            "../main.js",
            ".\\main.js",
            "..\\main.js",
            "\\\\server\\share\\main.js",
        ] {
            assert!(is_path_specifier(specifier), "{specifier}");
        }
        for specifier in [
            "main.js",
            "neo:assert",
            "file:///tmp/main.js",
            "https://example.com/main.js",
        ] {
            assert!(!is_path_specifier(specifier), "{specifier}");
        }
    }

    #[test]
    fn package_module_format_enforces_nearest_package_type() {
        for pkg_type in [
            PackageType::Module,
            PackageType::CommonJs,
            PackageType::Unspecified,
        ] {
            assert_eq!(
                package_module_format(Path::new("index.mjs"), pkg_type),
                Ok(ModuleFormat::JavaScript)
            );
            assert_eq!(
                package_module_format(Path::new("INDEX.MTS"), pkg_type),
                Ok(ModuleFormat::TypeScript)
            );
        }

        assert_eq!(
            package_module_format(Path::new("index.js"), PackageType::Module),
            Ok(ModuleFormat::JavaScript)
        );
        assert_eq!(
            package_module_format(Path::new("index.ts"), PackageType::Module),
            Ok(ModuleFormat::TypeScript)
        );

        for pkg_type in [PackageType::CommonJs, PackageType::Unspecified] {
            let js_err = package_module_format(Path::new("index.js"), pkg_type)
                .expect_err("non-module .js must fail closed");
            assert!(js_err.contains("CommonJS"), "{js_err}");

            let ts_err = package_module_format(Path::new("index.ts"), pkg_type)
                .expect_err("non-module .ts must fail closed");
            assert!(ts_err.contains("type"), "{ts_err}");
        }

        for rejected in [
            "mod.cjs",
            "mod.cts",
            "types.d.ts",
            "types.d.mts",
            "types.d.cts",
            "view.tsx",
            "view.jsx",
            "data.json",
            "mod.wasm",
            "native.node",
            "noext",
            "readme.md",
        ] {
            for pkg_type in [
                PackageType::Module,
                PackageType::CommonJs,
                PackageType::Unspecified,
            ] {
                assert!(
                    package_module_format(Path::new(rejected), pkg_type).is_err(),
                    "`{rejected}` should be rejected under {pkg_type:?}"
                );
            }
        }
    }
}

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use neojs_dts::{
    Profile, expected_exports, files, globals_dts, module_dts, parse_declared_exports,
    render_bundle, write_types_dir,
};

const FORBIDDEN_GLOBALS: &[&str] = &[
    "Deno",
    "Bun",
    "process",
    "Buffer",
    "window",
    "document",
    "require",
    "fetch",
    "setTimeout",
];

const V8_SPECIFIERS: &[&str] = &[
    "neo:age",
    "neo:assert",
    "neo:chars",
    "neo:console",
    "neo:cron",
    "neo:crypto",
    "neo:fs",
    "neo:os",
    "neo:path",
    "neo:slices",
    "neo:ssh",
    "neo:stdio",
    "neo:strings",
    "neo:test",
    "neo:uuid",
    "neo:uuid/v4",
    "neo:uuid/v7",
];

const LITE_SPECIFIERS: &[&str] = &[
    "neo:assert",
    "neo:chars",
    "neo:crypto",
    "neo:fs",
    "neo:os",
    "neo:ssh",
    "neo:strings",
    "neo:test",
    "neo:uuid",
    "neo:uuid/v4",
    "neo:uuid/v7",
];

const V8_ONLY_SPECIFIERS: &[&str] = &[
    "neo:age",
    "neo:console",
    "neo:cron",
    "neo:path",
    "neo:slices",
    "neo:stdio",
];

/// Validates balanced delimiters `{}` / `()` / `[]` and closed comments/strings
/// across a `.d.ts` source string.
fn assert_valid_dts_syntax(label: &str, source: &str) {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut stack: Vec<char> = Vec::new();

    while i < len {
        let b = bytes[i];

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            let mut closed = false;
            while i + 1 < len {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    closed = true;
                    break;
                }
                i += 1;
            }
            assert!(closed, "{label}: unterminated block comment");
            continue;
        }

        if b == b'"' || b == b'\'' || b == b'`' {
            let quote = b;
            i += 1;
            let mut closed = false;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            assert!(closed, "{label}: unterminated string literal");
            continue;
        }

        match b {
            b'{' => stack.push('{'),
            b'(' => stack.push('('),
            b'[' => stack.push('['),
            b'}' => assert_eq!(stack.pop(), Some('{'), "{label}: unmatched '}}'"),
            b')' => assert_eq!(stack.pop(), Some('('), "{label}: unmatched ')'"),
            b']' => assert_eq!(stack.pop(), Some('['), "{label}: unmatched ']'"),
            _ => {}
        }

        i += 1;
    }

    assert!(
        stack.is_empty(),
        "{label}: unclosed delimiters remaining: {stack:?}"
    );
}

fn contains_identifier_word(haystack: &str, needle: &str) -> bool {
    let mut start = 0;
    while let Some(offset) = haystack[start..].find(needle) {
        let abs = start + offset;
        let end = abs + needle.len();
        let before_ok = abs == 0
            || !haystack[..abs]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
        let after_ok = end == haystack.len()
            || !haystack[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
        if before_ok && after_ok {
            return true;
        }
        start = abs + needle.len();
    }
    false
}

fn assert_no_forbidden_globals(label: &str, source: &str) {
    for forbidden in FORBIDDEN_GLOBALS {
        assert!(
            !contains_identifier_word(source, forbidden),
            "{label}: forbidden global identifier `{forbidden}` found"
        );
    }
}

fn collect_tree(root: &Path, current: &Path, out: &mut BTreeMap<PathBuf, String>) {
    let mut entries: Vec<_> = fs::read_dir(current)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_tree(root, &path, out);
        } else {
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            let text = fs::read_to_string(&path).unwrap();
            out.insert(rel, text);
        }
    }
}

#[test]
fn profile_specifiers_and_metadata_match_expectations() {
    assert_eq!(Profile::V8.engine(), "neo");
    assert_eq!(Profile::V8.name(), "neojs");
    assert_eq!(Profile::Lite.engine(), "neo-lite");
    assert_eq!(Profile::Lite.name(), "neojs-lite");

    let v8_expected: Vec<&str> = expected_exports(Profile::V8)
        .iter()
        .map(|m| m.specifier)
        .collect();
    assert_eq!(v8_expected, V8_SPECIFIERS);

    let lite_expected: Vec<&str> = expected_exports(Profile::Lite)
        .iter()
        .map(|m| m.specifier)
        .collect();
    assert_eq!(lite_expected, LITE_SPECIFIERS);

    for specifier in V8_SPECIFIERS {
        assert!(
            module_dts(Profile::V8, specifier).is_some(),
            "missing V8 module dts for {specifier}"
        );
    }
    for specifier in LITE_SPECIFIERS {
        assert!(
            module_dts(Profile::Lite, specifier).is_some(),
            "missing Lite module dts for {specifier}"
        );
    }
    for specifier in V8_ONLY_SPECIFIERS {
        assert!(
            module_dts(Profile::Lite, specifier).is_none(),
            "V8-only specifier {specifier} must not exist in Lite"
        );
    }
}

#[test]
fn every_embedded_dts_file_has_valid_syntax_and_zero_forbidden_globals() {
    for profile in [Profile::V8, Profile::Lite] {
        for file in files(profile) {
            let label = format!("{}::{}", profile.name(), file.path);
            assert!(!file.content.is_empty(), "{label}: file is empty");
            assert!(
                file.content.ends_with('\n'),
                "{label}: file must end with a newline"
            );
            assert_valid_dts_syntax(&label, file.content);
            assert_no_forbidden_globals(&label, file.content);

            let parsed = parse_declared_exports(file.content);
            match file.specifier {
                Some(specifier) => {
                    assert_eq!(
                        parsed.len(),
                        1,
                        "{label}: expected exactly one `declare module` block"
                    );
                    assert_eq!(parsed[0].specifier, specifier);
                }
                None => {
                    assert!(
                        parsed.is_empty(),
                        "{label}: non-module file must not contain `declare module`"
                    );
                }
            }
        }
    }
}

#[test]
fn declared_exports_match_expected_exports_one_to_one_for_both_profiles() {
    for profile in [Profile::V8, Profile::Lite] {
        let expected = expected_exports(profile);
        let bundle = render_bundle(profile);
        assert_valid_dts_syntax(profile.name(), &bundle);
        assert_no_forbidden_globals(profile.name(), &bundle);

        let parsed_bundle = parse_declared_exports(&bundle);
        assert_eq!(
            parsed_bundle.len(),
            expected.len(),
            "{}: bundle module count mismatch",
            profile.name()
        );

        for (exp, got) in expected.iter().zip(parsed_bundle.iter()) {
            assert_eq!(got.specifier, exp.specifier);
            assert_eq!(
                got.has_default, exp.has_default,
                "{} default export mismatch",
                exp.specifier
            );
            let exp_set: BTreeSet<&str> = exp.exports.iter().copied().collect();
            let got_set: BTreeSet<&str> = got.exports.iter().map(String::as_str).collect();
            assert_eq!(got_set, exp_set, "{} named exports mismatch", exp.specifier);
            assert_eq!(
                got.exports.len(),
                exp.exports.len(),
                "{} duplicate or mismatched export count",
                exp.specifier
            );

            let exp_runtime = exp.runtime_keys();
            let got_runtime = got.runtime_keys();
            assert_eq!(
                got_runtime, exp_runtime,
                "{} runtime keys mismatch",
                exp.specifier
            );

            // Also verify parsing the standalone module file produces the exact same exports.
            let standalone = module_dts(profile, exp.specifier).unwrap();
            let parsed_standalone = parse_declared_exports(standalone);
            assert_eq!(parsed_standalone.len(), 1);
            assert_eq!(&parsed_standalone[0], got);
        }
    }
}

#[test]
fn globals_and_index_match_profile_surfaces() {
    let v8_globals = globals_dts(Profile::V8);
    assert!(v8_globals.contains("readonly engine: \"neo\";"));
    for method in ["log(", "debug(", "info(", "warn(", "error("] {
        assert!(
            v8_globals.contains(method),
            "V8 console must declare {method}"
        );
    }
    for spec in V8_SPECIFIERS {
        let key = format!("\"{spec}\":");
        assert!(
            v8_globals.contains(&key),
            "V8 NeoBuiltInModuleMap must include {spec}"
        );
    }

    let lite_globals = globals_dts(Profile::Lite);
    assert!(lite_globals.contains("readonly engine: \"neo-lite\";"));
    assert!(lite_globals.contains("log("));
    assert!(lite_globals.contains("error("));
    for forbidden_console in ["debug(", "info(", "warn("] {
        assert!(
            !lite_globals.contains(forbidden_console),
            "Lite console must NOT declare {forbidden_console}"
        );
    }
    for spec in LITE_SPECIFIERS {
        let key = format!("\"{spec}\":");
        assert!(
            lite_globals.contains(&key),
            "Lite NeoBuiltInModuleMap must include {spec}"
        );
    }
    for spec in V8_ONLY_SPECIFIERS {
        let key = format!("\"{spec}\":");
        assert!(
            !lite_globals.contains(&key),
            "Lite NeoBuiltInModuleMap must NOT include {spec}"
        );
    }

    let lite_crypto = module_dts(Profile::Lite, "neo:crypto").expect("Lite neo:crypto dts");
    let v8_crypto = module_dts(Profile::V8, "neo:crypto").expect("V8 neo:crypto dts");
    for async_name in [
        "hashAsync",
        "hmacAsync",
        "pbkdf2Async",
        "bcryptHashAsync",
        "bcryptVerifyAsync",
        "shamirSplitAsync",
        "shamirCombineAsync",
    ] {
        assert!(
            !lite_crypto.contains(async_name),
            "Profile::Lite neo:crypto must not declare `{async_name}`"
        );
        assert!(
            v8_crypto.contains(async_name),
            "Profile::V8 neo:crypto must declare `{async_name}`"
        );
    }

    for profile in [Profile::V8, Profile::Lite] {
        let profile_files = files(profile);
        let index = profile_files
            .iter()
            .find(|f| f.path == "index.d.ts")
            .expect("index.d.ts present")
            .content;
        let referenced: Vec<&str> = index
            .lines()
            .filter_map(|line| {
                let rest = line.strip_prefix("/// <reference path=\"./")?;
                rest.strip_suffix("\" />")
            })
            .collect();
        let expected_paths: Vec<&str> = profile_files
            .iter()
            .filter(|f| f.path != "index.d.ts")
            .map(|f| f.path)
            .collect();
        assert_eq!(referenced, expected_paths);
    }
}

#[test]
fn write_types_dir_is_deterministic_and_complete() {
    let base = std::env::temp_dir().join(format!(
        "neojs-dts-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    for profile in [Profile::V8, Profile::Lite] {
        let dir_a = base.join(format!("{}-a", profile.name()));
        let dir_b = base.join(format!("{}-b", profile.name()));

        write_types_dir(profile, &dir_a).unwrap();
        write_types_dir(profile, &dir_b).unwrap();
        // Second write into an existing directory must succeed identically.
        write_types_dir(profile, &dir_a).unwrap();

        let mut tree_a = BTreeMap::new();
        let mut tree_b = BTreeMap::new();
        collect_tree(&dir_a, &dir_a, &mut tree_a);
        collect_tree(&dir_b, &dir_b, &mut tree_b);

        assert_eq!(tree_a, tree_b);
        assert_eq!(tree_a.len(), files(profile).len());

        for file in files(profile) {
            let rel = PathBuf::from(file.path);
            assert_eq!(
                tree_a.get(&rel).map(String::as_str),
                Some(file.content),
                "written file mismatch for {}",
                file.path
            );
        }
    }

    fs::remove_dir_all(&base).unwrap();
}

// Unit-style parser test: parser edge-case verification (critical parser logic
// where exact assertions on comment/type-only/overload/re-export handling are needed).
#[test]
fn parse_declared_exports_handles_type_exports_overloads_comments_and_clauses() {
    let sample = r#"
        declare module "neo:sample" {
            // export const commentedOut: number;
            /*
               export function blockCommented(): void;
               export default fake;
            */
            export interface OnlyAType {
                value: string;
            }
            export type Alias = "export const inString = 1";
            export const enum ErasedEnum { A = 1 }
            export declare function overloaded(a: string): void;
            export declare function overloaded(a: number): void;
            export class MyError extends Error {
                code: string;
                constructor(message: string);
            }
            export const LIMIT: 100;
            const internal: string;
            export { type Alias as TypeAlias, internal as renamed, internal as default };
        }
    "#;

    let parsed = parse_declared_exports(sample);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].specifier, "neo:sample");
    assert!(parsed[0].has_default);
    assert_eq!(
        parsed[0].exports,
        vec!["overloaded", "MyError", "LIMIT", "renamed"]
    );
}

// Unit-style parser test: regression test for UTF-8 char-boundary safety when
// a backslash precedes a multi-byte UTF-8 scalar or end-of-input.
#[test]
fn parse_declared_exports_handles_escaped_multibyte_utf8_without_panicking() {
    assert!(parse_declared_exports("\"\\é\"").is_empty());
    assert!(parse_declared_exports("\"\\").is_empty());

    let module = r#"
        declare module "neo:utf8" {
            export const escaped: "\é";
            export function ok(): void;
        }
    "#;
    let parsed = parse_declared_exports(module);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].specifier, "neo:utf8");
    assert_eq!(parsed[0].exports, vec!["escaped", "ok"]);
}

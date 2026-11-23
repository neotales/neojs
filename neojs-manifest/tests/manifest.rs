//! Integration tests for manifest parsing and validation.
//!
//! These tests use only the public crate surface. They exercise the behaviour a
//! caller depends on: which documents are accepted, which fields are rejected,
//! and how import entries are normalized and ordered.

use neojs_manifest::Error;
use neojs_manifest::ExportKey;
use neojs_manifest::Exports;
use neojs_manifest::Manifest;
use neojs_manifest::SpecifierKind;
use neojs_manifest::Support;

#[test]
fn parses_every_documented_field() {
    let manifest = Manifest::parse(
        r#"
        {
            // A line comment.
            "name": "@scope/pkg",
            "version": "1.2.3",
            "description": "A package.",
            "authors": ["Ada", "Grace"],
            "exports": {
                ".": "./mod.ts",
                "./test": "./test.js",
            },
            "include": ["src/**/*.ts"],
            "exclude": ["**/*.test.ts"],
            "imports": {
                "std": "jsr:@scope/std@1.2.3",
                "local": "./vendor/mod.ts",
                "remote": "https://example.com/mod.ts",
                "gh": "gh:owner/repo/src/mod.ts/@v1.2.3",
                "node": "node:fs/promises",
            },
            "workspaces": ["packages/*"],
        }
        "#,
    )
    .expect("valid manifest");

    assert_eq!(manifest.name.as_str(), "@scope/pkg");
    assert_eq!(manifest.name.scope(), "scope");
    assert_eq!(manifest.name.name(), "pkg");
    assert_eq!(
        manifest.version.as_ref().map(ToString::to_string),
        Some("1.2.3".to_owned())
    );
    assert_eq!(manifest.description.as_deref(), Some("A package."));
    assert_eq!(manifest.authors, ["Ada", "Grace"]);
    assert_eq!(manifest.include[0].as_str(), "src/**/*.ts");
    assert_eq!(manifest.exclude[0].as_str(), "**/*.test.ts");
    assert_eq!(manifest.workspaces[0].as_str(), "packages/*");
    assert_eq!(manifest.imports.len(), 5);

    let Exports::Map(exports) = manifest.exports.expect("exports") else {
        panic!("expected a keyed export map");
    };
    assert_eq!(
        exports
            .get(&ExportKey::parse(".").expect("valid key"))
            .expect("root target")
            .as_str(),
        "./mod.ts"
    );
    assert_eq!(
        exports
            .get(&ExportKey::parse("./test").expect("valid key"))
            .expect("test target")
            .as_str(),
        "./test.js"
    );

    let keys: Vec<_> = manifest.imports.iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["gh", "local", "node", "remote", "std"]);
    assert_eq!(
        manifest.imports.get("std").expect("std").kind(),
        SpecifierKind::Jsr
    );
    assert_eq!(
        manifest.imports.get("local").expect("local").support(),
        Support::NormalizedOnly
    );
    assert!(matches!(
        manifest.imports.get("gh").expect("gh").support(),
        Support::Unsupported(_)
    ));
    assert_eq!(
        manifest
            .imports
            .first_unsupported()
            .map(|(key, support)| (key, matches!(support, Support::Unsupported(_)))),
        Some(("gh", true))
    );
}

#[test]
fn accepts_a_single_string_export() {
    let manifest = Manifest::parse(r#"{"name":"@scope/pkg","exports":"./mod.ts"}"#).expect("valid");
    let Exports::Single(target) = manifest.exports.expect("exports") else {
        panic!("expected a single target");
    };
    assert_eq!(target.as_str(), "./mod.ts");
}

#[test]
fn accepts_an_empty_optional_configuration() {
    let manifest = Manifest::parse(r#"{"name":"@scope/pkg"}"#).expect("valid");
    assert!(manifest.version.is_none());
    assert!(manifest.description.is_none());
    assert!(manifest.authors.is_empty());
    assert!(manifest.exports.is_none());
    assert!(manifest.include.is_empty());
    assert!(manifest.exclude.is_empty());
    assert!(manifest.imports.is_empty());
    assert!(manifest.workspaces.is_empty());
}

#[test]
fn requires_a_name() {
    let error = Manifest::parse(r#"{"description":"no name"}"#).expect_err("missing name");
    assert!(matches!(error, Error::MissingField { field: "name" }));
}

#[test]
fn rejects_unscoped_and_malformed_names() {
    for name in [
        "pkg",
        "@pkg",
        "@Scope/pkg",
        "@scope/Pkg",
        "@scope/pkg/extra",
        "@/pkg",
        "@scope/",
    ] {
        let text = format!(r#"{{"name":"{name}"}}"#);
        let error = Manifest::parse(&text).expect_err(name);
        assert!(
            matches!(error, Error::InvalidField { field: "name", .. }),
            "{name}: {error}"
        );
    }
}

#[test]
fn rejects_a_version_range() {
    let error = Manifest::parse(r#"{"name":"@scope/pkg","version":"^1.2.3"}"#).expect_err("range");
    assert!(
        matches!(
            error,
            Error::InvalidField {
                field: "version",
                ..
            }
        ),
        "{error}"
    );
}

#[test]
fn rejects_unknown_and_reserved_fields() {
    let unknown = Manifest::parse(r#"{"name":"@scope/pkg","description":"x","wat":true}"#)
        .expect_err("unknown field");
    assert!(matches!(unknown, Error::UnknownField { ref field } if field == "wat"));

    for (expected_field, text) in [
        ("node", r#"{"name":"@scope/pkg","node":true}"#),
        ("workspace", r#"{"name":"@scope/pkg","workspace":[]}"#),
        ("type", r#"{"name":"@scope/pkg","type":"module"}"#),
        ("importMap", r#"{"name":"@scope/pkg","importMap":{}}"#),
    ] {
        let error = Manifest::parse(text).expect_err(expected_field);
        assert!(
            matches!(
                error,
                Error::UnsupportedField { ref field, .. } if field == expected_field
            ),
            "{expected_field}: {error}"
        );
    }
}

#[test]
fn rejects_duplicate_keys_and_invalid_types() {
    let duplicate = Manifest::parse(r#"{"name":"@a/b","name":"@c/d"}"#).expect_err("duplicate");
    assert!(matches!(duplicate, Error::DuplicateKey { .. }));

    for text in [
        r#"{"name":42}"#,
        r#"{"name":"@a/b","description":42}"#,
        r#"{"name":"@a/b","authors":"Ada"}"#,
        r#"{"name":"@a/b","exports":42}"#,
        r#"{"name":"@a/b","imports":[]}"#,
        r#"{"name":"@a/b","workspaces":"packages/*"}"#,
        r#"{"name":"@a/b","version":null}"#,
    ] {
        let error = Manifest::parse(text).expect_err(text);
        assert!(
            matches!(
                error,
                Error::InvalidField { .. } | Error::UnsupportedField { .. }
            ),
            "{text}: {error}"
        );
    }
}

#[test]
fn rejects_unsupported_export_targets() {
    for (target, expected) in [
        ("./mod.tsx", "JSX"),
        ("./mod.jsx", "JSX"),
        ("./mod.cjs", ".cjs"),
        ("./mod.cts", "CommonJS"),
        ("./mod.json", "JSON"),
        ("./mod.wasm", "WebAssembly"),
        ("./mod.d.ts", "declaration"),
        ("../mod.ts", "start with `./`"),
        ("/mod.ts", "start with `./`"),
        ("./mod", "must end in"),
        ("./*.ts", "wildcard"),
    ] {
        let text = format!(r#"{{"name":"@a/b","exports":"{target}"}}"#);
        let error = Manifest::parse(&text).expect_err(target);
        assert!(error.to_string().contains(expected), "{target}: {error}");
    }
}

#[test]
fn rejects_unsupported_export_keys_and_shapes() {
    for text in [
        r#"{"name":"@a/b","exports":{"foo":"./mod.ts"}}"#,
        r#"{"name":"@a/b","exports":{"./a/../b":"./mod.ts"}}"#,
        r#"{"name":"@a/b","exports":{"./*":"./mod.ts"}}"#,
        r#"{"name":"@a/b","exports":{".":{"import":"./mod.ts"}}}"#,
        r#"{"name":"@a/b","exports":{".":null}}"#,
        r#"{"name":"@a/b","exports":{}}"#,
    ] {
        let error = Manifest::parse(text).expect_err(text);
        assert!(
            matches!(
                error,
                Error::InvalidField { .. } | Error::UnsupportedField { .. }
            ),
            "{text}: {error}"
        );
    }
}

#[test]
fn rejects_unportable_and_negated_patterns() {
    for (field, pattern) in [
        ("include", "/src/**/*.ts"),
        ("include", "../src/**/*.ts"),
        ("include", "src\\\\**\\\\*.ts"),
        ("include", "!src/**/*.ts"),
        ("include", "src//mod.ts"),
        ("exclude", "./src/../mod.ts"),
        ("workspaces", "C:/work/packages/*"),
        ("workspaces", "packages/"),
    ] {
        let text = format!(r#"{{"name":"@a/b","{field}":["{pattern}"]}}"#);
        let error = Manifest::parse(&text).expect_err(pattern);
        assert!(
            matches!(
                error,
                Error::InvalidField { .. } | Error::UnsupportedField { .. }
            ),
            "{field} {pattern}: {error}"
        );
    }
}

#[test]
fn rejects_bare_and_insecure_import_targets() {
    for (target, expected) in [
        ("react", "relative path"),
        ("http://example.com/mod.ts", "http:"),
        ("data:text/javascript,0", "data:"),
        ("blob:https://example.com/x", "blob:"),
        ("git:owner/repo", "git:"),
    ] {
        let text = format!(r#"{{"name":"@a/b","imports":{{"x":"{target}"}}}}"#);
        let error = Manifest::parse(&text).expect_err(target);
        assert!(error.to_string().contains(expected), "{target}: {error}");
    }
}

#[test]
fn rejects_empty_or_whitespace_import_keys() {
    for key in ["", " ", "a b"] {
        let text = format!(r#"{{"name":"@a/b","imports":{{"{key}":"./mod.ts"}}}}"#);
        let error = Manifest::parse(&text).expect_err(key);
        assert!(
            matches!(
                error,
                Error::InvalidField {
                    field: "imports",
                    ..
                }
            ),
            "{key}: {error}"
        );
    }
}

#[test]
fn reads_a_manifest_from_an_explicit_path() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("neo.jsonc");
    std::fs::write(&path, r#"{"name":"@scope/pkg","exports":"./mod.ts"}"#).expect("write manifest");

    let manifest = Manifest::from_path(&path).expect("read manifest");
    assert_eq!(manifest.name.as_str(), "@scope/pkg");
}

#[test]
fn missing_file_is_an_io_error() {
    let directory = tempfile::tempdir().expect("tempdir");
    let error =
        Manifest::from_path(directory.path().join("missing.jsonc")).expect_err("missing file");
    assert!(matches!(error, Error::Io { .. }));
}

#[test]
fn parsing_is_deterministic_and_repeatable() {
    let text = r#"{
        "name": "@scope/pkg",
        "imports": {"z": "./z.ts", "a": "./a.ts"},
        "exports": {"./z": "./z.ts", ".": "./mod.ts"}
    }"#;
    let first = Manifest::parse(text).expect("first parse");
    let second = Manifest::parse(text).expect("second parse");
    assert_eq!(first, second);
    let keys: Vec<_> = first.imports.iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["a", "z"]);
}

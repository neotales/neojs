//! Integration tests for import-specifier normalization.
//!
//! Normalization is the public contract: these tests assert canonical strings,
//! parsed fields, the normalized-only status for GitHub, and the explicit
//! unsupported status for npm, JSR, and Node.

use vaeljs_manifest::Error;
use vaeljs_manifest::NormalizedSpecifier;
use vaeljs_manifest::SpecifierKind;
use vaeljs_manifest::Support;
use vaeljs_manifest::normalize_specifier;

fn canonical(input: &str) -> String {
    normalize_specifier(input)
        .unwrap_or_else(|error| panic!("{input}: {error}"))
        .canonical()
}

#[test]
fn normalizes_gh_to_a_raw_url() {
    let specifier =
        normalize_specifier("gh:owner/repo/src/mod.ts/@v1.2.3").expect("valid gh specifier");
    assert_eq!(specifier.kind(), SpecifierKind::Gh);
    assert_eq!(
        specifier.canonical(),
        "https://raw.githubusercontent.com/owner/repo/v1.2.3/src/mod.ts"
    );
    assert_eq!(specifier.support(), Support::NormalizedOnly);

    let NormalizedSpecifier::Gh(gh) = specifier else {
        panic!("expected a gh specifier");
    };
    assert_eq!(gh.owner(), "owner");
    assert_eq!(gh.repo(), "repo");
    assert_eq!(gh.reference(), "v1.2.3");
    assert_eq!(gh.path(), "src/mod.ts");
    assert_eq!(
        gh.url(),
        "https://raw.githubusercontent.com/owner/repo/v1.2.3/src/mod.ts"
    );
}

#[test]
fn normalizes_the_new_gh_and_github_forms_to_one_url() {
    let expected = "https://raw.githubusercontent.com/owner/repo/v1.2.3/src/mod.ts";
    for input in [
        "gh:owner/repo@v1.2.3/src/mod.ts",
        "github:owner/repo@v1.2.3/src/mod.ts",
    ] {
        let specifier = normalize_specifier(input).expect(input);
        assert_eq!(specifier.kind(), SpecifierKind::Gh);
        assert_eq!(specifier.canonical(), expected);
        let NormalizedSpecifier::Gh(gh) = specifier else {
            panic!("expected a gh specifier");
        };
        assert_eq!(gh.owner(), "owner");
        assert_eq!(gh.repo(), "repo");
        assert_eq!(gh.reference(), "v1.2.3");
        assert_eq!(gh.path(), "src/mod.ts");
        assert_eq!(gh.url(), expected);
    }

    let sha = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(
        canonical(&format!("github:owner/repo@{sha}/a/b.ts")),
        format!("https://raw.githubusercontent.com/owner/repo/{sha}/a/b.ts")
    );
}

#[test]
fn a_new_form_reference_is_one_segment_and_the_path_may_contain_at() {
    assert_eq!(
        canonical("gh:owner/repo@feature/x/mod.ts"),
        "https://raw.githubusercontent.com/owner/repo/feature/x/mod.ts"
    );
    assert_eq!(
        canonical("github:owner/repo@v1/dir/@name/mod.ts"),
        "https://raw.githubusercontent.com/owner/repo/v1/dir/@name/mod.ts"
    );
    // The alias accepts the legacy spelling too, with the last `/@` winning.
    assert_eq!(
        canonical("github:owner/repo/dir@name/mod.ts/@v1"),
        "https://raw.githubusercontent.com/owner/repo/v1/dir@name/mod.ts"
    );
}

#[test]
fn rejects_malformed_new_gh_and_github_forms() {
    for input in [
        "gh:owner/repo",
        "gh:owner/repo@v1",
        "gh:owner/repo@v1/",
        "gh:owner/repo@/mod.ts",
        "gh:owner/repo@@v1/mod.ts",
        "gh:owner/repo@v1//mod.ts",
        "gh:owner/repo@v1/./mod.ts",
        "gh:owner/repo@v1/../mod.ts",
        "gh:owner/repo@v1/%2e%2e/mod.ts",
        "gh:owner/repo@v1/%2fmod.ts",
        "gh:owner/repo@v1/mod%2ets",
        "gh:owner/repo@v1/mod.ts?x=1",
        "gh:owner/repo@v1/mod.ts#frag",
        "gh:owner/repo@v1/a\\b.ts",
        "gh:owner/repo@..hidden/mod.ts",
        "gh:owner/repo@v1/.mod.ts/../x.ts",
        "github:owner/repo@v1",
        "github:owner/repo@v1/../x.ts",
        "github:-owner/repo@v1/mod.ts",
        "github:owner/.git@v1/mod.ts",
    ] {
        let error = normalize_specifier(input).expect_err(input);
        assert!(
            matches!(error, Error::InvalidSpecifier { .. }),
            "{input}: {error}"
        );
    }
}

#[test]
fn a_gh_path_may_contain_at_and_the_last_separator_wins() {
    assert_eq!(
        canonical("gh:owner/repo/dir@name/mod.ts/@0123456789abcdef"),
        "https://raw.githubusercontent.com/owner/repo/0123456789abcdef/dir@name/mod.ts"
    );
}

#[test]
fn rejects_ambiguous_or_invalid_gh_references() {
    for input in [
        "gh:owner/repo/mod.ts",
        "gh:owner/repo/mod.ts/",
        "gh:owner/repo/@v1",
        "gh:owner/repo/../secret.ts/@v1",
        "gh:owner/repo/mod.ts/@feature/x",
        "gh:-owner/repo/mod.ts/@v1",
        "gh:owner/.git/mod.ts/@v1",
        "gh:owner/repo/mod.ts/@.hidden",
    ] {
        let error = normalize_specifier(input).expect_err(input);
        assert!(
            matches!(error, Error::InvalidSpecifier { .. }),
            "{input}: {error}"
        );
    }
}

#[test]
fn normalizes_scoped_and_unscoped_npm() {
    let unscoped = normalize_specifier("npm:lodash@4.17.21").expect("npm");
    assert_eq!(unscoped.kind(), SpecifierKind::Npm);
    assert_eq!(unscoped.canonical(), "npm:lodash@4.17.21");
    let NormalizedSpecifier::Npm(unscoped) = unscoped else {
        panic!("expected npm");
    };
    assert_eq!(unscoped.name(), "lodash");
    assert_eq!(unscoped.version(), "4.17.21");
    assert_eq!(unscoped.subpath(), None);

    let scoped = normalize_specifier("npm:@scope/pkg@^1.2.3/sub/mod").expect("scoped npm");
    assert_eq!(scoped.canonical(), "npm:@scope/pkg@^1.2.3/sub/mod");
    let NormalizedSpecifier::Npm(scoped) = scoped else {
        panic!("expected npm");
    };
    assert_eq!(scoped.name(), "@scope/pkg");
    assert_eq!(scoped.version(), "^1.2.3");
    assert_eq!(scoped.subpath(), Some("sub/mod"));
}

#[test]
fn rejects_npm_without_a_version_or_with_an_unsafe_name() {
    for input in [
        "npm:lodash",
        "npm:@scope/pkg",
        "npm:Lodash@1.2.3",
        "npm:@scope/pkg@",
        "npm:@scope/pkg@1/../x",
        "npm:@scope/pkg@1/x//y",
    ] {
        let error = normalize_specifier(input).expect_err(input);
        assert!(
            matches!(error, Error::InvalidSpecifier { .. }),
            "{input}: {error}"
        );
    }
}

#[test]
fn normalizes_jsr_and_preserves_the_version_token() {
    let specifier = normalize_specifier("jsr:@scope/pkg@1.2.3/sub").expect("jsr");
    assert_eq!(specifier.kind(), SpecifierKind::Jsr);
    assert_eq!(specifier.canonical(), "jsr:@scope/pkg@1.2.3/sub");
    let NormalizedSpecifier::Jsr(jsr) = specifier else {
        panic!("expected jsr");
    };
    assert_eq!(jsr.scope(), "scope");
    assert_eq!(jsr.name(), "pkg");
    assert_eq!(jsr.version(), "1.2.3");
    assert_eq!(jsr.subpath(), Some("sub"));
}

#[test]
fn rejects_jsr_that_is_not_scoped_versioned_or_registry_shaped() {
    for input in [
        "jsr:pkg@1.2.3",
        "jsr:@pkg@1.2.3",
        "jsr:@scope/pkg",
        "jsr:@scope/pkg@",
        "jsr:@scope/../pkg@1.2.3",
        "jsr:@scope/Pkg@1.2.3",
        "jsr:@scope/pkg@1.2.3/x//y",
    ] {
        let error = normalize_specifier(input).expect_err(input);
        assert!(
            matches!(error, Error::InvalidSpecifier { .. }),
            "{input}: {error}"
        );
    }
}

#[test]
fn normalizes_node_but_marks_it_unsupported() {
    let specifier = normalize_specifier("node:fs/promises").expect("node");
    assert_eq!(specifier.kind(), SpecifierKind::Node);
    assert_eq!(specifier.canonical(), "node:fs/promises");
    assert!(matches!(
        specifier.support(),
        Support::Unsupported(reason) if reason.contains("Node")
    ));
}

#[test]
fn normalizes_https_and_file_urls() {
    let https = normalize_specifier("https://example.com/a/../mod.ts?x=1").expect("https");
    assert_eq!(https.kind(), SpecifierKind::Https);
    assert_eq!(https.canonical(), "https://example.com/mod.ts?x=1");
    assert_eq!(https.support(), Support::NormalizedOnly);

    let file = normalize_specifier("file:///tmp/mod.ts").expect("file");
    assert_eq!(file.kind(), SpecifierKind::FileUrl);
    assert_eq!(file.canonical(), "file:///tmp/mod.ts");
    assert_eq!(file.support(), Support::NormalizedOnly);
}

#[test]
fn normalizes_relative_paths_and_bare_names() {
    let path = normalize_specifier("./src/mod.ts").expect("path");
    assert_eq!(path.kind(), SpecifierKind::Path);
    assert_eq!(path.canonical(), "./src/mod.ts");
    assert_eq!(path.support(), Support::NormalizedOnly);

    let bare = normalize_specifier("react").expect("bare");
    assert_eq!(bare.kind(), SpecifierKind::Bare);
    assert_eq!(bare.canonical(), "react");
    assert!(matches!(bare.support(), Support::Unsupported(_)));
}

#[test]
fn rejects_unsupported_transports_explicitly() {
    for (input, expected) in [
        ("http://example.com/mod.ts", "only https:"),
        ("data:text/javascript,0", "data: and blob:"),
        ("blob:https://example.com/x", "data: and blob:"),
        ("git:owner/repo", "only https:,"),
    ] {
        let error = normalize_specifier(input).expect_err(input);
        assert!(
            matches!(error, Error::UnsupportedTransport { .. }),
            "{input}: {error}"
        );
        assert!(error.to_string().contains(expected), "{input}: {error}");
    }
}

#[test]
fn errors_do_not_echo_unknown_schemes_or_names() {
    let error = normalize_specifier("secret-scheme-token:opaque").expect_err("unknown scheme");
    assert!(matches!(error, Error::UnsupportedTransport { .. }));
    assert!(
        !error.to_string().contains("secret-scheme-token"),
        "{error}"
    );

    let error = normalize_specifier("npm:Secret_Token@1").expect_err("bad npm name");
    assert!(matches!(error, Error::InvalidSpecifier { .. }));
    assert!(!error.to_string().contains("Secret_Token"), "{error}");
}

#[test]
fn rejects_absolute_escaping_and_credentialed_targets() {
    for input in [
        "/tmp/mod.ts",
        "C:/work/mod.ts",
        "\\\\server\\share\\mod.ts",
        "./src/../mod.ts",
        "https://user:secret@example.com/mod.ts",
        "file://server/share/mod.ts",
        "file:///tmp/mod.ts?query",
    ] {
        let error = normalize_specifier(input).expect_err(input);
        assert!(
            matches!(
                error,
                Error::InvalidSpecifier { .. } | Error::UnsupportedTransport { .. }
            ),
            "{input}: {error}"
        );
    }
}

#[test]
fn rejects_empty_whitespace_and_control_specifiers() {
    for input in ["", " ./mod.ts", "./mod.ts ", "./mod\t.ts", "npm:a@1\n"] {
        let error = normalize_specifier(input).expect_err(input);
        assert!(
            matches!(error, Error::InvalidSpecifier { .. }),
            "{input:?}: {error}"
        );
    }
}

#[test]
fn normalization_is_deterministic() {
    let first = normalize_specifier("npm:@Scope/pkg@1.2.3").expect_err("uppercase is rejected");
    let second = normalize_specifier("npm:@Scope/pkg@1.2.3").expect_err("uppercase is rejected");
    assert_eq!(first.to_string(), second.to_string());

    let input = "gh:owner/repo/dir@name/mod.ts/@v1.2.3";
    assert_eq!(
        normalize_specifier(input).expect("first").canonical(),
        normalize_specifier(input).expect("second").canonical()
    );
}

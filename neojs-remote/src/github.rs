//! `gh:` and `github:` module specifiers, normalized to a GitHub raw URL.
//!
//! A specifier names a repository, a tag or commit reference, and a
//! repository-relative file:
//!
//! ```text
//! gh:owner/repo@ref/path/to/file.ts
//! github:owner/repo@ref/path/to/file.ts
//! ```
//!
//! Both schemes produce the same identity:
//! `https://raw.githubusercontent.com/owner/repo/ref/path/to/file.ts`. The
//! returned URL is then loaded as an ordinary remote module, so it uses the
//! configured [`crate::RemoteOptions`] limits, redirect policy, and cache, and
//! a relative import inside it resolves against the raw URL.
//!
//! # Accepted references
//!
//! The reference is the single segment between `@` and the next `/`. GitHub
//! resolves it as a tag, a branch, or a commit, and this parser does not
//! distinguish them: it accepts the same conservative character set for all
//! three. A full 40-character commit SHA and a tag such as `v1.2.3` are both
//! valid. A branch name containing `/` cannot be written in this grammar
//! because the first `/` ends the reference; use a tag or a full commit SHA.
//!
//! The legacy `gh:owner/repo/path/@ref` spelling continues to work. `@`
//! immediately after the repository selects the new form, and `/` selects the
//! legacy form, so the two are never guessed from ambiguous input.
//!
//! # Rejected input
//!
//! Owner, repository, reference, and path are validated before a URL is built.
//! Path traversal (`..`), empty segments, `.` and `..` segments, backslashes,
//! percent signs (so `%2e%2e` and `%2f` cannot hide traversal), queries,
//! fragments, whitespace, and control characters are rejected. The URL is
//! assembled through the `url` crate's path-segment API rather than string
//! concatenation, so no component can change the URL's structure.
//!
//! The scheme is matched case-insensitively, as URL schemes are. The grammar
//! itself lives in [`neojs_manifest::normalize_specifier`], so a runtime specifier
//! and a `neo.jsonc` import mapping cannot disagree about what is accepted.
//!
//! # Policy
//!
//! This module only maps a specifier to a URL. It performs no I/O and cannot
//! fetch anything. The loader accepts the result only when the runtime was
//! created with remote loading enabled. It is not an allowlist, a sandbox, or
//! SSRF protection: after the module runs it has the process's authority.

use neojs_manifest::Error as ManifestError;
use neojs_manifest::NormalizedSpecifier;
use neojs_manifest::normalize_specifier;
use url::Url;

/// Returns the raw HTTPS URL for a `gh:` or `github:` specifier.
///
/// `None` means the specifier uses a different scheme and is not this parser's
/// concern. `Some(Err(reason))` is a syntax or validation failure; the reason is
/// safe to show and never echoes the input.
///
/// Validation happens before any network access or cache lookup. A successful
/// URL is credential-free and has no query or fragment.
pub fn github_url(specifier: &str) -> Option<Result<Url, String>> {
    let (scheme, rest) = specifier.split_once(':')?;
    let canonical_scheme = if scheme.eq_ignore_ascii_case("gh") {
        "gh"
    } else if scheme.eq_ignore_ascii_case("github") {
        "github"
    } else {
        return None;
    };
    Some(parse(canonical_scheme, rest))
}

/// Parses the text after the scheme through the shared manifest grammar.
fn parse(scheme: &str, rest: &str) -> Result<Url, String> {
    // Rebuild the specifier with a lower-case scheme: URL schemes are
    // case-insensitive, while the shared parser documents case-sensitive
    // scheme matching for the manifest format it also serves.
    let canonical = format!("{scheme}:{rest}");
    match normalize_specifier(&canonical) {
        Ok(NormalizedSpecifier::Gh(gh)) => Url::parse(gh.url()).map_err(|_| {
            "the GitHub raw URL could not be built from the validated coordinate".to_string()
        }),
        Ok(_) => Err("the GitHub specifier did not normalize to a GitHub coordinate".to_string()),
        Err(ManifestError::InvalidSpecifier { reason }) => {
            Err(format!("the GitHub specifier is invalid: {reason}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(specifier: &str) -> String {
        github_url(specifier)
            .unwrap_or_else(|| panic!("{specifier}: not a GitHub scheme"))
            .unwrap_or_else(|reason| panic!("{specifier}: {reason}"))
            .to_string()
    }

    fn rejected(specifier: &str) -> String {
        github_url(specifier)
            .unwrap_or_else(|| panic!("{specifier}: not a GitHub scheme"))
            .expect_err(specifier)
    }

    #[test]
    fn builds_the_same_raw_url_for_both_schemes() {
        assert_eq!(
            ok("gh:owner/repo@v1.2.3/src/mod.ts"),
            "https://raw.githubusercontent.com/owner/repo/v1.2.3/src/mod.ts"
        );
        assert_eq!(
            ok("github:owner/repo@v1.2.3/src/mod.ts"),
            ok("gh:owner/repo@v1.2.3/src/mod.ts")
        );
        assert_eq!(
            ok("GitHub:owner/repo@0123456789abcdef0123456789abcdef01234567/a/b.ts"),
            "https://raw.githubusercontent.com/owner/repo/0123456789abcdef0123456789abcdef01234567/a/b.ts"
        );
    }

    #[test]
    fn keeps_the_legacy_spelling_working() {
        assert_eq!(
            ok("gh:owner/repo/src/mod.ts/@v1.2.3"),
            "https://raw.githubusercontent.com/owner/repo/v1.2.3/src/mod.ts"
        );
        assert_eq!(
            ok("gh:owner/repo/dir@name/mod.ts/@0123456789abcdef"),
            "https://raw.githubusercontent.com/owner/repo/0123456789abcdef/dir@name/mod.ts"
        );
    }

    #[test]
    fn reads_the_reference_as_one_segment() {
        // Documented ambiguity: `feature` is the reference and `x/mod.ts` the
        // path. A branch named `feature/x` cannot be written.
        assert_eq!(
            ok("github:o/r@feature/x/mod.ts"),
            "https://raw.githubusercontent.com/o/r/feature/x/mod.ts"
        );
        // A path may contain `@`, including a segment that starts with it.
        assert_eq!(
            ok("github:o/r@v1/dir/@x.ts"),
            "https://raw.githubusercontent.com/o/r/v1/dir/@x.ts"
        );
    }

    #[test]
    fn rejects_unsafe_or_ambiguous_input() {
        for specifier in [
            "gh:o/r",
            "gh:o/r@v1",
            "gh:o/r@v1/",
            "gh:o/r@/mod.ts",
            "gh:o/r@@v1/mod.ts",
            "gh:o/r@v1//mod.ts",
            "gh:o/r@v1/./mod.ts",
            "gh:o/r@v1/../mod.ts",
            "gh:o/r@v1/%2e%2e/mod.ts",
            "gh:o/r@v1/a%2fb.ts",
            "gh:o/r@v1/a\\b.ts",
            "gh:o/r@v1/a?token=secret",
            "gh:o/r@v1/a#fragment",
            "gh:o/r@v1/a b.ts",
            "gh:o/r@..hidden/mod.ts",
            "gh:o/r@-branch/mod.ts",
            "gh:o/r/mod.ts/@feature/x",
            "gh:-owner/r@v1/mod.ts",
            "gh:owner/.git@v1/mod.ts",
            "github:o/r@v1/../x.ts",
        ] {
            let reason = rejected(specifier);
            assert!(!reason.is_empty(), "{specifier}");
        }
    }

    #[test]
    fn rejects_a_credential_like_authority() {
        // The owner parser has no `@`, `:`, or `/`, so these cannot become a
        // URL authority with credentials.
        for specifier in [
            "gh:user:pass@host/r@v1/mod.ts",
            "github:user@host/repo@v1/mod.ts",
        ] {
            rejected(specifier);
        }
    }

    #[test]
    fn non_github_schemes_are_not_claimed() {
        assert!(github_url("https://mods.test/x.ts").is_none());
        assert!(github_url("./local.ts").is_none());
        assert!(github_url("neo:path").is_none());
    }
}

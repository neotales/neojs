//! Build-time registry of the built-in modules compiled into this crate.
//!
//! Lite currently has at most one built-in module, `neo:crypto`, and only when
//! the positive `crypto` Cargo feature is enabled. There is no runtime
//! registration hook and no introspection global in this crate yet. The
//! [`enabled_builtin_modules`] function is the single truthful source a future
//! introspection surface should read; it must not list a module that the
//! resolver and loader would reject.
//!
//! Cargo unifies features across a build graph. If any dependency enables
//! `neojs-lite-core/crypto`, every user of the crate in that build gets the
//! module, even if its own manifest left the feature off. An embedder that
//! needs the module absent must control the whole graph.

/// The canonical specifier of the `crypto` built-in.
///
/// Neo built-ins always require the `neo:` prefix. This crate does not publish
/// a bare `crypto` alias; a bare specifier is left to the normal bare-package
/// path. The `neo:` scheme is matched case-insensitively, so `NEO:crypto`
/// resolves to this same identity and QuickJS caches one module instance.
#[cfg(feature = "crypto")]
pub(crate) const CRYPTO_SPECIFIER: &str = "neo:crypto";

/// The canonical specifiers of the built-in modules compiled into this build.
///
/// The list is the same for every [`crate::Runtime`] in the process because it
/// is decided at build time. A module appears here only when its feature is
/// enabled and its resolver spelling is accepted. An empty slice means this
/// build accepts no built-in module specifier.
pub fn enabled_builtin_modules() -> &'static [&'static str] {
    #[cfg(feature = "crypto")]
    {
        &[CRYPTO_SPECIFIER]
    }
    #[cfg(not(feature = "crypto"))]
    {
        &[]
    }
}

/// Returns whether `specifier` names the `crypto` built-in with its required
/// `neo:` prefix, whether or not the feature is enabled.
///
/// The check is deliberately independent of the feature so a disabled build
/// can explain that the module exists but is not compiled in. A bare `crypto`
/// specifier is not this module: it stays a normal bare specifier, reserved for
/// Node compatibility and packages.
pub(crate) fn is_crypto_specifier(specifier: &str) -> bool {
    strip_neo_prefix(specifier) == Some("crypto")
}

/// Resolve an enabled built-in specifier to its canonical identity.
///
/// Returns `None` for a relative or absolute path, an unknown name, or a
/// built-in whose feature is disabled.
pub(crate) fn resolve(specifier: &str) -> Option<&'static str> {
    #[cfg(feature = "crypto")]
    {
        if is_crypto_specifier(specifier) {
            return Some(CRYPTO_SPECIFIER);
        }
    }
    let _ = specifier;
    None
}

/// Return the part after a case-insensitive `neo:` scheme.
fn strip_neo_prefix(specifier: &str) -> Option<&str> {
    let bytes = specifier.as_bytes();
    if bytes.len() >= 4 && bytes[..4].eq_ignore_ascii_case(b"neo:") {
        Some(&specifier[4..])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crypto_spellings_require_the_neo_prefix() {
        for specifier in ["neo:crypto", "NEO:crypto", "NeO:crypto"] {
            assert!(is_crypto_specifier(specifier), "{specifier}");
        }
        for specifier in [
            "crypto",
            "Crypto",
            "neo:Crypto",
            "node:crypto",
            "./crypto.js",
            "crypto2",
        ] {
            assert!(!is_crypto_specifier(specifier), "{specifier}");
        }
    }

    #[test]
    fn enabled_list_matches_the_feature() {
        #[cfg(feature = "crypto")]
        assert_eq!(enabled_builtin_modules(), &["neo:crypto"]);
        #[cfg(not(feature = "crypto"))]
        assert!(enabled_builtin_modules().is_empty());
    }

    #[test]
    fn resolve_only_returns_an_enabled_prefixed_module() {
        #[cfg(feature = "crypto")]
        assert_eq!(resolve("neo:crypto"), Some("neo:crypto"));
        #[cfg(not(feature = "crypto"))]
        assert_eq!(resolve("neo:crypto"), None);
        assert_eq!(resolve("crypto"), None);
        assert_eq!(resolve("./crypto.js"), None);
    }
}

//! The engine-neutral `neo:os` implementation.
//!
//! Exposes the compilation-target operating system (`std::env::consts::OS`)
//! and architecture (`std::env::consts::ARCH`). The module performs no host
//! probe, reads no environment variable, file, registry, sysctl, or process
//! table, and installs no global.

/// The canonical import specifier and registry key.
pub const SPECIFIER: &str = "neo:os";

/// The named exports, in the module object's property order.
pub const EXPORTS: &[&str] = &["platform", "arch"];

/// Returns the Rust build-target operating system (`std::env::consts::OS`).
#[must_use]
pub const fn platform() -> &'static str {
    std::env::consts::OS
}

/// Returns the Rust build-target architecture (`std::env::consts::ARCH`).
#[must_use]
pub const fn arch() -> &'static str {
    std::env::consts::ARCH
}

/// The shared module descriptor for `neo:os`.
pub const MODULE: crate::BuiltinModule = crate::BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: crate::Backend::Rust,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_rust_target_vocabulary() {
        assert_eq!(platform(), std::env::consts::OS);
        assert_eq!(arch(), std::env::consts::ARCH);
        assert!(!platform().is_empty());
        assert!(!arch().is_empty());
    }

    #[test]
    fn descriptor_matches_module_contract() {
        assert_eq!(MODULE.specifier, "neo:os");
        assert_eq!(MODULE.exports, &["platform", "arch"]);
        assert_eq!(MODULE.backend, crate::Backend::Rust);
    }
}

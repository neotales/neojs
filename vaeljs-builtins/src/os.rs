//! The engine-neutral `vael:os` implementation.
//!
//! Exposes the compilation-target operating system and architecture using
//! Deno's `Deno.build.os` / `Deno.build.arch` target vocabulary (`"darwin"`,
//! `"linux"`, `"windows"`, `"x86_64"`, `"aarch64"`, etc.). The module performs
//! no host probe, reads no environment variable, file, registry, sysctl, or
//! process table, and installs no global.

/// The canonical import specifier and registry key.
pub const SPECIFIER: &str = "vael:os";

/// The named exports, in the module object's property order.
pub const EXPORTS: &[&str] = &["platform", "arch"];

/// Returns the build-target operating system in Deno's `Deno.build.os`
/// vocabulary (`"darwin"`, `"linux"`, `"windows"`, `"freebsd"`, `"netbsd"`,
/// `"aix"`, `"solaris"`, `"illumos"`, `"android"`, etc.).
#[must_use]
pub const fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "darwin"
    } else {
        std::env::consts::OS
    }
}

/// Returns the build-target CPU architecture in Deno's `Deno.build.arch`
/// vocabulary (`"x86_64"`, `"aarch64"`, etc.).
#[must_use]
pub const fn arch() -> &'static str {
    std::env::consts::ARCH
}

/// The shared module descriptor for `vael:os`.
pub const MODULE: crate::BuiltinModule = crate::BuiltinModule {
    specifier: SPECIFIER,
    exports: EXPORTS,
    backend: crate::Backend::Rust,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_deno_target_vocabulary() {
        let expected_platform = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        };
        assert_eq!(platform(), expected_platform);
        assert_eq!(arch(), std::env::consts::ARCH);
        assert!(!platform().is_empty());
        assert!(!arch().is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(platform(), "darwin");
        }
    }

    #[test]
    fn descriptor_matches_module_contract() {
        assert_eq!(MODULE.specifier, "vael:os");
        assert_eq!(MODULE.exports, &["platform", "arch"]);
        assert_eq!(MODULE.backend, crate::Backend::Rust);
    }
}

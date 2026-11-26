//! The engine-neutral `vael:os` implementation.
//!
//! Exposes the compilation-target operating system using Deno's `Deno.build.os`
//! target vocabulary (`"darwin"`, `"linux"`, `"windows"`, etc.) and the
//! CPU architecture using normalized OCI/Go-style short names (`"amd64"`,
//! `"arm64"`, `"386"`, `"arm"`, `"riscv64"`, `"ppc64le"`, `"ppc64"`,
//! `"s390x"`, `"loong64"`, etc.). The module performs no host probe, reads no
//! environment variable, file, registry, sysctl, or process table, and installs
//! no global.

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

/// Normalizes a Rust `std::env::consts::ARCH` value and target endianness to
/// standard short architecture names (`"amd64"`, `"arm64"`, `"386"`, etc.).
#[must_use]
const fn map_arch(rust_arch: &'static str, little_endian: bool) -> &'static str {
    match rust_arch.as_bytes() {
        b"x86_64" => "amd64",
        b"aarch64" => "arm64",
        b"x86" => "386",
        b"arm" => "arm",
        b"riscv64" => "riscv64",
        b"powerpc64" => {
            if little_endian {
                "ppc64le"
            } else {
                "ppc64"
            }
        }
        b"s390x" => "s390x",
        b"loongarch64" => "loong64",
        _ => rust_arch,
    }
}

/// Returns the normalized build-target CPU architecture (`"amd64"`, `"arm64"`,
/// `"386"`, `"arm"`, `"riscv64"`, `"ppc64le"`, `"ppc64"`, `"s390x"`,
/// `"loong64"`, etc.).
#[must_use]
pub const fn arch() -> &'static str {
    map_arch(std::env::consts::ARCH, cfg!(target_endian = "little"))
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
    fn constants_match_normalized_target_vocabulary() {
        let expected_platform = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        };
        let expected_arch = map_arch(std::env::consts::ARCH, cfg!(target_endian = "little"));
        assert_eq!(platform(), expected_platform);
        assert_eq!(arch(), expected_arch);
        assert!(!platform().is_empty());
        assert!(!arch().is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(platform(), "darwin");
        }
        if cfg!(target_arch = "x86_64") {
            assert_eq!(arch(), "amd64");
        }
        if cfg!(target_arch = "aarch64") {
            assert_eq!(arch(), "arm64");
        }
    }

    #[test]
    fn map_arch_normalizes_documented_targets() {
        assert_eq!(map_arch("x86_64", true), "amd64");
        assert_eq!(map_arch("x86_64", false), "amd64");
        assert_eq!(map_arch("aarch64", true), "arm64");
        assert_eq!(map_arch("x86", true), "386");
        assert_eq!(map_arch("arm", true), "arm");
        assert_eq!(map_arch("riscv64", true), "riscv64");
        assert_eq!(map_arch("powerpc64", true), "ppc64le");
        assert_eq!(map_arch("powerpc64", false), "ppc64");
        assert_eq!(map_arch("s390x", false), "s390x");
        assert_eq!(map_arch("loongarch64", true), "loong64");
        assert_eq!(map_arch("sparc64", false), "sparc64");
    }

    #[test]
    fn descriptor_matches_module_contract() {
        assert_eq!(MODULE.specifier, "vael:os");
        assert_eq!(MODULE.exports, &["platform", "arch"]);
        assert_eq!(MODULE.backend, crate::Backend::Rust);
    }
}

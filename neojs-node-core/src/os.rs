//! Operating system metadata following `node:os` conventions.
//!
//! Uses only safe Rust standard library APIs (`#![forbid(unsafe_code)]`) and
//! compiles cleanly across Linux, macOS, and Windows.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use crate::error::NodeCoreError;

/// Returns the Node.js platform identifier (`"linux"`, `"darwin"`, `"win32"`, etc.).
///
/// Maps Rust's [`std::env::consts::OS`] vocabulary to Node.js `os.platform()`
/// names (`"macos"` -> `"darwin"`, `"windows"` -> `"win32"`).
///
/// # Example
///
/// ```
/// use neojs_node_core::os::platform;
///
/// let p = platform();
/// assert!(!p.is_empty());
/// assert_ne!(p, "macos");
/// assert_ne!(p, "windows");
/// ```
#[must_use]
pub const fn platform() -> &'static str {
    map_platform(std::env::consts::OS)
}

#[must_use]
const fn map_platform(rust_os: &'static str) -> &'static str {
    match rust_os.as_bytes() {
        b"linux" => "linux",
        b"macos" => "darwin",
        b"windows" => "win32",
        b"freebsd" => "freebsd",
        b"openbsd" => "openbsd",
        b"netbsd" => "netbsd",
        b"android" => "android",
        b"solaris" | b"illumos" | b"sunos" => "sunos",
        b"aix" => "aix",
        _ => rust_os,
    }
}

/// Returns the Node.js CPU architecture identifier (`"x64"`, `"arm64"`, `"ia32"`, etc.).
///
/// Maps Rust's [`std::env::consts::ARCH`] vocabulary to Node.js `os.arch()`
/// names (`"x86_64"` -> `"x64"`, `"aarch64"` -> `"arm64"`, `"x86"` -> `"ia32"`).
///
/// # Example
///
/// ```
/// use neojs_node_core::os::arch;
///
/// let a = arch();
/// assert!(!a.is_empty());
/// assert_ne!(a, "x86_64");
/// assert_ne!(a, "aarch64");
/// ```
#[must_use]
pub const fn arch() -> &'static str {
    map_arch(std::env::consts::ARCH)
}

#[must_use]
const fn map_arch(rust_arch: &'static str) -> &'static str {
    match rust_arch.as_bytes() {
        b"x86_64" => "x64",
        b"aarch64" => "arm64",
        b"x86" => "ia32",
        b"arm" => "arm",
        b"loongarch64" => "loong64",
        b"mips" => "mips",
        b"mipsel" => "mipsel",
        b"powerpc" => "ppc",
        b"powerpc64" => "ppc64",
        b"riscv64" => "riscv64",
        b"s390x" => "s390x",
        _ => rust_arch,
    }
}

/// Returns the platform end-of-line marker (`"\r\n"` on Windows, `"\n"` on Unix).
///
/// # Example
///
/// ```
/// use neojs_node_core::os::eol;
///
/// if cfg!(windows) {
///     assert_eq!(eol(), "\r\n");
/// } else {
///     assert_eq!(eol(), "\n");
/// }
/// ```
#[must_use]
pub const fn eol() -> &'static str {
    if cfg!(windows) { "\r\n" } else { "\n" }
}

/// Returns the current user's home directory path from environment variables.
///
/// On Unix, reads `HOME`. On Windows, checks `USERPROFILE`, then
/// `HOMEDRIVE` + `HOMEPATH`, then `HOME`. Empty values are treated as unset.
///
/// # Example
///
/// ```no_run
/// use neojs_node_core::os::homedir;
///
/// let home = homedir().expect("home directory should be set");
/// assert!(!home.as_os_str().is_empty());
/// ```
pub fn homedir() -> Result<PathBuf, NodeCoreError> {
    #[cfg(windows)]
    {
        if let Some(dir) = non_empty_env_os("USERPROFILE") {
            return Ok(PathBuf::from(dir));
        }
        if let (Some(drive), Some(path)) =
            (non_empty_env_os("HOMEDRIVE"), non_empty_env_os("HOMEPATH"))
        {
            let mut combined = drive;
            combined.push(path);
            return Ok(PathBuf::from(combined));
        }
        if let Some(dir) = non_empty_env_os("HOME") {
            return Ok(PathBuf::from(dir));
        }
        Err(NodeCoreError::Unavailable {
            reason: "user home directory environment variables are unset",
        })
    }

    #[cfg(not(windows))]
    {
        if let Some(dir) = non_empty_env_os("HOME") {
            return Ok(PathBuf::from(dir));
        }
        Err(NodeCoreError::Unavailable {
            reason: "HOME environment variable is unset or empty",
        })
    }
}

fn non_empty_env_os(name: &str) -> Option<std::ffi::OsString> {
    let val = std::env::var_os(name)?;
    if val.is_empty() { None } else { Some(val) }
}

/// Returns the operating system's default directory for temporary files.
///
/// Delegates to [`std::env::temp_dir`].
///
/// # Example
///
/// ```
/// use neojs_node_core::os::tmpdir;
///
/// assert!(!tmpdir().as_os_str().is_empty());
/// ```
#[must_use]
pub fn tmpdir() -> PathBuf {
    std::env::temp_dir()
}

/// Returns the host name of the operating system without using `unsafe` FFI.
///
/// * On Windows: reads `COMPUTERNAME`, falling back to `HOSTNAME`.
/// * On Linux: reads `/proc/sys/kernel/hostname`, falling back to
///   `/etc/hostname` and `HOSTNAME`.
/// * On other Unix targets (such as macOS and BSD): checks `/etc/hostname` and
///   `HOSTNAME`, then falls back to `/bin/hostname` with a short timeout.
///
/// # Example
///
/// ```
/// use neojs_node_core::os::hostname;
///
/// let name = hostname().expect("hostname should be available");
/// assert!(!name.trim().is_empty());
/// ```
pub fn hostname() -> Result<String, NodeCoreError> {
    #[cfg(windows)]
    {
        if let Some(name) = non_empty_env_utf8("COMPUTERNAME")? {
            return Ok(name);
        }
        if let Some(name) = non_empty_env_utf8("HOSTNAME")? {
            return Ok(name);
        }
        Err(NodeCoreError::Unavailable {
            reason: "system hostname could not be determined",
        })
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(name) = read_trimmed_hostname_file("/proc/sys/kernel/hostname") {
            return Ok(name);
        }
        if let Some(name) = read_trimmed_hostname_file("/etc/hostname") {
            return Ok(name);
        }
        if let Some(name) = non_empty_env_utf8("HOSTNAME")? {
            return Ok(name);
        }
        Err(NodeCoreError::Unavailable {
            reason: "system hostname could not be determined",
        })
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    {
        if let Some(name) = read_trimmed_hostname_file("/etc/hostname") {
            return Ok(name);
        }
        if let Some(name) = non_empty_env_utf8("HOSTNAME")? {
            return Ok(name);
        }
        let opts = crate::child_process::SpawnOptions {
            timeout_ms: Some(1_000),
            max_buffer: 4_096,
            ..crate::child_process::SpawnOptions::default()
        };
        if let Ok(output) = crate::child_process::spawn_sync("/bin/hostname", &[], &opts)
            && !output.timed_out
            && output.status == Some(0)
        {
            let text =
                String::from_utf8(output.stdout).map_err(|_| NodeCoreError::InvalidUtf8 {
                    reason: "hostname output is not valid UTF-8",
                })?;
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_owned());
            }
        }
        Err(NodeCoreError::Unavailable {
            reason: "system hostname could not be determined",
        })
    }
}

#[cfg(unix)]
fn read_trimmed_hostname_file(path: &str) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn non_empty_env_utf8(name: &str) -> Result<Option<String>, NodeCoreError> {
    let Some(val) = std::env::var_os(name) else {
        return Ok(None);
    };
    if val.is_empty() {
        return Ok(None);
    }
    let text = val.into_string().map_err(|_| NodeCoreError::InvalidUtf8 {
        reason: "hostname environment variable is not valid UTF-8",
    })?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

/// Returns the estimated number of logical CPU cores available to the process.
///
/// Uses [`std::thread::available_parallelism`], falling back to `1` if the
/// host cannot report parallelism.
///
/// # Example
///
/// ```
/// use neojs_node_core::os::cpus_count;
///
/// assert!(cpus_count() >= 1);
/// ```
#[must_use]
pub fn cpus_count() -> usize {
    std::thread::available_parallelism()
        .map(NonZeroUsize::get)
        .unwrap_or(1)
}

/// Returns the CPU endianness as `"LE"` or `"BE"`, matching `node:os`.
///
/// # Example
///
/// ```
/// use neojs_node_core::os::endianness;
///
/// assert!(matches!(endianness(), "LE" | "BE"));
/// ```
#[must_use]
pub const fn endianness() -> &'static str {
    if cfg!(target_endian = "big") {
        "BE"
    } else {
        "LE"
    }
}

/// Returns the Node.js OS type string (`"Linux"`, `"Darwin"`, `"Windows_NT"`, etc.).
///
/// # Example
///
/// ```
/// use neojs_node_core::os::os_type;
///
/// assert!(!os_type().is_empty());
/// ```
#[must_use]
pub const fn os_type() -> &'static str {
    map_os_type(std::env::consts::OS)
}

#[must_use]
const fn map_os_type(rust_os: &'static str) -> &'static str {
    match rust_os.as_bytes() {
        b"linux" => "Linux",
        b"macos" => "Darwin",
        b"windows" => "Windows_NT",
        b"freebsd" => "FreeBSD",
        b"openbsd" => "OpenBSD",
        b"netbsd" => "NetBSD",
        b"solaris" | b"illumos" | b"sunos" => "SunOS",
        b"aix" => "AIX",
        _ => rust_os,
    }
}

/// Returns the machine type string (`"x86_64"`, `"arm64"`, etc.), matching `os.machine()`.
///
/// # Example
///
/// ```
/// use neojs_node_core::os::machine;
///
/// assert!(!machine().is_empty());
/// ```
#[must_use]
pub const fn machine() -> &'static str {
    map_machine(std::env::consts::ARCH, cfg!(target_endian = "little"))
}

#[must_use]
const fn map_machine(rust_arch: &'static str, little_endian: bool) -> &'static str {
    match rust_arch.as_bytes() {
        b"x86_64" => "x86_64",
        b"aarch64" => "arm64",
        b"x86" => "i686",
        b"arm" => "arm",
        b"powerpc64" => {
            if little_endian {
                "ppc64le"
            } else {
                "ppc64"
            }
        }
        b"riscv64" => "riscv64",
        b"s390x" => "s390x",
        b"loongarch64" => "loongarch64",
        b"mips" => "mips",
        b"mips64" => "mips64",
        _ => rust_arch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_node_platform_and_arch_conventions() {
        assert_eq!(map_platform("linux"), "linux");
        assert_eq!(map_platform("macos"), "darwin");
        assert_eq!(map_platform("windows"), "win32");

        assert_eq!(map_arch("x86_64"), "x64");
        assert_eq!(map_arch("aarch64"), "arm64");
        assert_eq!(map_arch("x86"), "ia32");

        assert_eq!(map_os_type("linux"), "Linux");
        assert_eq!(map_os_type("macos"), "Darwin");
        assert_eq!(map_os_type("windows"), "Windows_NT");

        assert_eq!(map_machine("x86_64", true), "x86_64");
        assert_eq!(map_machine("aarch64", true), "arm64");
        assert_eq!(map_machine("powerpc64", true), "ppc64le");
        assert_eq!(map_machine("powerpc64", false), "ppc64");
    }

    #[test]
    fn host_queries_return_sensible_values() {
        assert!(!platform().is_empty());
        assert!(!arch().is_empty());
        assert!(matches!(eol(), "\n" | "\r\n"));
        assert!(cpus_count() >= 1);
        assert!(matches!(endianness(), "LE" | "BE"));
        assert!(!os_type().is_empty());
        assert!(!machine().is_empty());
        assert!(!tmpdir().as_os_str().is_empty());
        assert!(!hostname().unwrap().is_empty());
    }
}

// The `os` built-in module as seen by
// `import ... from "vael:os"`.
//
// The module object lives in `vael.js`, which the runtime evaluates once per
// realm. This file only re-exports it, so every named export is the same string
// as the matching property of `Vael.getBuiltInModule("vael:os")`. Nothing here
// reaches the private module registry: it reads the public module object and
// binds its properties.
//
// The exported list must match the properties of the `os` object in `vael.js`.
// `vaeljs-core/tests/os.rs` asserts that the two lists are identical, that each
// export is identity-equal to the registry property, and that the values equal
// the build target's normalized target constants.
//
// `platform` uses Deno's `Deno.build.os` vocabulary (`"darwin"` on macOS,
// `"linux"`, `"windows"`, etc.), and `arch` uses normalized OCI/Go-style short
// names (`"amd64"`, `"arm64"`, `"386"`, etc., not Rust's `"x86_64"`/`"aarch64"`
// or Node's `"x64"`/`"ia32"`). They are compilation-target constants, not a
// runtime probe. This module calls no host op, reads no environment variable,
// OS-release file, registry, sysctl value, or process table, and installs no
// global. There is no `version`, `codename`, `variant`, user, keyring, or
// scheduler surface here.
//
// The module namespace object is immutable by the language, and the module
// object behind it is frozen, so neither path can replace a value.

const os = Vael.getBuiltInModule("vael:os");

export const platform = os.platform;
export const arch = os.arch;

//! Engine-neutral TypeScript declaration (`.d.ts`) bundles for `neojs` and
//! `neojs-lite` runtime profiles.
//!
//! This crate embeds profile-accurate TypeScript declarations for `globalThis.Neo`,
//! `globalThis.console`, and every built-in `neo:*` module via `include_str!`.
//! It has zero JavaScript engine dependencies and zero runtime dependencies
//! beyond `std`, so `neojs lsp`, `neojs types`, release archive staging, and
//! runtime CI drift tests can consume it without linking V8 or QuickJS.
//!
//! # Profiles
//!
//! * [`Profile::V8`] targets the full `neojs` binary (`Neo.engine === "neo"`,
//!   `console.{log,debug,info,warn,error}`, and all 18 `neo:*` built-in
//!   modules).
//! * [`Profile::Lite`] targets the `neojs-lite` binary
//!   (`Neo.engine === "neo-lite"`, `console.{log,debug,info,warn,error}`, and
//!   the 14 Lite `neo:*` built-in modules).
//!
//! # Example
//!
//! ```
//! use neojs_dts::{Profile, expected_exports, parse_declared_exports, render_bundle};
//!
//! let bundle = render_bundle(Profile::Lite);
//! assert!(bundle.contains("readonly engine: \"neo-lite\""));
//!
//! let declared = parse_declared_exports(&bundle);
//! let expected = expected_exports(Profile::Lite);
//! assert_eq!(declared.len(), expected.len());
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod parser;

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Runtime target profile selecting the globals and `neo:*` built-in modules
/// included in generated TypeScript declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Profile {
    /// Full V8-backed `neojs` runtime (`engine: "neo"`, 5 `console` methods,
    /// and all 18 `neo:*` built-ins).
    V8,
    /// QuickJS-backed `neojs-lite` runtime (`engine: "neo-lite"`,
    /// `console.{log,debug,info,warn,error}`, and the 14 Lite `neo:*`
    /// built-ins).
    Lite,
}

impl Profile {
    /// Returns the runtime's `Neo.engine` identifier (`"neo"` or `"neo-lite"`).
    #[must_use]
    pub const fn engine(self) -> &'static str {
        match self {
            Self::V8 => "neo",
            Self::Lite => "neo-lite",
        }
    }

    /// Returns the canonical CLI binary / profile name (`"neojs"` or `"neojs-lite"`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::V8 => "neojs",
            Self::Lite => "neojs-lite",
        }
    }

    /// Returns the ordered list of embedded `.d.ts` files for this profile.
    #[must_use]
    pub fn files(self) -> &'static [DtsFile] {
        files(self)
    }

    /// Returns the ambient global `.d.ts` source (`Neo` and `console`) for this
    /// profile.
    #[must_use]
    pub fn globals_dts(self) -> &'static str {
        globals_dts(self)
    }

    /// Returns the `.d.ts` declaration source for a canonical `neo:*` module
    /// specifier in this profile, or `None` if that specifier is not enabled.
    #[must_use]
    pub fn module_dts(self, specifier: &str) -> Option<&'static str> {
        module_dts(self, specifier)
    }

    /// Returns the expected runtime export contract for every `neo:*` module in
    /// this profile.
    #[must_use]
    pub fn expected_exports(self) -> &'static [ModuleExports] {
        expected_exports(self)
    }

    /// Renders a self-contained single-file `.d.ts` bundle for stdout (`neojs types`).
    #[must_use]
    pub fn render_bundle(self) -> String {
        render_bundle(self)
    }

    /// Materializes this profile's `types/` directory layout under `dir`.
    ///
    /// See [`write_types_dir`] for overwrite, retention, and partial-write
    /// semantics.
    pub fn write_types_dir(self, dir: &Path) -> io::Result<()> {
        write_types_dir(self, dir)
    }
}

/// One embedded TypeScript declaration file belonging to a runtime profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DtsFile {
    /// Relative path within the staged `types/` directory (for example
    /// `"globals.d.ts"`, `"index.d.ts"`, or `"neo/fs.d.ts"`).
    pub path: &'static str,
    /// Canonical `neo:*` specifier declared by this file, or `None` for global
    /// and index files.
    pub specifier: Option<&'static str>,
    /// UTF-8 `.d.ts` declaration contents.
    pub content: &'static str,
}

/// Expected runtime export surface for one `neo:*` built-in module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleExports {
    /// Canonical `neo:*` specifier (for example `"neo:fs"`).
    pub specifier: &'static str,
    /// Named value exports in declaration order (excludes `"default"` and
    /// TypeScript-only `type`/`interface` declarations).
    pub exports: &'static [&'static str],
    /// Whether the ES module namespace also exports `"default"`.
    pub has_default: bool,
}

impl ModuleExports {
    /// Returns all runtime namespace keys expected from
    /// `Object.keys(moduleNamespace)`, sorted lexicographically (including
    /// `"default"` when [`Self::has_default`] is `true`).
    #[must_use]
    pub fn runtime_keys(&self) -> Vec<&'static str> {
        let mut keys = self.exports.to_vec();
        if self.has_default {
            keys.push("default");
        }
        keys.sort_unstable();
        keys
    }
}

/// Runtime value exports parsed from a `declare module "..." { ... }` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeclaredModuleExports {
    /// Canonical module specifier from `declare module "<specifier>"`.
    pub specifier: String,
    /// Named value exports declared in the module block (excludes `"default"`
    /// and TypeScript-only `type`/`interface` declarations).
    pub exports: Vec<String>,
    /// Whether `export default` (or `export { ... as default }`) is declared.
    pub has_default: bool,
}

impl DeclaredModuleExports {
    /// Returns all runtime namespace keys declared by this module, sorted
    /// lexicographically (including `"default"` when [`Self::has_default`] is `true`).
    #[must_use]
    pub fn runtime_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self.exports.iter().map(String::as_str).collect();
        if self.has_default {
            keys.push("default");
        }
        keys.sort_unstable();
        keys
    }
}

const V8_GLOBALS_DTS: &str = include_str!("../types/v8/globals.d.ts");
const V8_INDEX_DTS: &str = include_str!("../types/v8/index.d.ts");
const LITE_GLOBALS_DTS: &str = include_str!("../types/lite/globals.d.ts");
const LITE_INDEX_DTS: &str = include_str!("../types/lite/index.d.ts");

const NEO_AGE_DTS: &str = include_str!("../types/neo/age.d.ts");
const NEO_ASSERT_DTS: &str = include_str!("../types/neo/assert.d.ts");
const NEO_CHARS_DTS: &str = include_str!("../types/neo/chars.d.ts");
const NEO_CONSOLE_DTS: &str = include_str!("../types/neo/console.d.ts");
const NEO_CRON_DTS: &str = include_str!("../types/neo/cron.d.ts");
const NEO_CRYPTO_DTS: &str = include_str!("../types/neo/crypto.d.ts");
const NEO_CRYPTO_LITE_DTS: &str = include_str!("../types/neo/crypto-lite.d.ts");
const NEO_DB_DTS: &str = include_str!("../types/neo/db.d.ts");
const NEO_FS_DTS: &str = include_str!("../types/neo/fs.d.ts");
const NEO_OS_DTS: &str = include_str!("../types/neo/os.d.ts");
const NEO_PATH_DTS: &str = include_str!("../types/neo/path.d.ts");
const NEO_SLICES_DTS: &str = include_str!("../types/neo/slices.d.ts");
const NEO_SSH_DTS: &str = include_str!("../types/neo/ssh.d.ts");
const NEO_STDIO_DTS: &str = include_str!("../types/neo/stdio.d.ts");
const NEO_STRINGS_DTS: &str = include_str!("../types/neo/strings.d.ts");
const NEO_TEST_DTS: &str = include_str!("../types/neo/test.d.ts");
const NEO_UUID_DTS: &str = include_str!("../types/neo/uuid.d.ts");
const NEO_UUID_V4_DTS: &str = include_str!("../types/neo/uuid/v4.d.ts");
const NEO_UUID_V7_DTS: &str = include_str!("../types/neo/uuid/v7.d.ts");

const V8_FILES: &[DtsFile] = &[
    DtsFile {
        path: "index.d.ts",
        specifier: None,
        content: V8_INDEX_DTS,
    },
    DtsFile {
        path: "globals.d.ts",
        specifier: None,
        content: V8_GLOBALS_DTS,
    },
    DtsFile {
        path: "neo/age.d.ts",
        specifier: Some("neo:age"),
        content: NEO_AGE_DTS,
    },
    DtsFile {
        path: "neo/assert.d.ts",
        specifier: Some("neo:assert"),
        content: NEO_ASSERT_DTS,
    },
    DtsFile {
        path: "neo/chars.d.ts",
        specifier: Some("neo:chars"),
        content: NEO_CHARS_DTS,
    },
    DtsFile {
        path: "neo/console.d.ts",
        specifier: Some("neo:console"),
        content: NEO_CONSOLE_DTS,
    },
    DtsFile {
        path: "neo/cron.d.ts",
        specifier: Some("neo:cron"),
        content: NEO_CRON_DTS,
    },
    DtsFile {
        path: "neo/crypto.d.ts",
        specifier: Some("neo:crypto"),
        content: NEO_CRYPTO_DTS,
    },
    DtsFile {
        path: "neo/db.d.ts",
        specifier: Some("neo:db"),
        content: NEO_DB_DTS,
    },
    DtsFile {
        path: "neo/fs.d.ts",
        specifier: Some("neo:fs"),
        content: NEO_FS_DTS,
    },
    DtsFile {
        path: "neo/os.d.ts",
        specifier: Some("neo:os"),
        content: NEO_OS_DTS,
    },
    DtsFile {
        path: "neo/path.d.ts",
        specifier: Some("neo:path"),
        content: NEO_PATH_DTS,
    },
    DtsFile {
        path: "neo/slices.d.ts",
        specifier: Some("neo:slices"),
        content: NEO_SLICES_DTS,
    },
    DtsFile {
        path: "neo/ssh.d.ts",
        specifier: Some("neo:ssh"),
        content: NEO_SSH_DTS,
    },
    DtsFile {
        path: "neo/stdio.d.ts",
        specifier: Some("neo:stdio"),
        content: NEO_STDIO_DTS,
    },
    DtsFile {
        path: "neo/strings.d.ts",
        specifier: Some("neo:strings"),
        content: NEO_STRINGS_DTS,
    },
    DtsFile {
        path: "neo/test.d.ts",
        specifier: Some("neo:test"),
        content: NEO_TEST_DTS,
    },
    DtsFile {
        path: "neo/uuid.d.ts",
        specifier: Some("neo:uuid"),
        content: NEO_UUID_DTS,
    },
    DtsFile {
        path: "neo/uuid/v4.d.ts",
        specifier: Some("neo:uuid/v4"),
        content: NEO_UUID_V4_DTS,
    },
    DtsFile {
        path: "neo/uuid/v7.d.ts",
        specifier: Some("neo:uuid/v7"),
        content: NEO_UUID_V7_DTS,
    },
];

const LITE_FILES: &[DtsFile] = &[
    DtsFile {
        path: "index.d.ts",
        specifier: None,
        content: LITE_INDEX_DTS,
    },
    DtsFile {
        path: "globals.d.ts",
        specifier: None,
        content: LITE_GLOBALS_DTS,
    },
    DtsFile {
        path: "neo/assert.d.ts",
        specifier: Some("neo:assert"),
        content: NEO_ASSERT_DTS,
    },
    DtsFile {
        path: "neo/chars.d.ts",
        specifier: Some("neo:chars"),
        content: NEO_CHARS_DTS,
    },
    DtsFile {
        path: "neo/console.d.ts",
        specifier: Some("neo:console"),
        content: NEO_CONSOLE_DTS,
    },
    DtsFile {
        path: "neo/crypto.d.ts",
        specifier: Some("neo:crypto"),
        content: NEO_CRYPTO_LITE_DTS,
    },
    DtsFile {
        path: "neo/fs.d.ts",
        specifier: Some("neo:fs"),
        content: NEO_FS_DTS,
    },
    DtsFile {
        path: "neo/os.d.ts",
        specifier: Some("neo:os"),
        content: NEO_OS_DTS,
    },
    DtsFile {
        path: "neo/path.d.ts",
        specifier: Some("neo:path"),
        content: NEO_PATH_DTS,
    },
    DtsFile {
        path: "neo/slices.d.ts",
        specifier: Some("neo:slices"),
        content: NEO_SLICES_DTS,
    },
    DtsFile {
        path: "neo/ssh.d.ts",
        specifier: Some("neo:ssh"),
        content: NEO_SSH_DTS,
    },
    DtsFile {
        path: "neo/strings.d.ts",
        specifier: Some("neo:strings"),
        content: NEO_STRINGS_DTS,
    },
    DtsFile {
        path: "neo/test.d.ts",
        specifier: Some("neo:test"),
        content: NEO_TEST_DTS,
    },
    DtsFile {
        path: "neo/uuid.d.ts",
        specifier: Some("neo:uuid"),
        content: NEO_UUID_DTS,
    },
    DtsFile {
        path: "neo/uuid/v4.d.ts",
        specifier: Some("neo:uuid/v4"),
        content: NEO_UUID_V4_DTS,
    },
    DtsFile {
        path: "neo/uuid/v7.d.ts",
        specifier: Some("neo:uuid/v7"),
        content: NEO_UUID_V7_DTS,
    },
];

const AGE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:age",
    exports: &[
        "generateIdentity",
        "identityToRecipient",
        "encrypt",
        "decrypt",
    ],
    has_default: false,
};

const ASSERT_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:assert",
    exports: &[
        "assert",
        "AssertionError",
        "ok",
        "equals",
        "notEquals",
        "strictEquals",
        "notStrictEquals",
        "fail",
        "throws",
        "doesNotThrow",
        "rejects",
        "doesNotReject",
    ],
    has_default: true,
};

const CHARS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:chars",
    exports: &[
        "isChar",
        "isAscii",
        "isLatin1",
        "isControl",
        "isDigit",
        "isLetter",
        "isLetterOrDigit",
        "isLower",
        "isUpper",
        "isPunc",
        "isSpace",
        "isSymbol",
        "toLower",
        "toUpper",
    ],
    has_default: false,
};

const CONSOLE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:console",
    exports: &["log", "debug", "info", "warn", "error"],
    has_default: true,
};

const CRON_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:cron",
    exports: &["cron", "next", "CronError"],
    has_default: false,
};

const CRYPTO_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:crypto",
    exports: &[
        "generateEnvelopeKey",
        "randomBytes",
        "encrypt",
        "decrypt",
        "createEncryptor",
        "createDecryptor",
        "createHash",
        "createHmac",
        "hashAsync",
        "hmacAsync",
        "pbkdf2",
        "pbkdf2Async",
        "bcryptHash",
        "bcryptHashAsync",
        "bcryptVerify",
        "bcryptVerifyAsync",
        "shamirSplit",
        "shamirSplitAsync",
        "shamirCombine",
        "shamirCombineAsync",
    ],
    has_default: false,
};

const CRYPTO_LITE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:crypto",
    exports: &[
        "generateEnvelopeKey",
        "randomBytes",
        "encrypt",
        "decrypt",
        "createEncryptor",
        "createDecryptor",
        "createHash",
        "createHmac",
        "pbkdf2",
        "bcryptHash",
        "bcryptVerify",
        "shamirSplit",
        "shamirCombine",
    ],
    has_default: false,
};

const DB_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:db",
    exports: &["DbError", "open", "openSqlite"],
    has_default: false,
};

const FS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:fs",
    exports: &[
        "readFile",
        "readTextFile",
        "writeFile",
        "writeTextFile",
        "writeFileAtomic",
        "exists",
        "stat",
        "lstat",
        "readDir",
        "mkdir",
        "remove",
        "rename",
        "copyFile",
        "realpath",
    ],
    has_default: false,
};

const OS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:os",
    exports: &["platform", "arch"],
    has_default: false,
};

const PATH_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:path",
    exports: &[
        "posix",
        "win32",
        "join",
        "normalize",
        "basename",
        "dirname",
        "extname",
        "isAbsolute",
        "parse",
        "format",
        "common",
        "resolve",
        "relative",
        "toNamespacedPath",
        "SEPARATOR",
        "DELIMITER",
        "SEPARATOR_PATTERN",
    ],
    has_default: false,
};

const SLICES_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:slices",
    exports: &[
        "Slice",
        "ReadOnlySlice",
        "CharSlice",
        "ReadOnlyCharSlice",
        "slice",
        "readonlySlice",
        "charSlice",
        "readonlyCharSlice",
    ],
    has_default: false,
};

const SSH_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:ssh",
    exports: &[
        "MAX_COMMAND_BYTES",
        "MAX_OUTPUT_BYTES",
        "DEFAULT_CONNECT_TIMEOUT_MS",
        "DEFAULT_EXEC_TIMEOUT_MS",
        "connect",
    ],
    has_default: false,
};

const STDIO_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:stdio",
    exports: &["MAX_BYTES", "Readable", "Writable", "stdout", "stderr"],
    has_default: false,
};

const STRINGS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:strings",
    exports: &[
        "equalFold",
        "startsWithFold",
        "endsWithFold",
        "indexOfFold",
        "lastIndexOfFold",
        "trim",
        "trimStart",
        "trimEnd",
    ],
    has_default: false,
};

const TEST_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:test",
    exports: &["MAX_TESTS", "TestFailureError", "test", "run"],
    has_default: true,
};

const UUID_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:uuid",
    exports: &["v4", "v7"],
    has_default: false,
};

const UUID_V4_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:uuid/v4",
    exports: &["v4"],
    has_default: true,
};

const UUID_V7_EXPORTS: ModuleExports = ModuleExports {
    specifier: "neo:uuid/v7",
    exports: &["v7"],
    has_default: true,
};

const V8_EXPECTED_EXPORTS: &[ModuleExports] = &[
    AGE_EXPORTS,
    ASSERT_EXPORTS,
    CHARS_EXPORTS,
    CONSOLE_EXPORTS,
    CRON_EXPORTS,
    CRYPTO_EXPORTS,
    DB_EXPORTS,
    FS_EXPORTS,
    OS_EXPORTS,
    PATH_EXPORTS,
    SLICES_EXPORTS,
    SSH_EXPORTS,
    STDIO_EXPORTS,
    STRINGS_EXPORTS,
    TEST_EXPORTS,
    UUID_EXPORTS,
    UUID_V4_EXPORTS,
    UUID_V7_EXPORTS,
];

const LITE_EXPECTED_EXPORTS: &[ModuleExports] = &[
    ASSERT_EXPORTS,
    CHARS_EXPORTS,
    CONSOLE_EXPORTS,
    CRYPTO_LITE_EXPORTS,
    FS_EXPORTS,
    OS_EXPORTS,
    PATH_EXPORTS,
    SLICES_EXPORTS,
    SSH_EXPORTS,
    STRINGS_EXPORTS,
    TEST_EXPORTS,
    UUID_EXPORTS,
    UUID_V4_EXPORTS,
    UUID_V7_EXPORTS,
];

/// Returns all embedded `.d.ts` files for `profile`, suitable for release
/// archive `types/` staging or synthetic LSP workspace projection.
#[must_use]
pub fn files(profile: Profile) -> &'static [DtsFile] {
    match profile {
        Profile::V8 => V8_FILES,
        Profile::Lite => LITE_FILES,
    }
}

/// Returns the ambient global `.d.ts` source (`Neo` and `console`) for `profile`.
#[must_use]
pub fn globals_dts(profile: Profile) -> &'static str {
    match profile {
        Profile::V8 => V8_GLOBALS_DTS,
        Profile::Lite => LITE_GLOBALS_DTS,
    }
}

/// Returns the `.d.ts` declaration source for a canonical `neo:*` module
/// specifier in `profile`, or `None` if that specifier is not enabled in
/// `profile`.
#[must_use]
pub fn module_dts(profile: Profile, specifier: &str) -> Option<&'static str> {
    files(profile)
        .iter()
        .find(|file| file.specifier == Some(specifier))
        .map(|file| file.content)
}

/// Renders a deterministic, self-contained single-file `.d.ts` bundle for
/// `profile` (for example for `neojs types` stdout).
///
/// The bundle omits `/// <reference path="..." />` directives and inlines the
/// profile's `globals.d.ts` followed by every enabled `declare module "neo:*"`
/// declaration in registration order.
///
/// # Example
///
/// ```
/// use neojs_dts::{Profile, render_bundle};
///
/// let v8 = render_bundle(Profile::V8);
/// assert!(v8.contains("declare module \"neo:age\""));
/// assert!(v8.contains("declare module \"neo:fs\""));
/// ```
#[must_use]
pub fn render_bundle(profile: Profile) -> String {
    let profile_files = files(profile);
    let capacity: usize = profile_files
        .iter()
        .filter(|file| file.path != "index.d.ts")
        .map(|file| file.content.len() + 2)
        .sum();
    let mut out = String::with_capacity(capacity + 96);
    out.push_str("// TypeScript declarations for ");
    out.push_str(profile.name());
    out.push_str(" (Neo.engine = \"");
    out.push_str(profile.engine());
    out.push_str("\")\n\n");

    for file in profile_files {
        if file.path == "index.d.ts" {
            continue;
        }
        out.push_str(file.content.trim_end());
        out.push_str("\n\n");
    }
    out
}

/// Writes the complete `types/` declaration directory for `profile` into `dir`.
///
/// Creates `dir` and any nested subdirectories (such as `neo/uuid/`) as needed.
/// Output is deterministic and overwrites files at the same paths while
/// retaining any unrelated pre-existing files in `dir`.
///
/// # Errors
///
/// Returns an [`io::Error`] if directory creation or writing any `.d.ts` file
/// fails. Because files are written sequentially, an I/O error mid-operation
/// may leave earlier files written in `dir` without rolling back.
///
/// # Example
///
/// ```
/// use neojs_dts::{Profile, write_types_dir};
///
/// let dir = std::env::temp_dir().join(format!("neojs-dts-doctest-{}", std::process::id()));
/// write_types_dir(Profile::Lite, &dir).unwrap();
/// assert!(dir.join("index.d.ts").is_file());
/// assert!(dir.join("neo/fs.d.ts").is_file());
/// assert!(!dir.join("neo/age.d.ts").exists());
/// std::fs::remove_dir_all(&dir).unwrap();
/// ```
pub fn write_types_dir(profile: Profile, dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    for file in files(profile) {
        let mut dest = PathBuf::from(dir);
        for component in Path::new(file.path).components() {
            match component {
                Component::Normal(part) => dest.push(part),
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid embedded declaration path component: {other:?}"),
                    ));
                }
            }
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, file.content)?;
    }
    Ok(())
}

/// Returns the expected runtime export surface for every `neo:*` module
/// enabled in `profile`, in registration order.
#[must_use]
pub fn expected_exports(profile: Profile) -> &'static [ModuleExports] {
    match profile {
        Profile::V8 => V8_EXPECTED_EXPORTS,
        Profile::Lite => LITE_EXPECTED_EXPORTS,
    }
}

/// Parses every `declare module "<specifier>" { ... }` block in `dts` and
/// returns its declared runtime value exports.
///
/// TypeScript-only exports (`export type`, `export interface`, `export const enum`,
/// and `export { type ... }`) are excluded so runtime CI drift tests can compare
/// declared exports 1:1 against `Object.keys(moduleNamespace)`.
///
/// # Example
///
/// ```
/// use neojs_dts::{Profile, module_dts, parse_declared_exports};
///
/// let fs_dts = module_dts(Profile::V8, "neo:fs").unwrap();
/// let parsed = parse_declared_exports(fs_dts);
/// assert_eq!(parsed.len(), 1);
/// assert_eq!(parsed[0].specifier, "neo:fs");
/// assert!(parsed[0].exports.contains(&"writeFileAtomic".to_string()));
/// assert!(!parsed[0].has_default);
/// ```
#[must_use]
pub fn parse_declared_exports(dts: &str) -> Vec<DeclaredModuleExports> {
    parser::parse_declared_exports(dts)
}

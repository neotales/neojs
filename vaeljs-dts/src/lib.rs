//! Engine-neutral TypeScript declaration (`.d.ts`) bundles for `vaeljs` and
//! `vaeljs-lite` runtime profiles.
//!
//! This crate embeds profile-accurate TypeScript declarations for `globalThis.Vael`,
//! `globalThis.console`, and every built-in `vael:*` module via `include_str!`.
//! It has zero JavaScript engine dependencies and zero runtime dependencies
//! beyond `std`, so `vjs lsp`, `vjs types`, release archive staging, and
//! runtime CI drift tests can consume it without linking V8 or QuickJS.
//!
//! # Profiles
//!
//! * [`Profile::V8`] targets the full `vjs` (`vaeljs`) binary (`Vael.engine === "vael"`,
//!   `console.{log,debug,info,warn,error}`, 22 `vael:*` built-in modules, and
//!   11 `node:*` compatibility modules — 33 modules total).
//! * [`Profile::Lite`] targets the `vjs-lite` (`vaeljs-lite`) binary
//!   (`Vael.engine === "vael-lite"`, `console.{log,debug,info,warn,error}`,
//!   17 `vael:*` built-in modules, and 10 `node:*` compatibility modules —
//!   27 modules total).
//!
//! # Example
//!
//! ```
//! use vaeljs_dts::{Profile, expected_exports, parse_declared_exports, render_bundle};
//!
//! let bundle = render_bundle(Profile::Lite);
//! assert!(bundle.contains("readonly engine: \"vael-lite\""));
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

/// Runtime target profile selecting the globals and `vael:*` built-in modules
/// included in generated TypeScript declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Profile {
    /// Full V8-backed `vjs` (`vaeljs`) runtime (`engine: "vael"`, 5 `console`
    /// methods, 22 `vael:*` built-ins, and 11 `node:*` compatibility modules —
    /// 33 modules total).
    V8,
    /// QuickJS-backed `vjs-lite` (`vaeljs-lite`) runtime (`engine: "vael-lite"`,
    /// `console.{log,debug,info,warn,error}`, 17 `vael:*` built-ins, and 10
    /// `node:*` compatibility modules — 27 modules total).
    Lite,
}

impl Profile {
    /// Returns the runtime's `Vael.engine` identifier (`"vael"` or `"vael-lite"`).
    #[must_use]
    pub const fn engine(self) -> &'static str {
        match self {
            Self::V8 => "vael",
            Self::Lite => "vael-lite",
        }
    }

    /// Returns the canonical CLI binary / profile name (`"vjs"` or `"vjs-lite"`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::V8 => "vjs",
            Self::Lite => "vjs-lite",
        }
    }

    /// Returns the ordered list of embedded `.d.ts` files for this profile.
    #[must_use]
    pub fn files(self) -> &'static [DtsFile] {
        files(self)
    }

    /// Returns the ambient global `.d.ts` source (`Vael` and `console`) for this
    /// profile.
    #[must_use]
    pub fn globals_dts(self) -> &'static str {
        globals_dts(self)
    }

    /// Returns the `.d.ts` declaration source for a canonical `vael:*` module
    /// specifier in this profile, or `None` if that specifier is not enabled.
    #[must_use]
    pub fn module_dts(self, specifier: &str) -> Option<&'static str> {
        module_dts(self, specifier)
    }

    /// Returns the expected runtime export contract for every `vael:*` module in
    /// this profile.
    #[must_use]
    pub fn expected_exports(self) -> &'static [ModuleExports] {
        expected_exports(self)
    }

    /// Renders a self-contained single-file `.d.ts` bundle for stdout (`vjs types`).
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
    /// `"globals.d.ts"`, `"index.d.ts"`, or `"vael/fs.d.ts"`).
    pub path: &'static str,
    /// Canonical `vael:*` specifier declared by this file, or `None` for global
    /// and index files.
    pub specifier: Option<&'static str>,
    /// UTF-8 `.d.ts` declaration contents.
    pub content: &'static str,
}

/// Expected runtime export surface for one `vael:*` built-in module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleExports {
    /// Canonical `vael:*` specifier (for example `"vael:fs"`).
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

const VAEL_AGE_DTS: &str = include_str!("../types/vael/age.d.ts");
const VAEL_ASSERT_DTS: &str = include_str!("../types/vael/assert.d.ts");
const VAEL_CHARS_DTS: &str = include_str!("../types/vael/chars.d.ts");
const VAEL_CONSOLE_DTS: &str = include_str!("../types/vael/console.d.ts");
const VAEL_CRON_DTS: &str = include_str!("../types/vael/cron.d.ts");
const VAEL_CRYPTO_DTS: &str = include_str!("../types/vael/crypto.d.ts");
const VAEL_CRYPTO_LITE_DTS: &str = include_str!("../types/vael/crypto-lite.d.ts");
const VAEL_DB_DTS: &str = include_str!("../types/vael/db.d.ts");
const VAEL_EXEC_DTS: &str = include_str!("../types/vael/exec.d.ts");
const VAEL_EXEC_LITE_DTS: &str = include_str!("../types/vael/exec.lite.d.ts");
const VAEL_FETCH_DTS: &str = include_str!("../types/vael/fetch.d.ts");
const VAEL_FETCH_LITE_DTS: &str = include_str!("../types/vael/fetch.lite.d.ts");
const VAEL_FS_DTS: &str = include_str!("../types/vael/fs.d.ts");
const VAEL_HTTP_DTS: &str = include_str!("../types/vael/http.d.ts");
const VAEL_OS_DTS: &str = include_str!("../types/vael/os.d.ts");
const VAEL_PATH_DTS: &str = include_str!("../types/vael/path.d.ts");
const VAEL_SLICES_DTS: &str = include_str!("../types/vael/slices.d.ts");
const VAEL_SSH_DTS: &str = include_str!("../types/vael/ssh.d.ts");
const VAEL_STDIO_DTS: &str = include_str!("../types/vael/stdio.d.ts");
const VAEL_STRINGS_DTS: &str = include_str!("../types/vael/strings.d.ts");
const VAEL_TEMPLATE_DTS: &str = include_str!("../types/vael/template.d.ts");
const VAEL_TEST_DTS: &str = include_str!("../types/vael/test.d.ts");
const VAEL_UUID_DTS: &str = include_str!("../types/vael/uuid.d.ts");
const VAEL_UUID_V4_DTS: &str = include_str!("../types/vael/uuid/v4.d.ts");
const VAEL_UUID_V7_DTS: &str = include_str!("../types/vael/uuid/v7.d.ts");

const NODE_BUFFER_DTS: &str = include_str!("../types/node/buffer.d.ts");
const NODE_CHILD_PROCESS_DTS: &str = include_str!("../types/node/child_process.d.ts");
const NODE_CHILD_PROCESS_LITE_DTS: &str = include_str!("../types/node/child_process-lite.d.ts");
const NODE_CRYPTO_DTS: &str = include_str!("../types/node/crypto.d.ts");
const NODE_FS_DTS: &str = include_str!("../types/node/fs.d.ts");
const NODE_FS_LITE_DTS: &str = include_str!("../types/node/fs-lite.d.ts");
const NODE_FS_PROMISES_DTS: &str = include_str!("../types/node/fs-promises.d.ts");
const NODE_OS_DTS: &str = include_str!("../types/node/os.d.ts");
const NODE_PATH_DTS: &str = include_str!("../types/node/path.d.ts");
const NODE_PATH_POSIX_DTS: &str = include_str!("../types/node/path/posix.d.ts");
const NODE_PATH_WIN32_DTS: &str = include_str!("../types/node/path/win32.d.ts");
const NODE_PROCESS_DTS: &str = include_str!("../types/node/process.d.ts");
const NODE_URL_DTS: &str = include_str!("../types/node/url.d.ts");

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
        path: "vael/age.d.ts",
        specifier: Some("vael:age"),
        content: VAEL_AGE_DTS,
    },
    DtsFile {
        path: "vael/assert.d.ts",
        specifier: Some("vael:assert"),
        content: VAEL_ASSERT_DTS,
    },
    DtsFile {
        path: "vael/chars.d.ts",
        specifier: Some("vael:chars"),
        content: VAEL_CHARS_DTS,
    },
    DtsFile {
        path: "vael/console.d.ts",
        specifier: Some("vael:console"),
        content: VAEL_CONSOLE_DTS,
    },
    DtsFile {
        path: "vael/cron.d.ts",
        specifier: Some("vael:cron"),
        content: VAEL_CRON_DTS,
    },
    DtsFile {
        path: "vael/crypto.d.ts",
        specifier: Some("vael:crypto"),
        content: VAEL_CRYPTO_DTS,
    },
    DtsFile {
        path: "vael/db.d.ts",
        specifier: Some("vael:db"),
        content: VAEL_DB_DTS,
    },
    DtsFile {
        path: "vael/exec.d.ts",
        specifier: Some("vael:exec"),
        content: VAEL_EXEC_DTS,
    },
    DtsFile {
        path: "vael/fetch.d.ts",
        specifier: Some("vael:fetch"),
        content: VAEL_FETCH_DTS,
    },
    DtsFile {
        path: "vael/fs.d.ts",
        specifier: Some("vael:fs"),
        content: VAEL_FS_DTS,
    },
    DtsFile {
        path: "vael/http.d.ts",
        specifier: Some("vael:http"),
        content: VAEL_HTTP_DTS,
    },
    DtsFile {
        path: "vael/os.d.ts",
        specifier: Some("vael:os"),
        content: VAEL_OS_DTS,
    },
    DtsFile {
        path: "vael/path.d.ts",
        specifier: Some("vael:path"),
        content: VAEL_PATH_DTS,
    },
    DtsFile {
        path: "vael/slices.d.ts",
        specifier: Some("vael:slices"),
        content: VAEL_SLICES_DTS,
    },
    DtsFile {
        path: "vael/ssh.d.ts",
        specifier: Some("vael:ssh"),
        content: VAEL_SSH_DTS,
    },
    DtsFile {
        path: "vael/stdio.d.ts",
        specifier: Some("vael:stdio"),
        content: VAEL_STDIO_DTS,
    },
    DtsFile {
        path: "vael/strings.d.ts",
        specifier: Some("vael:strings"),
        content: VAEL_STRINGS_DTS,
    },
    DtsFile {
        path: "vael/template.d.ts",
        specifier: Some("vael:template"),
        content: VAEL_TEMPLATE_DTS,
    },
    DtsFile {
        path: "vael/test.d.ts",
        specifier: Some("vael:test"),
        content: VAEL_TEST_DTS,
    },
    DtsFile {
        path: "vael/uuid.d.ts",
        specifier: Some("vael:uuid"),
        content: VAEL_UUID_DTS,
    },
    DtsFile {
        path: "vael/uuid/v4.d.ts",
        specifier: Some("vael:uuid/v4"),
        content: VAEL_UUID_V4_DTS,
    },
    DtsFile {
        path: "vael/uuid/v7.d.ts",
        specifier: Some("vael:uuid/v7"),
        content: VAEL_UUID_V7_DTS,
    },
    DtsFile {
        path: "node/buffer.d.ts",
        specifier: Some("node:buffer"),
        content: NODE_BUFFER_DTS,
    },
    DtsFile {
        path: "node/child_process.d.ts",
        specifier: Some("node:child_process"),
        content: NODE_CHILD_PROCESS_DTS,
    },
    DtsFile {
        path: "node/crypto.d.ts",
        specifier: Some("node:crypto"),
        content: NODE_CRYPTO_DTS,
    },
    DtsFile {
        path: "node/fs.d.ts",
        specifier: Some("node:fs"),
        content: NODE_FS_DTS,
    },
    DtsFile {
        path: "node/fs-promises.d.ts",
        specifier: Some("node:fs/promises"),
        content: NODE_FS_PROMISES_DTS,
    },
    DtsFile {
        path: "node/os.d.ts",
        specifier: Some("node:os"),
        content: NODE_OS_DTS,
    },
    DtsFile {
        path: "node/path.d.ts",
        specifier: Some("node:path"),
        content: NODE_PATH_DTS,
    },
    DtsFile {
        path: "node/path/posix.d.ts",
        specifier: Some("node:path/posix"),
        content: NODE_PATH_POSIX_DTS,
    },
    DtsFile {
        path: "node/path/win32.d.ts",
        specifier: Some("node:path/win32"),
        content: NODE_PATH_WIN32_DTS,
    },
    DtsFile {
        path: "node/process.d.ts",
        specifier: Some("node:process"),
        content: NODE_PROCESS_DTS,
    },
    DtsFile {
        path: "node/url.d.ts",
        specifier: Some("node:url"),
        content: NODE_URL_DTS,
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
        path: "vael/assert.d.ts",
        specifier: Some("vael:assert"),
        content: VAEL_ASSERT_DTS,
    },
    DtsFile {
        path: "vael/chars.d.ts",
        specifier: Some("vael:chars"),
        content: VAEL_CHARS_DTS,
    },
    DtsFile {
        path: "vael/console.d.ts",
        specifier: Some("vael:console"),
        content: VAEL_CONSOLE_DTS,
    },
    DtsFile {
        path: "vael/crypto.d.ts",
        specifier: Some("vael:crypto"),
        content: VAEL_CRYPTO_LITE_DTS,
    },
    DtsFile {
        path: "vael/exec.d.ts",
        specifier: Some("vael:exec"),
        content: VAEL_EXEC_LITE_DTS,
    },
    DtsFile {
        path: "vael/fetch.d.ts",
        specifier: Some("vael:fetch"),
        content: VAEL_FETCH_LITE_DTS,
    },
    DtsFile {
        path: "vael/fs.d.ts",
        specifier: Some("vael:fs"),
        content: VAEL_FS_DTS,
    },
    DtsFile {
        path: "vael/os.d.ts",
        specifier: Some("vael:os"),
        content: VAEL_OS_DTS,
    },
    DtsFile {
        path: "vael/path.d.ts",
        specifier: Some("vael:path"),
        content: VAEL_PATH_DTS,
    },
    DtsFile {
        path: "vael/slices.d.ts",
        specifier: Some("vael:slices"),
        content: VAEL_SLICES_DTS,
    },
    DtsFile {
        path: "vael/ssh.d.ts",
        specifier: Some("vael:ssh"),
        content: VAEL_SSH_DTS,
    },
    DtsFile {
        path: "vael/strings.d.ts",
        specifier: Some("vael:strings"),
        content: VAEL_STRINGS_DTS,
    },
    DtsFile {
        path: "vael/template.d.ts",
        specifier: Some("vael:template"),
        content: VAEL_TEMPLATE_DTS,
    },
    DtsFile {
        path: "vael/test.d.ts",
        specifier: Some("vael:test"),
        content: VAEL_TEST_DTS,
    },
    DtsFile {
        path: "vael/uuid.d.ts",
        specifier: Some("vael:uuid"),
        content: VAEL_UUID_DTS,
    },
    DtsFile {
        path: "vael/uuid/v4.d.ts",
        specifier: Some("vael:uuid/v4"),
        content: VAEL_UUID_V4_DTS,
    },
    DtsFile {
        path: "vael/uuid/v7.d.ts",
        specifier: Some("vael:uuid/v7"),
        content: VAEL_UUID_V7_DTS,
    },
    DtsFile {
        path: "node/buffer.d.ts",
        specifier: Some("node:buffer"),
        content: NODE_BUFFER_DTS,
    },
    DtsFile {
        path: "node/child_process.d.ts",
        specifier: Some("node:child_process"),
        content: NODE_CHILD_PROCESS_LITE_DTS,
    },
    DtsFile {
        path: "node/crypto.d.ts",
        specifier: Some("node:crypto"),
        content: NODE_CRYPTO_DTS,
    },
    DtsFile {
        path: "node/fs.d.ts",
        specifier: Some("node:fs"),
        content: NODE_FS_LITE_DTS,
    },
    DtsFile {
        path: "node/os.d.ts",
        specifier: Some("node:os"),
        content: NODE_OS_DTS,
    },
    DtsFile {
        path: "node/path.d.ts",
        specifier: Some("node:path"),
        content: NODE_PATH_DTS,
    },
    DtsFile {
        path: "node/path/posix.d.ts",
        specifier: Some("node:path/posix"),
        content: NODE_PATH_POSIX_DTS,
    },
    DtsFile {
        path: "node/path/win32.d.ts",
        specifier: Some("node:path/win32"),
        content: NODE_PATH_WIN32_DTS,
    },
    DtsFile {
        path: "node/process.d.ts",
        specifier: Some("node:process"),
        content: NODE_PROCESS_DTS,
    },
    DtsFile {
        path: "node/url.d.ts",
        specifier: Some("node:url"),
        content: NODE_URL_DTS,
    },
];

const AGE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:age",
    exports: &[
        "generateIdentity",
        "identityToRecipient",
        "encrypt",
        "decrypt",
    ],
    has_default: false,
};

const ASSERT_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:assert",
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
    specifier: "vael:chars",
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
    specifier: "vael:console",
    exports: &["log", "debug", "info", "warn", "error"],
    has_default: true,
};

const CRON_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:cron",
    exports: &["cron", "next", "CronError"],
    has_default: false,
};

const CRYPTO_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:crypto",
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
    specifier: "vael:crypto",
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
    specifier: "vael:db",
    exports: &["DbError", "open", "openSqlite"],
    has_default: false,
};

const EXEC_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:exec",
    exports: &[
        "ChildProcess",
        "Command",
        "CommandError",
        "FileLock",
        "NotFoundOnPathError",
        "Output",
        "PathFinder",
        "addLogger",
        "clearLoggers",
        "cmd",
        "exec",
        "execSync",
        "lockFile",
        "lockFileSync",
        "pathFinder",
        "run",
        "runSync",
        "spawn",
        "which",
        "whichAll",
        "whichAllSync",
        "whichSync",
    ],
    has_default: false,
};

const EXEC_LITE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:exec",
    exports: &[
        "Command",
        "CommandError",
        "FileLock",
        "NotFoundOnPathError",
        "Output",
        "PathFinder",
        "addLogger",
        "clearLoggers",
        "cmd",
        "execSync",
        "lockFileSync",
        "pathFinder",
        "runSync",
        "whichAllSync",
        "whichSync",
    ],
    has_default: false,
};

const FETCH_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:fetch",
    exports: &[
        "FetchError",
        "fetch",
        "fetchSync",
        "download",
        "downloadSync",
    ],
    has_default: false,
};

const FETCH_LITE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:fetch",
    exports: &["FetchError", "fetchSync", "downloadSync"],
    has_default: false,
};

const FS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:fs",
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

const HTTP_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:http",
    exports: &[
        "HttpError",
        "HttpRequest",
        "HttpResponse",
        "ServeMux",
        "createRouter",
        "response",
        "json",
        "html",
        "text",
        "redirect",
        "noContent",
        "setCookie",
        "httpError",
        "badRequest",
        "unauthorized",
        "forbidden",
        "notFound",
        "conflict",
        "methodNotAllowed",
        "recover",
        "requestId",
        "logger",
        "securityHeaders",
        "cors",
        "compose",
        "serve",
        "parseCookies",
        "formatSetCookie",
    ],
    has_default: false,
};

const OS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:os",
    exports: &["platform", "arch"],
    has_default: false,
};

const PATH_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:path",
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
    specifier: "vael:slices",
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
    specifier: "vael:ssh",
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
    specifier: "vael:stdio",
    exports: &["MAX_BYTES", "Readable", "Writable", "stdout", "stderr"],
    has_default: false,
};

const STRINGS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:strings",
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

const TEMPLATE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:template",
    exports: &["render", "compile", "createTemplateSet", "escapeHtml"],
    has_default: true,
};

const TEST_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:test",
    exports: &["MAX_TESTS", "TestFailureError", "test", "run"],
    has_default: true,
};

const UUID_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:uuid",
    exports: &["v4", "v7"],
    has_default: false,
};

const UUID_V4_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:uuid/v4",
    exports: &["v4"],
    has_default: true,
};

const UUID_V7_EXPORTS: ModuleExports = ModuleExports {
    specifier: "vael:uuid/v7",
    exports: &["v7"],
    has_default: true,
};

const NODE_BUFFER_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:buffer",
    exports: &["Buffer"],
    has_default: true,
};

const NODE_CHILD_PROCESS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:child_process",
    exports: &["spawnSync", "execFileSync", "execFile"],
    has_default: true,
};

const NODE_CHILD_PROCESS_LITE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:child_process",
    exports: &["spawnSync", "execFileSync"],
    has_default: true,
};

const NODE_CRYPTO_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:crypto",
    exports: &[
        "randomBytes",
        "randomUUID",
        "createHash",
        "createHmac",
        "pbkdf2Sync",
        "timingSafeEqual",
    ],
    has_default: true,
};

const NODE_FS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:fs",
    exports: &[
        "readFileSync",
        "writeFileSync",
        "appendFileSync",
        "existsSync",
        "statSync",
        "lstatSync",
        "readdirSync",
        "mkdirSync",
        "rmSync",
        "unlinkSync",
        "renameSync",
        "copyFileSync",
        "realpathSync",
        "mkdtempSync",
        "constants",
        "promises",
    ],
    has_default: true,
};

const NODE_FS_LITE_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:fs",
    exports: &[
        "readFileSync",
        "writeFileSync",
        "appendFileSync",
        "existsSync",
        "statSync",
        "lstatSync",
        "readdirSync",
        "mkdirSync",
        "rmSync",
        "unlinkSync",
        "renameSync",
        "copyFileSync",
        "realpathSync",
        "mkdtempSync",
        "constants",
    ],
    has_default: true,
};

const NODE_FS_PROMISES_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:fs/promises",
    exports: &[
        "readFile",
        "writeFile",
        "appendFile",
        "stat",
        "lstat",
        "readdir",
        "mkdir",
        "rm",
        "unlink",
        "rename",
        "copyFile",
        "realpath",
        "mkdtemp",
        "constants",
    ],
    has_default: true,
};

const NODE_OS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:os",
    exports: &[
        "platform",
        "arch",
        "EOL",
        "homedir",
        "tmpdir",
        "hostname",
        "cpus",
        "endianness",
        "type",
        "machine",
    ],
    has_default: true,
};

const NODE_PATH_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:path",
    exports: &[
        "sep",
        "delimiter",
        "join",
        "resolve",
        "normalize",
        "relative",
        "dirname",
        "basename",
        "extname",
        "parse",
        "format",
        "isAbsolute",
        "posix",
        "win32",
    ],
    has_default: true,
};

const NODE_PATH_POSIX_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:path/posix",
    exports: &[
        "sep",
        "delimiter",
        "join",
        "resolve",
        "normalize",
        "relative",
        "dirname",
        "basename",
        "extname",
        "parse",
        "format",
        "isAbsolute",
        "posix",
        "win32",
    ],
    has_default: true,
};

const NODE_PATH_WIN32_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:path/win32",
    exports: &[
        "sep",
        "delimiter",
        "join",
        "resolve",
        "normalize",
        "relative",
        "dirname",
        "basename",
        "extname",
        "parse",
        "format",
        "isAbsolute",
        "posix",
        "win32",
    ],
    has_default: true,
};

const NODE_PROCESS_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:process",
    exports: &[
        "argv", "cwd", "chdir", "env", "exit", "exitCode", "platform", "arch", "pid", "version",
        "versions", "hrtime",
    ],
    has_default: true,
};

const NODE_URL_EXPORTS: ModuleExports = ModuleExports {
    specifier: "node:url",
    exports: &["fileURLToPath", "pathToFileURL", "URL", "URLSearchParams"],
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
    EXEC_EXPORTS,
    FETCH_EXPORTS,
    FS_EXPORTS,
    HTTP_EXPORTS,
    OS_EXPORTS,
    PATH_EXPORTS,
    SLICES_EXPORTS,
    SSH_EXPORTS,
    STDIO_EXPORTS,
    STRINGS_EXPORTS,
    TEMPLATE_EXPORTS,
    TEST_EXPORTS,
    UUID_EXPORTS,
    UUID_V4_EXPORTS,
    UUID_V7_EXPORTS,
    NODE_BUFFER_EXPORTS,
    NODE_CHILD_PROCESS_EXPORTS,
    NODE_CRYPTO_EXPORTS,
    NODE_FS_EXPORTS,
    NODE_FS_PROMISES_EXPORTS,
    NODE_OS_EXPORTS,
    NODE_PATH_EXPORTS,
    NODE_PATH_POSIX_EXPORTS,
    NODE_PATH_WIN32_EXPORTS,
    NODE_PROCESS_EXPORTS,
    NODE_URL_EXPORTS,
];

const LITE_EXPECTED_EXPORTS: &[ModuleExports] = &[
    ASSERT_EXPORTS,
    CHARS_EXPORTS,
    CONSOLE_EXPORTS,
    CRYPTO_LITE_EXPORTS,
    EXEC_LITE_EXPORTS,
    FETCH_LITE_EXPORTS,
    FS_EXPORTS,
    OS_EXPORTS,
    PATH_EXPORTS,
    SLICES_EXPORTS,
    SSH_EXPORTS,
    STRINGS_EXPORTS,
    TEMPLATE_EXPORTS,
    TEST_EXPORTS,
    UUID_EXPORTS,
    UUID_V4_EXPORTS,
    UUID_V7_EXPORTS,
    NODE_BUFFER_EXPORTS,
    NODE_CHILD_PROCESS_LITE_EXPORTS,
    NODE_CRYPTO_EXPORTS,
    NODE_FS_LITE_EXPORTS,
    NODE_OS_EXPORTS,
    NODE_PATH_EXPORTS,
    NODE_PATH_POSIX_EXPORTS,
    NODE_PATH_WIN32_EXPORTS,
    NODE_PROCESS_EXPORTS,
    NODE_URL_EXPORTS,
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

/// Returns the ambient global `.d.ts` source (`Vael` and `console`) for `profile`.
#[must_use]
pub fn globals_dts(profile: Profile) -> &'static str {
    match profile {
        Profile::V8 => V8_GLOBALS_DTS,
        Profile::Lite => LITE_GLOBALS_DTS,
    }
}

/// Returns the `.d.ts` declaration source for a canonical `vael:*` module
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
/// `profile` (for example for `vjs types` stdout).
///
/// The bundle omits `/// <reference path="..." />` directives and inlines the
/// profile's `globals.d.ts` followed by every enabled `declare module "vael:*"`
/// declaration in registration order.
///
/// # Example
///
/// ```
/// use vaeljs_dts::{Profile, render_bundle};
///
/// let v8 = render_bundle(Profile::V8);
/// assert!(v8.contains("declare module \"vael:age\""));
/// assert!(v8.contains("declare module \"vael:fs\""));
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
    out.push_str(" (Vael.engine = \"");
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
/// Creates `dir` and any nested subdirectories (such as `vael/uuid/`) as needed.
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
/// use vaeljs_dts::{Profile, write_types_dir};
///
/// let dir = std::env::temp_dir().join(format!("vaeljs-dts-doctest-{}", std::process::id()));
/// write_types_dir(Profile::Lite, &dir).unwrap();
/// assert!(dir.join("index.d.ts").is_file());
/// assert!(dir.join("vael/fs.d.ts").is_file());
/// assert!(!dir.join("vael/age.d.ts").exists());
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

/// Returns the expected runtime export surface for every `vael:*` module
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
/// use vaeljs_dts::{Profile, module_dts, parse_declared_exports};
///
/// let fs_dts = module_dts(Profile::V8, "vael:fs").unwrap();
/// let parsed = parse_declared_exports(fs_dts);
/// assert_eq!(parsed.len(), 1);
/// assert_eq!(parsed[0].specifier, "vael:fs");
/// assert!(parsed[0].exports.contains(&"writeFileAtomic".to_string()));
/// assert!(!parsed[0].has_default);
/// ```
#[must_use]
pub fn parse_declared_exports(dts: &str) -> Vec<DeclaredModuleExports> {
    parser::parse_declared_exports(dts)
}

# `neo.jsonc`: manifest foundation

**Status:** parsing, field validation, import-specifier normalization, bounded upward discovery
(`discover_manifest`), deterministic workspace expansion (`Workspace::load`), and pure
import-map/workspace resolution (`ImportResolver`) are implemented in the `neojs-manifest` crate.
Runtime manifest integration is **not** implemented. No function in this crate fetches a package,
installs an import map into a JS engine, matches `include`/`exclude`, or runs a Node compatibility
transform.

The shape is inspired by Deno's `deno.json`, not compatible with it. Unknown keys are rejected
rather than ignored.

## File and discovery

The conventional name is `neo.jsonc`. The crate exposes the name as `MANIFEST_FILE_NAME` and
provides bounded upward discovery via `discover_manifest` (bounded by `MAX_DISCOVERY_DEPTH = 64`,
canonicalized visited directories, and an optional `stop_dir`). Callers can also pass an explicit
path to `Manifest::from_path`, or pass the text to `Manifest::parse`. `from_path` reads at most
`MAX_MANIFEST_BYTES + 1` bytes, so an oversized file is rejected without being loaded whole.

No command-line flag reads `neo.jsonc` yet. `neojs` loads `jsr:` imports, and direct
`gh:`/`github:` specifiers, when remote loading is enabled (`--allow-remote` or
`NEOJS_ALLOW_REMOTE=1`), but this crate still does not install an import map. A `gh:`/`github:`
target is normalized here; the runtime loads a direct specifier of that form. `npm:` and `node:`
imports remain rejected at module resolution.

## JSONC dialect

The parser accepts standard JSON plus:

- `//` line comments and `/* ... */` block comments, outside strings.
- One trailing comma per object or array, for example `{"a": 1,}` or `["a",]`.
- An optional leading UTF-8 byte-order mark. It is skipped as syntax, but its three bytes count
  toward the size limit and toward byte columns, so positions remain relative to the original
  input.

Comments do not nest. Single quotes, unquoted keys, `NaN`, `Infinity`, hexadecimal numbers, and
multiple top-level values are rejected. Duplicate keys are rejected at every object depth; the
last value does not silently win.

The parser is bounded:

| Limit | Value |
| --- | --- |
| Document bytes | 1 MiB (`MAX_JSONC_BYTES`, `MAX_MANIFEST_BYTES`) |
| Nesting depth | 128 (`MAX_JSONC_DEPTH`) |

A document of exactly the size or depth limit is accepted; one byte or one level over is rejected.
Line starts are precomputed in one `O(n)` pass. Recording one top-level key then does an
`O(log L)` binary search over line starts and a `BTreeMap` insertion that is logarithmic in the
number of recorded keys, with string comparison and key-copy work proportional to the key text;
`L` is the line count. Recording many keys is therefore bounded and avoids the previous per-key
prefix rescan, but it is **not linear** in the document size. Errors carry a line and byte column
when one is available. They do not echo document text, field values, unknown keys, schemes,
filesystem paths, or specifiers. An unknown-field error reports the opening quote of the top-level
key but omits the key and its value.

## Fields

Only `name` is required. An absent optional field is omitted or empty, not `null`; an explicit
`null` is rejected.

```jsonc
{
    // Required: scoped package name.
    "name": "@scope/pkg",

    // Optional: exact SemVer. Not a range.
    "version": "1.2.3",

    "description": "One package in the example project.",
    "authors": ["Ada", "Grace"],

    // Optional. A string, or a map of `.` / `./subpath` keys.
    "exports": {
        ".": "./mod.ts",
        "./testing": "./testing.js",
    },

    // Syntax-checked only. Not applied.
    "include": ["src/**/*.ts"],
    "exclude": ["**/*.test.ts"],

    // Normalized only. Not installed in a runtime.
    "imports": {
        "std": "jsr:@scope/std@1.2.3",
        "local": "./vendor/mod.ts",
        "remote": "https://example.com/mod.ts",
    },

    // Syntax-checked only. Not expanded.
    "workspaces": ["packages/*"],
}
```

### `name`

Required. Must be `@scope/name`.

- The complete spelling is at most 214 bytes.
- Scope and name are each 1 to 213 bytes.
- Each part uses only lowercase ASCII letters, digits, `-`, `_`, and `.`, and starts with a letter
  or digit.
- Exactly one `/` is allowed, immediately after the scope.

This is a local validation rule, not a registry account, publication, or npm identity check.

### `version`

Optional. When present it must be an exact SemVer version such as `1.2.3` or `1.2.3-alpha.1`.
Ranges, tags, and partial versions are rejected. The field does not imply publishing support.

### `description`

Optional string, at most 4096 bytes. Tabs, newlines, and carriage returns are allowed; other
control characters are rejected. An empty string is accepted.

### `authors`

Optional array of strings. At most 64 entries, each 1 to 256 bytes, non-empty, without leading or
trailing whitespace or control characters other than tab, newline, and carriage return. Order is
preserved.

### `exports`

Optional. Either a single string or a non-empty object whose keys are `.` or `./subpath`.

- A key may not be absolute, may not contain `*` or `?`, and may not contain empty, `.`, or `..`
  segments.
- A target must start with `./`, stay inside the manifest directory, and end in `.ts`, `.mts`,
  `.js`, or `.mjs`.
- `.tsx`, `.jsx`, `.cts`, `.cjs`, declaration files, JSON, and WebAssembly are rejected with the
  same reason the runtime uses for those extensions.
- Conditional export objects, `null` exclusions, and wildcard targets are not implemented and are
  rejected explicitly.
- At most 1024 entries. A key is at most 512 bytes; a target is at most 1024 bytes.

There is no `main` fallback. `"main"` is a reserved, rejected key with a message to use `exports`.

### `include` and `exclude`

Optional arrays of relative glob patterns. At most 256 entries each; one pattern is at most 1024
bytes.

- Patterns must be relative, use `/`, and have no empty, `.`, or `..` segment.
- A leading `!` is rejected because negation is not implemented.
- `*`, `**`, `?`, `[`, `]`, `{`, and `}` are accepted as syntax.
- This crate does **not** match or expand globs. The fields are metadata for a future packaging
  step.

### `imports`

Optional object mapping a non-empty specifier key to a target string. Keys may not contain
whitespace, control characters, or `\`. At most 1024 entries; a key is at most 1024 bytes.

Targets are normalized by the same public function, `normalize_specifier`. Bare targets such as
`"react"` are rejected because the import map does not have a resolution stage yet. `http:`,
`data:`, and `blob:` targets are rejected as unsupported transports.

| Target | Canonical form | Load status |
| --- | --- | --- |
| `./mod.ts` | `./mod.ts` | Normalized only |
| `file:///tmp/mod.ts` | the normalized `file:` URL | Normalized only |
| `https://example.com/mod.ts` | the normalized HTTPS URL | Normalized only |
| `npm:lodash@4.17.21` | `npm:lodash@4.17.21` | Unsupported |
| `npm:@scope/pkg@^1.2.3/sub` | `npm:@scope/pkg@^1.2.3/sub` | Unsupported |
| `jsr:@scope/pkg@1.2.3/sub` | `jsr:@scope/pkg@1.2.3/sub` | Unsupported |
| `gh:owner/repo@v1.2.3/src/mod.ts` | `https://raw.githubusercontent.com/owner/repo/v1.2.3/src/mod.ts` | Normalized only |
| `github:owner/repo@v1.2.3/src/mod.ts` | the same raw URL | Normalized only |
| `gh:owner/repo/src/mod.ts/@v1.2.3` (legacy) | the same raw URL | Normalized only |
| `node:fs/promises` | `node:fs/promises` | Unsupported |

`ImportMap::first_not_loadable` identifies the first entry, in key order, that
`Support::is_supported` reports as not loadable. That includes `NormalizedOnly` entries, not only
`Unsupported` ones. Because `is_supported` is false for every kind today, a non-empty map always
returns its first entry. A caller that intends to install the map must reject that entry; this
crate never installs it.

#### `npm:`

Grammar: `npm:[@scope/]name@version[/subpath]`.

- An explicit `@version` is required. Omission is rejected instead of guessing `latest`.
- The version is preserved as one token. `^1.2.3`, `1.2.3`, and `latest` are all accepted as
  text; none is resolved. Complex ranges containing spaces are rejected.
- Package names use lowercase ASCII letters, digits, `-`, `_`, and `.`, start with a letter or
  digit, and are at most 214 bytes including a scope.
- A subpath may not contain empty, `.`, or `..` segments.
- No registry metadata, tarball, `package.json`, `node_modules`, CommonJS, lifecycle script, or
  native addon is read.

#### `jsr:`

Grammar: `jsr:@scope/name@version[/subpath]`.

- The package must be scoped. Scope is 2 to 20 bytes; name is 2 to 58 bytes. Both use lowercase
  ASCII letters, digits, and `-`, and may not start or end with `-`.
- An explicit version token is required. It is preserved, not resolved.
- A subpath may not contain empty, `.`, or `..` segments.
- No JSR metadata, module graph, file, checksum, or cache entry is read.

#### `gh:` and `github:`

Grammar: `gh:owner/repo@reference/path`, or the alias `github:owner/repo@reference/path`. The
legacy `gh:owner/repo/path/@reference` spelling remains accepted.

- `owner` is 1 to 39 ASCII letters, digits, or `-`, and may not start or end with `-`.
- `repo` is 1 to 100 ASCII letters, digits, `.`, `_`, or `-`, and may not be `.`, `..`, or `.git`.
- `path` is a repository-relative module path with no empty, `.`, or `..` segment. `\`, `%`, `?`,
  and `#` are rejected, so `%2e%2e` and `%2f` cannot hide traversal.
- In the new form, the reference is the single segment between `@` and the next `/`. A tag such as
  `v1.2.3` and a full 40-character commit SHA are both accepted. A branch name containing `/`
  cannot be expressed, because the first `/` ends the reference; use a tag or a full SHA. The
  path may contain `@`, including a segment that starts with it.
- In the legacy form, the separator is the **last** `/@`, the reference may not contain `/`, and
  an earlier path segment may contain `@`: `gh:o/r/dir@name/mod.ts/@v1` names the path
  `dir@name/mod.ts` at reference `v1`.
- A reference is 1 to 256 bytes of ASCII letters, digits, `.`, `_`, `+`, `-`, or `@`, starts with
  a letter or digit, and may not contain `..`.
- `@` immediately after the repository selects the new form and `/` selects the legacy form, so a
  specifier is never guessed from an ambiguous separator. A specifier with neither is rejected.
- The scheme is matched case-sensitively here: `GH:` is an unknown scheme. (The V8 runtime accepts
  either case because URL schemes are case-insensitive.)

The canonical form is a `raw.githubusercontent.com` HTTPS URL. This crate does not fetch it; a
manifest import map is not installed. The V8 runtime loads a direct `gh:`/`github:` specifier, or
the same raw URL, only when remote loading is enabled. The URL is not a promise that the repository,
reference, or file exists.

#### `node:`

Recognized so the rejection is explicit. `node:fs`, `node:fs/promises`, and similar names are
normalized and marked unsupported. Node built-ins and a Node compatibility transform are not
implemented.

### `workspaces`

Optional array of relative directory patterns, at most 256 entries. Same spelling rules as
`include`/`exclude`; `packages/*` is valid. `Workspace::load` expands member globs
deterministically (sorted by canonical path, bounded by `MAX_WORKSPACE_MEMBERS = 256`), enforces
canonical workspace-root containment, rejects missing member manifests, duplicate `@scope/name`
names, and nested workspaces, and indexes members for `ImportResolver`.

Deno calls this key `workspace` (singular). This foundation uses `workspaces`; `workspace` is
rejected with a message naming the accepted key. That is a deliberate incompatibility, not an
alias.

## Reserved and unknown fields

Unknown keys are rejected. The error reports the key's line and byte column, but not the key or
its value, because an arbitrary key is caller text. These reserved names get a specific reason
instead of a generic unknown field:

| Field | Reason |
| --- | --- |
| `node` | Node compatibility transform not implemented |
| `nodeModulesDir` | `node_modules` layout not implemented |
| `type` | CommonJS/package type selection not implemented; exports are ESM |
| `main` | use `exports`; no `main` fallback |
| `workspace` | use `workspaces` |
| `importMap` | use `imports`; application not implemented |
| `tasks` | task execution not implemented |
| `compilerOptions` | compiler options not implemented |
| `lint` | lint configuration not implemented |
| `fmt` | format configuration not implemented |
| `lock` | lockfile configuration not implemented |

## What is deliberately not implemented

- Wiring `discover_manifest`, `Workspace`, or `ImportResolver` into the CLI or JS runtimes.
- Match or expansion for `include` or `exclude`.
- Publishing a workspace package.
- npm registry resolution, semver-range selection, tarballs, `package.json` fields, CommonJS,
  lifecycle scripts, or native addons.
- JSR metadata, checksums, module graphs, or cache policy.
- GitHub fetching, redirects, or authentication.
- Node built-ins, a Node compatibility transform, or `package.json` generation.
- Conditional exports, export wildcards, `null` export exclusions, or a `main` fallback.
- Lockfiles, offline/frozen modes, and package publishing.

## Security notes

- The crate reads no network. `Manifest::from_path` reads only the path it is given, and at most
  `MAX_MANIFEST_BYTES + 1` bytes from it.
- It reads no credential, environment variable, or cache.
- Error messages omit the supplied filesystem path, unknown keys and values, unknown schemes, and
  specifier text. Unknown-field errors locate the key by line and byte column instead. Canonical
  HTTPS URLs may contain a query because a query is part of module identity, but errors never echo
  the URL or its query. A manifest author should still avoid putting a secret in a committed import
  target.
- Validation is not authorization. A well-formed manifest does not prove a package, path, or host
  is safe to use.
- Import mapping is not a sandbox. A future runtime integration must keep the existing remote
  opt-in and must reject unsupported transports before fetching.

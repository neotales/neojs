---
title: vael:os & vael:path
description: Build-target OS constants and lexical POSIX/Win32 path manipulation modules across vjs and vjs-lite.
---

`vael:os` exposes compilation-target platform constants, and `vael:path` provides lexical POSIX and Windows path manipulation.

:::note[Availability]
Both `vael:os` and `vael:path` are available in **both `vjs` (V8) and `vjs-lite` (QuickJS)**.
:::

## `vael:os`

`vael:os` exports two constant strings derived from Rust's `std::env::consts` at build time without probing the host at runtime:

| Export | Type | Values |
| --- | --- | --- |
| `platform` | `"linux" \| "macos" \| "windows" \| (string & {})` | Rust target OS (`"macos"`, `"linux"`, `"windows"`, etc.). |
| `arch` | `"x86_64" \| "aarch64" \| (string & {})` | Rust target architecture (`"x86_64"`, `"aarch64"`, etc.). |

:::tip[Rust vs Node Vocabulary]
`vael:os` uses Rust target names (`"macos"`, `"x86_64"`, `"aarch64"`). If you need Node.js names (`"darwin"`, `"win32"`, `"x64"`, `"arm64"`), use [`node:os`](/vaeljs/modules/node-compat/#nodeos).
:::

## `vael:path`

`vael:path` performs lexical path operations and never touches the filesystem except when `resolve()`, `relative()`, or Windows `toNamespacedPath()` needs the process working directory because inputs lack an absolute base. All string inputs must be free of NUL (`\0`) characters.

### Dialect Objects & Constants

| Export | Type / Value | Description |
| --- | --- | --- |
| `posix` | `PathDialect` | Frozen POSIX lexical path dialect (`sep = "/"`, `delimiter = ":"`). |
| `win32` | `PathDialect` | Frozen Windows lexical path dialect (`sep = "\\"`, `delimiter = ";"`) . |
| `SEPARATOR` | `"/" \| "\\"` | Host path component separator. |
| `DELIMITER` | `":" \| ";"` | Host path list delimiter. |
| `SEPARATOR_PATTERN` | `RegExp` | Host separator matcher (`/\/+/` on POSIX, `/[\\/]+/` on Windows). |

### Functions (`vael:path` and `PathDialect`)

Every function below is exported at top level (using the host dialect) and as a method on `posix` and `win32`:

| Export | Signature | Description |
| --- | --- | --- |
| `join` | `(...paths: string[]) => string` | Join and normalize path segments. |
| `normalize` | `(path: string) => string` | Collapse `.` / `..` segments and separators. |
| `basename` | `(path: string, suffix?: string) => string` | Final path component, optionally stripping `suffix`. |
| `dirname` | `(path: string) => string` | Directory portion of `path`. |
| `extname` | `(path: string) => string` | Extension including leading `"."`, or `""`. |
| `isAbsolute` | `(path: string) => boolean` | Whether `path` is absolute in that dialect. |
| `parse` | `(path: string) => ParsedPath` | Split into `{ root, dir, base, ext, name }`. |
| `format` | `(pathObject: FormatInputPathObject) => string` | Build a path string (`dir` overrides `root`; `base` overrides `name` + `ext`). |
| `common` | `(paths: readonly string[]) => string` | Shared prefix at component boundaries without normalizing or folding case. |
| `resolve` | `(...paths: string[]) => string` | Resolve segments into an absolute path (reads cwd at most once if needed). |
| `relative` | `(from: string, to: string) => string` | Compute relative path from `from` to `to`. |
| `toNamespacedPath` | `(path: string) => string` | Convert to Windows `\\?\` namespaced path on Windows; unchanged on POSIX. |

### Error Codes (`PathErrorCode`)

Errors thrown on working-directory resolution or invalid Windows roots carry `PathError.code`:

- `"PATH_CWD"` — Process working directory could not be read.
- `"PATH_CWD_DIALECT"` — Working directory does not match the dialect.
- `"PATH_WINDOWS_ROOT"` — Invalid Windows root syntax.

## Example

```ts
import { arch, platform } from "vael:os";
import { common, join, posix, win32 } from "vael:path";

console.log(platform, arch);
console.log(join("var", "log", "..", "lib", "app.db"));
console.log(common(["/srv/app/bin", "/srv/app/etc"])); // "/srv/app"
console.log(posix.sep, win32.toNamespacedPath("C:\\data\\file.txt"));
```

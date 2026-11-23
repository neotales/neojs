---
title: path
description: Lexical POSIX and Windows path manipulation with explicit namespaces and host-selected functions.
---

`path` is available in every V8 runtime and is not available in NeoJS Lite. Import it as `path` or
`neo:path`; the registry form is `Neo.getBuiltInModule("path")`.

It exports frozen `posix` and `win32` namespaces plus host-selected functions, so the same code can
use an explicit syntax on any host.

```js
import { posix, win32, SEPARATOR, SEPARATOR_PATTERN } from "path";

posix.join("/srv", "app", "..", "data"); // "/srv/data"
posix.parse("/srv/data/file.txt");
// { root: "/", dir: "/srv/data", base: "file.txt", ext: ".txt", name: "file" }
win32.normalize("C:/work/out/../log"); // "C:\\work\\log"
win32.resolve("C:/work/out", "../log"); // "C:\\work\\log", on any host
["app", "data"].join(SEPARATOR); // "app/data" or "app\\data"
"app///data".split(SEPARATOR_PATTERN); // ["app", "data"]
```

## Exports

| Export | Notes |
| --- | --- |
| `posix`, `win32` | Frozen namespaces with the same functions and constants |
| `join`, `normalize`, `basename`, `dirname`, `extname` | Host-selected functions |
| `isAbsolute`, `parse`, `format`, `common`, `resolve`, `relative`, `toNamespacedPath` | Host-selected functions |
| `SEPARATOR`, `DELIMITER`, `SEPARATOR_PATTERN` | Match the selected namespace's `sep` and `delimiter` |

`SEPARATOR` and `DELIMITER` are `/` and `:` on POSIX, `\` and `;` on Windows.
`SEPARATOR_PATTERN` matches runs of host separators; on Windows it matches both `/` and `\`.

## Arguments

Every path argument is a primitive string. A boxed `String`, `URL`, buffer, number, `null`,
`undefined`, or other value throws `TypeError`. `basename`'s optional suffix must be a string.
`format` requires a non-null, non-array object whose `root`, `dir`, `base`, `ext`, and `name` fields
are strings or absent. `common` requires an array of strings.

A NUL (`U+0000`) in any argument throws `TypeError`: it cannot name a usable file, and returning a
lexical result containing one would look usable when it is not. Error messages do not repeat the
rejected path.

Empty input follows the conventional dot where a path is required: `normalize("")`, `join()` with no
non-empty segment, and `dirname("")` are `"."`. `basename("")` and `extname("")` are empty,
`parse("")` has five empty fields, `isAbsolute("")` is false, and `format({})` is empty. `parse`
returns a fresh plain object on every call.

## POSIX rules

The separator is `/` only; a backslash is an ordinary filename character.

- `normalize` collapses `/` runs, removes `.`, applies `..`, preserves leading `..` only in relative
  paths, and preserves at most one trailing `/`.
- `//` and `///` collapse to `/`; there is no distinct `//` network root.
- `join` validates every argument, concatenates with `/`, and normalizes. It does not restart at a
  later absolute segment, so `join("a", "/b")` is `"a/b"`.
- `basename` ignores trailing separators. A separator-only path has an empty basename. A suffix is
  removed only when it is non-empty, strictly shorter than the name, and an exact suffix.
- `dirname` returns the text before the final separator with trailing separators removed. A
  separator-only path returns `/`; an empty or relative path with no parent returns `"."`.
- `extname` ignores trailing separators. Dot files, `.`, and `..` have no extension; the last dot of
  the final component starts the extension.
- `parse` does not normalize the whole path.
- `format` appends a non-empty `base` literally, even when it is a separator, so
  `format({ dir: "foo", base: "/" })` is `"foo//"`.

## Windows rules

Both `\` and `/` are input separators; `join`, `normalize`, and `format` produce `\`.

Roots are recognized in this order: UNC `\\server\share`; ASCII drive root `C:\`; drive-relative
`C:`; a single rooted separator; otherwise relative. Exactly two leading separators plus a non-empty
server and share form a UNC root; three or more collapse to one rooted separator.

`normalize` preserves the root and at most one trailing `\`; a bare drive becomes `C:.`; `C:..` is
preserved, and no per-drive working directory is consulted. `join("C:", "foo")` is `"C:\\foo"`;
`join("//server", "share")` is `"\\\\server\\share\\"`.

A complete UNC root, `C:\`, and `C:` have an empty basename and extension even when the share
contains a dot. `dirname("C:/")` preserves the root as written. `relative` compares roots and
components case-insensitively but preserves destination spelling; a different drive or share returns
the absolute destination. `toNamespacedPath` maps drive paths to `\\?\` and UNC paths to
`\\?\UNC\`; empty, already namespaced, and device paths are returned unchanged.

`common` compares components as written, without normalization or case folding, and splits on `\`
only, so `/` is not a component boundary there.

## Working directory and errors

`resolve` works right-to-left, validates every segment, ignores empty strings, and resets at an
absolute segment. `resolve`, `relative`, and Windows `toNamespacedPath` read the current process
working directory only when their inputs do not supply a complete base. A call takes one lazy
snapshot: `relative` shares one snapshot across both endpoints and reads cwd at most once, and does
not read it at all when both endpoints are absolute.

There is no runtime-local `chdir` and no per-drive environment lookup. A failure is an ordinary
`Error` with `name = "PathError"` and a stable own `code`:

| Code | Meaning |
| --- | --- |
| `PATH_CWD` | The working directory is unavailable or not valid UTF-8 |
| `PATH_CWD_DIALECT` | The non-host dialect needs a working directory |
| `PATH_WINDOWS_ROOT` | Windows cannot establish a matching drive, share, or complete namespace root |

There is no `PATH_PERMISSION`; NeoJS has no permission system.

## Scope

This is lexical string manipulation, not a filesystem, permission, or containment boundary. It does
not read, write, stat, or authorize a file; resolve symlinks, junctions, drive mappings, or short
names; validate Windows reserved device names or alternate data streams; fold Unicode confusables; or
promise that a result stays inside a directory. `normalize("a/../../b")` is `"../b"`; enforce
containment with the operating system after resolving the real path.

Glob helpers and file URL conversion are deliberately not implemented. This is not Node's `path`,
not `@neotales/path`, and not NeoLiteJS `neo:path`; URL inputs, coercion, and those compatibility
surfaces are not promised.

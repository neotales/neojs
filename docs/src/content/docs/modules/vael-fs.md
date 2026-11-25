---
title: vael:fs
description: Synchronous filesystem operations with atomic writes, strict UTF-8 decoding, bounded reads, and symlink-safe metadata and removal.
---

`vael:fs` provides 14 synchronous filesystem functions with explicit byte limits, atomic sibling-file replacement (`writeFileAtomic`), strict UTF-8 decoding, and deterministic directory listings.

:::note[Availability]
`vael:fs` is available in **both `vjs` (V8) and `vjs-lite` (QuickJS)**.
:::

## Exports

| Export | Signature | Description |
| --- | --- | --- |
| `readFile` | `(path: string, options?: ReadOptions) => Uint8Array` | Read file bytes (follows symlinks). |
| `readTextFile` | `(path: string, options?: ReadOptions) => string` | Read strict UTF-8 text (follows symlinks; fails on invalid UTF-8). |
| `writeFile` | `(path: string, bytes: Uint8Array, options?: WriteOptions) => void` | Non-atomic binary write (creates and truncates by default). |
| `writeTextFile` | `(path: string, text: string, options?: WriteOptions) => void` | Non-atomic UTF-8 text write. |
| `writeFileAtomic` | `(path: string, bytes: Uint8Array, options?: AtomicWriteOptions) => void` | Atomic write via sibling temp file (`0o600` on Unix) and rename/link. |
| `exists` | `(path: string) => boolean` | Check existence (follows symlinks). Throws on permission/path errors. |
| `stat` | `(path: string) => FileInfo` | Metadata following symlinks. |
| `lstat` | `(path: string) => FileInfo` | Metadata without following symlinks. |
| `readDir` | `(path: string) => DirEntry[]` | List direct entries sorted lexicographically by `name` (excludes `.` and `..`). |
| `mkdir` | `(path: string, options?: MkdirOptions) => void` | Create a directory. |
| `remove` | `(path: string, options?: RemoveOptions) => void` | Remove file, symlink, or directory without following symlinks. |
| `rename` | `(from: string, to: string) => void` | Rename or move a file or directory. |
| `copyFile` | `(from: string, to: string, options?: CopyOptions) => void` | Copy a regular file. |
| `realpath` | `(path: string) => string` | Resolve canonical absolute path following all symlinks. |

## Options & Return Types

| Type | Fields | Defaults & Notes |
| --- | --- | --- |
| `ReadOptions` | `maxBytes?: number` | Unlimited by default; fails with `VAEL_FS_TOO_LARGE` without unbounded allocation. |
| `WriteOptions` | `create?: boolean`, `createNew?: boolean`, `append?: boolean` | Defaults: `create: true`, `createNew: false`, `append: false`. `createNew: true` + `append: true` throws `VAEL_FS_INVALID_ARGUMENT`. |
| `AtomicWriteOptions` | `sync?: boolean`, `overwrite?: boolean` | Defaults: `sync: true`, `overwrite: true`. When `overwrite: false`, existing entries (including symlinks) fail with `VAEL_FS_ALREADY_EXISTS`. |
| `MkdirOptions` | `recursive?: boolean` | Default: `false` (fails if present or parent missing). |
| `RemoveOptions` | `recursive?: boolean` | Default: `false` (fails with `VAEL_FS_DIRECTORY_NOT_EMPTY` on non-empty dirs). |
| `CopyOptions` | `overwrite?: boolean` | Default: `false` (`VAEL_FS_ALREADY_EXISTS` if destination exists). |
| `FileInfo` | `isFile`, `isDirectory`, `isSymlink`, `size`, `modifiedMs: number \| null`, `readonly` | `isSymlink` is always `false` for `stat()`. |
| `DirEntry` | `name: string`, `isFile`, `isDirectory`, `isSymlink` | Does not follow symlinks; fails closed with `VAEL_FS_INVALID_UTF8` on non-UTF-8 entry names. |

## Symlink Safety & Validation Rules

- **Invalid paths**: Empty strings and paths containing NUL (`\0`) bytes fail with `VAEL_FS_INVALID_PATH`.
- **Trailing segments**: `writeFileAtomic` rejects trailing separators, `.`, and trailing `/.`; `lstat` and `remove` reject trailing `/.` with `VAEL_FS_INVALID_PATH` (`lstat` strips trailing separators on non-root paths so directory symlinks are inspected directly).
- **No silent error swallowing**: `exists(path)` returns `false` only when the OS reports `NotFound`; permission errors or invalid paths throw `Error` with `code: FsErrorCode`.
- **Safe `copyFile`**: With `overwrite: false` (default), destination creation is exclusive (`O_EXCL`) to avoid TOCTOU clobbering, and partial files are cleaned up on failure. When `overwrite: true` and `from`/`to` resolve to the same underlying file, `copyFile` returns without truncating.

## Error Codes (`FsErrorCode`)

Wrong argument types throw `TypeError`. All filesystem and validation failures throw `Error` with `code` set to one of:

`VAEL_FS_NOT_FOUND` | `VAEL_FS_ALREADY_EXISTS` | `VAEL_FS_PERMISSION_DENIED` | `VAEL_FS_IS_A_DIRECTORY` | `VAEL_FS_NOT_A_DIRECTORY` | `VAEL_FS_DIRECTORY_NOT_EMPTY` | `VAEL_FS_INVALID_PATH` | `VAEL_FS_INVALID_ARGUMENT` | `VAEL_FS_INVALID_UTF8` | `VAEL_FS_TOO_LARGE` | `VAEL_FS_IO`

## Example

```ts
import { mkdir, readDir, readTextFile, writeFileAtomic } from "vael:fs";

mkdir("./tmp-config", { recursive: true });

const payload = new TextEncoder().encode(JSON.stringify({ port: 8080 }));
writeFileAtomic("./tmp-config/app.json", payload, { sync: true });

const text = readTextFile("./tmp-config/app.json", { maxBytes: 65536 });
const entries = readDir("./tmp-config");
console.log(text, entries.map((e) => e.name));
```

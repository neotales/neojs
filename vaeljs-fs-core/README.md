# vaeljs-fs-core

Engine-neutral core filesystem operations for the `vael:fs` module, designed to
be shared by both the V8 runtime (`vaeljs-core`) and the QuickJS runtime
(`vaeljs-lite-core`).

This crate has zero JavaScript engine dependencies (`deno_core`, `v8`, and
`rquickjs` are absent) and zero network dependencies. Its direct runtime
dependencies are `thiserror` and `getrandom` only.

```sh
cargo test --locked -p vaeljs-fs-core
cargo clippy --locked -p vaeljs-fs-core --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p vaeljs-fs-core --no-deps
cargo tree --locked -p vaeljs-fs-core -e normal
```

## Module surface

* `SPECIFIER`: `"vael:fs"`
* `EXPORTS`: `readFile`, `readTextFile`, `writeFile`, `writeTextFile`,
  `writeFileAtomic`, `exists`, `stat`, `lstat`, `readDir`, `mkdir`, `remove`,
  `rename`, `copyFile`, `realpath`

## Guarantees and error model

* **Input path guard**: every operation rejects empty paths and paths
  containing NUL (`\0`) bytes with `FsErrorCode::InvalidPath`
  (`VAEL_FS_INVALID_PATH`). Error display strings never echo caller-supplied
  paths.
* **Typed error codes**: `FsError` (`#[non_exhaustive]`) maps every failure to
  a stable `FsErrorCode`: `VAEL_FS_NOT_FOUND`, `VAEL_FS_ALREADY_EXISTS`,
  `VAEL_FS_PERMISSION_DENIED`, `VAEL_FS_IS_A_DIRECTORY`, `VAEL_FS_NOT_A_DIRECTORY`,
  `VAEL_FS_DIRECTORY_NOT_EMPTY`, `VAEL_FS_INVALID_PATH`,
  `VAEL_FS_INVALID_ARGUMENT`, `VAEL_FS_INVALID_UTF8`, `VAEL_FS_TOO_LARGE`, and
  `VAEL_FS_IO`.
* **Bounded reads**: `read_file` and `read_text_file` accept
  `ReadOptions { max_bytes }`, pre-checking regular-file metadata and reading
  through a `take(max_bytes + 1)` bound so oversized files fail with
  `VAEL_FS_TOO_LARGE` without unbounded allocation. `read_text_file` requires
  strict UTF-8 (`VAEL_FS_INVALID_UTF8`).
* **Atomic writes**: `write_file_atomic` writes to a unique sibling temporary
  file (`create_new(true)` + random suffix from fallible `getrandom::fill`,
  `0o600` on Unix), optionally calls `sync_all()`, cleans up the temporary file
  via RAII on any error without masking the original cause, and atomically
  commits the target (`overwrite: false` fails with `VAEL_FS_ALREADY_EXISTS`
  without clobbering).
* **Deterministic directory listing**: `read_dir` returns entries sorted
  lexicographically by `name` without following symlinks and fails closed with
  `VAEL_FS_INVALID_UTF8` on non-UTF-8 entry names.
* **Safe removal and copying**: `remove` never follows symbolic links (even
  when `recursive: true`), and `copy_file` with `overwrite: false` creates the
  destination using `create_new(true)` to avoid TOCTOU clobbering.

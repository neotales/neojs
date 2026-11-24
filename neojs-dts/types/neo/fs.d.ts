declare module "neo:fs" {
  /**
   * Stable machine-readable `NEO_FS_*` error codes exposed on `Error.code`.
   */
  export type FsErrorCode =
    | "NEO_FS_NOT_FOUND"
    | "NEO_FS_ALREADY_EXISTS"
    | "NEO_FS_PERMISSION_DENIED"
    | "NEO_FS_IS_A_DIRECTORY"
    | "NEO_FS_NOT_A_DIRECTORY"
    | "NEO_FS_DIRECTORY_NOT_EMPTY"
    | "NEO_FS_INVALID_PATH"
    | "NEO_FS_INVALID_ARGUMENT"
    | "NEO_FS_INVALID_UTF8"
    | "NEO_FS_TOO_LARGE"
    | "NEO_FS_IO";

  /**
   * Options controlling `readFile` and `readTextFile`.
   */
  export interface ReadOptions {
    /**
     * Maximum number of bytes allowed when reading a file (default: unlimited).
     * Files larger than `maxBytes` fail with `NEO_FS_TOO_LARGE` without
     * unbounded allocation.
     */
    maxBytes?: number;
  }

  /**
   * Options controlling `writeFile` and `writeTextFile`.
   */
  export interface WriteOptions {
    /** Create the file if it does not already exist (default: `true`). */
    create?: boolean;
    /**
     * Require exclusive creation and fail with `NEO_FS_ALREADY_EXISTS` if the
     * file already exists (default: `false`).
     */
    createNew?: boolean;
    /**
     * Append writes to the end of the file instead of truncating it
     * (default: `false`). Combining `createNew: true` and `append: true` fails
     * with `NEO_FS_INVALID_ARGUMENT`.
     */
    append?: boolean;
  }

  /**
   * Options controlling `writeFileAtomic`.
   */
  export interface AtomicWriteOptions {
    /**
     * Call `sync_all` on the temporary file (and best-effort on its parent
     * directory on Unix) before replacing the target (default: `true`).
     */
    sync?: boolean;
    /**
     * Replace an existing target entry if present (default: `true`). When
     * `false`, any existing entry (including a symlink) fails with
     * `NEO_FS_ALREADY_EXISTS` without clobbering.
     */
    overwrite?: boolean;
  }

  /**
   * Options controlling `mkdir`.
   */
  export interface MkdirOptions {
    /**
     * Create missing parent directories and succeed if the directory already
     * exists (default: `false`).
     */
    recursive?: boolean;
  }

  /**
   * Options controlling `remove`.
   */
  export interface RemoveOptions {
    /**
     * Recursively remove directory contents without following symbolic links
     * (default: `false`).
     */
    recursive?: boolean;
  }

  /**
   * Options controlling `copyFile`.
   */
  export interface CopyOptions {
    /**
     * Overwrite the destination file if it already exists (default: `false`).
     * When `false`, existing targets fail with `NEO_FS_ALREADY_EXISTS` without
     * TOCTOU clobbering.
     */
    overwrite?: boolean;
  }

  /**
   * Snapshot metadata returned by `stat` and `lstat`.
   */
  export interface FileInfo {
    /** Whether the entry is a regular file. */
    isFile: boolean;
    /** Whether the entry is a directory. */
    isDirectory: boolean;
    /** Whether the entry is a symbolic link (always `false` for `stat`). */
    isSymlink: boolean;
    /** Size of the entry in bytes. */
    size: number;
    /**
     * Last modification time in whole milliseconds since the Unix epoch, or
     * `null` if unavailable from the filesystem or outside signed 64-bit range.
     */
    modifiedMs: number | null;
    /** Whether the entry's permissions mark it as read-only. */
    readonly: boolean;
  }

  /**
   * Snapshot directory entry returned in sorted order by `readDir`.
   */
  export interface DirEntry {
    /** Single path component name (valid UTF-8). */
    name: string;
    /** Whether the entry is a regular file (does not follow symlinks). */
    isFile: boolean;
    /** Whether the entry is a directory (does not follow symlinks). */
    isDirectory: boolean;
    /** Whether the entry is a symbolic link. */
    isSymlink: boolean;
  }

  /**
   * Synchronously read the entire file at `path` into a `Uint8Array`,
   * following symbolic links.
   *
   * Rejects empty paths and paths containing NUL (`\0`) bytes with
   * `NEO_FS_INVALID_PATH`, directories with `NEO_FS_IS_A_DIRECTORY`, and files
   * exceeding `options.maxBytes` with `NEO_FS_TOO_LARGE`.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` on filesystem or validation failure.
   */
  export function readFile(path: string, options?: ReadOptions): Uint8Array;

  /**
   * Synchronously read the entire file at `path` as strict UTF-8 text,
   * following symbolic links.
   *
   * Fails closed with `NEO_FS_INVALID_UTF8` on non-UTF-8 byte sequences (no
   * lossy replacement). Also enforces `options.maxBytes` (`NEO_FS_TOO_LARGE`).
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` on filesystem, UTF-8, or validation failure.
   */
  export function readTextFile(path: string, options?: ReadOptions): string;

  /**
   * Synchronously write `bytes` to `path` (non-atomic; creates and truncates by
   * default).
   *
   * An I/O failure mid-write can leave a partial file; use `writeFileAtomic`
   * when all-or-nothing replacement is required.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` (`NEO_FS_INVALID_ARGUMENT` if both
   * `createNew` and `append` are `true`).
   */
  export function writeFile(path: string, bytes: Uint8Array, options?: WriteOptions): void;

  /**
   * Synchronously write UTF-8 `text` to `path` (non-atomic; creates and
   * truncates by default).
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function writeTextFile(path: string, text: string, options?: WriteOptions): void;

  /**
   * Synchronously and atomically write `bytes` to `path` via a unique sibling
   * temporary file (`0o600` on Unix) and atomic rename/link commit.
   *
   * Defaults: `sync: true`, `overwrite: true`. Rejects trailing separators,
   * `.`, and trailing `/.` segments with `NEO_FS_INVALID_PATH`. Temporary files
   * are cleaned up on failure without masking the original error.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function writeFileAtomic(
    path: string,
    bytes: Uint8Array,
    options?: AtomicWriteOptions,
  ): void;

  /**
   * Synchronously check whether `path` resolves to an existing entry, following
   * symbolic links.
   *
   * Returns `false` only when the OS reports `NotFound`; permission errors and
   * invalid paths throw rather than returning `false`.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function exists(path: string): boolean;

  /**
   * Synchronously return metadata for `path`, following symbolic links.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` (`NEO_FS_NOT_FOUND` if missing or dangling symlink).
   */
  export function stat(path: string): FileInfo;

  /**
   * Synchronously return metadata for `path` without following symbolic links.
   *
   * Trailing separators on non-root paths are stripped so symlinks to
   * directories are inspected directly; trailing `/.` segments fail with
   * `NEO_FS_INVALID_PATH`.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function lstat(path: string): FileInfo;

  /**
   * Synchronously list direct entries in directory `path`, sorted
   * lexicographically by `name`.
   *
   * Excludes `.` and `..` and does not follow symbolic links. Fails closed with
   * `NEO_FS_INVALID_UTF8` if any entry name is not valid UTF-8.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function readDir(path: string): DirEntry[];

  /**
   * Synchronously create a directory at `path`.
   *
   * Default `recursive: false` fails with `NEO_FS_ALREADY_EXISTS` if present or
   * `NEO_FS_NOT_FOUND` if parent is missing.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function mkdir(path: string, options?: MkdirOptions): void;

  /**
   * Synchronously remove a file, symbolic link, or directory at `path` without
   * following symbolic links.
   *
   * Default `recursive: false` fails with `NEO_FS_DIRECTORY_NOT_EMPTY` on
   * non-empty directories. Trailing `/.` segments fail with `NEO_FS_INVALID_PATH`.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function remove(path: string, options?: RemoveOptions): void;

  /**
   * Synchronously rename or move a file or directory from `from` to `to`.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function rename(from: string, to: string): void;

  /**
   * Synchronously copy a regular file from `from` to `to`.
   *
   * Default `overwrite: false` creates `to` exclusively (`NEO_FS_ALREADY_EXISTS`
   * if occupied) and cleans up partial targets on error. When `overwrite: true`
   * and `from`/`to` resolve to the same underlying file, returns without truncating.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` (`NEO_FS_IS_A_DIRECTORY` if `from` is a directory).
   */
  export function copyFile(from: string, to: string, options?: CopyOptions): void;

  /**
   * Synchronously resolve the canonical, absolute filesystem path for `path`,
   * following all symbolic links and normalizing `.` and `..` components.
   *
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   */
  export function realpath(path: string): string;
}

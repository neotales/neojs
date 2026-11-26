/**
 * Bounded synchronous filesystem operations and atomic file writes.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * All operations are synchronous, validate NUL-free UTF-8 paths, and report
 * machine-readable `VAEL_FS_*` error codes on `Error.code`.
 *
 * @example
 * ```ts
 * import { exists, mkdir, readTextFile, remove, writeTextFile } from "vael:fs";
 *
 * mkdir("/tmp/vael-fs-example", { recursive: true });
 * writeTextFile("/tmp/vael-fs-example/hello.txt", "hello vael\n");
 * if (exists("/tmp/vael-fs-example/hello.txt")) {
 *   console.log(readTextFile("/tmp/vael-fs-example/hello.txt"));
 * }
 * remove("/tmp/vael-fs-example", { recursive: true });
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:fs" {
  /**
   * Stable machine-readable `VAEL_FS_*` error codes exposed on `Error.code`.
   *
   * @since 0.0.0-alpha.0
   */
  export type FsErrorCode =
    | "VAEL_FS_NOT_FOUND"
    | "VAEL_FS_ALREADY_EXISTS"
    | "VAEL_FS_PERMISSION_DENIED"
    | "VAEL_FS_IS_A_DIRECTORY"
    | "VAEL_FS_NOT_A_DIRECTORY"
    | "VAEL_FS_DIRECTORY_NOT_EMPTY"
    | "VAEL_FS_INVALID_PATH"
    | "VAEL_FS_INVALID_ARGUMENT"
    | "VAEL_FS_INVALID_UTF8"
    | "VAEL_FS_TOO_LARGE"
    | "VAEL_FS_IO";

  /**
   * Options controlling `readFile` and `readTextFile`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface ReadOptions {
    /**
     * Maximum number of bytes allowed when reading a file (default: unlimited).
     * Files larger than `maxBytes` fail with `VAEL_FS_TOO_LARGE` without
     * unbounded allocation.
     *
     * @since 0.0.0-alpha.0
     */
    maxBytes?: number;
  }

  /**
   * Options controlling `writeFile` and `writeTextFile`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface WriteOptions {
    /**
     * Create the file if it does not already exist (default: `true`).
     *
     * @since 0.0.0-alpha.0
     */
    create?: boolean;

    /**
     * Require exclusive creation and fail with `VAEL_FS_ALREADY_EXISTS` if the
     * file already exists (default: `false`).
     *
     * @since 0.0.0-alpha.0
     */
    createNew?: boolean;

    /**
     * Append writes to the end of the file instead of truncating it
     * (default: `false`). Combining `createNew: true` and `append: true` fails
     * with `VAEL_FS_INVALID_ARGUMENT`.
     *
     * @since 0.0.0-alpha.0
     */
    append?: boolean;
  }

  /**
   * Options controlling `writeFileAtomic`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface AtomicWriteOptions {
    /**
     * Call `sync_all` on the temporary file (and best-effort on its parent
     * directory on Unix) before replacing the target (default: `true`).
     *
     * @since 0.0.0-alpha.0
     */
    sync?: boolean;

    /**
     * Replace an existing target entry if present (default: `true`). When
     * `false`, any existing entry (including a symlink) fails with
     * `VAEL_FS_ALREADY_EXISTS` without clobbering.
     *
     * @since 0.0.0-alpha.0
     */
    overwrite?: boolean;
  }

  /**
   * Options controlling `mkdir`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface MkdirOptions {
    /**
     * Create missing parent directories and succeed if the directory already
     * exists (default: `false`).
     *
     * @since 0.0.0-alpha.0
     */
    recursive?: boolean;
  }

  /**
   * Options controlling `remove`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface RemoveOptions {
    /**
     * Recursively remove directory contents without following symbolic links
     * (default: `false`).
     *
     * @since 0.0.0-alpha.0
     */
    recursive?: boolean;
  }

  /**
   * Options controlling `copyFile`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface CopyOptions {
    /**
     * Overwrite the destination file if it already exists (default: `false`).
     * When `false`, existing targets fail with `VAEL_FS_ALREADY_EXISTS` without
     * TOCTOU clobbering.
     *
     * @since 0.0.0-alpha.0
     */
    overwrite?: boolean;
  }

  /**
   * Snapshot metadata returned by `stat` and `lstat`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FileInfo {
    /** Whether the entry is a regular file. @since 0.0.0-alpha.0 */
    isFile: boolean;
    /** Whether the entry is a directory. @since 0.0.0-alpha.0 */
    isDirectory: boolean;
    /** Whether the entry is a symbolic link (always `false` for `stat`). @since 0.0.0-alpha.0 */
    isSymlink: boolean;
    /** Size of the entry in bytes. @since 0.0.0-alpha.0 */
    size: number;
    /**
     * Last modification time in whole milliseconds since the Unix epoch, or
     * `null` if unavailable from the filesystem or outside signed 64-bit range.
     *
     * @since 0.0.0-alpha.0
     */
    modifiedMs: number | null;
    /** Whether the entry's permissions mark it as read-only. @since 0.0.0-alpha.0 */
    readonly: boolean;
  }

  /**
   * Snapshot directory entry returned in sorted order by `readDir`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface DirEntry {
    /** Single path component name (valid UTF-8). @since 0.0.0-alpha.0 */
    name: string;
    /** Whether the entry is a regular file (does not follow symlinks). @since 0.0.0-alpha.0 */
    isFile: boolean;
    /** Whether the entry is a directory (does not follow symlinks). @since 0.0.0-alpha.0 */
    isDirectory: boolean;
    /** Whether the entry is a symbolic link. @since 0.0.0-alpha.0 */
    isSymlink: boolean;
  }

  /**
   * Synchronously read the entire file at `path` into a `Uint8Array`,
   * following symbolic links.
   *
   * Rejects empty paths and paths containing NUL (`\0`) bytes with
   * `VAEL_FS_INVALID_PATH`, directories with `VAEL_FS_IS_A_DIRECTORY`, and files
   * exceeding `options.maxBytes` with `VAEL_FS_TOO_LARGE`.
   *
   * @param path Filesystem path to read.
   * @param options Optional read bounds (`{ maxBytes }`).
   * @returns File contents as a `Uint8Array`.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` on filesystem or validation failure.
   * @example
   * ```ts
   * import { readFile } from "vael:fs";
   *
   * const bytes = readFile("README.md", { maxBytes: 1024 * 1024 });
   * console.log(bytes.byteLength);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function readFile(path: string, options?: ReadOptions): Uint8Array;

  /**
   * Synchronously read the entire file at `path` as strict UTF-8 text,
   * following symbolic links.
   *
   * Fails closed with `VAEL_FS_INVALID_UTF8` on non-UTF-8 byte sequences (no
   * lossy replacement). Also enforces `options.maxBytes` (`VAEL_FS_TOO_LARGE`).
   *
   * @param path Filesystem path to read.
   * @param options Optional read bounds (`{ maxBytes }`).
   * @returns Decoded strict UTF-8 string.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` on filesystem, UTF-8, or validation failure.
   * @example
   * ```ts
   * import { readTextFile } from "vael:fs";
   *
   * const content = readTextFile("README.md", { maxBytes: 65_536 });
   * console.log(content.length);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function readTextFile(path: string, options?: ReadOptions): string;

  /**
   * Synchronously write `bytes` to `path` (non-atomic; creates and truncates by
   * default).
   *
   * An I/O failure mid-write can leave a partial file; use `writeFileAtomic`
   * when all-or-nothing replacement is required.
   *
   * @param path Target filesystem path.
   * @param bytes Bytes to write.
   * @param options Optional `{ create, createNew, append }` flags.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` (`VAEL_FS_INVALID_ARGUMENT` if both
   * `createNew` and `append` are `true`).
   * @example
   * ```ts
   * import { writeFile } from "vael:fs";
   *
   * writeFile("/tmp/data.bin", new Uint8Array([1, 2, 3, 4]));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function writeFile(path: string, bytes: Uint8Array, options?: WriteOptions): void;

  /**
   * Synchronously write UTF-8 `text` to `path` (non-atomic; creates and
   * truncates by default).
   *
   * @param path Target filesystem path.
   * @param text UTF-8 string to write.
   * @param options Optional `{ create, createNew, append }` flags.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @example
   * ```ts
   * import { writeTextFile } from "vael:fs";
   *
   * writeTextFile("/tmp/greeting.txt", "hello\n");
   * ```
   * @since 0.0.0-alpha.0
   */
  export function writeTextFile(path: string, text: string, options?: WriteOptions): void;

  /**
   * Synchronously and atomically write `bytes` to `path` via a unique sibling
   * temporary file (`0o600` on Unix) and atomic rename/link commit.
   *
   * Defaults: `sync: true`, `overwrite: true`. Rejects trailing separators,
   * `.`, and trailing `/.` segments with `VAEL_FS_INVALID_PATH`. Temporary files
   * are cleaned up on failure without masking the original error.
   *
   * @param path Target filesystem path.
   * @param bytes Bytes to write atomically.
   * @param options Optional `{ sync, overwrite }` flags.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @example
   * ```ts
   * import { writeFileAtomic } from "vael:fs";
   *
   * writeFileAtomic("/tmp/state.bin", new Uint8Array([7, 8, 9]), { sync: true });
   * ```
   * @since 0.0.0-alpha.0
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
   * @param path Filesystem path to probe.
   * @returns `true` if `path` resolves to an existing entry.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @example
   * ```ts
   * import { exists } from "vael:fs";
   *
   * console.log(exists("README.md"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function exists(path: string): boolean;

  /**
   * Synchronously return metadata for `path`, following symbolic links.
   *
   * @param path Filesystem path to inspect.
   * @returns Snapshot `FileInfo` metadata.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` (`VAEL_FS_NOT_FOUND` if missing or dangling symlink).
   * @since 0.0.0-alpha.0
   */
  export function stat(path: string): FileInfo;

  /**
   * Synchronously return metadata for `path` without following symbolic links.
   *
   * Trailing separators on non-root paths are stripped so symlinks to
   * directories are inspected directly; trailing `/.` segments fail with
   * `VAEL_FS_INVALID_PATH`.
   *
   * @param path Filesystem path to inspect.
   * @returns Snapshot `FileInfo` metadata for the entry itself.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @since 0.0.0-alpha.0
   */
  export function lstat(path: string): FileInfo;

  /**
   * Synchronously list direct entries in directory `path`, sorted
   * lexicographically by `name`.
   *
   * Excludes `.` and `..` and does not follow symbolic links. Fails closed with
   * `VAEL_FS_INVALID_UTF8` if any entry name is not valid UTF-8.
   *
   * @param path Directory path to list.
   * @returns Sorted array of `DirEntry` records.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @example
   * ```ts
   * import { readDir } from "vael:fs";
   *
   * for (const entry of readDir(".")) {
   *   console.log(entry.name, entry.isDirectory);
   * }
   * ```
   * @since 0.0.0-alpha.0
   */
  export function readDir(path: string): DirEntry[];

  /**
   * Synchronously create a directory at `path`.
   *
   * Default `recursive: false` fails with `VAEL_FS_ALREADY_EXISTS` if present or
   * `VAEL_FS_NOT_FOUND` if parent is missing.
   *
   * @param path Directory path to create.
   * @param options Optional `{ recursive }` flag.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @since 0.0.0-alpha.0
   */
  export function mkdir(path: string, options?: MkdirOptions): void;

  /**
   * Synchronously remove a file, symbolic link, or directory at `path` without
   * following symbolic links.
   *
   * Default `recursive: false` fails with `VAEL_FS_DIRECTORY_NOT_EMPTY` on
   * non-empty directories. Trailing `/.` segments fail with `VAEL_FS_INVALID_PATH`.
   *
   * @param path Filesystem entry path to remove.
   * @param options Optional `{ recursive }` flag.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @since 0.0.0-alpha.0
   */
  export function remove(path: string, options?: RemoveOptions): void;

  /**
   * Synchronously rename or move a file or directory from `from` to `to`.
   *
   * @param from Source path.
   * @param to Destination path.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @since 0.0.0-alpha.0
   */
  export function rename(from: string, to: string): void;

  /**
   * Synchronously copy a regular file from `from` to `to`.
   *
   * Default `overwrite: false` creates `to` exclusively (`VAEL_FS_ALREADY_EXISTS`
   * if occupied) and cleans up partial targets on error. When `overwrite: true`
   * and `from`/`to` resolve to the same underlying file, returns without truncating.
   *
   * @param from Source regular file path.
   * @param to Destination file path.
   * @param options Optional `{ overwrite }` flag.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode` (`VAEL_FS_IS_A_DIRECTORY` if `from` is a directory).
   * @since 0.0.0-alpha.0
   */
  export function copyFile(from: string, to: string, options?: CopyOptions): void;

  /**
   * Synchronously resolve the canonical, absolute filesystem path for `path`,
   * following all symbolic links and normalizing `.` and `..` components.
   *
   * @param path Filesystem path to canonicalize.
   * @returns Canonical absolute path string.
   * @throws {TypeError} on wrong argument types.
   * @throws {Error} with `code: FsErrorCode`.
   * @since 0.0.0-alpha.0
   */
  export function realpath(path: string): string;
}

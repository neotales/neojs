/**
 * Node-compatible synchronous and promise-based filesystem operations.
 *
 * Available in `vjs` (`vjs-lite` omits `promises` and `node:fs/promises`,
 * providing synchronous operations only). Enabled by default unless
 * `--no-node-compat` is passed.
 *
 * @example
 * ```ts
 * import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
 *
 * mkdirSync("/tmp/vael-fs-demo", { recursive: true });
 * writeFileSync("/tmp/vael-fs-demo/hello.txt", "hello vaeljs", "utf8");
 * console.log(readFileSync("/tmp/vael-fs-demo/hello.txt", "utf8"));
 * rmSync("/tmp/vael-fs-demo", { recursive: true, force: true });
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:fs" {
  /**
   * Supported text encodings for filesystem string operations.
   *
   * @since 0.0.0-alpha.0
   */
  export type TextEncoding = "utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii";
  /**
   * Filesystem path string or `file://` URL-like object.
   *
   * @since 0.0.0-alpha.0
   */
  export type PathInput = string | { readonly href: string; readonly protocol: string };

  /**
   * File metadata returned by `statSync`, `lstatSync`, `stat`, and `lstat`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Stats {
    /**
     * Size of the file in bytes.
     *
     * @since 0.0.0-alpha.0
     */
    readonly size: number;
    /**
     * Last modification timestamp in milliseconds since the Unix epoch.
     *
     * @since 0.0.0-alpha.0
     */
    readonly mtimeMs: number;
    /**
     * Last modification timestamp as a `Date` instance.
     *
     * @since 0.0.0-alpha.0
     */
    readonly mtime: Date;
    /**
     * Return `true` if the entry is a regular file.
     *
     * @returns `true` for a regular file, otherwise `false`.
     * @since 0.0.0-alpha.0
     */
    isFile(): boolean;
    /**
     * Return `true` if the entry is a directory.
     *
     * @returns `true` for a directory, otherwise `false`.
     * @since 0.0.0-alpha.0
     */
    isDirectory(): boolean;
    /**
     * Return `true` if the entry is a symbolic link.
     *
     * @returns `true` for a symbolic link, otherwise `false`.
     * @since 0.0.0-alpha.0
     */
    isSymbolicLink(): boolean;
  }

  /**
   * Directory entry returned by `readdirSync` / `readdir` with `withFileTypes: true`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Dirent {
    /**
     * Entry filename relative to the directory passed to `readdirSync` / `readdir`.
     *
     * @since 0.0.0-alpha.0
     */
    readonly name: string;
    /**
     * Return `true` if the entry is a regular file.
     *
     * @returns `true` for a regular file, otherwise `false`.
     * @since 0.0.0-alpha.0
     */
    isFile(): boolean;
    /**
     * Return `true` if the entry is a directory.
     *
     * @returns `true` for a directory, otherwise `false`.
     * @since 0.0.0-alpha.0
     */
    isDirectory(): boolean;
    /**
     * Return `true` if the entry is a symbolic link.
     *
     * @returns `true` for a symbolic link, otherwise `false`.
     * @since 0.0.0-alpha.0
     */
    isSymbolicLink(): boolean;
  }

  /**
   * Filesystem access and copy mode constants.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FsConstants {
    /**
     * Flag testing existence of the path (`0`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly F_OK: number;
    /**
     * Flag testing read permission (`4`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly R_OK: number;
    /**
     * Flag testing write permission (`2`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly W_OK: number;
    /**
     * Flag testing execute permission (`1`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly X_OK: number;
    /**
     * Copy flag failing if the destination file already exists (`1`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly COPYFILE_EXCL: number;
    /**
     * Copy flag requesting copy-on-write reflink if supported (`2`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly COPYFILE_FICLONE: number;
    /**
     * Copy flag requiring copy-on-write reflink (`4`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly COPYFILE_FICLONE_FORCE: number;
  }

  /**
   * Promise-based filesystem API exposed as `fs.promises`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FsPromises {
    /**
     * Asynchronously read the entire contents of `path` as bytes.
     *
     * @param path Filesystem path or `file://` URL.
     * @param options Optional object with `encoding: null | undefined` (or `null`).
     * @returns Promise resolving to a `Buffer` (`Uint8Array` subclass) with the file bytes.
     * @throws {TypeError} on invalid `path` or `options` types.
     * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EISDIR"`, `"EINVAL"`, `"ERR_FS_FILE_TOO_LARGE"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    readFile(path: PathInput, options?: { encoding?: null | undefined } | null): Promise<Uint8Array>;
    /**
     * Asynchronously read the entire contents of `path` as an encoded string.
     *
     * @param path Filesystem path or `file://` URL.
     * @param options Encoding name or `{ encoding }` object.
     * @returns Promise resolving to the decoded string contents.
     * @throws {TypeError} on invalid argument types or unknown encoding (`ERR_UNKNOWN_ENCODING`).
     * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EISDIR"`, `"EINVAL"`, `"ERR_FS_FILE_TOO_LARGE"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    readFile(path: PathInput, options: TextEncoding | { encoding: TextEncoding }): Promise<string>;
    /**
     * Asynchronously write `data` to `path`, replacing the file if it already exists.
     *
     * @param path Destination filesystem path or `file://` URL.
     * @param data String or byte view to write.
     * @param options Encoding for string `data` (default `"utf8"`) or options object.
     * @returns Promise resolving when the write completes.
     * @throws {TypeError} on invalid argument types or unknown encoding (`ERR_UNKNOWN_ENCODING`).
     * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EISDIR"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    writeFile(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): Promise<void>;
    /**
     * Asynchronously append `data` to `path`, creating the file if it does not exist.
     *
     * @param path Destination filesystem path or `file://` URL.
     * @param data String or byte view to append.
     * @param options Encoding for string `data` (default `"utf8"`) or options object.
     * @returns Promise resolving when the append completes.
     * @throws {TypeError} on invalid argument types or unknown encoding (`ERR_UNKNOWN_ENCODING`).
     * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EISDIR"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    appendFile(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): Promise<void>;
    /**
     * Asynchronously return `Stats` for `path`, following symbolic links.
     *
     * @param path Filesystem path or `file://` URL.
     * @returns Promise resolving to `Stats` for the resolved target.
     * @throws {TypeError} when `path` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    stat(path: PathInput): Promise<Stats>;
    /**
     * Asynchronously return `Stats` for `path`, without following symbolic links.
     *
     * @param path Filesystem path or `file://` URL.
     * @returns Promise resolving to `Stats` for the entry itself.
     * @throws {TypeError} when `path` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    lstat(path: PathInput): Promise<Stats>;
    /**
     * Asynchronously read directory entry names.
     *
     * @param path Directory path or `file://` URL.
     * @param options Optional `{ withFileTypes?: false }`.
     * @returns Promise resolving to an array of entry name strings.
     * @throws {TypeError} when `path` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"`, `"ENOTDIR"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    readdir(path: PathInput, options?: { withFileTypes?: false }): Promise<string[]>;
    /**
     * Asynchronously read directory entries as `Dirent` objects.
     *
     * @param path Directory path or `file://` URL.
     * @param options `{ withFileTypes: true }`.
     * @returns Promise resolving to an array of `Dirent` entries.
     * @throws {TypeError} when `path` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"`, `"ENOTDIR"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    readdir(path: PathInput, options: { withFileTypes: true }): Promise<Dirent[]>;
    /**
     * Asynchronously create a directory at `path`.
     *
     * @param path Directory path or `file://` URL.
     * @param options Set `recursive: true` to create missing parent directories and ignore existing directories.
     * @returns Promise resolving when the directory exists.
     * @throws {TypeError} when `path` has an invalid type.
     * @throws {Error} with `code` `"EEXIST"`, `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    mkdir(path: PathInput, options?: { recursive?: boolean }): Promise<void>;
    /**
     * Asynchronously remove files and directories at `path`.
     *
     * @param path Target path or `file://` URL.
     * @param options Set `recursive: true` to delete directories and `force: true` to ignore missing paths (`ENOENT`).
     * @returns Promise resolving when removal completes.
     * @throws {TypeError} when `path` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"` (unless `force: true`), `"EISDIR"`, `"ENOTEMPTY"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    rm(path: PathInput, options?: { recursive?: boolean; force?: boolean }): Promise<void>;
    /**
     * Asynchronously remove a file or symbolic link at `path`.
     *
     * @param path File or symlink path or `file://` URL.
     * @returns Promise resolving when the entry is removed.
     * @throws {TypeError} when `path` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"`, `"EISDIR"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    unlink(path: PathInput): Promise<void>;
    /**
     * Asynchronously rename `oldPath` to `newPath`.
     *
     * @param oldPath Existing source path or `file://` URL.
     * @param newPath Destination path or `file://` URL.
     * @returns Promise resolving when the rename completes.
     * @throws {TypeError} when `oldPath` or `newPath` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    rename(oldPath: PathInput, newPath: PathInput): Promise<void>;
    /**
     * Asynchronously copy `src` to `dest`.
     *
     * @param src Source file path or `file://` URL.
     * @param dest Destination file path or `file://` URL.
     * @param mode Optional bitmask (pass `constants.COPYFILE_EXCL` to fail with `EEXIST` if `dest` exists).
     * @returns Promise resolving when the copy completes.
     * @throws {TypeError} when `src` or `dest` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"`, `"EEXIST"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    copyFile(src: PathInput, dest: PathInput, mode?: number): Promise<void>;
    /**
     * Asynchronously compute the canonical pathname for `path`.
     *
     * @param path Path or `file://` URL to resolve.
     * @returns Promise resolving to the canonicalized path string.
     * @throws {TypeError} when `path` has an invalid type.
     * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    realpath(path: PathInput): Promise<string>;
    /**
     * Asynchronously create a unique temporary directory starting with `prefix`.
     *
     * Appends 6 random alphanumeric characters to `prefix` and creates the
     * directory (mode `0o700` on Unix). The parent directory must already exist.
     *
     * @param prefix Path prefix for the new temporary directory.
     * @returns Promise resolving to the path of the created directory.
     * @throws {TypeError} when `prefix` is not a string.
     * @throws {Error} with `code` `"EINVAL"` (empty/NUL prefix), `"ENOENT"`, `"EACCES"`, or `"EIO"`.
     * @since 0.0.0-alpha.0
     */
    mkdtemp(prefix: string): Promise<string>;
    /**
     * Filesystem access and copy mode constants.
     *
     * @since 0.0.0-alpha.0
     */
    readonly constants: FsConstants;
  }

  /**
   * Synchronously read the entire contents of `path` as bytes.
   *
   * @param path Filesystem path or `file://` URL.
   * @param options Optional object with `encoding: null | undefined` (or `null`).
   * @returns `Buffer` (`Uint8Array` subclass) containing the file bytes.
   * @throws {TypeError} on invalid `path` or `options` types.
   * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EISDIR"`, `"EINVAL"`, `"ERR_FS_FILE_TOO_LARGE"`, or `"EIO"`.
   * @example
   * ```ts
   * import { readFileSync } from "node:fs";
   *
   * const raw = readFileSync("/etc/hosts");
   * console.log(raw.byteLength);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function readFileSync(path: PathInput, options?: { encoding?: null | undefined } | null): Uint8Array;
  /**
   * Synchronously read the entire contents of `path` as an encoded string.
   *
   * @param path Filesystem path or `file://` URL.
   * @param options Encoding name or `{ encoding }` object.
   * @returns Decoded string contents of the file.
   * @throws {TypeError} on invalid argument types or unknown encoding (`ERR_UNKNOWN_ENCODING`).
   * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EISDIR"`, `"EINVAL"`, `"ERR_FS_FILE_TOO_LARGE"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function readFileSync(path: PathInput, options: TextEncoding | { encoding: TextEncoding }): string;
  /**
   * Synchronously write `data` to `path`, replacing the file if it already exists.
   *
   * @param path Destination filesystem path or `file://` URL.
   * @param data String or byte view to write.
   * @param options Encoding for string `data` (default `"utf8"`) or options object.
   * @throws {TypeError} on invalid argument types or unknown encoding (`ERR_UNKNOWN_ENCODING`).
   * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EISDIR"`, `"EINVAL"`, or `"EIO"`.
   * @example
   * ```ts
   * import { readFileSync, writeFileSync } from "node:fs";
   *
   * writeFileSync("/tmp/vael-note.txt", "saved", "utf8");
   * console.log(readFileSync("/tmp/vael-note.txt", "utf8"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function writeFileSync(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): void;
  /**
   * Synchronously append `data` to `path`, creating the file if it does not exist.
   *
   * @param path Destination filesystem path or `file://` URL.
   * @param data String or byte view to append.
   * @param options Encoding for string `data` (default `"utf8"`) or options object.
   * @throws {TypeError} on invalid argument types or unknown encoding (`ERR_UNKNOWN_ENCODING`).
   * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EISDIR"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function appendFileSync(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): void;
  /**
   * Synchronously return `true` if `path` exists.
   *
   * @param path Filesystem path or `file://` URL.
   * @returns `true` if `path` exists, otherwise `false`.
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code = "EINVAL"` if `path` is invalid (for example, contains a NUL byte).
   * @example
   * ```ts
   * import { existsSync } from "node:fs";
   *
   * console.log(existsSync("/tmp"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function existsSync(path: PathInput): boolean;
  /**
   * Synchronously return `Stats` for `path`, following symbolic links.
   *
   * @param path Filesystem path or `file://` URL.
   * @returns `Stats` metadata for the resolved entry.
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @example
   * ```ts
   * import { statSync } from "node:fs";
   *
   * const info = statSync("/tmp");
   * console.log(info.isDirectory(), info.size);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function statSync(path: PathInput): Stats;
  /**
   * Synchronously return `Stats` for `path`, without following symbolic links.
   *
   * @param path Filesystem path or `file://` URL.
   * @returns `Stats` metadata for the entry itself (including symlinks).
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function lstatSync(path: PathInput): Stats;
  /**
   * Synchronously read directory entry names.
   *
   * @param path Directory path or `file://` URL.
   * @param options Optional `{ withFileTypes?: false }`.
   * @returns Array of entry names inside `path`.
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"`, `"ENOTDIR"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @example
   * ```ts
   * import { readdirSync } from "node:fs";
   *
   * const entries = readdirSync("/tmp");
   * console.log(entries.length);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function readdirSync(path: PathInput, options?: { withFileTypes?: false }): string[];
  /**
   * Synchronously read directory entries as `Dirent` objects.
   *
   * @param path Directory path or `file://` URL.
   * @param options `{ withFileTypes: true }`.
   * @returns Array of `Dirent` objects inside `path`.
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"`, `"ENOTDIR"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function readdirSync(path: PathInput, options: { withFileTypes: true }): Dirent[];
  /**
   * Synchronously create a directory at `path`.
   *
   * @param path Directory path or `file://` URL.
   * @param options Set `recursive: true` to create missing parent directories and succeed if already present.
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code` `"EEXIST"`, `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function mkdirSync(path: PathInput, options?: { recursive?: boolean }): void;
  /**
   * Synchronously remove files and directories at `path`.
   *
   * @param path Target path or `file://` URL.
   * @param options Set `recursive: true` to remove directory trees and `force: true` to ignore missing paths (`ENOENT`).
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"` (unless `force: true`), `"EISDIR"`, `"ENOTEMPTY"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function rmSync(path: PathInput, options?: { recursive?: boolean; force?: boolean }): void;
  /**
   * Synchronously remove a file or symbolic link at `path`.
   *
   * @param path File or symlink path or `file://` URL.
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"`, `"EISDIR"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function unlinkSync(path: PathInput): void;
  /**
   * Synchronously rename `oldPath` to `newPath`.
   *
   * @param oldPath Existing source path or `file://` URL.
   * @param newPath Destination path or `file://` URL.
   * @throws {TypeError} when `oldPath` or `newPath` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function renameSync(oldPath: PathInput, newPath: PathInput): void;
  /**
   * Synchronously copy `src` to `dest`.
   *
   * @param src Source file path or `file://` URL.
   * @param dest Destination file path or `file://` URL.
   * @param mode Optional bitmask (pass `constants.COPYFILE_EXCL` to fail with `EEXIST` if `dest` exists).
   * @throws {TypeError} when `src` or `dest` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"`, `"EEXIST"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function copyFileSync(src: PathInput, dest: PathInput, mode?: number): void;
  /**
   * Synchronously compute the canonical pathname for `path`.
   *
   * @param path Path or `file://` URL to canonicalize.
   * @returns Resolved canonical path string.
   * @throws {TypeError} when `path` has an invalid type.
   * @throws {Error} with `code` `"ENOENT"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function realpathSync(path: PathInput): string;
  /**
   * Synchronously create a unique temporary directory starting with `prefix`.
   *
   * Appends 6 random alphanumeric characters to `prefix` and creates the
   * directory (mode `0o700` on Unix). The parent directory must already exist.
   *
   * @param prefix Path prefix for the new temporary directory.
   * @returns Path of the created temporary directory.
   * @throws {TypeError} when `prefix` is not a string.
   * @throws {Error} with `code` `"EINVAL"` (empty/NUL prefix), `"ENOENT"`, `"EACCES"`, or `"EIO"`.
   * @example
   * ```ts
   * import { mkdtempSync, rmSync } from "node:fs";
   *
   * const dir = mkdtempSync("/tmp/vael-");
   * console.log(dir);
   * rmSync(dir, { recursive: true, force: true });
   * ```
   * @since 0.0.0-alpha.0
   */
  export function mkdtempSync(prefix: string): string;
  /**
   * Filesystem access and copy mode constants.
   *
   * @since 0.0.0-alpha.0
   */
  export const constants: FsConstants;
  /**
   * Promise-based filesystem API (`node:fs/promises`).
   *
   * @since 0.0.0-alpha.0
   */
  export const promises: FsPromises;

  /**
   * Default export mirroring `node:fs` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    /** Synchronously read file contents. @since 0.0.0-alpha.0 */
    readFileSync: typeof readFileSync;
    /** Synchronously write file contents. @since 0.0.0-alpha.0 */
    writeFileSync: typeof writeFileSync;
    /** Synchronously append file contents. @since 0.0.0-alpha.0 */
    appendFileSync: typeof appendFileSync;
    /** Synchronously test whether a path exists. @since 0.0.0-alpha.0 */
    existsSync: typeof existsSync;
    /** Synchronously stat a path, following symlinks. @since 0.0.0-alpha.0 */
    statSync: typeof statSync;
    /** Synchronously stat a path without following symlinks. @since 0.0.0-alpha.0 */
    lstatSync: typeof lstatSync;
    /** Synchronously read directory entries. @since 0.0.0-alpha.0 */
    readdirSync: typeof readdirSync;
    /** Synchronously create a directory. @since 0.0.0-alpha.0 */
    mkdirSync: typeof mkdirSync;
    /** Synchronously remove files or directories. @since 0.0.0-alpha.0 */
    rmSync: typeof rmSync;
    /** Synchronously remove a file or symlink. @since 0.0.0-alpha.0 */
    unlinkSync: typeof unlinkSync;
    /** Synchronously rename a file or directory. @since 0.0.0-alpha.0 */
    renameSync: typeof renameSync;
    /** Synchronously copy a file. @since 0.0.0-alpha.0 */
    copyFileSync: typeof copyFileSync;
    /** Synchronously canonicalize a path. @since 0.0.0-alpha.0 */
    realpathSync: typeof realpathSync;
    /** Synchronously create a unique temporary directory. @since 0.0.0-alpha.0 */
    mkdtempSync: typeof mkdtempSync;
    /** Filesystem access and copy mode constants. @since 0.0.0-alpha.0 */
    readonly constants: FsConstants;
    /** Promise-based filesystem API (`node:fs/promises`). @since 0.0.0-alpha.0 */
    readonly promises: FsPromises;
  };
  /**
   * Default export object providing all `node:fs` functions, `constants`, and `promises`.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}

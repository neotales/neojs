/**
 * Node-compatible promise-based filesystem operations.
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:fs/promises" {
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
   * File metadata returned by `stat` and `lstat`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Stats {
    /** @since 0.0.0-alpha.0 */
    readonly size: number;
    /** @since 0.0.0-alpha.0 */
    readonly mtimeMs: number;
    /** @since 0.0.0-alpha.0 */
    readonly mtime: Date;
    /** Return `true` if the entry is a regular file. @since 0.0.0-alpha.0 */
    isFile(): boolean;
    /** Return `true` if the entry is a directory. @since 0.0.0-alpha.0 */
    isDirectory(): boolean;
    /** Return `true` if the entry is a symbolic link. @since 0.0.0-alpha.0 */
    isSymbolicLink(): boolean;
  }

  /**
   * Directory entry returned by `readdir` with `withFileTypes: true`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Dirent {
    /** @since 0.0.0-alpha.0 */
    readonly name: string;
    /** Return `true` if the entry is a regular file. @since 0.0.0-alpha.0 */
    isFile(): boolean;
    /** Return `true` if the entry is a directory. @since 0.0.0-alpha.0 */
    isDirectory(): boolean;
    /** Return `true` if the entry is a symbolic link. @since 0.0.0-alpha.0 */
    isSymbolicLink(): boolean;
  }

  /**
   * Filesystem access and copy mode constants.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FsConstants {
    /** @since 0.0.0-alpha.0 */
    readonly F_OK: number;
    /** @since 0.0.0-alpha.0 */
    readonly R_OK: number;
    /** @since 0.0.0-alpha.0 */
    readonly W_OK: number;
    /** @since 0.0.0-alpha.0 */
    readonly X_OK: number;
    /** @since 0.0.0-alpha.0 */
    readonly COPYFILE_EXCL: number;
    /** @since 0.0.0-alpha.0 */
    readonly COPYFILE_FICLONE: number;
    /** @since 0.0.0-alpha.0 */
    readonly COPYFILE_FICLONE_FORCE: number;
  }

  /** Asynchronously read the entire contents of `path` as bytes. @since 0.0.0-alpha.0 */
  export function readFile(path: PathInput, options?: { encoding?: null | undefined } | null): Promise<Uint8Array>;
  /** Asynchronously read the entire contents of `path` as an encoded string. @since 0.0.0-alpha.0 */
  export function readFile(path: PathInput, options: TextEncoding | { encoding: TextEncoding }): Promise<string>;
  /** Asynchronously write `data` to `path`, replacing the file if it already exists. @since 0.0.0-alpha.0 */
  export function writeFile(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): Promise<void>;
  /** Asynchronously append `data` to `path`, creating the file if it does not exist. @since 0.0.0-alpha.0 */
  export function appendFile(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): Promise<void>;
  /** Asynchronously return `Stats` for `path`, following symbolic links. @since 0.0.0-alpha.0 */
  export function stat(path: PathInput): Promise<Stats>;
  /** Asynchronously return `Stats` for `path`, without following symbolic links. @since 0.0.0-alpha.0 */
  export function lstat(path: PathInput): Promise<Stats>;
  /** Asynchronously read directory entry names. @since 0.0.0-alpha.0 */
  export function readdir(path: PathInput, options?: { withFileTypes?: false }): Promise<string[]>;
  /** Asynchronously read directory entries as `Dirent` objects. @since 0.0.0-alpha.0 */
  export function readdir(path: PathInput, options: { withFileTypes: true }): Promise<Dirent[]>;
  /** Asynchronously create a directory at `path`. @since 0.0.0-alpha.0 */
  export function mkdir(path: PathInput, options?: { recursive?: boolean }): Promise<void>;
  /** Asynchronously remove files and directories at `path`. @since 0.0.0-alpha.0 */
  export function rm(path: PathInput, options?: { recursive?: boolean; force?: boolean }): Promise<void>;
  /** Asynchronously remove a file or symbolic link at `path`. @since 0.0.0-alpha.0 */
  export function unlink(path: PathInput): Promise<void>;
  /** Asynchronously rename `oldPath` to `newPath`. @since 0.0.0-alpha.0 */
  export function rename(oldPath: PathInput, newPath: PathInput): Promise<void>;
  /** Asynchronously copy `src` to `dest`. @since 0.0.0-alpha.0 */
  export function copyFile(src: PathInput, dest: PathInput, mode?: number): Promise<void>;
  /** Asynchronously compute the canonical pathname for `path`. @since 0.0.0-alpha.0 */
  export function realpath(path: PathInput): Promise<string>;
  /** Asynchronously create a unique temporary directory starting with `prefix`. @since 0.0.0-alpha.0 */
  export function mkdtemp(prefix: string): Promise<string>;
  /** Filesystem access and copy mode constants. @since 0.0.0-alpha.0 */
  export const constants: FsConstants;

  /**
   * Default export mirroring `node:fs/promises` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    readFile: typeof readFile;
    writeFile: typeof writeFile;
    appendFile: typeof appendFile;
    stat: typeof stat;
    lstat: typeof lstat;
    readdir: typeof readdir;
    mkdir: typeof mkdir;
    rm: typeof rm;
    unlink: typeof unlink;
    rename: typeof rename;
    copyFile: typeof copyFile;
    realpath: typeof realpath;
    mkdtemp: typeof mkdtemp;
    readonly constants: FsConstants;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

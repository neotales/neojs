/**
 * Node-compatible synchronous filesystem operations (`vaeljs-lite` profile).
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
   * File metadata returned by `statSync` and `lstatSync`.
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
   * Directory entry returned by `readdirSync` with `withFileTypes: true`.
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

  /** Synchronously read the entire contents of `path` as bytes. @since 0.0.0-alpha.0 */
  export function readFileSync(path: PathInput, options?: { encoding?: null | undefined } | null): Uint8Array;
  /** Synchronously read the entire contents of `path` as an encoded string. @since 0.0.0-alpha.0 */
  export function readFileSync(path: PathInput, options: TextEncoding | { encoding: TextEncoding }): string;
  /** Synchronously write `data` to `path`, replacing the file if it already exists. @since 0.0.0-alpha.0 */
  export function writeFileSync(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): void;
  /** Synchronously append `data` to `path`, creating the file if it does not exist. @since 0.0.0-alpha.0 */
  export function appendFileSync(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): void;
  /** Synchronously return `true` if `path` exists. @since 0.0.0-alpha.0 */
  export function existsSync(path: PathInput): boolean;
  /** Synchronously return `Stats` for `path`, following symbolic links. @since 0.0.0-alpha.0 */
  export function statSync(path: PathInput): Stats;
  /** Synchronously return `Stats` for `path`, without following symbolic links. @since 0.0.0-alpha.0 */
  export function lstatSync(path: PathInput): Stats;
  /** Synchronously read directory entry names. @since 0.0.0-alpha.0 */
  export function readdirSync(path: PathInput, options?: { withFileTypes?: false }): string[];
  /** Synchronously read directory entries as `Dirent` objects. @since 0.0.0-alpha.0 */
  export function readdirSync(path: PathInput, options: { withFileTypes: true }): Dirent[];
  /** Synchronously create a directory at `path`. @since 0.0.0-alpha.0 */
  export function mkdirSync(path: PathInput, options?: { recursive?: boolean }): void;
  /** Synchronously remove files and directories at `path`. @since 0.0.0-alpha.0 */
  export function rmSync(path: PathInput, options?: { recursive?: boolean; force?: boolean }): void;
  /** Synchronously remove a file or symbolic link at `path`. @since 0.0.0-alpha.0 */
  export function unlinkSync(path: PathInput): void;
  /** Synchronously rename `oldPath` to `newPath`. @since 0.0.0-alpha.0 */
  export function renameSync(oldPath: PathInput, newPath: PathInput): void;
  /** Synchronously copy `src` to `dest`. @since 0.0.0-alpha.0 */
  export function copyFileSync(src: PathInput, dest: PathInput, mode?: number): void;
  /** Synchronously compute the canonical pathname for `path`. @since 0.0.0-alpha.0 */
  export function realpathSync(path: PathInput): string;
  /** Synchronously create a unique temporary directory starting with `prefix`. @since 0.0.0-alpha.0 */
  export function mkdtempSync(prefix: string): string;
  /** Filesystem access and copy mode constants. @since 0.0.0-alpha.0 */
  export const constants: FsConstants;

  /**
   * Default export mirroring `node:fs` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    readFileSync: typeof readFileSync;
    writeFileSync: typeof writeFileSync;
    appendFileSync: typeof appendFileSync;
    existsSync: typeof existsSync;
    statSync: typeof statSync;
    lstatSync: typeof lstatSync;
    readdirSync: typeof readdirSync;
    mkdirSync: typeof mkdirSync;
    rmSync: typeof rmSync;
    unlinkSync: typeof unlinkSync;
    renameSync: typeof renameSync;
    copyFileSync: typeof copyFileSync;
    realpathSync: typeof realpathSync;
    mkdtempSync: typeof mkdtempSync;
    readonly constants: FsConstants;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

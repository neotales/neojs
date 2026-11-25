/**
 * Lexical POSIX and Windows path manipulation utilities.
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:path" {
  /**
   * Parsed components returned by `parse(path)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface ParsedPath {
    /** Root prefix (`"/"`, `"C:\\"`, `"\\\\server\\share\\"`, or `""`). @since 0.0.0-alpha.0 */
    root: string;
    /** Directory portion without trailing separator (except root). @since 0.0.0-alpha.0 */
    dir: string;
    /** Final path component including extension. @since 0.0.0-alpha.0 */
    base: string;
    /** Extension including leading `"."`, or `""`. @since 0.0.0-alpha.0 */
    ext: string;
    /** File name without extension. @since 0.0.0-alpha.0 */
    name: string;
  }

  /**
   * Input object accepted by `format(pathObject)`.
   *
   * `dir` takes precedence over `root`; `base` takes precedence over `name` + `ext`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FormatInputPathObject {
    /** @since 0.0.0-alpha.0 */
    root?: string;
    /** @since 0.0.0-alpha.0 */
    dir?: string;
    /** @since 0.0.0-alpha.0 */
    base?: string;
    /** @since 0.0.0-alpha.0 */
    ext?: string;
    /** @since 0.0.0-alpha.0 */
    name?: string;
  }

  /**
   * Error codes thrown by `vael:path` on `PathError.code` when resolving against
   * the working directory or invalid Windows roots.
   *
   * @since 0.0.0-alpha.0
   */
  export type PathErrorCode = "PATH_CWD" | "PATH_CWD_DIALECT" | "PATH_WINDOWS_ROOT";

  /**
   * Lexical path operations for a fixed syntax dialect (`posix` or `win32`).
   *
   * All arguments must be strings without NUL (`\0`) characters; performs no
   * filesystem I/O (only `resolve`, `relative`, and Windows `toNamespacedPath`
   * read the working directory when inputs lack an absolute base).
   *
   * @since 0.0.0-alpha.0
   */
  export interface PathDialect {
    /** Path component separator (`"/"` for `posix`, `"\\"` for `win32`). @since 0.0.0-alpha.0 */
    readonly sep: string;
    /** Path list delimiter (`":"` for `posix`, `";"` for `win32`). @since 0.0.0-alpha.0 */
    readonly delimiter: string;
    /** Join and normalize path segments using this dialect. @since 0.0.0-alpha.0 */
    join(...paths: string[]): string;
    /** Normalize `.` and `..` segments and separators using this dialect. @since 0.0.0-alpha.0 */
    normalize(path: string): string;
    /** Return the final path component, optionally stripping `suffix`. @since 0.0.0-alpha.0 */
    basename(path: string, suffix?: string): string;
    /** Return the directory portion of `path`. @since 0.0.0-alpha.0 */
    dirname(path: string): string;
    /** Return the extension (including leading `"."`) of the last component, or `""`. @since 0.0.0-alpha.0 */
    extname(path: string): string;
    /** Return `true` if `path` is absolute in this dialect. @since 0.0.0-alpha.0 */
    isAbsolute(path: string): boolean;
    /** Split `path` into `{ root, dir, base, ext, name }`. @since 0.0.0-alpha.0 */
    parse(path: string): ParsedPath;
    /** Format a `{ root, dir, base, ext, name }` object into a path string. @since 0.0.0-alpha.0 */
    format(pathObject: FormatInputPathObject): string;
    /** Return the shared prefix at component boundaries across `paths`. @since 0.0.0-alpha.0 */
    common(paths: readonly string[]): string;
    /** Resolve a sequence of paths into an absolute path in this dialect. @since 0.0.0-alpha.0 */
    resolve(...paths: string[]): string;
    /** Compute the relative path from `from` to `to` in this dialect. @since 0.0.0-alpha.0 */
    relative(from: string, to: string): string;
    /** Convert `path` to a Windows `\\?\` namespaced path when applicable. @since 0.0.0-alpha.0 */
    toNamespacedPath(path: string): string;
  }

  /**
   * Frozen POSIX lexical path dialect.
   *
   * @since 0.0.0-alpha.0
   */
  export const posix: PathDialect;

  /**
   * Frozen Windows lexical path dialect.
   *
   * @since 0.0.0-alpha.0
   */
  export const win32: PathDialect;

  /**
   * Join and normalize path segments using the build host's path dialect.
   *
   * @since 0.0.0-alpha.0
   */
  export function join(...paths: string[]): string;

  /**
   * Normalize `.` and `..` segments and separators using the host dialect.
   *
   * @since 0.0.0-alpha.0
   */
  export function normalize(path: string): string;

  /**
   * Return the final path component, optionally stripping `suffix`.
   *
   * @since 0.0.0-alpha.0
   */
  export function basename(path: string, suffix?: string): string;

  /**
   * Return the directory portion of `path`.
   *
   * @since 0.0.0-alpha.0
   */
  export function dirname(path: string): string;

  /**
   * Return the extension (including leading `"."`) of the last component, or `""`.
   *
   * @since 0.0.0-alpha.0
   */
  export function extname(path: string): string;

  /**
   * Return `true` if `path` is absolute in the host dialect.
   *
   * @since 0.0.0-alpha.0
   */
  export function isAbsolute(path: string): boolean;

  /**
   * Split `path` into `{ root, dir, base, ext, name }`.
   *
   * @since 0.0.0-alpha.0
   */
  export function parse(path: string): ParsedPath;

  /**
   * Format a `{ root, dir, base, ext, name }` object into a path string.
   *
   * @since 0.0.0-alpha.0
   */
  export function format(pathObject: FormatInputPathObject): string;

  /**
   * Return the shared prefix at component boundaries across `paths` without
   * normalizing or folding case.
   *
   * @since 0.0.0-alpha.0
   */
  export function common(paths: readonly string[]): string;

  /**
   * Resolve a sequence of paths into an absolute path using the host dialect.
   *
   * Reads the working directory at most once if inputs lack an absolute base.
   *
   * @throws {Error} with `code: PathErrorCode` if the working directory is unavailable.
   * @since 0.0.0-alpha.0
   */
  export function resolve(...paths: string[]): string;

  /**
   * Compute the relative path from `from` to `to` in the host dialect.
   *
   * @throws {Error} with `code: PathErrorCode` if the working directory is unavailable.
   * @since 0.0.0-alpha.0
   */
  export function relative(from: string, to: string): string;

  /**
   * Convert `path` to a Windows `\\?\` namespaced path on Windows, or return `path` unchanged on POSIX.
   *
   * @since 0.0.0-alpha.0
   */
  export function toNamespacedPath(path: string): string;

  /**
   * Host path component separator (`"/"` on POSIX, `"\\"` on Windows).
   *
   * @since 0.0.0-alpha.0
   */
  export const SEPARATOR: "/" | "\\";

  /**
   * Host path list delimiter (`":"` on POSIX, `";"` on Windows).
   *
   * @since 0.0.0-alpha.0
   */
  export const DELIMITER: ":" | ";";

  /**
   * Host path separator pattern (`/\/+/` on POSIX, `/[\\/]+/` on Windows).
   *
   * @since 0.0.0-alpha.0
   */
  export const SEPARATOR_PATTERN: RegExp;
}

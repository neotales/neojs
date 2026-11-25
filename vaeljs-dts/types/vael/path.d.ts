declare module "vael:path" {
  /**
   * Parsed components returned by `parse(path)`.
   */
  export interface ParsedPath {
    /** Root prefix (`"/"`, `"C:\\"`, `"\\\\server\\share\\"`, or `""`). */
    root: string;
    /** Directory portion without trailing separator (except root). */
    dir: string;
    /** Final path component including extension. */
    base: string;
    /** Extension including leading `"."`, or `""`. */
    ext: string;
    /** File name without extension. */
    name: string;
  }

  /**
   * Input object accepted by `format(pathObject)`.
   *
   * `dir` takes precedence over `root`; `base` takes precedence over `name` + `ext`.
   */
  export interface FormatInputPathObject {
    root?: string;
    dir?: string;
    base?: string;
    ext?: string;
    name?: string;
  }

  /**
   * Error codes thrown by `vael:path` on `PathError.code` when resolving against
   * the working directory or invalid Windows roots.
   */
  export type PathErrorCode = "PATH_CWD" | "PATH_CWD_DIALECT" | "PATH_WINDOWS_ROOT";

  /**
   * Lexical path operations for a fixed syntax dialect (`posix` or `win32`).
   *
   * All arguments must be strings without NUL (`\0`) characters; performs no
   * filesystem I/O (only `resolve`, `relative`, and Windows `toNamespacedPath`
   * read the working directory when inputs lack an absolute base).
   */
  export interface PathDialect {
    /** Path component separator (`"/"` for `posix`, `"\\"` for `win32`). */
    readonly sep: string;
    /** Path list delimiter (`":"` for `posix`, `";"` for `win32`). */
    readonly delimiter: string;
    join(...paths: string[]): string;
    normalize(path: string): string;
    basename(path: string, suffix?: string): string;
    dirname(path: string): string;
    extname(path: string): string;
    isAbsolute(path: string): boolean;
    parse(path: string): ParsedPath;
    format(pathObject: FormatInputPathObject): string;
    common(paths: readonly string[]): string;
    resolve(...paths: string[]): string;
    relative(from: string, to: string): string;
    toNamespacedPath(path: string): string;
  }

  /** Frozen POSIX lexical path dialect. */
  export const posix: PathDialect;

  /** Frozen Windows lexical path dialect. */
  export const win32: PathDialect;

  /** Join and normalize path segments using the build host's path dialect. */
  export function join(...paths: string[]): string;

  /** Normalize `.` and `..` segments and separators using the host dialect. */
  export function normalize(path: string): string;

  /** Return the final path component, optionally stripping `suffix`. */
  export function basename(path: string, suffix?: string): string;

  /** Return the directory portion of `path`. */
  export function dirname(path: string): string;

  /** Return the extension (including leading `"."`) of the last component, or `""`. */
  export function extname(path: string): string;

  /** Return `true` if `path` is absolute in the host dialect. */
  export function isAbsolute(path: string): boolean;

  /** Split `path` into `{ root, dir, base, ext, name }`. */
  export function parse(path: string): ParsedPath;

  /** Format a `{ root, dir, base, ext, name }` object into a path string. */
  export function format(pathObject: FormatInputPathObject): string;

  /**
   * Return the shared prefix at component boundaries across `paths` without
   * normalizing or folding case.
   */
  export function common(paths: readonly string[]): string;

  /**
   * Resolve a sequence of paths into an absolute path using the host dialect.
   *
   * Reads the working directory at most once if inputs lack an absolute base.
   *
   * @throws {Error} with `code: PathErrorCode` if the working directory is unavailable.
   */
  export function resolve(...paths: string[]): string;

  /**
   * Compute the relative path from `from` to `to` in the host dialect.
   *
   * @throws {Error} with `code: PathErrorCode` if the working directory is unavailable.
   */
  export function relative(from: string, to: string): string;

  /** Convert `path` to a Windows `\\?\` namespaced path on Windows, or return `path` unchanged on POSIX. */
  export function toNamespacedPath(path: string): string;

  /** Host path component separator (`"/"` on POSIX, `"\\"` on Windows). */
  export const SEPARATOR: "/" | "\\";

  /** Host path list delimiter (`":"` on POSIX, `";"` on Windows). */
  export const DELIMITER: ":" | ";";

  /** Host path separator pattern (`/\/+/` on POSIX, `/[\\/]+/` on Windows). */
  export const SEPARATOR_PATTERN: RegExp;
}

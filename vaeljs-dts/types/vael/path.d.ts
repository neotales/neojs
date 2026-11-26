/**
 * Lexical POSIX and Windows path manipulation utilities.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Provides host-dialect top-level helpers as well as explicit `posix` and
 * `win32` dialects without touching the filesystem (except `resolve`,
 * `relative`, and Windows `toNamespacedPath` when reading the working directory).
 *
 * @example
 * ```ts
 * import { basename, dirname, extname, join, posix } from "vael:path";
 *
 * const full = join("src", "app", "main.ts");
 * console.log(dirname(full), basename(full), extname(full));
 * console.log(posix.join("/etc", "nginx", "..", "hosts")); // "/etc/hosts"
 * ```
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
    /** Root prefix used when `dir` is omitted. @since 0.0.0-alpha.0 */
    root?: string;
    /** Directory path (takes precedence over `root`). @since 0.0.0-alpha.0 */
    dir?: string;
    /** File base name including extension (takes precedence over `name` + `ext`). @since 0.0.0-alpha.0 */
    base?: string;
    /** File extension including leading `"."`. @since 0.0.0-alpha.0 */
    ext?: string;
    /** File name without extension. @since 0.0.0-alpha.0 */
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

    /**
     * Join and normalize path segments using this dialect.
     *
     * @param paths Path segments to join.
     * @returns Normalized joined path string.
     * @throws {TypeError} when any segment is not a string or contains NUL.
     * @since 0.0.0-alpha.0
     */
    join(...paths: string[]): string;

    /**
     * Normalize `.` and `..` segments and separators using this dialect.
     *
     * @param path Path string to normalize.
     * @returns Normalized path string.
     * @throws {TypeError} when `path` is not a string or contains NUL.
     * @since 0.0.0-alpha.0
     */
    normalize(path: string): string;

    /**
     * Return the final path component, optionally stripping `suffix`.
     *
     * @param path Path string.
     * @param suffix Optional trailing suffix to strip when matched.
     * @returns Final path component.
     * @throws {TypeError} on non-string arguments or NUL bytes.
     * @since 0.0.0-alpha.0
     */
    basename(path: string, suffix?: string): string;

    /**
     * Return the directory portion of `path`.
     *
     * @param path Path string.
     * @returns Directory portion of `path`.
     * @throws {TypeError} on non-string `path` or NUL bytes.
     * @since 0.0.0-alpha.0
     */
    dirname(path: string): string;

    /**
     * Return the extension (including leading `"."`) of the last component, or `""`.
     *
     * @param path Path string.
     * @returns Extension string or `""`.
     * @throws {TypeError} on non-string `path` or NUL bytes.
     * @since 0.0.0-alpha.0
     */
    extname(path: string): string;

    /**
     * Return `true` if `path` is absolute in this dialect.
     *
     * @param path Path string to test.
     * @returns `true` when `path` is absolute in this dialect.
     * @throws {TypeError} on non-string `path` or NUL bytes.
     * @since 0.0.0-alpha.0
     */
    isAbsolute(path: string): boolean;

    /**
     * Split `path` into `{ root, dir, base, ext, name }`.
     *
     * @param path Path string to split.
     * @returns `ParsedPath` component record.
     * @throws {TypeError} on non-string `path` or NUL bytes.
     * @since 0.0.0-alpha.0
     */
    parse(path: string): ParsedPath;

    /**
     * Format a `{ root, dir, base, ext, name }` object into a path string.
     *
     * @param pathObject Partial path component object.
     * @returns Formatted path string.
     * @throws {TypeError} on invalid `pathObject` or NUL bytes.
     * @since 0.0.0-alpha.0
     */
    format(pathObject: FormatInputPathObject): string;

    /**
     * Return the shared prefix at component boundaries across `paths`.
     *
     * @param paths Array of path strings to compare.
     * @returns Common directory prefix string.
     * @throws {TypeError} on invalid array or path strings.
     * @since 0.0.0-alpha.0
     */
    common(paths: readonly string[]): string;

    /**
     * Resolve a sequence of paths into an absolute path in this dialect.
     *
     * @param paths Sequence of path segments resolved right-to-left.
     * @returns Absolute resolved path string.
     * @throws {TypeError} on non-string segments or NUL bytes.
     * @throws {Error} with `code: PathErrorCode` if working directory resolution fails.
     * @since 0.0.0-alpha.0
     */
    resolve(...paths: string[]): string;

    /**
     * Compute the relative path from `from` to `to` in this dialect.
     *
     * @param from Origin path.
     * @param to Destination path.
     * @returns Relative path from `from` to `to`.
     * @throws {TypeError} on non-string arguments or NUL bytes.
     * @throws {Error} with `code: PathErrorCode` if working directory resolution fails.
     * @since 0.0.0-alpha.0
     */
    relative(from: string, to: string): string;

    /**
     * Convert `path` to a Windows `\\?\` namespaced path when applicable.
     *
     * @param path Path string to namespace.
     * @returns Namespaced path (on `win32`) or `path` unchanged (on `posix`).
     * @throws {TypeError} on non-string `path` or NUL bytes.
     * @throws {Error} with `code: PathErrorCode` if working directory resolution fails.
     * @since 0.0.0-alpha.0
     */
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
   * @param paths Path segments to join.
   * @returns Normalized joined path string.
   * @throws {TypeError} when any segment is not a string or contains NUL.
   * @example
   * ```ts
   * import { join } from "vael:path";
   *
   * console.log(join("var", "log", "..", "lib", "app.db"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function join(...paths: string[]): string;

  /**
   * Normalize `.` and `..` segments and separators using the host dialect.
   *
   * @param path Path string to normalize.
   * @returns Normalized path string.
   * @throws {TypeError} when `path` is not a string or contains NUL.
   * @since 0.0.0-alpha.0
   */
  export function normalize(path: string): string;

  /**
   * Return the final path component, optionally stripping `suffix`.
   *
   * @param path Path string.
   * @param suffix Optional trailing suffix to strip when matched.
   * @returns Final path component.
   * @throws {TypeError} on non-string arguments or NUL bytes.
   * @example
   * ```ts
   * import { basename } from "vael:path";
   *
   * console.log(basename("/srv/app/config.json", ".json")); // "config"
   * ```
   * @since 0.0.0-alpha.0
   */
  export function basename(path: string, suffix?: string): string;

  /**
   * Return the directory portion of `path`.
   *
   * @param path Path string.
   * @returns Directory portion of `path`.
   * @throws {TypeError} on non-string `path` or NUL bytes.
   * @since 0.0.0-alpha.0
   */
  export function dirname(path: string): string;

  /**
   * Return the extension (including leading `"."`) of the last component, or `""`.
   *
   * @param path Path string.
   * @returns Extension string or `""`.
   * @throws {TypeError} on non-string `path` or NUL bytes.
   * @since 0.0.0-alpha.0
   */
  export function extname(path: string): string;

  /**
   * Return `true` if `path` is absolute in the host dialect.
   *
   * @param path Path string to test.
   * @returns `true` if `path` is absolute.
   * @throws {TypeError} on non-string `path` or NUL bytes.
   * @since 0.0.0-alpha.0
   */
  export function isAbsolute(path: string): boolean;

  /**
   * Split `path` into `{ root, dir, base, ext, name }`.
   *
   * @param path Path string to parse.
   * @returns `ParsedPath` component record.
   * @throws {TypeError} on non-string `path` or NUL bytes.
   * @example
   * ```ts
   * import { parse } from "vael:path";
   *
   * console.log(parse("/home/user/notes.md"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function parse(path: string): ParsedPath;

  /**
   * Format a `{ root, dir, base, ext, name }` object into a path string.
   *
   * @param pathObject Partial path component object.
   * @returns Formatted path string.
   * @throws {TypeError} on invalid `pathObject` or NUL bytes.
   * @since 0.0.0-alpha.0
   */
  export function format(pathObject: FormatInputPathObject): string;

  /**
   * Return the shared prefix at component boundaries across `paths` without
   * normalizing or folding case.
   *
   * @param paths Array of path strings to compare.
   * @returns Shared component-boundary prefix string.
   * @throws {TypeError} on invalid array or path strings.
   * @since 0.0.0-alpha.0
   */
  export function common(paths: readonly string[]): string;

  /**
   * Resolve a sequence of paths into an absolute path using the host dialect.
   *
   * Reads the working directory at most once if inputs lack an absolute base.
   *
   * @param paths Path segments resolved right-to-left.
   * @returns Absolute resolved path string.
   * @throws {TypeError} on non-string segments or NUL bytes.
   * @throws {Error} with `code: PathErrorCode` if the working directory is unavailable.
   * @example
   * ```ts
   * import { resolve } from "vael:path";
   *
   * console.log(resolve("examples", "webapp", "main.ts"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function resolve(...paths: string[]): string;

  /**
   * Compute the relative path from `from` to `to` in the host dialect.
   *
   * @param from Origin path.
   * @param to Destination path.
   * @returns Relative path string from `from` to `to`.
   * @throws {TypeError} on non-string arguments or NUL bytes.
   * @throws {Error} with `code: PathErrorCode` if the working directory is unavailable.
   * @since 0.0.0-alpha.0
   */
  export function relative(from: string, to: string): string;

  /**
   * Convert `path` to a Windows `\\?\` namespaced path on Windows, or return `path` unchanged on POSIX.
   *
   * @param path Path string to convert.
   * @returns Namespaced path on Windows, or `path` unchanged on POSIX.
   * @throws {TypeError} on non-string `path` or NUL bytes.
   * @throws {Error} with `code: PathErrorCode` if working directory resolution fails.
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

/**
 * Node-compatible filesystem path manipulation utilities.
 *
 * Available in both `vjs` and `vjs-lite` runtimes. Enabled by default unless
 * `--no-node-compat` is passed. Uses the host platform dialect (`posix` on
 * Unix/macOS, `win32` on Windows) while also exposing `.posix` and `.win32`.
 *
 * @example
 * ```ts
 * import { basename, dirname, extname, join, resolve } from "node:path";
 *
 * const full = join("src", "lib", "index.ts");
 * console.log(dirname(full), basename(full, extname(full)));
 * console.log(resolve("."));
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:path" {
  /**
   * Parsed components returned by `parse(path)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface ParsedPath {
    /**
     * Root of the path such as `"/"` or `"C:\\"` (or `""` for relative paths).
     *
     * @since 0.0.0-alpha.0
     */
    root: string;
    /**
     * Directory portion of the path without trailing separator (except root).
     *
     * @since 0.0.0-alpha.0
     */
    dir: string;
    /**
     * Final path component including file extension (for example `"file.txt"`).
     *
     * @since 0.0.0-alpha.0
     */
    base: string;
    /**
     * File extension including leading dot (for example `".txt"`), or `""`.
     *
     * @since 0.0.0-alpha.0
     */
    ext: string;
    /**
     * Final path component without its extension (for example `"file"`).
     *
     * @since 0.0.0-alpha.0
     */
    name: string;
  }

  /**
   * Input object accepted by `format(pathObject)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FormatInputPathObject {
    /**
     * Root prefix used when `dir` is omitted.
     *
     * @since 0.0.0-alpha.0
     */
    root?: string;
    /**
     * Directory portion (takes precedence over `root` when provided).
     *
     * @since 0.0.0-alpha.0
     */
    dir?: string;
    /**
     * Complete filename (takes precedence over `name` and `ext` when provided).
     *
     * @since 0.0.0-alpha.0
     */
    base?: string;
    /**
     * File extension used when `base` is omitted.
     *
     * @since 0.0.0-alpha.0
     */
    ext?: string;
    /**
     * Base filename without extension used when `base` is omitted.
     *
     * @since 0.0.0-alpha.0
     */
    name?: string;
  }

  /**
   * Platform-specific path manipulation interface (`node:path`, `posix`, `win32`).
   *
   * @since 0.0.0-alpha.0
   */
  export interface PlatformPath {
    /**
     * Platform-specific path segment separator (`"/"` on POSIX, `"\\"` on Windows).
     *
     * @since 0.0.0-alpha.0
     */
    readonly sep: string;
    /**
     * Platform-specific `PATH` environment variable delimiter (`":"` on POSIX, `";"` on Windows).
     *
     * @since 0.0.0-alpha.0
     */
    readonly delimiter: string;
    /**
     * Join all given path segments together and normalize the resulting path.
     *
     * @param paths Path segments to join.
     * @returns Normalized joined path string (`"."` when empty).
     * @throws {TypeError} when any segment is not a string.
     * @since 0.0.0-alpha.0
     */
    join(...paths: string[]): string;
    /**
     * Resolve a sequence of paths or path segments into an absolute path.
     *
     * @param paths Sequence of path segments processed from right to left.
     * @returns Resolved absolute path string.
     * @throws {TypeError} when any segment is not a string.
     * @since 0.0.0-alpha.0
     */
    resolve(...paths: string[]): string;
    /**
     * Normalize `path`, resolving `"."` and `".."` segments.
     *
     * @param path Path string to normalize.
     * @returns Normalized path string.
     * @throws {TypeError} when `path` is not a string.
     * @since 0.0.0-alpha.0
     */
    normalize(path: string): string;
    /**
     * Return the relative path from `from` to `to` based on the current working directory.
     *
     * @param from Origin path.
     * @param to Destination path.
     * @returns Relative path string from `from` to `to`.
     * @throws {TypeError} when `from` or `to` is not a string.
     * @since 0.0.0-alpha.0
     */
    relative(from: string, to: string): string;
    /**
     * Return the directory name of `path`.
     *
     * @param path Path string to inspect.
     * @returns Parent directory portion of `path`.
     * @throws {TypeError} when `path` is not a string.
     * @since 0.0.0-alpha.0
     */
    dirname(path: string): string;
    /**
     * Return the last portion of `path`, optionally stripping a trailing `suffix`.
     *
     * @param path Path string to inspect.
     * @param suffix Optional trailing extension or suffix to remove.
     * @returns Final path component.
     * @throws {TypeError} when `path` or `suffix` (when provided) is not a string.
     * @since 0.0.0-alpha.0
     */
    basename(path: string, suffix?: string): string;
    /**
     * Return the extension of `path`, from the last `"."` in the basename to end of string.
     *
     * @param path Path string to inspect.
     * @returns Extension string including leading dot, or `""` if none.
     * @throws {TypeError} when `path` is not a string.
     * @since 0.0.0-alpha.0
     */
    extname(path: string): string;
    /**
     * Parse `path` into `{ root, dir, base, ext, name }`.
     *
     * @param path Path string to parse.
     * @returns Object describing the constituent components of `path`.
     * @throws {TypeError} when `path` is not a string.
     * @since 0.0.0-alpha.0
     */
    parse(path: string): ParsedPath;
    /**
     * Format a `FormatInputPathObject` into a path string.
     *
     * @param pathObject Object specifying path components (`dir`/`root`, `base`/`name`/`ext`).
     * @returns Formatted path string.
     * @throws {TypeError} when `pathObject` is not an object or any present component is not a string.
     * @since 0.0.0-alpha.0
     */
    format(pathObject: FormatInputPathObject): string;
    /**
     * Return `true` if `path` is an absolute path.
     *
     * @param path Path string to test.
     * @returns `true` if `path` is absolute, otherwise `false`.
     * @throws {TypeError} when `path` is not a string.
     * @since 0.0.0-alpha.0
     */
    isAbsolute(path: string): boolean;
    /**
     * POSIX-specific implementation of `PlatformPath`.
     *
     * @since 0.0.0-alpha.0
     */
    readonly posix: PlatformPath;
    /**
     * Windows-specific implementation of `PlatformPath`.
     *
     * @since 0.0.0-alpha.0
     */
    readonly win32: PlatformPath;
  }

  /**
   * Platform-specific path segment separator (`"/"` on POSIX, `"\\"` on Windows).
   *
   * @since 0.0.0-alpha.0
   */
  export const sep: string;
  /**
   * Platform-specific path list delimiter (`":"` on POSIX, `";"` on Windows).
   *
   * @since 0.0.0-alpha.0
   */
  export const delimiter: string;
  /**
   * Join path segments and normalize the result.
   *
   * @param paths Path segments to join.
   * @returns Normalized joined path string.
   * @throws {TypeError} when any segment is not a string.
   * @example
   * ```ts
   * import { join } from "node:path";
   *
   * console.log(join("/var", "log", "..", "tmp", "app.log"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function join(...paths: string[]): string;
  /**
   * Resolve a sequence of paths into an absolute path.
   *
   * @param paths Sequence of path segments processed from right to left.
   * @returns Resolved absolute path string.
   * @throws {TypeError} when any segment is not a string.
   * @example
   * ```ts
   * import { resolve } from "node:path";
   *
   * console.log(resolve("src", "index.ts"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function resolve(...paths: string[]): string;
  /**
   * Normalize a path, resolving `.` and `..` segments.
   *
   * @param path Path string to normalize.
   * @returns Normalized path string.
   * @throws {TypeError} when `path` is not a string.
   * @since 0.0.0-alpha.0
   */
  export function normalize(path: string): string;
  /**
   * Solve the relative path from `from` to `to`.
   *
   * @param from Origin path.
   * @param to Destination path.
   * @returns Relative path string from `from` to `to`.
   * @throws {TypeError} when `from` or `to` is not a string.
   * @since 0.0.0-alpha.0
   */
  export function relative(from: string, to: string): string;
  /**
   * Return the directory name of `path`.
   *
   * @param path Path string to inspect.
   * @returns Parent directory portion of `path`.
   * @throws {TypeError} when `path` is not a string.
   * @since 0.0.0-alpha.0
   */
  export function dirname(path: string): string;
  /**
   * Return the last portion of `path`, optionally stripping `suffix`.
   *
   * @param path Path string to inspect.
   * @param suffix Optional trailing suffix to strip when matched.
   * @returns Final path component.
   * @throws {TypeError} when `path` or `suffix` (when provided) is not a string.
   * @since 0.0.0-alpha.0
   */
  export function basename(path: string, suffix?: string): string;
  /**
   * Return the extension of `path`.
   *
   * @param path Path string to inspect.
   * @returns Extension including leading dot, or `""` if none.
   * @throws {TypeError} when `path` is not a string.
   * @since 0.0.0-alpha.0
   */
  export function extname(path: string): string;
  /**
   * Parse `path` into `{ root, dir, base, ext, name }`.
   *
   * @param path Path string to parse.
   * @returns Parsed path components.
   * @throws {TypeError} when `path` is not a string.
   * @example
   * ```ts
   * import { parse } from "node:path";
   *
   * console.log(parse("/home/user/report.pdf"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function parse(path: string): ParsedPath;
  /**
   * Format a `FormatInputPathObject` into a path string.
   *
   * @param pathObject Object specifying path components.
   * @returns Formatted path string.
   * @throws {TypeError} when `pathObject` is not an object or any component is not a string.
   * @since 0.0.0-alpha.0
   */
  export function format(pathObject: FormatInputPathObject): string;
  /**
   * Return `true` if `path` is an absolute path.
   *
   * @param path Path string to test.
   * @returns `true` if `path` is absolute, otherwise `false`.
   * @throws {TypeError} when `path` is not a string.
   * @since 0.0.0-alpha.0
   */
  export function isAbsolute(path: string): boolean;
  /**
   * POSIX path utilities.
   *
   * @since 0.0.0-alpha.0
   */
  export const posix: PlatformPath;
  /**
   * Windows path utilities.
   *
   * @since 0.0.0-alpha.0
   */
  export const win32: PlatformPath;

  /**
   * Default export mirroring `node:path` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: PlatformPath;
  /**
   * Default export object providing the host `PlatformPath` API.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}

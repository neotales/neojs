/**
 * Node-compatible POSIX path manipulation utilities (`node:path/posix`).
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:path/posix" {
  /**
   * Parsed components returned by `parse(path)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface ParsedPath {
    /** @since 0.0.0-alpha.0 */
    root: string;
    /** @since 0.0.0-alpha.0 */
    dir: string;
    /** @since 0.0.0-alpha.0 */
    base: string;
    /** @since 0.0.0-alpha.0 */
    ext: string;
    /** @since 0.0.0-alpha.0 */
    name: string;
  }

  /**
   * Input object accepted by `format(pathObject)`.
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
   * Platform-specific path manipulation interface (`node:path`, `posix`, `win32`).
   *
   * @since 0.0.0-alpha.0
   */
  export interface PlatformPath {
    /** @since 0.0.0-alpha.0 */
    readonly sep: string;
    /** @since 0.0.0-alpha.0 */
    readonly delimiter: string;
    /** @since 0.0.0-alpha.0 */
    join(...paths: string[]): string;
    /** @since 0.0.0-alpha.0 */
    resolve(...paths: string[]): string;
    /** @since 0.0.0-alpha.0 */
    normalize(path: string): string;
    /** @since 0.0.0-alpha.0 */
    relative(from: string, to: string): string;
    /** @since 0.0.0-alpha.0 */
    dirname(path: string): string;
    /** @since 0.0.0-alpha.0 */
    basename(path: string, suffix?: string): string;
    /** @since 0.0.0-alpha.0 */
    extname(path: string): string;
    /** @since 0.0.0-alpha.0 */
    parse(path: string): ParsedPath;
    /** @since 0.0.0-alpha.0 */
    format(pathObject: FormatInputPathObject): string;
    /** @since 0.0.0-alpha.0 */
    isAbsolute(path: string): boolean;
    /** @since 0.0.0-alpha.0 */
    readonly posix: PlatformPath;
    /** @since 0.0.0-alpha.0 */
    readonly win32: PlatformPath;
  }

  /** POSIX path segment separator (`"/"`). @since 0.0.0-alpha.0 */
  export const sep: "/";
  /** POSIX path list delimiter (`":"`). @since 0.0.0-alpha.0 */
  export const delimiter: ":";
  /** Join path segments and normalize the result using POSIX rules. @since 0.0.0-alpha.0 */
  export function join(...paths: string[]): string;
  /** Resolve a sequence of paths into an absolute POSIX path. @since 0.0.0-alpha.0 */
  export function resolve(...paths: string[]): string;
  /** Normalize a POSIX path, resolving `.` and `..` segments. @since 0.0.0-alpha.0 */
  export function normalize(path: string): string;
  /** Solve the relative POSIX path from `from` to `to`. @since 0.0.0-alpha.0 */
  export function relative(from: string, to: string): string;
  /** Return the directory name of `path` using POSIX rules. @since 0.0.0-alpha.0 */
  export function dirname(path: string): string;
  /** Return the last portion of `path`, optionally stripping `suffix`. @since 0.0.0-alpha.0 */
  export function basename(path: string, suffix?: string): string;
  /** Return the extension of `path`. @since 0.0.0-alpha.0 */
  export function extname(path: string): string;
  /** Parse `path` into `{ root, dir, base, ext, name }` using POSIX rules. @since 0.0.0-alpha.0 */
  export function parse(path: string): ParsedPath;
  /** Format a `FormatInputPathObject` into a POSIX path string. @since 0.0.0-alpha.0 */
  export function format(pathObject: FormatInputPathObject): string;
  /** Return `true` if `path` is an absolute POSIX path. @since 0.0.0-alpha.0 */
  export function isAbsolute(path: string): boolean;
  /** POSIX path utilities. @since 0.0.0-alpha.0 */
  export const posix: PlatformPath;
  /** Windows path utilities. @since 0.0.0-alpha.0 */
  export const win32: PlatformPath;

  /**
   * Default export mirroring `node:path/posix` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: PlatformPath;
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

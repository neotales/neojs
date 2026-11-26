/**
 * Node-compatible process metadata, environment variables, working directory, and exit control.
 *
 * Available in both `vjs` and `vjs-lite` runtimes. Enabled by default unless
 * `--no-node-compat` is passed.
 *
 * @example
 * ```ts
 * import process, { cwd, env, hrtime, platform } from "node:process";
 *
 * const start = hrtime.bigint();
 * console.log(platform, cwd(), env.HOME ?? "");
 * const elapsedNs = hrtime.bigint() - start;
 * console.log(`Elapsed ns: ${elapsedNs}`);
 * process.exitCode = 0;
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:process" {
  /**
   * High-resolution real-time clock helper (`process.hrtime`).
   *
   * @since 0.0.0-alpha.0
   */
  export interface HrTime {
    /**
     * Return `[seconds, nanoseconds]` tuple relative to an arbitrary time or `time`.
     *
     * @param time Optional previous `[seconds, nanoseconds]` tuple to compute a diff against.
     * @returns Two-element `[seconds, nanoseconds]` tuple.
     * @throws {TypeError} when `time` is supplied and is not a two-element `[number, number]` array.
     * @example
     * ```ts
     * import { hrtime } from "node:process";
     *
     * const start = hrtime();
     * const diff = hrtime(start);
     * console.log(diff[0], diff[1]);
     * ```
     * @since 0.0.0-alpha.0
     */
    (time?: [number, number]): [number, number];
    /**
     * Return current high-resolution time in nanoseconds as a `bigint`.
     *
     * @returns Monotonic clock reading in nanoseconds.
     * @example
     * ```ts
     * import { hrtime } from "node:process";
     *
     * const t0 = hrtime.bigint();
     * const t1 = hrtime.bigint();
     * console.log(t1 - t0);
     * ```
     * @since 0.0.0-alpha.0
     */
    bigint(): bigint;
  }

  /**
   * Command-line arguments passed to the runtime.
   *
   * @since 0.0.0-alpha.0
   */
  export const argv: readonly string[];
  /**
   * Return the current working directory.
   *
   * @returns Current working directory path string.
   * @throws {Error} with `code = "EIO"` or `"EINVAL"` if the working directory cannot be queried or is not valid UTF-8.
   * @example
   * ```ts
   * import { cwd } from "node:process";
   *
   * console.log(cwd());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function cwd(): string;
  /**
   * Change the current working directory to `directory`.
   *
   * @param directory Target directory path.
   * @throws {TypeError} when `directory` is not a string.
   * @throws {Error} with `code` `"ENOENT"`, `"ENOTDIR"`, `"EACCES"`, `"EINVAL"`, or `"EIO"`.
   * @since 0.0.0-alpha.0
   */
  export function chdir(directory: string): void;
  /**
   * Live environment variable map.
   *
   * Supports reading, setting (values are coerced to strings), checking with
   * `in`, enumerating keys, and deleting entries within the runtime context.
   *
   * @since 0.0.0-alpha.0
   */
  export const env: Record<string, string | undefined>;
  /**
   * Terminate the process synchronously with `code`.
   *
   * Uses `process.exitCode` (default `0`) when `code` is `undefined`. Never returns.
   *
   * @param code Optional integer exit status code.
   * @throws {TypeError} when `code` is provided and is not a finite number.
   * @example
   * ```ts
   * import { exit } from "node:process";
   *
   * exit(0);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function exit(code?: number): never;
  /**
   * Exit code to use when the process exits normally.
   *
   * Setting a non-finite number throws a `TypeError`.
   *
   * @since 0.0.0-alpha.0
   */
  export let exitCode: number | undefined;
  /**
   * Operating system platform identifier (such as `"linux"`, `"darwin"`, or `"win32"`).
   *
   * @since 0.0.0-alpha.0
   */
  export const platform: string;
  /**
   * CPU architecture identifier (such as `"x64"` or `"arm64"`).
   *
   * @since 0.0.0-alpha.0
   */
  export const arch: string;
  /**
   * Current process ID.
   *
   * @since 0.0.0-alpha.0
   */
  export const pid: number;
  /**
   * Runtime Node-compatibility version string (for example `"v22.0.0"`).
   *
   * @since 0.0.0-alpha.0
   */
  export const version: string;
  /**
   * Runtime component version map (includes `node` and engine keys such as `vjs`/`v8` or `vaeljs`/`quickjs`).
   *
   * @since 0.0.0-alpha.0
   */
  export const versions: Readonly<Record<string, string>>;
  /**
   * High-resolution timer function with `.bigint()`.
   *
   * @since 0.0.0-alpha.0
   */
  export const hrtime: HrTime;

  /**
   * Default export mirroring `node:process` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    /** Command-line arguments passed to the runtime. @since 0.0.0-alpha.0 */
    readonly argv: typeof argv;
    /** Return the current working directory. @since 0.0.0-alpha.0 */
    cwd: typeof cwd;
    /** Change the current working directory. @since 0.0.0-alpha.0 */
    chdir: typeof chdir;
    /** Live environment variable map. @since 0.0.0-alpha.0 */
    readonly env: typeof env;
    /** Terminate the process synchronously. @since 0.0.0-alpha.0 */
    exit: typeof exit;
    /** Exit code used when the process exits normally. @since 0.0.0-alpha.0 */
    exitCode: number | undefined;
    /** Operating system platform identifier. @since 0.0.0-alpha.0 */
    readonly platform: typeof platform;
    /** CPU architecture identifier. @since 0.0.0-alpha.0 */
    readonly arch: typeof arch;
    /** Current process ID. @since 0.0.0-alpha.0 */
    readonly pid: typeof pid;
    /** Runtime Node-compatibility version string. @since 0.0.0-alpha.0 */
    readonly version: typeof version;
    /** Runtime component version map. @since 0.0.0-alpha.0 */
    readonly versions: typeof versions;
    /** High-resolution timer function with `.bigint()`. @since 0.0.0-alpha.0 */
    readonly hrtime: typeof hrtime;
  };
  /**
   * Default export object providing the `node:process` API.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}

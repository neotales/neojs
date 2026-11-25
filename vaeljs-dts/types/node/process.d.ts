/**
 * Node-compatible process metadata, environment variables, working directory, and exit control.
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
    /** Return `[seconds, nanoseconds]` tuple relative to an arbitrary time or `time`. @since 0.0.0-alpha.0 */
    (time?: [number, number]): [number, number];
    /** Return current high-resolution time in nanoseconds as a `bigint`. @since 0.0.0-alpha.0 */
    bigint(): bigint;
  }

  /** Command-line arguments passed to the runtime. @since 0.0.0-alpha.0 */
  export const argv: readonly string[];
  /** Return the current working directory. @since 0.0.0-alpha.0 */
  export function cwd(): string;
  /** Change the current working directory to `directory`. @since 0.0.0-alpha.0 */
  export function chdir(directory: string): void;
  /** Live environment variable map. @since 0.0.0-alpha.0 */
  export const env: Record<string, string | undefined>;
  /** Terminate the process synchronously with `code`. @since 0.0.0-alpha.0 */
  export function exit(code?: number): never;
  /** Exit code to use when the process exits normally. @since 0.0.0-alpha.0 */
  export let exitCode: number | undefined;
  /** Operating system platform identifier. @since 0.0.0-alpha.0 */
  export const platform: string;
  /** CPU architecture identifier. @since 0.0.0-alpha.0 */
  export const arch: string;
  /** Current process ID. @since 0.0.0-alpha.0 */
  export const pid: number;
  /** Runtime Node-compatibility version string. @since 0.0.0-alpha.0 */
  export const version: string;
  /** Runtime component version map. @since 0.0.0-alpha.0 */
  export const versions: Readonly<Record<string, string>>;
  /** High-resolution timer function with `.bigint()`. @since 0.0.0-alpha.0 */
  export const hrtime: HrTime;

  /**
   * Default export mirroring `node:process` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    readonly argv: typeof argv;
    cwd: typeof cwd;
    chdir: typeof chdir;
    readonly env: typeof env;
    exit: typeof exit;
    exitCode: number | undefined;
    readonly platform: typeof platform;
    readonly arch: typeof arch;
    readonly pid: typeof pid;
    readonly version: typeof version;
    readonly versions: typeof versions;
    readonly hrtime: typeof hrtime;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

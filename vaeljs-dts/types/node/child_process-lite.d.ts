/**
 * Node-compatible synchronous child process execution without a shell (`vaeljs-lite` profile).
 *
 * Available in `vjs-lite` (synchronous `spawnSync` and `execFileSync` only; async
 * `execFile` is available in `vjs` only). Enabled by default unless
 * `--no-node-compat` is passed.
 *
 * For safety, executables are invoked directly without spawning a shell;
 * passing `shell: true` fails with `ERR_UNSUPPORTED`.
 *
 * @example
 * ```ts
 * import { execFileSync, spawnSync } from "node:child_process";
 *
 * const res = spawnSync("echo", ["hello"], { encoding: "utf8" });
 * console.log(res.status, res.stdout.trim());
 *
 * const out = execFileSync("echo", ["done"], { encoding: "utf8" });
 * console.log(out.trim());
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:child_process" {
  /**
   * Supported text encodings for child process output.
   *
   * @since 0.0.0-alpha.0
   */
  export type TextEncoding = "utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii";

  /**
   * Options controlling working directory, environment, stdin, encoding, and limits.
   *
   * @since 0.0.0-alpha.0
   */
  export interface ChildProcessOptions {
    /**
     * Working directory for the child process (inherits parent cwd when omitted).
     *
     * @since 0.0.0-alpha.0
     */
    cwd?: string;
    /**
     * Environment key/value map applied to the child process (max 4096 entries).
     *
     * When provided, replaces the inherited environment with the specified entries.
     *
     * @since 0.0.0-alpha.0
     */
    env?: Record<string, string | undefined>;
    /**
     * Data written to the child process standard input before closing stdin.
     *
     * @since 0.0.0-alpha.0
     */
    input?: string | Uint8Array | ArrayBufferView;
    /**
     * Output encoding for `stdout` and `stderr` (`"buffer"` or `null` returns `Buffer` / `Uint8Array` bytes).
     *
     * @since 0.0.0-alpha.0
     */
    encoding?: TextEncoding | "buffer" | null;
    /**
     * Maximum execution duration in milliseconds before killing and reaping the child process.
     *
     * @since 0.0.0-alpha.0
     */
    timeout?: number;
    /**
     * Maximum bytes captured per output stream (`stdout` and `stderr`, default 10 MiB).
     *
     * @since 0.0.0-alpha.0
     */
    maxBuffer?: number;
  }

  /**
   * Result returned by `spawnSync`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface SpawnSyncReturns<T = Uint8Array | string> {
    /**
     * Process exit code, or `null` if terminated by a signal or spawn failed.
     *
     * @since 0.0.0-alpha.0
     */
    status: number | null;
    /**
     * Signal name (such as `"SIGKILL"`) if terminated by a signal, otherwise `null`.
     *
     * @since 0.0.0-alpha.0
     */
    signal: string | null;
    /**
     * Captured standard output (decoded string or `Buffer` bytes depending on `options.encoding`).
     *
     * @since 0.0.0-alpha.0
     */
    stdout: T;
    /**
     * Captured standard error (decoded string or `Buffer` bytes depending on `options.encoding`).
     *
     * @since 0.0.0-alpha.0
     */
    stderr: T;
    /**
     * Error object populated when spawning failed or the child process timed out.
     *
     * @since 0.0.0-alpha.0
     */
    error?: Error;
    /**
     * `true` when the child exceeded `options.timeout` and was terminated.
     *
     * @since 0.0.0-alpha.0
     */
    timedOut: boolean;
  }

  /**
   * Synchronously spawn `command` returning raw `Uint8Array` stdout/stderr.
   *
   * Executes `command` directly without a shell (max 4096 `args`). Non-zero exit
   * codes do not throw; inspect `status` and `error` on the returned object.
   *
   * @param command Executable name or path.
   * @param args Argument list passed to `command` (max 4096 entries).
   * @param options Execution options with binary output (`encoding: "buffer" | null` or omitted).
   * @returns Captured exit status, signal, `stdout`, `stderr`, optional `error`, and `timedOut` flag.
   * @throws {TypeError} when arguments or options have invalid types, or `encoding` is unknown (`ERR_UNKNOWN_ENCODING`).
   * @throws {RangeError} when `args`/`env` limits are exceeded or `timeout`/`maxBuffer` is invalid.
   * @throws {Error} with `code = "ERR_UNSUPPORTED"` if `shell` is enabled in `options`.
   * @example
   * ```ts
   * import { Buffer } from "node:buffer";
   * import { spawnSync } from "node:child_process";
   *
   * const res = spawnSync("echo", ["raw-bytes"]);
   * console.log(Buffer.from(res.stdout).toString("utf8").trim());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function spawnSync(command: string, args?: readonly string[], options?: ChildProcessOptions & { encoding?: "buffer" | null }): SpawnSyncReturns<Uint8Array>;
  /**
   * Synchronously spawn `command` returning string-decoded stdout/stderr.
   *
   * @param command Executable name or path.
   * @param args Argument list passed to `command` (max 4096 entries), or `undefined`.
   * @param options Execution options specifying a `TextEncoding` for `stdout` and `stderr`.
   * @returns Captured exit status, signal, string `stdout`, string `stderr`, optional `error`, and `timedOut` flag.
   * @throws {TypeError} when arguments or options have invalid types, or `encoding` is unknown (`ERR_UNKNOWN_ENCODING`).
   * @throws {RangeError} when `args`/`env` limits are exceeded or `timeout`/`maxBuffer` is invalid.
   * @throws {Error} with `code = "ERR_UNSUPPORTED"` if `shell` is enabled in `options`.
   * @since 0.0.0-alpha.0
   */
  export function spawnSync(command: string, args: readonly string[] | undefined, options: ChildProcessOptions & { encoding: TextEncoding }): SpawnSyncReturns<string>;
  /**
   * Synchronously spawn `command` with options.
   *
   * @param command Executable name or path.
   * @param options Execution options without an explicit `args` array.
   * @returns Captured exit status, signal, `stdout`, `stderr`, optional `error`, and `timedOut` flag.
   * @throws {TypeError} when arguments or options have invalid types, or `encoding` is unknown (`ERR_UNKNOWN_ENCODING`).
   * @throws {RangeError} when `env` limits are exceeded or `timeout`/`maxBuffer` is invalid.
   * @throws {Error} with `code = "ERR_UNSUPPORTED"` if `shell` is enabled in `options`.
   * @since 0.0.0-alpha.0
   */
  export function spawnSync(command: string, options?: ChildProcessOptions): SpawnSyncReturns<Uint8Array | string>;

  /**
   * Synchronously execute `file` returning raw `Uint8Array` stdout.
   *
   * Unlike `spawnSync`, throws an `Error` (with `status`, `signal`, `stdout`,
   * `stderr`, `timedOut`, and `code`) if the process fails to spawn, times out,
   * or exits with a non-zero status.
   *
   * @param file Executable name or path.
   * @param args Argument list passed to `file` (max 4096 entries).
   * @param options Execution options with binary output.
   * @returns Captured `stdout` bytes.
   * @throws {TypeError} when arguments or options have invalid types, or `encoding` is unknown (`ERR_UNKNOWN_ENCODING`).
   * @throws {RangeError} when `args`/`env` limits are exceeded or `timeout`/`maxBuffer` is invalid.
   * @throws {Error} on non-zero exit, signal termination, timeout (`ETIMEDOUT`), spawn failure (`ENOENT`, `EACCES`, `EIO`), or `ERR_UNSUPPORTED` if `shell` is enabled.
   * @example
   * ```ts
   * import { execFileSync } from "node:child_process";
   *
   * const out = execFileSync("echo", ["hello"], { encoding: "utf8" });
   * console.log(out.trim());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function execFileSync(file: string, args?: readonly string[], options?: ChildProcessOptions & { encoding?: "buffer" | null }): Uint8Array;
  /**
   * Synchronously execute `file` returning string-decoded stdout.
   *
   * @param file Executable name or path.
   * @param args Argument list passed to `file` (max 4096 entries), or `undefined`.
   * @param options Execution options specifying a `TextEncoding` for `stdout`.
   * @returns Decoded `stdout` string.
   * @throws {TypeError} when arguments or options have invalid types, or `encoding` is unknown (`ERR_UNKNOWN_ENCODING`).
   * @throws {RangeError} when `args`/`env` limits are exceeded or `timeout`/`maxBuffer` is invalid.
   * @throws {Error} on non-zero exit, signal termination, timeout (`ETIMEDOUT`), spawn failure (`ENOENT`, `EACCES`, `EIO`), or `ERR_UNSUPPORTED` if `shell` is enabled.
   * @since 0.0.0-alpha.0
   */
  export function execFileSync(file: string, args: readonly string[] | undefined, options: ChildProcessOptions & { encoding: TextEncoding }): string;
  /**
   * Synchronously execute `file` with options.
   *
   * @param file Executable name or path.
   * @param options Execution options without an explicit `args` array.
   * @returns Captured `stdout` as `Uint8Array` or decoded `string`.
   * @throws {TypeError} when arguments or options have invalid types, or `encoding` is unknown (`ERR_UNKNOWN_ENCODING`).
   * @throws {RangeError} when `env` limits are exceeded or `timeout`/`maxBuffer` is invalid.
   * @throws {Error} on non-zero exit, signal termination, timeout (`ETIMEDOUT`), spawn failure (`ENOENT`, `EACCES`, `EIO`), or `ERR_UNSUPPORTED` if `shell` is enabled.
   * @since 0.0.0-alpha.0
   */
  export function execFileSync(file: string, options?: ChildProcessOptions): Uint8Array | string;

  /**
   * Default export mirroring `node:child_process` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    /** Synchronously spawn a child process. @since 0.0.0-alpha.0 */
    spawnSync: typeof spawnSync;
    /** Synchronously execute a file and return stdout. @since 0.0.0-alpha.0 */
    execFileSync: typeof execFileSync;
  };
  /**
   * Default export object providing `{ spawnSync, execFileSync }`.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}

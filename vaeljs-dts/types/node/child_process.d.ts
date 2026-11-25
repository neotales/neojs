/**
 * Node-compatible synchronous and async child process execution without a shell.
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
    /** @since 0.0.0-alpha.0 */
    cwd?: string;
    /** @since 0.0.0-alpha.0 */
    env?: Record<string, string | undefined>;
    /** @since 0.0.0-alpha.0 */
    input?: string | Uint8Array | ArrayBufferView;
    /** @since 0.0.0-alpha.0 */
    encoding?: TextEncoding | "buffer" | null;
    /** @since 0.0.0-alpha.0 */
    timeout?: number;
    /** @since 0.0.0-alpha.0 */
    maxBuffer?: number;
  }

  /**
   * Result returned by `spawnSync`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface SpawnSyncReturns<T = Uint8Array | string> {
    /** @since 0.0.0-alpha.0 */
    status: number | null;
    /** @since 0.0.0-alpha.0 */
    signal: string | null;
    /** @since 0.0.0-alpha.0 */
    stdout: T;
    /** @since 0.0.0-alpha.0 */
    stderr: T;
    /** @since 0.0.0-alpha.0 */
    error?: Error;
    /** @since 0.0.0-alpha.0 */
    timedOut: boolean;
  }

  /**
   * Synchronously spawn `command` returning raw `Uint8Array` stdout/stderr.
   *
   * @since 0.0.0-alpha.0
   */
  export function spawnSync(command: string, args?: readonly string[], options?: ChildProcessOptions & { encoding?: "buffer" | null }): SpawnSyncReturns<Uint8Array>;
  /**
   * Synchronously spawn `command` returning string-decoded stdout/stderr.
   *
   * @since 0.0.0-alpha.0
   */
  export function spawnSync(command: string, args: readonly string[] | undefined, options: ChildProcessOptions & { encoding: TextEncoding }): SpawnSyncReturns<string>;
  /**
   * Synchronously spawn `command` with options.
   *
   * @since 0.0.0-alpha.0
   */
  export function spawnSync(command: string, options?: ChildProcessOptions): SpawnSyncReturns<Uint8Array | string>;

  /**
   * Synchronously execute `file` returning raw `Uint8Array` stdout.
   *
   * @since 0.0.0-alpha.0
   */
  export function execFileSync(file: string, args?: readonly string[], options?: ChildProcessOptions & { encoding?: "buffer" | null }): Uint8Array;
  /**
   * Synchronously execute `file` returning string-decoded stdout.
   *
   * @since 0.0.0-alpha.0
   */
  export function execFileSync(file: string, args: readonly string[] | undefined, options: ChildProcessOptions & { encoding: TextEncoding }): string;
  /**
   * Synchronously execute `file` with options.
   *
   * @since 0.0.0-alpha.0
   */
  export function execFileSync(file: string, options?: ChildProcessOptions): Uint8Array | string;

  /**
   * Executes `file` asynchronously without spawning a shell.
   *
   * Resolves to `{ stdout, stderr }` when called without a callback, or invokes
   * `callback(err, stdout, stderr)` when a trailing callback is provided.
   *
   * @since 0.0.0-alpha.0
   */
  export function execFile(file: string, options?: ChildProcessOptions): Promise<{ stdout: Uint8Array | string; stderr: Uint8Array | string }>;
  /**
   * Execute `file` asynchronously with `args` and optional `options`, returning a `Promise`.
   *
   * @since 0.0.0-alpha.0
   */
  export function execFile(file: string, args?: readonly string[], options?: ChildProcessOptions): Promise<{ stdout: Uint8Array | string; stderr: Uint8Array | string }>;
  /**
   * Execute `file` asynchronously and invoke `callback(error, stdout, stderr)`.
   *
   * @since 0.0.0-alpha.0
   */
  export function execFile(
    file: string,
    callback: (error: Error | null, stdout: Uint8Array | string, stderr: Uint8Array | string) => void,
  ): void;
  /**
   * Execute `file` asynchronously with `args` and invoke `callback(error, stdout, stderr)`.
   *
   * @since 0.0.0-alpha.0
   */
  export function execFile(
    file: string,
    args: readonly string[],
    callback: (error: Error | null, stdout: Uint8Array | string, stderr: Uint8Array | string) => void,
  ): void;
  /**
   * Execute `file` asynchronously with `args` and `options`, invoking `callback(error, stdout, stderr)`.
   *
   * @since 0.0.0-alpha.0
   */
  export function execFile(
    file: string,
    args: readonly string[],
    options: ChildProcessOptions,
    callback: (error: Error | null, stdout: Uint8Array | string, stderr: Uint8Array | string) => void,
  ): void;

  /**
   * Default export mirroring `node:child_process` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    spawnSync: typeof spawnSync;
    execFileSync: typeof execFileSync;
    execFile: typeof execFile;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

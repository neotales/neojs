declare module "node:child_process" {
  export type TextEncoding = "utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii";

  export interface ChildProcessOptions {
    cwd?: string;
    env?: Record<string, string | undefined>;
    input?: string | Uint8Array | ArrayBufferView;
    encoding?: TextEncoding | "buffer" | null;
    timeout?: number;
    maxBuffer?: number;
  }

  export interface SpawnSyncReturns<T = Uint8Array | string> {
    status: number | null;
    signal: string | null;
    stdout: T;
    stderr: T;
    error?: Error;
    timedOut: boolean;
  }

  export function spawnSync(command: string, args?: readonly string[], options?: ChildProcessOptions & { encoding?: "buffer" | null }): SpawnSyncReturns<Uint8Array>;
  export function spawnSync(command: string, args: readonly string[] | undefined, options: ChildProcessOptions & { encoding: TextEncoding }): SpawnSyncReturns<string>;
  export function spawnSync(command: string, options?: ChildProcessOptions): SpawnSyncReturns<Uint8Array | string>;

  export function execFileSync(file: string, args?: readonly string[], options?: ChildProcessOptions & { encoding?: "buffer" | null }): Uint8Array;
  export function execFileSync(file: string, args: readonly string[] | undefined, options: ChildProcessOptions & { encoding: TextEncoding }): string;
  export function execFileSync(file: string, options?: ChildProcessOptions): Uint8Array | string;

  /**
   * Executes `file` asynchronously without spawning a shell.
   *
   * Resolves to `{ stdout, stderr }` when called without a callback, or invokes
   * `callback(err, stdout, stderr)` when a trailing callback is provided.
   */
  export function execFile(file: string, options?: ChildProcessOptions): Promise<{ stdout: Uint8Array | string; stderr: Uint8Array | string }>;
  export function execFile(file: string, args?: readonly string[], options?: ChildProcessOptions): Promise<{ stdout: Uint8Array | string; stderr: Uint8Array | string }>;
  export function execFile(
    file: string,
    callback: (error: Error | null, stdout: Uint8Array | string, stderr: Uint8Array | string) => void,
  ): void;
  export function execFile(
    file: string,
    args: readonly string[],
    callback: (error: Error | null, stdout: Uint8Array | string, stderr: Uint8Array | string) => void,
  ): void;
  export function execFile(
    file: string,
    args: readonly string[],
    options: ChildProcessOptions,
    callback: (error: Error | null, stdout: Uint8Array | string, stderr: Uint8Array | string) => void,
  ): void;

  const _default: {
    spawnSync: typeof spawnSync;
    execFileSync: typeof execFileSync;
    execFile: typeof execFile;
  };
  export default _default;
}

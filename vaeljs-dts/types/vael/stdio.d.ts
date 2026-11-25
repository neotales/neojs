declare module "vael:stdio" {
  /**
   * Maximum byte capacity of one memory-backed stream or single `writeSync`
   * chunk (`8 MiB` / `8_388_608` bytes).
   */
  export const MAX_BYTES: 8388608;

  /**
   * Error codes reported on `StdioError.code` (`name = "StdioError"`).
   */
  export type StdioErrorCode =
    | "VAEL_STDIO_DETACHED"
    | "VAEL_STDIO_LIMIT"
    | "VAEL_STDIO_ARGUMENT"
    | "VAEL_STDIO_WRITE"
    | "VAEL_STDIO_CLOSED"
    | "VAEL_STDIO_ENDED"
    | "VAEL_STDIO_NOT_MEMORY";

  /**
   * Synchronous reader over an owned snapshot copy of a `Uint8Array`.
   */
  export class Readable {
    /**
     * Copy `bytes` (attached view, at most `MAX_BYTES`) into reader-owned storage.
     *
     * @throws {TypeError} when `bytes` is not a `Uint8Array` or arity is wrong.
     * @throws {Error} (`name = "StdioError"`) with `code` `"VAEL_STDIO_DETACHED"` or `"VAEL_STDIO_LIMIT"`.
     */
    constructor(bytes: Uint8Array);

    /** Remaining unread bytes (`0` at EOF and after `close()`). */
    readonly remaining: number;

    /** Whether `close()` has been called. */
    readonly closed: boolean;

    /**
     * Copy up to `target.byteLength` remaining bytes into `target`, returning
     * the number of bytes copied (`0` at EOF).
     *
     * @param target Attached, non-empty `Uint8Array`.
     * @throws {TypeError} when `target` is not a `Uint8Array` or arity is wrong.
     * @throws {Error} (`name = "StdioError"`) with `code` `"VAEL_STDIO_CLOSED"`,
     * `"VAEL_STDIO_DETACHED"`, or `"VAEL_STDIO_ARGUMENT"` (empty `target`).
     */
    readSync(target: Uint8Array): number;

    /** Release the owned byte snapshot. Idempotent. */
    close(): void;
  }

  /**
   * Synchronous byte writer (either a bounded in-memory buffer or a shared
   * host stream `stdout`/`stderr`).
   */
  export class Writable {
    /**
     * Create an empty memory-backed writer (capacity bounded by `MAX_BYTES`).
     *
     * @throws {TypeError} when any argument is supplied.
     */
    constructor();

    /** Stream backing kind. */
    readonly kind: "memory" | "stdout" | "stderr";

    /**
     * Bytes retained by a memory writer (`0` for host streams or after `close()`).
     */
    readonly length: number;

    /** Whether `close()` has been called. */
    readonly closed: boolean;

    /** Whether `end()` or `close()` has been called. */
    readonly ended: boolean;

    /**
     * Synchronously append `chunk` (or write it to the host stream), returning
     * the number of bytes accepted.
     *
     * @param chunk `Uint8Array` or UTF-8 string (total memory capacity max `MAX_BYTES`).
     * @throws {TypeError} on invalid argument type or arity.
     * @throws {Error} (`name = "StdioError"`) with `code` `"VAEL_STDIO_CLOSED"`,
     * `"VAEL_STDIO_ENDED"`, `"VAEL_STDIO_DETACHED"`, `"VAEL_STDIO_LIMIT"`, or `"VAEL_STDIO_WRITE"`.
     */
    writeSync(chunk: Uint8Array | string): number;

    /** Stop accepting writes while keeping memory-backed content readable. Idempotent. */
    end(): void;

    /**
     * Return a fresh `Uint8Array` copy of a memory-backed writer's contents.
     *
     * @throws {Error} (`name = "StdioError"`) with `code` `"VAEL_STDIO_CLOSED"`
     * or `"VAEL_STDIO_NOT_MEMORY"` (when called on `stdout` or `stderr`).
     */
    toUint8Array(): Uint8Array;

    /**
     * Release memory-backed content and retire the writer. Idempotent.
     * Does not close the underlying OS file descriptor for `stdout`/`stderr`.
     */
    close(): void;
  }

  /** Shared host writer targeting the current runtime's standard output. */
  export const stdout: Writable;

  /** Shared host writer targeting the current runtime's standard error. */
  export const stderr: Writable;
}

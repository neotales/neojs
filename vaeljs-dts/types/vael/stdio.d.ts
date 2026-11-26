/**
 * Bounded synchronous byte readers, writers, and standard output streams.
 *
 * Available in the `vjs` (`vaeljs`) runtime only; absent in `vjs-lite`.
 * Provides memory-backed `Readable` / `Writable` byte streams bounded by
 * `MAX_BYTES` (`8 MiB`) and synchronous host `stdout` / `stderr` writers.
 *
 * @example
 * ```ts
 * import { Readable, Writable, stdout } from "vael:stdio";
 *
 * const reader = new Readable(new Uint8Array([104, 105]));
 * const out = new Uint8Array(2);
 * reader.readSync(out);
 * reader.close();
 *
 * const writer = new Writable();
 * writer.writeSync("hello\n");
 * writer.end();
 * stdout.writeSync(writer.toUint8Array());
 * writer.close();
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:stdio" {
  /**
   * Maximum byte capacity of one memory-backed stream or single `writeSync`
   * chunk (`8 MiB` / `8_388_608` bytes).
   *
   * @since 0.0.0-alpha.0
   */
  export const MAX_BYTES: 8388608;

  /**
   * Error codes reported on `StdioError.code` (`name = "StdioError"`).
   *
   * @since 0.0.0-alpha.0
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
   *
   * @since 0.0.0-alpha.0
   */
  export class Readable {
    /**
     * Copy `bytes` (attached view, at most `MAX_BYTES`) into reader-owned storage.
     *
     * @param bytes Attached source byte view (at most `MAX_BYTES`).
     * @throws {TypeError} when `bytes` is not a `Uint8Array` or arity is wrong.
     * @throws {Error} (`name = "StdioError"`) with `code` `"VAEL_STDIO_DETACHED"` or `"VAEL_STDIO_LIMIT"`.
     * @since 0.0.0-alpha.0
     */
    constructor(bytes: Uint8Array);

    /** Remaining unread bytes (`0` at EOF and after `close()`). @since 0.0.0-alpha.0 */
    readonly remaining: number;

    /** Whether `close()` has been called. @since 0.0.0-alpha.0 */
    readonly closed: boolean;

    /**
     * Copy up to `target.byteLength` remaining bytes into `target`, returning
     * the number of bytes copied (`0` at EOF).
     *
     * @param target Attached, non-empty `Uint8Array`.
     * @returns Number of bytes copied into `target` (`0` at EOF).
     * @throws {TypeError} when `target` is not a `Uint8Array` or arity is wrong.
     * @throws {Error} (`name = "StdioError"`) with `code` `"VAEL_STDIO_CLOSED"`,
     * `"VAEL_STDIO_DETACHED"`, or `"VAEL_STDIO_ARGUMENT"` (empty `target`).
     * @example
     * ```ts
     * import { Readable } from "vael:stdio";
     *
     * const r = new Readable(new Uint8Array([1, 2, 3]));
     * const buf = new Uint8Array(2);
     * console.log(r.readSync(buf), r.remaining); // 2 1
     * r.close();
     * ```
     * @since 0.0.0-alpha.0
     */
    readSync(target: Uint8Array): number;

    /**
     * Release the owned byte snapshot. Idempotent.
     *
     * @since 0.0.0-alpha.0
     */
    close(): void;
  }

  /**
   * Synchronous byte writer (either a bounded in-memory buffer or a shared
   * host stream `stdout`/`stderr`).
   *
   * @since 0.0.0-alpha.0
   */
  export class Writable {
    /**
     * Create an empty memory-backed writer (capacity bounded by `MAX_BYTES`).
     *
     * @throws {TypeError} when any argument is supplied.
     * @since 0.0.0-alpha.0
     */
    constructor();

    /** Stream backing kind. @since 0.0.0-alpha.0 */
    readonly kind: "memory" | "stdout" | "stderr";

    /**
     * Bytes retained by a memory writer (`0` for host streams or after `close()`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly length: number;

    /** Whether `close()` has been called. @since 0.0.0-alpha.0 */
    readonly closed: boolean;

    /** Whether `end()` or `close()` has been called. @since 0.0.0-alpha.0 */
    readonly ended: boolean;

    /**
     * Synchronously append `chunk` (or write it to the host stream), returning
     * the number of bytes accepted.
     *
     * @param chunk `Uint8Array` or UTF-8 string (total memory capacity max `MAX_BYTES`).
     * @returns Number of bytes written.
     * @throws {TypeError} on invalid argument type or arity.
     * @throws {Error} (`name = "StdioError"`) with `code` `"VAEL_STDIO_CLOSED"`,
     * `"VAEL_STDIO_ENDED"`, `"VAEL_STDIO_DETACHED"`, `"VAEL_STDIO_LIMIT"`, or `"VAEL_STDIO_WRITE"`.
     * @example
     * ```ts
     * import { Writable } from "vael:stdio";
     *
     * const w = new Writable();
     * console.log(w.writeSync("hello")); // 5
     * w.close();
     * ```
     * @since 0.0.0-alpha.0
     */
    writeSync(chunk: Uint8Array | string): number;

    /**
     * Stop accepting writes while keeping memory-backed content readable. Idempotent.
     *
     * @since 0.0.0-alpha.0
     */
    end(): void;

    /**
     * Return a fresh `Uint8Array` copy of a memory-backed writer's contents.
     *
     * @returns Fresh `Uint8Array` copy of accumulated bytes.
     * @throws {Error} (`name = "StdioError"`) with `code` `"VAEL_STDIO_CLOSED"`
     * or `"VAEL_STDIO_NOT_MEMORY"` (when called on `stdout` or `stderr`).
     * @since 0.0.0-alpha.0
     */
    toUint8Array(): Uint8Array;

    /**
     * Release memory-backed content and retire the writer. Idempotent.
     * Does not close the underlying OS file descriptor for `stdout`/`stderr`.
     *
     * @since 0.0.0-alpha.0
     */
    close(): void;
  }

  /**
   * Shared host writer targeting the current runtime's standard output.
   *
   * @since 0.0.0-alpha.0
   */
  export const stdout: Writable;

  /**
   * Shared host writer targeting the current runtime's standard error.
   *
   * @since 0.0.0-alpha.0
   */
  export const stderr: Writable;
}

/**
 * Node-compatible `Buffer` implementation backed by `Uint8Array`.
 *
 * Available in both `vjs` and `vjs-lite` runtimes. Enabled by default unless
 * `--no-node-compat` is passed.
 *
 * @example
 * ```ts
 * import { Buffer } from "node:buffer";
 *
 * const buf = Buffer.from("hello vaeljs", "utf8");
 * console.log(buf.toString("hex"));
 * console.log(buf.toString("base64url"));
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:buffer" {
  /**
   * Supported character encodings for `Buffer` string conversions.
   *
   * Supports `"utf8"` / `"utf-8"`, `"hex"`, `"base64"`, `"base64url"`
   * (unpadded), `"latin1"`, and `"ascii"`.
   *
   * @since 0.0.0-alpha.0
   */
  export type BufferEncoding = "utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii";

  /**
   * Fixed-length sequence of bytes extending `Uint8Array`.
   *
   * Provides Node-compatible static constructors (`from`, `alloc`,
   * `allocUnsafe`, `concat`, `isBuffer`, `byteLength`) and instance helpers
   * (`toString`, `toJSON`, `equals`).
   *
   * @example
   * ```ts
   * import { Buffer } from "node:buffer";
   *
   * const a = Buffer.from("vael", "utf8");
   * const b = Buffer.alloc(4, 0x21);
   * const combined = Buffer.concat([a, b]);
   * console.log(combined.toString("utf8")); // "vael!!!!"
   * ```
   *
   * @since 0.0.0-alpha.0
   */
  export class Buffer extends Uint8Array {
    /**
     * Create a `Buffer` containing the encoded string `value`.
     *
     * Decodes up to 16 MiB of input using `encoding` (default `"utf8"`).
     *
     * @param value String to encode into bytes.
     * @param encoding Character encoding of `value` (default `"utf8"`).
     * @returns Newly allocated `Buffer` containing the decoded bytes.
     * @throws {TypeError} with `code = "ERR_UNKNOWN_ENCODING"` when `encoding` is unsupported, or on malformed encoded input (`hex`, `base64`, `base64url`, `latin1`, `ascii`).
     * @throws {RangeError} with `code = "ERR_OUT_OF_RANGE"` when `value` exceeds 16 MiB.
     * @example
     * ```ts
     * import { Buffer } from "node:buffer";
     *
     * const bytes = Buffer.from("6869", "hex");
     * console.log(bytes.toString("utf8")); // "hi"
     * ```
     * @since 0.0.0-alpha.0
     */
    static from(value: string, encoding?: BufferEncoding): Buffer;
    /**
     * Create a `Buffer` view over an `ArrayBuffer` or `SharedArrayBuffer`.
     *
     * Shares memory with the underlying buffer without copying.
     *
     * @param value Backing `ArrayBuffer` or `SharedArrayBuffer`.
     * @param byteOffset Byte offset to start the view at (default `0`).
     * @param length Number of bytes to include in the view (default: to end of buffer).
     * @returns `Buffer` view sharing memory with `value`.
     * @throws {RangeError} when `byteOffset` or `length` is out of bounds for `value`.
     * @since 0.0.0-alpha.0
     */
    static from(value: ArrayBuffer | SharedArrayBuffer, byteOffset?: number, length?: number): Buffer;
    /**
     * Create a `Buffer` copied from an array-like or `ArrayBufferView`.
     *
     * @param value Array of byte values, `TypedArray`, or `DataView` to copy.
     * @returns Newly allocated `Buffer` containing a copy of `value`.
     * @throws {TypeError} when `value` is not an array-like object or `ArrayBufferView`.
     * @since 0.0.0-alpha.0
     */
    static from(value: ArrayLike<number> | ArrayBufferView): Buffer;
    /**
     * Allocate a zero-filled (or `fill`-initialized) `Buffer` of `size` bytes.
     *
     * @param size Number of bytes to allocate (non-negative integer).
     * @param fill Optional byte value, string, or `Uint8Array` pattern repeated to fill the buffer.
     * @param encoding Encoding used when `fill` is a string (default `"utf8"`).
     * @returns Newly allocated and initialized `Buffer` of length `size`.
     * @throws {RangeError} when `size` is negative or not an integer.
     * @throws {TypeError} with `code = "ERR_UNKNOWN_ENCODING"` when `encoding` is unsupported.
     * @example
     * ```ts
     * import { Buffer } from "node:buffer";
     *
     * const padded = Buffer.alloc(6, "ab", "utf8");
     * console.log(padded.toString("utf8")); // "ababab"
     * ```
     * @since 0.0.0-alpha.0
     */
    static alloc(size: number, fill?: string | number | Uint8Array, encoding?: BufferEncoding): Buffer;
    /**
     * Allocate a `Buffer` of `size` bytes.
     *
     * In `vaeljs`, newly allocated memory is zero-initialized for safety.
     *
     * @param size Number of bytes to allocate (non-negative integer).
     * @returns Newly allocated `Buffer` of length `size`.
     * @throws {RangeError} when `size` is negative or not an integer.
     * @since 0.0.0-alpha.0
     */
    static allocUnsafe(size: number): Buffer;
    /**
     * Concatenate an array of `Uint8Array` instances into a new `Buffer`.
     *
     * @param list Array of `Buffer` or `Uint8Array` instances to concatenate.
     * @param totalLength Optional exact byte length of the resulting `Buffer` (truncated or zero-padded if different from the sum).
     * @returns New `Buffer` containing the concatenated bytes.
     * @throws {TypeError} when `list` is not an array or contains a non-`Uint8Array` element.
     * @throws {RangeError} when `totalLength` is negative or not an integer.
     * @example
     * ```ts
     * import { Buffer } from "node:buffer";
     *
     * const out = Buffer.concat([Buffer.from("foo"), Buffer.from("bar")]);
     * console.log(out.toString("utf8")); // "foobar"
     * ```
     * @since 0.0.0-alpha.0
     */
    static concat(list: readonly Uint8Array[], totalLength?: number): Buffer;
    /**
     * Return `true` if `value` is a `Buffer`.
     *
     * @param value Value to inspect.
     * @returns `true` if `value` is an instance of `Buffer`, otherwise `false`.
     * @since 0.0.0-alpha.0
     */
    static isBuffer(value: unknown): value is Buffer;
    /**
     * Return the byte length of `value` when encoded with `encoding`.
     *
     * @param value String, `ArrayBufferView`, `ArrayBuffer`, or `SharedArrayBuffer` to measure.
     * @param encoding Encoding applied when `value` is a string (default `"utf8"`).
     * @returns Byte length of `value`.
     * @throws {TypeError} when `value` has an unsupported type or `encoding` is unknown (`ERR_UNKNOWN_ENCODING`).
     * @example
     * ```ts
     * import { Buffer } from "node:buffer";
     *
     * console.log(Buffer.byteLength("€", "utf8")); // 3
     * ```
     * @since 0.0.0-alpha.0
     */
    static byteLength(value: string | ArrayBufferView | ArrayBuffer | SharedArrayBuffer, encoding?: BufferEncoding): number;

    /**
     * Decode bytes `[start, end)` to a string using `encoding` (default `"utf8"`).
     *
     * Negative `start` or `end` indices are resolved relative to `this.length`.
     * Invalid UTF-8 sequences are replaced with `U+FFFD`.
     *
     * @param encoding Output string encoding (default `"utf8"`).
     * @param start Byte offset to begin decoding (default `0`).
     * @param end Byte offset to stop decoding, exclusive (default `this.length`).
     * @returns Encoded string representation of the slice.
     * @throws {TypeError} with `code = "ERR_UNKNOWN_ENCODING"` when `encoding` is unsupported.
     * @example
     * ```ts
     * import { Buffer } from "node:buffer";
     *
     * const buf = Buffer.from("hello", "utf8");
     * console.log(buf.toString("base64")); // "aGVsbG8="
     * ```
     * @since 0.0.0-alpha.0
     */
    toString(encoding?: BufferEncoding, start?: number, end?: number): string;
    /**
     * Return a JSON representation `{ type: "Buffer", data: number[] }`.
     *
     * Invoked automatically by `JSON.stringify()`.
     *
     * @returns Plain object with `type: "Buffer"` and byte values in `data`.
     * @since 0.0.0-alpha.0
     */
    toJSON(): { type: "Buffer"; data: number[] };
    /**
     * Return `true` if `this` and `otherBuffer` have identical bytes.
     *
     * @param otherBuffer Other `Buffer` or `Uint8Array` to compare against.
     * @returns `true` if both buffers have the same length and identical byte values.
     * @throws {TypeError} when `otherBuffer` is not a `Uint8Array`.
     * @since 0.0.0-alpha.0
     */
    equals(otherBuffer: Uint8Array): boolean;
  }

  /**
   * Default export mirroring `node:buffer` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    /** The `Buffer` class constructor. @since 0.0.0-alpha.0 */
    Buffer: typeof Buffer;
  };
  /**
   * Default export object providing `{ Buffer }`.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}

/**
 * Node-compatible `Buffer` implementation backed by `Uint8Array`.
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:buffer" {
  /**
   * Supported character encodings for `Buffer` string conversions.
   *
   * @since 0.0.0-alpha.0
   */
  export type BufferEncoding = "utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii";

  /**
   * Fixed-length sequence of bytes extending `Uint8Array`.
   *
   * @since 0.0.0-alpha.0
   */
  export class Buffer extends Uint8Array {
    /**
     * Create a `Buffer` containing the encoded string `value`.
     *
     * @since 0.0.0-alpha.0
     */
    static from(value: string, encoding?: BufferEncoding): Buffer;
    /**
     * Create a `Buffer` view over an `ArrayBuffer` or `SharedArrayBuffer`.
     *
     * @since 0.0.0-alpha.0
     */
    static from(value: ArrayBuffer | SharedArrayBuffer, byteOffset?: number, length?: number): Buffer;
    /**
     * Create a `Buffer` copied from an array-like or `ArrayBufferView`.
     *
     * @since 0.0.0-alpha.0
     */
    static from(value: ArrayLike<number> | ArrayBufferView): Buffer;
    /**
     * Allocate a zero-filled (or `fill`-initialized) `Buffer` of `size` bytes.
     *
     * @since 0.0.0-alpha.0
     */
    static alloc(size: number, fill?: string | number | Uint8Array, encoding?: BufferEncoding): Buffer;
    /**
     * Allocate a `Buffer` of `size` bytes.
     *
     * @since 0.0.0-alpha.0
     */
    static allocUnsafe(size: number): Buffer;
    /**
     * Concatenate an array of `Uint8Array` instances into a new `Buffer`.
     *
     * @since 0.0.0-alpha.0
     */
    static concat(list: readonly Uint8Array[], totalLength?: number): Buffer;
    /**
     * Return `true` if `value` is a `Buffer`.
     *
     * @since 0.0.0-alpha.0
     */
    static isBuffer(value: unknown): value is Buffer;
    /**
     * Return the byte length of `value` when encoded with `encoding`.
     *
     * @since 0.0.0-alpha.0
     */
    static byteLength(value: string | ArrayBufferView | ArrayBuffer | SharedArrayBuffer, encoding?: BufferEncoding): number;

    /**
     * Decode bytes `[start, end)` to a string using `encoding` (default `"utf8"`).
     *
     * @since 0.0.0-alpha.0
     */
    toString(encoding?: BufferEncoding, start?: number, end?: number): string;
    /**
     * Return a JSON representation `{ type: "Buffer", data: number[] }`.
     *
     * @since 0.0.0-alpha.0
     */
    toJSON(): { type: "Buffer"; data: number[] };
    /**
     * Return `true` if `this` and `otherBuffer` have identical bytes.
     *
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
    Buffer: typeof Buffer;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

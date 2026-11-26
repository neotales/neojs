/**
 * Node-compatible cryptographic hashes, HMACs, CSPRNG, PBKDF2, and constant-time comparison.
 *
 * Available in both `vjs` and `vjs-lite` runtimes. Enabled by default unless
 * `--no-node-compat` is passed.
 *
 * Supported digest algorithms are `"sha256"`, `"sha384"`, and `"sha512"`
 * (case-insensitive). Legacy algorithms such as MD5 and SHA-1 are intentionally
 * rejected with `ERR_CRYPTO_HASH_UNKNOWN` / `ERR_CRYPTO_INVALID_DIGEST`.
 *
 * @example
 * ```ts
 * import { createHash, createHmac, randomBytes, timingSafeEqual } from "node:crypto";
 *
 * const digest = createHash("sha256").update("hello").digest("hex");
 * const mac = createHmac("sha256", "secret").update("hello").digest();
 * const token = randomBytes(16);
 * console.log(digest, mac.byteLength, token.byteLength);
 * console.log(timingSafeEqual(mac, mac));
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:crypto" {
  /**
   * Supported digest output encodings.
   *
   * @since 0.0.0-alpha.0
   */
  export type DigestEncoding = "hex" | "base64" | "base64url" | "latin1" | "binary";
  /**
   * Binary or string input accepted by `Hash`, `Hmac`, and `pbkdf2Sync`.
   *
   * @since 0.0.0-alpha.0
   */
  export type BinaryLike = string | Uint8Array | ArrayBufferView;

  /**
   * Incremental cryptographic hash object returned by `createHash`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Hash {
    /**
     * Update hash content with `data`.
     *
     * May be called repeatedly before `digest()` up to 16 MiB total input per
     * digest object. Fails after `digest()` has been called.
     *
     * @param data Input bytes or string to feed into the hash state.
     * @param inputEncoding Encoding used when `data` is a string (default `"utf8"`).
     * @returns `this` for method chaining.
     * @throws {Error} if `digest()` was already called, or with `code = "VAEL_CRYPTO_LIMIT"` if cumulative input exceeds 16 MiB.
     * @throws {TypeError} when `data` has an invalid type or `inputEncoding` is unknown (`ERR_UNKNOWN_ENCODING`).
     * @since 0.0.0-alpha.0
     */
    update(data: BinaryLike, inputEncoding?: string): this;
    /**
     * Compute raw digest bytes.
     *
     * Finalizes the hash state; subsequent `update()` or `digest()` calls throw.
     *
     * @returns Raw digest bytes as a `Buffer` (`Uint8Array` subclass).
     * @throws {Error} if `digest()` was already called.
     * @since 0.0.0-alpha.0
     */
    digest(): Uint8Array;
    /**
     * Compute encoded digest string.
     *
     * Finalizes the hash state; subsequent `update()` or `digest()` calls throw.
     *
     * @param encoding Output string encoding (`"hex"`, `"base64"`, `"base64url"`, `"latin1"`, or `"binary"`).
     * @returns Encoded digest string.
     * @throws {Error} if `digest()` was already called.
     * @throws {TypeError} with `code = "ERR_UNKNOWN_ENCODING"` when `encoding` is unsupported.
     * @since 0.0.0-alpha.0
     */
    digest(encoding: DigestEncoding): string;
  }

  /**
   * Incremental cryptographic HMAC object returned by `createHmac`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Hmac {
    /**
     * Update HMAC content with `data`.
     *
     * May be called repeatedly before `digest()` up to 16 MiB total input per
     * HMAC object. Fails after `digest()` has been called.
     *
     * @param data Input bytes or string to authenticate.
     * @param inputEncoding Encoding used when `data` is a string (default `"utf8"`).
     * @returns `this` for method chaining.
     * @throws {Error} if `digest()` was already called, or with `code = "VAEL_CRYPTO_LIMIT"` if cumulative input exceeds 16 MiB.
     * @throws {TypeError} when `data` has an invalid type or `inputEncoding` is unknown (`ERR_UNKNOWN_ENCODING`).
     * @since 0.0.0-alpha.0
     */
    update(data: BinaryLike, inputEncoding?: string): this;
    /**
     * Compute raw digest bytes.
     *
     * Finalizes the HMAC state; subsequent `update()` or `digest()` calls throw.
     *
     * @returns Raw HMAC tag bytes as a `Buffer` (`Uint8Array` subclass).
     * @throws {Error} if `digest()` was already called.
     * @since 0.0.0-alpha.0
     */
    digest(): Uint8Array;
    /**
     * Compute encoded digest string.
     *
     * Finalizes the HMAC state; subsequent `update()` or `digest()` calls throw.
     *
     * @param encoding Output string encoding (`"hex"`, `"base64"`, `"base64url"`, `"latin1"`, or `"binary"`).
     * @returns Encoded HMAC tag string.
     * @throws {Error} if `digest()` was already called.
     * @throws {TypeError} with `code = "ERR_UNKNOWN_ENCODING"` when `encoding` is unsupported.
     * @since 0.0.0-alpha.0
     */
    digest(encoding: DigestEncoding): string;
  }

  /**
   * Generate `size` cryptographically strong pseudo-random bytes.
   *
   * Backed by the operating system CSPRNG and bounded to 8 MiB per call.
   *
   * @param size Number of random bytes to generate (`0` to `8_388_608`).
   * @returns Newly allocated `Buffer` (`Uint8Array` subclass) filled with random bytes.
   * @throws {RangeError} when `size` is negative, not an integer, or exceeds 8 MiB (`VAEL_CRYPTO_LIMIT`).
   * @throws {Error} if the operating system CSPRNG fails.
   * @example
   * ```ts
   * import { Buffer } from "node:buffer";
   * import { randomBytes } from "node:crypto";
   *
   * const nonce = randomBytes(12);
   * console.log(Buffer.from(nonce).toString("hex"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function randomBytes(size: number): Uint8Array;
  /**
   * Generate a random RFC 4122/9562 version 4 UUID string.
   *
   * @returns Lowercase hyphenated UUID v4 string (36 characters).
   * @throws {Error} with `code = "ERR_UNAVAILABLE"` if the operating system CSPRNG fails.
   * @example
   * ```ts
   * import { randomUUID } from "node:crypto";
   *
   * const id = randomUUID();
   * console.log(id);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function randomUUID(): string;
  /**
   * Create an incremental `Hash` instance for `algorithm`.
   *
   * Supports `"sha256"`, `"sha384"`, and `"sha512"` (case-insensitive). Up to
   * 1024 digest handles may be open simultaneously.
   *
   * @param algorithm Hash algorithm name (`"sha256"`, `"sha384"`, or `"sha512"`).
   * @returns Incremental `Hash` instance.
   * @throws {TypeError} when `algorithm` is not a string.
   * @throws {Error} with `code = "ERR_CRYPTO_HASH_UNKNOWN"` when `algorithm` is unsupported.
   * @throws {RangeError} with `code = "ERR_CRYPTO_TOO_MANY_OBJECTS"` if more than 1024 digest handles are open.
   * @example
   * ```ts
   * import { createHash } from "node:crypto";
   *
   * const hex = createHash("sha256").update("vaeljs", "utf8").digest("hex");
   * console.log(hex);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function createHash(algorithm: string): Hash;
  /**
   * Create an incremental `Hmac` instance for `algorithm` and `key`.
   *
   * Supports `"sha256"`, `"sha384"`, and `"sha512"` (case-insensitive), with
   * keys up to 1 MiB. Up to 1024 digest handles may be open simultaneously.
   *
   * @param algorithm Hash algorithm name (`"sha256"`, `"sha384"`, or `"sha512"`).
   * @param key Secret HMAC key (UTF-8 string or byte view, max 1 MiB).
   * @returns Incremental `Hmac` instance.
   * @throws {TypeError} when `algorithm` is not a string or `key` has an invalid type.
   * @throws {Error} with `code = "ERR_CRYPTO_HASH_UNKNOWN"` when `algorithm` is unsupported.
   * @throws {RangeError} with `code = "VAEL_CRYPTO_LIMIT"` if `key` exceeds 1 MiB, or `"ERR_CRYPTO_TOO_MANY_OBJECTS"` if too many handles are open.
   * @example
   * ```ts
   * import { createHmac } from "node:crypto";
   *
   * const tag = createHmac("sha256", "signing-key").update("payload").digest("base64url");
   * console.log(tag);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function createHmac(algorithm: string, key: BinaryLike): Hmac;
  /**
   * Synchronously derive a key using PBKDF2-HMAC.
   *
   * Supports `"sha256"`, `"sha384"`, and `"sha512"` digests. Bounded to
   * `1..=1_000_000` iterations, `keylen <= 1024`, and 1 MiB max password/salt.
   *
   * @param password Input password bytes or UTF-8 string (max 1 MiB).
   * @param salt Cryptographic salt bytes or UTF-8 string (max 1 MiB).
   * @param iterations Iteration count (`1` to `1_000_000`).
   * @param keylen Derived key length in bytes (`0` to `1024`).
   * @param digest Hash algorithm name (`"sha256"`, `"sha384"`, or `"sha512"`).
   * @returns Derived key bytes as a `Buffer` (`Uint8Array` subclass).
   * @throws {TypeError} with `code = "ERR_CRYPTO_INVALID_DIGEST"` when `digest` is unsupported, or on invalid argument types.
   * @throws {RangeError} when `iterations`, `keylen`, `password`, or `salt` is out of bounds.
   * @example
   * ```ts
   * import { Buffer } from "node:buffer";
   * import { pbkdf2Sync } from "node:crypto";
   *
   * const key = pbkdf2Sync("passphrase", "salt-value", 10000, 32, "sha256");
   * console.log(Buffer.from(key).toString("hex"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function pbkdf2Sync(
    password: BinaryLike,
    salt: BinaryLike,
    iterations: number,
    keylen: number,
    digest: string,
  ): Uint8Array;
  /**
   * Constant-time equality comparison for equal-length byte buffers.
   *
   * @param a First `TypedArray` or `DataView` to compare.
   * @param b Second `TypedArray` or `DataView` to compare (must have the same `byteLength` as `a`).
   * @returns `true` if `a` and `b` contain identical bytes, otherwise `false`.
   * @throws {TypeError} when `a` or `b` is not an `ArrayBufferView`.
   * @throws {RangeError} with `code = "ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH"` when `a.byteLength !== b.byteLength`.
   * @example
   * ```ts
   * import { Buffer } from "node:buffer";
   * import { timingSafeEqual } from "node:crypto";
   *
   * const left = Buffer.from("secret", "utf8");
   * const right = Buffer.from("secret", "utf8");
   * console.log(timingSafeEqual(left, right)); // true
   * ```
   * @since 0.0.0-alpha.0
   */
  export function timingSafeEqual(a: ArrayBufferView, b: ArrayBufferView): boolean;

  /**
   * Default export mirroring `node:crypto` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    /** Generate cryptographically strong random bytes. @since 0.0.0-alpha.0 */
    randomBytes: typeof randomBytes;
    /** Generate an RFC 4122/9562 v4 UUID string. @since 0.0.0-alpha.0 */
    randomUUID: typeof randomUUID;
    /** Create an incremental hash object. @since 0.0.0-alpha.0 */
    createHash: typeof createHash;
    /** Create an incremental HMAC object. @since 0.0.0-alpha.0 */
    createHmac: typeof createHmac;
    /** Derive a key using PBKDF2-HMAC. @since 0.0.0-alpha.0 */
    pbkdf2Sync: typeof pbkdf2Sync;
    /** Compare two equal-length buffers in constant time. @since 0.0.0-alpha.0 */
    timingSafeEqual: typeof timingSafeEqual;
  };
  /**
   * Default export object providing all `node:crypto` functions.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}

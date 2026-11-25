/**
 * Node-compatible cryptographic hashes, HMACs, CSPRNG, PBKDF2, and constant-time comparison.
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
    /** Update hash content with `data`. @since 0.0.0-alpha.0 */
    update(data: BinaryLike, inputEncoding?: string): this;
    /** Compute raw digest bytes. @since 0.0.0-alpha.0 */
    digest(): Uint8Array;
    /** Compute encoded digest string. @since 0.0.0-alpha.0 */
    digest(encoding: DigestEncoding): string;
  }

  /**
   * Incremental cryptographic HMAC object returned by `createHmac`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Hmac {
    /** Update HMAC content with `data`. @since 0.0.0-alpha.0 */
    update(data: BinaryLike, inputEncoding?: string): this;
    /** Compute raw digest bytes. @since 0.0.0-alpha.0 */
    digest(): Uint8Array;
    /** Compute encoded digest string. @since 0.0.0-alpha.0 */
    digest(encoding: DigestEncoding): string;
  }

  /**
   * Generate `size` cryptographically strong pseudo-random bytes.
   *
   * @since 0.0.0-alpha.0
   */
  export function randomBytes(size: number): Uint8Array;
  /**
   * Generate a random RFC 4122/9562 version 4 UUID string.
   *
   * @since 0.0.0-alpha.0
   */
  export function randomUUID(): string;
  /**
   * Create an incremental `Hash` instance for `algorithm`.
   *
   * @since 0.0.0-alpha.0
   */
  export function createHash(algorithm: string): Hash;
  /**
   * Create an incremental `Hmac` instance for `algorithm` and `key`.
   *
   * @since 0.0.0-alpha.0
   */
  export function createHmac(algorithm: string, key: BinaryLike): Hmac;
  /**
   * Synchronously derive a key using PBKDF2-HMAC.
   *
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
   * @since 0.0.0-alpha.0
   */
  export function timingSafeEqual(a: ArrayBufferView, b: ArrayBufferView): boolean;

  /**
   * Default export mirroring `node:crypto` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    randomBytes: typeof randomBytes;
    randomUUID: typeof randomUUID;
    createHash: typeof createHash;
    createHmac: typeof createHmac;
    pbkdf2Sync: typeof pbkdf2Sync;
    timingSafeEqual: typeof timingSafeEqual;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

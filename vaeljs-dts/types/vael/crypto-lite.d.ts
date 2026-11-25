/**
 * Synchronous symmetric AES-256 encryption, SHA-2 hashes/HMACs, PBKDF2, bcrypt, and Shamir secret sharing (`vaeljs-lite` profile).
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:crypto" {
  /**
   * Supported SHA-2 hash and HMAC algorithms (case-insensitive).
   *
   * @since 0.0.0-alpha.0
   */
  export type HashAlgorithm = "sha256" | "sha384" | "sha512" | (string & {});

  /**
   * Supported symmetric cipher algorithms (case-insensitive).
   *
   * @since 0.0.0-alpha.0
   */
  export type CipherAlgorithm = "aes-256-cbc" | "aes-256-gcm" | (string & {});

  /**
   * Supported string input encoding (UTF-8 only, case-insensitive).
   *
   * @since 0.0.0-alpha.0
   */
  export type InputEncoding = "utf8" | "utf-8" | (string & {});

  /**
   * Supported digest output encodings (case-insensitive).
   *
   * @since 0.0.0-alpha.0
   */
  export type DigestEncoding = "hex" | "base64" | "base64url" | "latin1" | "binary" | (string & {});

  /**
   * Options accepted by `createEncryptor` and `createDecryptor` (`aes-256-cbc`
   * only; `aes-256-gcm` accepts no options).
   *
   * @since 0.0.0-alpha.0
   */
  export interface CipherOptions {
    /**
     * Enable or disable PKCS#7 padding for `aes-256-cbc` (default: `true`).
     *
     * @since 0.0.0-alpha.0
     */
    padding?: boolean;
  }

  /**
   * Frozen share record produced by `shamirSplit` and consumed by
   * `shamirCombine`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface ShamirShare {
    /**
     * One-based GF(256) share index (`1..=255`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly index: number;
    /**
     * Share bytes (same length as the secret; record property is frozen, array elements remain mutable).
     *
     * @since 0.0.0-alpha.0
     */
    readonly value: Uint8Array;
  }

  /**
   * Error codes reported on `Error.code` by synchronous `vael:crypto` operations
   * in `vaeljs-lite`.
   *
   * @since 0.0.0-alpha.0
   */
  export type CryptoErrorCode =
    | "VAEL_CRYPTO_KEY"
    | "VAEL_CRYPTO_FORMAT"
    | "VAEL_CRYPTO_AUTH"
    | "VAEL_CRYPTO_RANDOM"
    | "VAEL_CRYPTO_LIMIT"
    | "VAEL_CRYPTO_DETACHED"
    | "VAEL_CRYPTO_ALGORITHM"
    | "VAEL_CRYPTO_IV"
    | "VAEL_CRYPTO_TAG"
    | "VAEL_CRYPTO_PADDING"
    | "VAEL_CRYPTO_STATE"
    | "VAEL_CRYPTO_CLOSED"
    | "VAEL_CRYPTO_UNSUPPORTED"
    | "VAEL_CRYPTO_TOO_MANY_OBJECTS"
    | "VAEL_CRYPTO_ENCODING"
    | "VAEL_CRYPTO_DIGESTED"
    | "VAEL_CRYPTO_BUSY"
    | "VAEL_CRYPTO_INTERNAL"
    | "VAEL_CRYPTO_SHAMIR_DUPLICATE"
    | "VAEL_CRYPTO_SHAMIR_LENGTH"
    | "VAEL_CRYPTO_SHAMIR_SHARED";

  /**
   * Bounded, stateful synchronous AES-256-CBC or AES-256-GCM cipher returned
   * by `createEncryptor` or `createDecryptor`.
   *
   * Call `close()` or `dispose()` in a `finally` block (even after `final()`)
   * to stay within the per-runtime open-object (64) and buffered-byte (64 MiB) caps.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Cipher {
    /**
     * Accumulate bytes or a UTF-8 string into the cipher (at most 8 MiB per call).
     * Always returns an empty `Uint8Array`; output is produced by `final()`.
     *
     * @throws {TypeError} on wrong argument types or supplying an encoding with binary input.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
     * @since 0.0.0-alpha.0
     */
    update(data: Uint8Array | string, encoding?: InputEncoding): Uint8Array;

    /**
     * Perform the cipher operation over all buffered input and finalize the
     * cipher. For GCM decryption, verifies the authentication tag before
     * returning any plaintext.
     *
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, `"VAEL_CRYPTO_AUTH"`, `"VAEL_CRYPTO_PADDING"`, `"VAEL_CRYPTO_TAG"`, or `"VAEL_CRYPTO_LIMIT"`.
     * @since 0.0.0-alpha.0
     */
    final(): Uint8Array;

    /**
     * Set GCM associated data (`aes-256-gcm` only, at most once before
     * `update` or `final`; max 8 MiB).
     *
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, `"VAEL_CRYPTO_UNSUPPORTED"`, or `"VAEL_CRYPTO_LIMIT"`.
     * @since 0.0.0-alpha.0
     */
    setAAD(aad: Uint8Array): this;

    /**
     * Set the expected 16-byte GCM authentication tag before `final()`
     * (`aes-256-gcm` decryptors only).
     *
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, `"VAEL_CRYPTO_UNSUPPORTED"`, or `"VAEL_CRYPTO_TAG"`.
     * @since 0.0.0-alpha.0
     */
    setAuthTag(tag: Uint8Array): this;

    /**
     * Return the 16-byte GCM authentication tag produced by `final()`
     * (`aes-256-gcm` encryptors only, after `final()`).
     *
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, or `"VAEL_CRYPTO_UNSUPPORTED"`.
     * @since 0.0.0-alpha.0
     */
    getAuthTag(): Uint8Array;

    /**
     * Enable or disable CBC PKCS#7 padding before `final()` (`aes-256-cbc` only).
     *
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, or `"VAEL_CRYPTO_UNSUPPORTED"`.
     * @since 0.0.0-alpha.0
     */
    setAutoPadding(enabled: boolean): this;

    /**
     * Release this cipher's native slot and buffered bytes. Idempotent.
     * Later calls throw `VAEL_CRYPTO_CLOSED`.
     *
     * @since 0.0.0-alpha.0
     */
    close(): void;

    /**
     * Alias for `close()`. Idempotent.
     *
     * @since 0.0.0-alpha.0
     */
    dispose(): void;
  }

  /**
   * Synchronous incremental SHA-2 hash or HMAC returned by `createHash` or
   * `createHmac`.
   *
   * Open digest objects are capped at 1024 per runtime; call `close()` or
   * `dispose()` if not calling `digest()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Hash {
    /**
     * Add bytes or a UTF-8 string (max 16 MiB per update).
     *
     * @throws {TypeError} on wrong argument types or supplying an encoding with binary input.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_DIGESTED"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
     * @since 0.0.0-alpha.0
     */
    update(data: Uint8Array | string, encoding?: InputEncoding): this;

    /**
     * Consume the hash state and return raw digest bytes.
     *
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_DIGESTED"`, or `"VAEL_CRYPTO_LIMIT"`.
     * @since 0.0.0-alpha.0
     */
    digest(): Uint8Array;
    /**
     * Consume the hash state and return the digest encoded as a string.
     *
     * @throws {TypeError} on unsupported encoding (checked before consuming state).
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_DIGESTED"`, or `"VAEL_CRYPTO_LIMIT"`.
     * @since 0.0.0-alpha.0
     */
    digest(encoding: DigestEncoding): string;

    /**
     * Release native state without producing a digest. Idempotent.
     *
     * @since 0.0.0-alpha.0
     */
    close(): void;

    /**
     * Alias for `close()`. Idempotent.
     *
     * @since 0.0.0-alpha.0
     */
    dispose(): void;
  }

  /**
   * Generate a 32-byte AES-256 key from the operating system CSPRNG.
   *
   * Retire keys well before `2^32` encryptions under AES-GCM.
   *
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code = "VAEL_CRYPTO_RANDOM"` when the CSPRNG fails.
   * @since 0.0.0-alpha.0
   */
  export function generateEnvelopeKey(): Uint8Array;

  /**
   * Synchronously read `length` bytes (`0..=8_388_608`) from the OS CSPRNG.
   *
   * @param length Non-negative integer up to 8 MiB (`8_388_608`).
   * @throws {TypeError} when `length` is not a number or arity is wrong.
   * @throws {RangeError} when `length` is not an integer in `0..=8_388_608`.
   * @throws {Error} with `code = "VAEL_CRYPTO_RANDOM"` when the CSPRNG fails.
   * @since 0.0.0-alpha.0
   */
  export function randomBytes(length: number): Uint8Array;

  /**
   * Synchronously encrypt `plaintext` (max 8 MiB) into a versioned NEO1
   * AES-256-GCM envelope with a fresh random 96-bit nonce.
   *
   * @param key Exactly 32 bytes.
   * @param plaintext Up to 8 MiB (`8_388_608` bytes).
   * @param associatedData Optional authenticated non-encrypted bytes (max 8 MiB).
   * @throws {TypeError} on invalid argument types or arity.
   * @throws {Error} with `code` `"VAEL_CRYPTO_KEY"`, `"VAEL_CRYPTO_RANDOM"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @since 0.0.0-alpha.0
   */
  export function encrypt(
    key: Uint8Array,
    plaintext: Uint8Array,
    associatedData?: Uint8Array,
  ): Uint8Array;

  /**
   * Synchronously verify and decrypt a NEO1 AES-256-GCM envelope.
   *
   * Returns no plaintext when tag verification fails.
   *
   * @param key Exactly 32 bytes.
   * @param envelope Up to 8 MiB + 34 bytes.
   * @param associatedData Must match the value passed to `encrypt`.
   * @throws {TypeError} on invalid argument types or arity.
   * @throws {Error} with `code` `"VAEL_CRYPTO_AUTH"`, `"VAEL_CRYPTO_FORMAT"`, `"VAEL_CRYPTO_KEY"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @since 0.0.0-alpha.0
   */
  export function decrypt(
    key: Uint8Array,
    envelope: Uint8Array,
    associatedData?: Uint8Array,
  ): Uint8Array;

  /**
   * Create a bounded stateful encryptor for `"aes-256-cbc"` or `"aes-256-gcm"`.
   *
   * @param algorithm `"aes-256-cbc"` or `"aes-256-gcm"` (case-insensitive).
   * @param key Exactly 32 bytes.
   * @param iv 16 bytes for CBC, 12 bytes for GCM.
   * @param options CBC only (`{ padding?: boolean }`).
   * @throws {TypeError} on invalid arguments or unsupported options.
   * @throws {Error} with `code` `"VAEL_CRYPTO_ALGORITHM"`, `"VAEL_CRYPTO_KEY"`, `"VAEL_CRYPTO_IV"`, `"VAEL_CRYPTO_DETACHED"`, or `"VAEL_CRYPTO_TOO_MANY_OBJECTS"`.
   * @since 0.0.0-alpha.0
   */
  export function createEncryptor(
    algorithm: CipherAlgorithm,
    key: Uint8Array,
    iv: Uint8Array,
    options?: CipherOptions,
  ): Cipher;

  /**
   * Create a bounded stateful decryptor for `"aes-256-cbc"` or `"aes-256-gcm"`.
   *
   * GCM decryptors must call `setAuthTag(tag)` before `final()`.
   *
   * @param algorithm `"aes-256-cbc"` or `"aes-256-gcm"` (case-insensitive).
   * @param key Exactly 32 bytes.
   * @param iv 16 bytes for CBC, 12 bytes for GCM.
   * @param options CBC only (`{ padding?: boolean }`).
   * @throws {TypeError} on invalid arguments or unsupported options.
   * @throws {Error} with `code` `"VAEL_CRYPTO_ALGORITHM"`, `"VAEL_CRYPTO_KEY"`, `"VAEL_CRYPTO_IV"`, `"VAEL_CRYPTO_DETACHED"`, or `"VAEL_CRYPTO_TOO_MANY_OBJECTS"`.
   * @since 0.0.0-alpha.0
   */
  export function createDecryptor(
    algorithm: CipherAlgorithm,
    key: Uint8Array,
    iv: Uint8Array,
    options?: CipherOptions,
  ): Cipher;

  /**
   * Create a synchronous incremental SHA-2 hash (`"sha256"`, `"sha384"`, or `"sha512"`).
   *
   * @throws {TypeError} on wrong argument type or arity.
   * @throws {Error} with `code` `"VAEL_CRYPTO_ALGORITHM"` or `"VAEL_CRYPTO_TOO_MANY_OBJECTS"`.
   * @since 0.0.0-alpha.0
   */
  export function createHash(algorithm: HashAlgorithm): Hash;

  /**
   * Create a synchronous incremental HMAC (`"sha256"`, `"sha384"`, or `"sha512"`).
   *
   * @param key Key bytes or UTF-8 string (max 1 MiB).
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {Error} with `code` `"VAEL_CRYPTO_ALGORITHM"`, `"VAEL_CRYPTO_TOO_MANY_OBJECTS"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @since 0.0.0-alpha.0
   */
  export function createHmac(algorithm: HashAlgorithm, key: Uint8Array | string): Hash;

  /**
   * Synchronously derive key material using PBKDF2-HMAC-SHA2.
   *
   * @param algorithm `"sha256"`, `"sha384"`, or `"sha512"` (case-insensitive).
   * @param password Bytes or UTF-8 string (max 1 MiB).
   * @param salt Bytes or UTF-8 string (max 1 MiB).
   * @param iterations Integer in `1..=1_000_000`.
   * @param length Output byte length in `1..=1024`.
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {RangeError} when `iterations` or `length` is outside its bounds.
   * @throws {Error} with `code` `"VAEL_CRYPTO_ALGORITHM"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @since 0.0.0-alpha.0
   */
  export function pbkdf2(
    algorithm: HashAlgorithm,
    password: Uint8Array | string,
    salt: Uint8Array | string,
    iterations: number,
    length: number,
  ): Uint8Array;

  /**
   * Synchronously hash a password with bcrypt, returning a 60-character `$2b$`
   * modular-crypt string.
   *
   * @param password Bytes or UTF-8 string (max 72 bytes; never truncated).
   * @param cost Logarithmic work factor integer in `4..=16` (default: `10`).
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {RangeError} when `cost` is not an integer in `4..=16`.
   * @throws {Error} with `code` `"VAEL_CRYPTO_LIMIT"`, `"VAEL_CRYPTO_RANDOM"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @since 0.0.0-alpha.0
   */
  export function bcryptHash(password: Uint8Array | string, cost?: number): string;

  /**
   * Synchronously verify `password` against a `$2a$`/`$2b$`/`$2y$` bcrypt hash
   * in constant time.
   *
   * Passwords over 72 bytes and malformed or over-cost (> 16) hashes return `false`.
   *
   * @since 0.0.0-alpha.0
   */
  export function bcryptVerify(password: Uint8Array | string, hash: string): boolean;

  /**
   * Synchronously split `secret` into `shareCount` GF(256) Shamir shares
   * requiring `threshold` shares to reconstruct.
   *
   * Shares are **not** authenticated. Limits: `secret` `1..=8 MiB`,
   * `2 <= threshold <= shareCount <= 255`, `secret.length * shareCount <= 64 MiB`.
   * Rejects `SharedArrayBuffer`-backed views with `VAEL_CRYPTO_SHAMIR_SHARED`.
   *
   * @since 0.0.0-alpha.0
   */
  export function shamirSplit(
    secret: Uint8Array,
    threshold: number,
    shareCount: number,
  ): ShamirShare[];

  /**
   * Synchronously reconstruct a secret from GF(256) Shamir shares.
   *
   * Shares are **not** authenticated. Limits: `2..=255` shares with identical
   * non-zero length (max 8 MiB per share, 64 MiB total).
   *
   * @param threshold Optional minimum share count (`2..=255`, `<= shares.length`).
   * @since 0.0.0-alpha.0
   */
  export function shamirCombine(shares: readonly ShamirShare[], threshold?: number): Uint8Array;
}

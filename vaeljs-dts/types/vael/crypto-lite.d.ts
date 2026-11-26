/**
 * Synchronous symmetric AES-256 encryption, SHA-2 hashes/HMACs, PBKDF2, bcrypt,
 * and Shamir secret sharing (`vjs-lite` profile).
 *
 * Available in the `vjs-lite` (`vaeljs-lite`) runtime. Exposes the 13
 * synchronous exports from `vael:crypto`; worker-pool `*Async` APIs are only
 * available in `vjs` (`vaeljs`).
 *
 * @example
 * ```ts
 * import { createHash, decrypt, encrypt, generateEnvelopeKey } from "vael:crypto";
 *
 * const key = generateEnvelopeKey();
 * const envelope = encrypt(key, new Uint8Array([1, 2, 3, 4]));
 * const plain = decrypt(key, envelope);
 * const digest = createHash("sha256").update("hello").digest("hex");
 * console.log(plain.length, digest);
 * ```
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
   * in `vjs-lite`.
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
     * @param data Input bytes or UTF-8 string (max 8 MiB per call).
     * @param encoding Optional string input encoding (`"utf8"` / `"utf-8"` when `data` is a string).
     * @returns Always an empty `Uint8Array`; finalized bytes are returned by `final()`.
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
     * @returns Finalized ciphertext or authenticated plaintext bytes.
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, `"VAEL_CRYPTO_AUTH"`, `"VAEL_CRYPTO_PADDING"`, `"VAEL_CRYPTO_TAG"`, or `"VAEL_CRYPTO_LIMIT"`.
     * @since 0.0.0-alpha.0
     */
    final(): Uint8Array;

    /**
     * Set GCM associated data (`aes-256-gcm` only, at most once before
     * `update` or `final`; max 8 MiB).
     *
     * @param aad Associated authenticated data bytes (max 8 MiB).
     * @returns `this` for chaining.
     * @throws {TypeError} when `aad` is not a `Uint8Array` or arity is wrong.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, `"VAEL_CRYPTO_UNSUPPORTED"`, or `"VAEL_CRYPTO_LIMIT"`.
     * @since 0.0.0-alpha.0
     */
    setAAD(aad: Uint8Array): this;

    /**
     * Set the expected 16-byte GCM authentication tag before `final()`
     * (`aes-256-gcm` decryptors only).
     *
     * @param tag Expected 16-byte GCM authentication tag.
     * @returns `this` for chaining.
     * @throws {TypeError} when `tag` is not a `Uint8Array` or arity is wrong.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, `"VAEL_CRYPTO_UNSUPPORTED"`, or `"VAEL_CRYPTO_TAG"`.
     * @since 0.0.0-alpha.0
     */
    setAuthTag(tag: Uint8Array): this;

    /**
     * Return the 16-byte GCM authentication tag produced by `final()`
     * (`aes-256-gcm` encryptors only, after `final()`).
     *
     * @returns The 16-byte GCM authentication tag.
     * @throws {TypeError} when any argument is supplied.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_STATE"`, or `"VAEL_CRYPTO_UNSUPPORTED"`.
     * @since 0.0.0-alpha.0
     */
    getAuthTag(): Uint8Array;

    /**
     * Enable or disable CBC PKCS#7 padding before `final()` (`aes-256-cbc` only).
     *
     * @param enabled Whether PKCS#7 padding is enabled (`true` by default).
     * @returns `this` for chaining.
     * @throws {TypeError} when `enabled` is not a boolean or arity is wrong.
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
     * @param data Input bytes or UTF-8 string (max 16 MiB).
     * @param encoding Optional string encoding (`"utf8"` / `"utf-8"` when `data` is a string).
     * @returns `this` for chaining.
     * @throws {TypeError} on wrong argument types or supplying an encoding with binary input.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_DIGESTED"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
     * @since 0.0.0-alpha.0
     */
    update(data: Uint8Array | string, encoding?: InputEncoding): this;

    /**
     * Consume the hash state and return raw digest bytes.
     *
     * @returns Raw digest bytes.
     * @throws {Error} with `code` `"VAEL_CRYPTO_CLOSED"`, `"VAEL_CRYPTO_DIGESTED"`, or `"VAEL_CRYPTO_LIMIT"`.
     * @since 0.0.0-alpha.0
     */
    digest(): Uint8Array;

    /**
     * Consume the hash state and return the digest encoded as a string.
     *
     * @param encoding Output digest encoding (`"hex"`, `"base64"`, `"base64url"`, `"latin1"`, or `"binary"`).
     * @returns Encoded digest string.
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
   * @returns A fresh 32-byte random `Uint8Array` key.
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code = "VAEL_CRYPTO_RANDOM"` when the CSPRNG fails.
   * @example
   * ```ts
   * import { generateEnvelopeKey } from "vael:crypto";
   *
   * const key = generateEnvelopeKey();
   * console.log(key.byteLength); // 32
   * ```
   * @since 0.0.0-alpha.0
   */
  export function generateEnvelopeKey(): Uint8Array;

  /**
   * Synchronously read `length` bytes (`0..=8_388_608`) from the OS CSPRNG.
   *
   * @param length Non-negative integer up to 8 MiB (`8_388_608`).
   * @returns Random bytes of length `length`.
   * @throws {TypeError} when `length` is not a number or arity is wrong.
   * @throws {RangeError} when `length` is not an integer in `0..=8_388_608`.
   * @throws {Error} with `code = "VAEL_CRYPTO_RANDOM"` when the CSPRNG fails.
   * @example
   * ```ts
   * import { randomBytes } from "vael:crypto";
   *
   * const nonce = randomBytes(12);
   * console.log(nonce.byteLength); // 12
   * ```
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
   * @returns Versioned NEO1 authenticated envelope bytes.
   * @throws {TypeError} on invalid argument types or arity.
   * @throws {Error} with `code` `"VAEL_CRYPTO_KEY"`, `"VAEL_CRYPTO_RANDOM"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @example
   * ```ts
   * import { decrypt, encrypt, generateEnvelopeKey } from "vael:crypto";
   *
   * const key = generateEnvelopeKey();
   * const sealed = encrypt(key, new Uint8Array([10, 20, 30]));
   * console.log(decrypt(key, sealed));
   * ```
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
   * @returns Authenticated plaintext bytes.
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
   * @returns Stateful `Cipher` encryptor instance; call `close()` in `finally`.
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
   * @returns Stateful `Cipher` decryptor instance; call `close()` in `finally`.
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
   * @param algorithm SHA-2 algorithm name (`"sha256"`, `"sha384"`, or `"sha512"`).
   * @returns Incremental `Hash` instance.
   * @throws {TypeError} on wrong argument type or arity.
   * @throws {Error} with `code` `"VAEL_CRYPTO_ALGORITHM"` or `"VAEL_CRYPTO_TOO_MANY_OBJECTS"`.
   * @example
   * ```ts
   * import { createHash } from "vael:crypto";
   *
   * const hex = createHash("sha256").update("vael").digest("hex");
   * console.log(hex);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function createHash(algorithm: HashAlgorithm): Hash;

  /**
   * Create a synchronous incremental HMAC (`"sha256"`, `"sha384"`, or `"sha512"`).
   *
   * @param algorithm SHA-2 algorithm name (`"sha256"`, `"sha384"`, or `"sha512"`).
   * @param key Key bytes or UTF-8 string (max 1 MiB).
   * @returns Incremental `Hash` HMAC instance.
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {Error} with `code` `"VAEL_CRYPTO_ALGORITHM"`, `"VAEL_CRYPTO_TOO_MANY_OBJECTS"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @example
   * ```ts
   * import { createHmac } from "vael:crypto";
   *
   * const mac = createHmac("sha256", "secret-key").update("payload").digest("hex");
   * console.log(mac);
   * ```
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
   * @returns Derived key bytes of length `length`.
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {RangeError} when `iterations` or `length` is outside its bounds.
   * @throws {Error} with `code` `"VAEL_CRYPTO_ALGORITHM"`, `"VAEL_CRYPTO_LIMIT"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @example
   * ```ts
   * import { pbkdf2, randomBytes } from "vael:crypto";
   *
   * const salt = randomBytes(16);
   * const derived = pbkdf2("sha256", "passphrase", salt, 100_000, 32);
   * console.log(derived.byteLength); // 32
   * ```
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
   * @returns 60-character `$2b$` bcrypt hash string.
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {RangeError} when `cost` is not an integer in `4..=16`.
   * @throws {Error} with `code` `"VAEL_CRYPTO_LIMIT"`, `"VAEL_CRYPTO_RANDOM"`, or `"VAEL_CRYPTO_DETACHED"`.
   * @example
   * ```ts
   * import { bcryptHash, bcryptVerify } from "vael:crypto";
   *
   * const hash = bcryptHash("correct horse battery staple", 4);
   * console.log(bcryptVerify("correct horse battery staple", hash)); // true
   * ```
   * @since 0.0.0-alpha.0
   */
  export function bcryptHash(password: Uint8Array | string, cost?: number): string;

  /**
   * Synchronously verify `password` against a `$2a$`/`$2b$`/`$2y$` bcrypt hash
   * in constant time.
   *
   * Passwords over 72 bytes and malformed or over-cost (> 16) hashes return `false`.
   *
   * @param password Candidate password bytes or UTF-8 string.
   * @param hash Modular-crypt bcrypt hash string (`$2a$`, `$2b$`, or `$2y$`).
   * @returns `true` if `password` matches `hash`.
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {Error} with `code = "VAEL_CRYPTO_DETACHED"` when a detached buffer is supplied.
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
   * @param secret Non-empty secret byte slice (`1..=8 MiB`).
   * @param threshold Minimum shares needed for reconstruction (`2..=shareCount`).
   * @param shareCount Total number of shares to produce (`threshold..=255`).
   * @returns Array of `shareCount` frozen `{ index, value }` share records.
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {RangeError} when `threshold` or `shareCount` is out of range.
   * @throws {Error} with `code` `"VAEL_CRYPTO_LIMIT"`, `"VAEL_CRYPTO_RANDOM"`, `"VAEL_CRYPTO_DETACHED"`, or `"VAEL_CRYPTO_SHAMIR_SHARED"`.
   * @example
   * ```ts
   * import { shamirCombine, shamirSplit } from "vael:crypto";
   *
   * const shares = shamirSplit(new Uint8Array([1, 2, 3, 4]), 2, 3);
   * const recovered = shamirCombine([shares[0], shares[2]]);
   * console.log(recovered);
   * ```
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
   * @param shares At least 2 valid `ShamirShare` records with identical non-zero byte lengths.
   * @param threshold Optional minimum share count (`2..=255`, `<= shares.length`).
   * @returns Reconstructed secret bytes.
   * @throws {TypeError} on wrong argument types or arity.
   * @throws {RangeError} when `threshold` or share index is out of range.
   * @throws {Error} with `code` `"VAEL_CRYPTO_LIMIT"`, `"VAEL_CRYPTO_DETACHED"`, `"VAEL_CRYPTO_SHAMIR_DUPLICATE"`, `"VAEL_CRYPTO_SHAMIR_LENGTH"`, or `"VAEL_CRYPTO_SHAMIR_SHARED"`.
   * @since 0.0.0-alpha.0
   */
  export function shamirCombine(shares: readonly ShamirShare[], threshold?: number): Uint8Array;
}

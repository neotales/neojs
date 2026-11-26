/**
 * X25519 binary age v1 encryption, decryption, and keypair utilities.
 *
 * Available in the `vjs` (`vaeljs`) runtime only; absent in `vjs-lite`.
 * Supports single-recipient binary age v1 encryption and authenticated
 * decryption up to 8 MiB of plaintext.
 *
 * @example
 * ```ts
 * import { decrypt, encrypt, generateIdentity } from "vael:age";
 *
 * const { identity, recipient } = generateIdentity();
 * const plaintext = new Uint8Array([115, 101, 99, 114, 101, 116]);
 * const ciphertext = encrypt(recipient, plaintext);
 * const recovered = decrypt(identity, ciphertext);
 * console.log(recovered.length === plaintext.length);
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:age" {
  /**
   * Generated X25519 age keypair in standard Bech32 text form.
   *
   * @since 0.0.0-alpha.0
   */
  export interface AgeIdentity {
    /**
     * Secret identity string (`AGE-SECRET-KEY-1...`). Keep confidential.
     *
     * @since 0.0.0-alpha.0
     */
    identity: string;

    /**
     * Public recipient string (`age1...`).
     *
     * @since 0.0.0-alpha.0
     */
    recipient: string;
  }

  /**
   * Error codes thrown by `vael:age` operations on `Error.code`.
   *
   * @since 0.0.0-alpha.0
   */
  export type AgeErrorCode =
    | "VAEL_AGE_IDENTITY"
    | "VAEL_AGE_RECIPIENT"
    | "VAEL_AGE_UNSUPPORTED"
    | "VAEL_AGE_FORMAT"
    | "VAEL_AGE_IDENTITY_MISMATCH"
    | "VAEL_AGE_AUTH"
    | "VAEL_AGE_DETACHED"
    | "VAEL_AGE_LIMIT"
    | "VAEL_AGE_RANDOM"
    | "VAEL_AGE_INTERNAL";

  /**
   * Generate a new X25519 identity and its matching recipient using the OS CSPRNG.
   *
   * @returns A `{ identity, recipient }` pair (`AGE-SECRET-KEY-1...` / `age1...`).
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code = "VAEL_AGE_RANDOM"` when the OS CSPRNG fails.
   * @example
   * ```ts
   * import { generateIdentity } from "vael:age";
   *
   * const keypair = generateIdentity();
   * console.log(keypair.recipient.startsWith("age1"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function generateIdentity(): AgeIdentity;

  /**
   * Derive the Bech32 X25519 `age1...` recipient for an `AGE-SECRET-KEY-1...`
   * identity.
   *
   * Leading and trailing whitespace is trimmed; input is bounded to 512 bytes.
   * Non-X25519 identities (such as post-quantum keys) fail with
   * `VAEL_AGE_UNSUPPORTED`.
   *
   * @param identity Bech32 X25519 identity string (max 512 bytes).
   * @returns The corresponding `age1...` Bech32 public recipient string.
   * @throws {TypeError} when `identity` is not a string or arity is wrong.
   * @throws {Error} with `code` `"VAEL_AGE_IDENTITY"`, `"VAEL_AGE_UNSUPPORTED"`, or `"VAEL_AGE_INTERNAL"`.
   * @example
   * ```ts
   * import { generateIdentity, identityToRecipient } from "vael:age";
   *
   * const { identity, recipient } = generateIdentity();
   * console.log(identityToRecipient(identity) === recipient);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function identityToRecipient(identity: string): string;

  /**
   * Synchronously encrypt `plaintext` to one X25519 `recipient` as a binary
   * age v1 file.
   *
   * Limits: `plaintext` is at most 8 MiB (`8_388_608` bytes) and ciphertext at
   * most 9 MiB (`9_437_184` bytes). ASCII armor, passphrases, and SSH keys are
   * not supported.
   *
   * @param recipient Bech32 X25519 recipient string (`age1...`).
   * @param plaintext Attached `Uint8Array` of at most 8 MiB.
   * @returns Binary age v1 ciphertext bytes.
   * @throws {TypeError} when an argument has the wrong type or arity is wrong.
   * @throws {Error} with `code` `"VAEL_AGE_RECIPIENT"`, `"VAEL_AGE_UNSUPPORTED"`,
   * `"VAEL_AGE_LIMIT"`, `"VAEL_AGE_RANDOM"`, `"VAEL_AGE_DETACHED"`, or `"VAEL_AGE_INTERNAL"`.
   * @example
   * ```ts
   * import { encrypt, generateIdentity } from "vael:age";
   *
   * const { recipient } = generateIdentity();
   * const ciphertext = encrypt(recipient, new Uint8Array([1, 2, 3, 4]));
   * console.log(ciphertext.byteLength > 0);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function encrypt(recipient: string, plaintext: Uint8Array): Uint8Array;

  /**
   * Synchronously verify and decrypt a binary age v1 file using one X25519
   * `identity`.
   *
   * Authenticates the header MAC and every payload chunk tag before returning
   * plaintext. Limits: `ciphertext` is at most 9 MiB and recovered plaintext
   * at most 8 MiB.
   *
   * @param identity Bech32 X25519 identity string (`AGE-SECRET-KEY-1...`).
   * @param ciphertext Attached `Uint8Array` containing a binary age v1 file.
   * @returns Authenticated plaintext bytes.
   * @throws {TypeError} when an argument has the wrong type or arity is wrong.
   * @throws {Error} with `code` `"VAEL_AGE_AUTH"`, `"VAEL_AGE_FORMAT"`,
   * `"VAEL_AGE_IDENTITY"`, `"VAEL_AGE_IDENTITY_MISMATCH"`, `"VAEL_AGE_UNSUPPORTED"`,
   * `"VAEL_AGE_LIMIT"`, `"VAEL_AGE_DETACHED"`, or `"VAEL_AGE_INTERNAL"`.
   * @example
   * ```ts
   * import { decrypt, encrypt, generateIdentity } from "vael:age";
   *
   * const { identity, recipient } = generateIdentity();
   * const sealed = encrypt(recipient, new Uint8Array([42]));
   * const plain = decrypt(identity, sealed);
   * console.log(plain[0]);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function decrypt(identity: string, ciphertext: Uint8Array): Uint8Array;
}

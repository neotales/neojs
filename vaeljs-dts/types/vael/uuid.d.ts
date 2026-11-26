/**
 * RFC 9562 UUID v4 and v7 generators backed by the operating system CSPRNG.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Both generators are synchronous, take no arguments, and return canonical
 * 36-character lowercase hyphenated UUID strings.
 *
 * @example
 * ```ts
 * import { v4, v7 } from "vael:uuid";
 *
 * console.log("v4:", v4());
 * console.log("v7:", v7());
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:uuid" {
  /**
   * Error codes thrown by `vael:uuid` generators on `Error.code`.
   *
   * @since 0.0.0-alpha.0
   */
  export type UuidErrorCode = "VAEL_UUID_RANDOM" | "VAEL_UUID_CLOCK" | "VAEL_UUID_INTERNAL";

  /**
   * Generate a canonical lowercase RFC 9562 version 4 (random) UUID string
   * (36 characters) using the OS CSPRNG.
   *
   * @returns Canonical 36-character lowercase RFC 9562 UUIDv4 string.
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code = "VAEL_UUID_RANDOM"` when the OS CSPRNG fails.
   * @example
   * ```ts
   * import { v4 } from "vael:uuid";
   *
   * const id = v4();
   * console.log(id.length); // 36
   * ```
   * @since 0.0.0-alpha.0
   */
  export function v4(): string;

  /**
   * Generate a canonical lowercase RFC 9562 version 7 (Unix-time-ordered +
   * random) UUID string (36 characters) using the OS CSPRNG.
   *
   * Embeds the current Unix millisecond timestamp in the high 48 bits; does not
   * promise strict monotonicity within a millisecond or across clock steps.
   * Refuses timestamps before the Unix epoch or `>= 2^48` ms with `VAEL_UUID_CLOCK`.
   *
   * @returns Canonical 36-character lowercase RFC 9562 UUIDv7 string.
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code` `"VAEL_UUID_CLOCK"` or `"VAEL_UUID_RANDOM"`.
   * @example
   * ```ts
   * import { v7 } from "vael:uuid";
   *
   * const id = v7();
   * console.log(id.length); // 36
   * ```
   * @since 0.0.0-alpha.0
   */
  export function v7(): string;
}

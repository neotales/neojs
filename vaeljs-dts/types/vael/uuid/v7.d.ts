/**
 * RFC 9562 version 7 (Unix-time-ordered + random) UUID generator.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Exports the same synchronous CSPRNG-backed `v7` function as `"vael:uuid"`.
 *
 * @example
 * ```ts
 * import v7 from "vael:uuid/v7";
 *
 * console.log(v7());
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:uuid/v7" {
  /**
   * Generate a canonical lowercase RFC 9562 version 7 (Unix-time-ordered +
   * random) UUID string (36 characters) using the OS CSPRNG.
   *
   * Identical function object to `v7` from `"vael:uuid"`.
   *
   * @returns Canonical 36-character lowercase RFC 9562 UUIDv7 string.
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code` `"VAEL_UUID_CLOCK"` or `"VAEL_UUID_RANDOM"`.
   * @example
   * ```ts
   * import { v7 } from "vael:uuid/v7";
   *
   * console.log(v7().length); // 36
   * ```
   * @since 0.0.0-alpha.0
   */
  export function v7(): string;

  /**
   * Default export: identical to `v7`.
   *
   * @since 0.0.0-alpha.0
   */
  export default v7;
}

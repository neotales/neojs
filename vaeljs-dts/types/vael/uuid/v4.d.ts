/**
 * RFC 9562 version 4 (random) UUID generator.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Exports the same synchronous CSPRNG-backed `v4` function as `"vael:uuid"`.
 *
 * @example
 * ```ts
 * import v4 from "vael:uuid/v4";
 *
 * console.log(v4());
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:uuid/v4" {
  /**
   * Generate a canonical lowercase RFC 9562 version 4 (random) UUID string
   * (36 characters) using the OS CSPRNG.
   *
   * Identical function object to `v4` from `"vael:uuid"`.
   *
   * @returns Canonical 36-character lowercase RFC 9562 UUIDv4 string.
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code = "VAEL_UUID_RANDOM"` when the OS CSPRNG fails.
   * @example
   * ```ts
   * import { v4 } from "vael:uuid/v4";
   *
   * console.log(v4().length); // 36
   * ```
   * @since 0.0.0-alpha.0
   */
  export function v4(): string;

  /**
   * Default export: identical to `v4`.
   *
   * @since 0.0.0-alpha.0
   */
  export default v4;
}

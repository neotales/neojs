/**
 * RFC 9562 UUID v4 and v7 generators backed by the operating system CSPRNG.
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
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code = "VAEL_UUID_RANDOM"` when the OS CSPRNG fails.
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
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code` `"VAEL_UUID_CLOCK"` or `"VAEL_UUID_RANDOM"`.
   * @since 0.0.0-alpha.0
   */
  export function v7(): string;
}

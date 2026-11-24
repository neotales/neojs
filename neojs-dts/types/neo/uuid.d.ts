declare module "neo:uuid" {
  /**
   * Error codes thrown by `neo:uuid` generators on `Error.code`.
   */
  export type UuidErrorCode = "NEO_UUID_RANDOM" | "NEO_UUID_CLOCK" | "NEO_UUID_INTERNAL";

  /**
   * Generate a canonical lowercase RFC 9562 version 4 (random) UUID string
   * (36 characters) using the OS CSPRNG.
   *
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code = "NEO_UUID_RANDOM"` when the OS CSPRNG fails.
   */
  export function v4(): string;

  /**
   * Generate a canonical lowercase RFC 9562 version 7 (Unix-time-ordered +
   * random) UUID string (36 characters) using the OS CSPRNG.
   *
   * Embeds the current Unix millisecond timestamp in the high 48 bits; does not
   * promise strict monotonicity within a millisecond or across clock steps.
   * Refuses timestamps before the Unix epoch or `>= 2^48` ms with `NEO_UUID_CLOCK`.
   *
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code` `"NEO_UUID_CLOCK"` or `"NEO_UUID_RANDOM"`.
   */
  export function v7(): string;
}

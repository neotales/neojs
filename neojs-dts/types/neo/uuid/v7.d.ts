declare module "neo:uuid/v7" {
  /**
   * Generate a canonical lowercase RFC 9562 version 7 (Unix-time-ordered +
   * random) UUID string (36 characters) using the OS CSPRNG.
   *
   * Identical function object to `v7` from `"neo:uuid"`.
   *
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code` `"NEO_UUID_CLOCK"` or `"NEO_UUID_RANDOM"`.
   */
  export function v7(): string;

  /** Default export: identical to `v7`. */
  export default v7;
}

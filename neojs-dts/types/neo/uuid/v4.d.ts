declare module "neo:uuid/v4" {
  /**
   * Generate a canonical lowercase RFC 9562 version 4 (random) UUID string
   * (36 characters) using the OS CSPRNG.
   *
   * Identical function object to `v4` from `"neo:uuid"`.
   *
   * @throws {TypeError} when any argument is supplied.
   * @throws {Error} with `code = "NEO_UUID_RANDOM"` when the OS CSPRNG fails.
   */
  export function v4(): string;

  /** Default export: identical to `v4`. */
  export default v4;
}

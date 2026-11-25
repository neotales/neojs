/**
 * Host compilation-target operating system and CPU architecture constants.
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:os" {
  /**
   * Compilation-target operating system (`std::env::consts::OS`).
   *
   * Uses Rust target vocabulary (for example `"linux"`, `"macos"`, `"windows"`)
   * rather than legacy platform names. Constant from build time; performs no
   * runtime host probe.
   *
   * @since 0.0.0-alpha.0
   */
  export const platform: "linux" | "macos" | "windows" | (string & {});

  /**
   * Compilation-target CPU architecture (`std::env::consts::ARCH`).
   *
   * Uses Rust target vocabulary (for example `"x86_64"`, `"aarch64"`).
   * Constant from build time; performs no runtime host probe.
   *
   * @since 0.0.0-alpha.0
   */
  export const arch: "x86_64" | "aarch64" | (string & {});
}

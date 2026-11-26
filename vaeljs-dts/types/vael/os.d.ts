/**
 * Build-target operating system and CPU architecture constants.
 *
 * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`) runtimes.
 * Exposes compilation-target identifiers using standard cross-platform target
 * names (`"darwin"` on macOS, `"linux"`, `"windows"`, `"amd64"`, `"arm64"`,
 * `"386"`). Performs no host probe, reads no environment variables or files, and
 * installs no globals.
 *
 * @example
 * ```ts
 * import { arch, platform } from "vael:os";
 *
 * console.log(`Running on ${platform}/${arch}`);
 * if (platform === "darwin" && arch === "arm64") {
 *   console.log("Apple Silicon target");
 * }
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:os" {
  /**
   * Compilation-target operating system identifier.
   *
   * Uses standard cross-platform target names (`"darwin"` on macOS, `"linux"`,
   * `"windows"`, `"freebsd"`, `"netbsd"`, `"aix"`, `"solaris"`, `"illumos"`,
   * `"android"`) rather than legacy names. Constant from build time; performs
   * no runtime host probe.
   *
   * @example
   * ```ts
   * import { platform } from "vael:os";
   *
   * const sep = platform === "windows" ? "\\" : "/";
   * console.log(sep);
   * ```
   *
   * @since 0.0.0-alpha.0
   */
  export const platform:
    | "darwin"
    | "linux"
    | "windows"
    | "freebsd"
    | "netbsd"
    | "aix"
    | "solaris"
    | "illumos"
    | "android"
    | (string & {});

  /**
   * Compilation-target CPU architecture identifier.
   *
   * Uses normalized OCI/Go-style short architecture names (`"amd64"`,
   * `"arm64"`, `"386"`, `"arm"`, `"riscv64"`, `"ppc64le"`, `"ppc64"`,
   * `"s390x"`, `"loong64"`). Constant from build time; performs no runtime
   * host probe.
   *
   * @example
   * ```ts
   * import { arch } from "vael:os";
   *
   * if (arch === "amd64") {
   *   console.log("64-bit x86 target");
   * }
   * ```
   *
   * @since 0.0.0-alpha.0
   */
  export const arch:
    | "amd64"
    | "arm64"
    | "386"
    | "arm"
    | "riscv64"
    | "ppc64le"
    | "ppc64"
    | "s390x"
    | "loong64"
    | (string & {});
}

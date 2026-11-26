/**
 * Node-compatible operating system metadata utilities.
 *
 * Available in both `vjs` and `vjs-lite` runtimes. Enabled by default unless
 * `--no-node-compat` is passed.
 *
 * @example
 * ```ts
 * import { arch, cpus, homedir, hostname, platform, tmpdir } from "node:os";
 *
 * console.log(platform(), arch(), cpus().length);
 * console.log(hostname(), homedir(), tmpdir());
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:os" {
  /**
   * CPU core descriptor returned in the array from `cpus()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface CpuInfo {
    /**
     * CPU model string (currently `"unknown"` in `vaeljs`).
     *
     * @since 0.0.0-alpha.0
     */
    model: string;
    /**
     * CPU clock speed in MHz (currently `0` in `vaeljs`).
     *
     * @since 0.0.0-alpha.0
     */
    speed: number;
    /**
     * Cumulative CPU time breakdown in milliseconds (zero-initialized in `vaeljs`).
     *
     * @since 0.0.0-alpha.0
     */
    times: {
      /**
       * Milliseconds spent in user mode.
       *
       * @since 0.0.0-alpha.0
       */
      user: number;
      /**
       * Milliseconds spent in nice mode.
       *
       * @since 0.0.0-alpha.0
       */
      nice: number;
      /**
       * Milliseconds spent in system (kernel) mode.
       *
       * @since 0.0.0-alpha.0
       */
      sys: number;
      /**
       * Milliseconds spent in idle mode.
       *
       * @since 0.0.0-alpha.0
       */
      idle: number;
      /**
       * Milliseconds spent servicing hardware interrupts.
       *
       * @since 0.0.0-alpha.0
       */
      irq: number;
    };
  }

  /**
   * Return the operating system platform identifier.
   *
   * @returns Node-compatible platform string such as `"linux"`, `"darwin"`, or `"win32"`.
   * @example
   * ```ts
   * import { platform } from "node:os";
   *
   * console.log(platform());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function platform(): string;
  /**
   * Return the operating system CPU architecture identifier.
   *
   * @returns Node-compatible architecture string such as `"x64"` or `"arm64"`.
   * @example
   * ```ts
   * import { arch } from "node:os";
   *
   * console.log(arch());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function arch(): string;
  /**
   * Operating system end-of-line marker (`"\n"` or `"\r\n"`).
   *
   * @since 0.0.0-alpha.0
   */
  export const EOL: "\n" | "\r\n";
  /**
   * Return the current user's home directory path.
   *
   * @returns Path string of the current user's home directory.
   * @throws {Error} with `code = "ERR_UNAVAILABLE"` or `"EINVAL"` if the home directory cannot be determined or is not valid UTF-8.
   * @example
   * ```ts
   * import { homedir } from "node:os";
   *
   * console.log(homedir());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function homedir(): string;
  /**
   * Return the operating system's default directory for temporary files.
   *
   * @returns Path string of the system temporary directory.
   * @throws {Error} with `code = "EINVAL"` if the temporary directory path is not valid UTF-8.
   * @example
   * ```ts
   * import { tmpdir } from "node:os";
   *
   * console.log(tmpdir());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function tmpdir(): string;
  /**
   * Return the host name of the operating system.
   *
   * @returns Hostname string of the machine.
   * @throws {Error} with `code = "ERR_UNAVAILABLE"` or `"EINVAL"` if querying the hostname fails or returns invalid UTF-8.
   * @example
   * ```ts
   * import { hostname } from "node:os";
   *
   * console.log(hostname());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function hostname(): string;
  /**
   * Return an array of objects containing information about each logical CPU core.
   *
   * @returns Array of `CpuInfo` descriptors matching the host logical CPU count.
   * @example
   * ```ts
   * import { cpus } from "node:os";
   *
   * console.log(`Logical cores: ${cpus().length}`);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function cpus(): CpuInfo[];
  /**
   * Return the CPU endianness (`"BE"` or `"LE"`).
   *
   * @returns `"BE"` for big-endian or `"LE"` for little-endian architectures.
   * @example
   * ```ts
   * import { endianness } from "node:os";
   *
   * console.log(endianness());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function endianness(): "BE" | "LE";
  /**
   * Return the operating system name.
   *
   * @returns OS name such as `"Linux"`, `"Darwin"`, or `"Windows_NT"`.
   * @example
   * ```ts
   * import { type } from "node:os";
   *
   * console.log(type());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function type(): string;
  /**
   * Return the machine type string.
   *
   * @returns Machine hardware identifier such as `"x86_64"` or `"arm64"`.
   * @example
   * ```ts
   * import { machine } from "node:os";
   *
   * console.log(machine());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function machine(): string;

  /**
   * Default export mirroring `node:os` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    /** Return the operating system platform identifier. @since 0.0.0-alpha.0 */
    platform: typeof platform;
    /** Return the operating system CPU architecture identifier. @since 0.0.0-alpha.0 */
    arch: typeof arch;
    /** Operating system end-of-line marker (`"\n"` or `"\r\n"`). @since 0.0.0-alpha.0 */
    EOL: typeof EOL;
    /** Return the current user's home directory path. @since 0.0.0-alpha.0 */
    homedir: typeof homedir;
    /** Return the system temporary directory path. @since 0.0.0-alpha.0 */
    tmpdir: typeof tmpdir;
    /** Return the operating system host name. @since 0.0.0-alpha.0 */
    hostname: typeof hostname;
    /** Return an array of logical CPU core descriptors. @since 0.0.0-alpha.0 */
    cpus: typeof cpus;
    /** Return the CPU endianness (`"BE"` or `"LE"`). @since 0.0.0-alpha.0 */
    endianness: typeof endianness;
    /** Return the operating system name. @since 0.0.0-alpha.0 */
    type: typeof type;
    /** Return the machine hardware type string. @since 0.0.0-alpha.0 */
    machine: typeof machine;
  };
  /**
   * Default export object providing all `node:os` functions and `EOL`.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}

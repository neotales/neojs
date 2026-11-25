/**
 * Node-compatible operating system metadata utilities.
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
    /** @since 0.0.0-alpha.0 */
    model: string;
    /** @since 0.0.0-alpha.0 */
    speed: number;
    /** @since 0.0.0-alpha.0 */
    times: {
      user: number;
      nice: number;
      sys: number;
      idle: number;
      irq: number;
    };
  }

  /** Return the operating system platform identifier. @since 0.0.0-alpha.0 */
  export function platform(): string;
  /** Return the operating system CPU architecture identifier. @since 0.0.0-alpha.0 */
  export function arch(): string;
  /** Operating system end-of-line marker (`"\n"` or `"\r\n"`). @since 0.0.0-alpha.0 */
  export const EOL: "\n" | "\r\n";
  /** Return the current user's home directory path. @since 0.0.0-alpha.0 */
  export function homedir(): string;
  /** Return the operating system's default directory for temporary files. @since 0.0.0-alpha.0 */
  export function tmpdir(): string;
  /** Return the host name of the operating system. @since 0.0.0-alpha.0 */
  export function hostname(): string;
  /** Return an array of objects containing information about each logical CPU core. @since 0.0.0-alpha.0 */
  export function cpus(): CpuInfo[];
  /** Return the CPU endianness (`"BE"` or `"LE"`). @since 0.0.0-alpha.0 */
  export function endianness(): "BE" | "LE";
  /** Return the operating system name. @since 0.0.0-alpha.0 */
  export function type(): string;
  /** Return the machine type string. @since 0.0.0-alpha.0 */
  export function machine(): string;

  /**
   * Default export mirroring `node:os` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    platform: typeof platform;
    arch: typeof arch;
    EOL: typeof EOL;
    homedir: typeof homedir;
    tmpdir: typeof tmpdir;
    hostname: typeof hostname;
    cpus: typeof cpus;
    endianness: typeof endianness;
    type: typeof type;
    machine: typeof machine;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

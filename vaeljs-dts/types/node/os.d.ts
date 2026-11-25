declare module "node:os" {
  export interface CpuInfo {
    model: string;
    speed: number;
    times: {
      user: number;
      nice: number;
      sys: number;
      idle: number;
      irq: number;
    };
  }

  export function platform(): string;
  export function arch(): string;
  export const EOL: "\n" | "\r\n";
  export function homedir(): string;
  export function tmpdir(): string;
  export function hostname(): string;
  export function cpus(): CpuInfo[];
  export function endianness(): "BE" | "LE";
  export function type(): string;
  export function machine(): string;

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
  export default _default;
}

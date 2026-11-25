declare module "node:process" {
  export interface HrTime {
    (time?: [number, number]): [number, number];
    bigint(): bigint;
  }

  export const argv: readonly string[];
  export function cwd(): string;
  export function chdir(directory: string): void;
  export const env: Record<string, string | undefined>;
  export function exit(code?: number): never;
  export let exitCode: number | undefined;
  export const platform: string;
  export const arch: string;
  export const pid: number;
  export const version: string;
  export const versions: Readonly<Record<string, string>>;
  export const hrtime: HrTime;

  const _default: {
    readonly argv: typeof argv;
    cwd: typeof cwd;
    chdir: typeof chdir;
    readonly env: typeof env;
    exit: typeof exit;
    exitCode: number | undefined;
    readonly platform: typeof platform;
    readonly arch: typeof arch;
    readonly pid: typeof pid;
    readonly version: typeof version;
    readonly versions: typeof versions;
    readonly hrtime: typeof hrtime;
  };
  export default _default;
}

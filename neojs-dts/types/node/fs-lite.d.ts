declare module "node:fs" {
  export type TextEncoding = "utf8" | "utf-8" | "hex" | "base64" | "base64url" | "latin1" | "ascii";
  export type PathInput = string | { readonly href: string; readonly protocol: string };

  export interface Stats {
    readonly size: number;
    readonly mtimeMs: number;
    readonly mtime: Date;
    isFile(): boolean;
    isDirectory(): boolean;
    isSymbolicLink(): boolean;
  }

  export interface Dirent {
    readonly name: string;
    isFile(): boolean;
    isDirectory(): boolean;
    isSymbolicLink(): boolean;
  }

  export interface FsConstants {
    readonly F_OK: number;
    readonly R_OK: number;
    readonly W_OK: number;
    readonly X_OK: number;
    readonly COPYFILE_EXCL: number;
    readonly COPYFILE_FICLONE: number;
    readonly COPYFILE_FICLONE_FORCE: number;
  }

  export function readFileSync(path: PathInput, options?: { encoding?: null | undefined } | null): Uint8Array;
  export function readFileSync(path: PathInput, options: TextEncoding | { encoding: TextEncoding }): string;
  export function writeFileSync(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): void;
  export function appendFileSync(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): void;
  export function existsSync(path: PathInput): boolean;
  export function statSync(path: PathInput): Stats;
  export function lstatSync(path: PathInput): Stats;
  export function readdirSync(path: PathInput, options?: { withFileTypes?: false }): string[];
  export function readdirSync(path: PathInput, options: { withFileTypes: true }): Dirent[];
  export function mkdirSync(path: PathInput, options?: { recursive?: boolean }): void;
  export function rmSync(path: PathInput, options?: { recursive?: boolean; force?: boolean }): void;
  export function unlinkSync(path: PathInput): void;
  export function renameSync(oldPath: PathInput, newPath: PathInput): void;
  export function copyFileSync(src: PathInput, dest: PathInput, mode?: number): void;
  export function realpathSync(path: PathInput): string;
  export function mkdtempSync(prefix: string): string;
  export const constants: FsConstants;

  const _default: {
    readFileSync: typeof readFileSync;
    writeFileSync: typeof writeFileSync;
    appendFileSync: typeof appendFileSync;
    existsSync: typeof existsSync;
    statSync: typeof statSync;
    lstatSync: typeof lstatSync;
    readdirSync: typeof readdirSync;
    mkdirSync: typeof mkdirSync;
    rmSync: typeof rmSync;
    unlinkSync: typeof unlinkSync;
    renameSync: typeof renameSync;
    copyFileSync: typeof copyFileSync;
    realpathSync: typeof realpathSync;
    mkdtempSync: typeof mkdtempSync;
    readonly constants: FsConstants;
  };
  export default _default;
}

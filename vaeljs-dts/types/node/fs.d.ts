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

  export interface FsPromises {
    readFile(path: PathInput, options?: { encoding?: null | undefined } | null): Promise<Uint8Array>;
    readFile(path: PathInput, options: TextEncoding | { encoding: TextEncoding }): Promise<string>;
    writeFile(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): Promise<void>;
    appendFile(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): Promise<void>;
    stat(path: PathInput): Promise<Stats>;
    lstat(path: PathInput): Promise<Stats>;
    readdir(path: PathInput, options?: { withFileTypes?: false }): Promise<string[]>;
    readdir(path: PathInput, options: { withFileTypes: true }): Promise<Dirent[]>;
    mkdir(path: PathInput, options?: { recursive?: boolean }): Promise<void>;
    rm(path: PathInput, options?: { recursive?: boolean; force?: boolean }): Promise<void>;
    unlink(path: PathInput): Promise<void>;
    rename(oldPath: PathInput, newPath: PathInput): Promise<void>;
    copyFile(src: PathInput, dest: PathInput, mode?: number): Promise<void>;
    realpath(path: PathInput): Promise<string>;
    mkdtemp(prefix: string): Promise<string>;
    readonly constants: FsConstants;
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
  export const promises: FsPromises;

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
    readonly promises: FsPromises;
  };
  export default _default;
}

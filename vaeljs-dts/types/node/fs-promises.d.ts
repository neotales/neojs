declare module "node:fs/promises" {
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

  export function readFile(path: PathInput, options?: { encoding?: null | undefined } | null): Promise<Uint8Array>;
  export function readFile(path: PathInput, options: TextEncoding | { encoding: TextEncoding }): Promise<string>;
  export function writeFile(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): Promise<void>;
  export function appendFile(path: PathInput, data: string | Uint8Array | ArrayBufferView, options?: TextEncoding | { encoding?: TextEncoding | null }): Promise<void>;
  export function stat(path: PathInput): Promise<Stats>;
  export function lstat(path: PathInput): Promise<Stats>;
  export function readdir(path: PathInput, options?: { withFileTypes?: false }): Promise<string[]>;
  export function readdir(path: PathInput, options: { withFileTypes: true }): Promise<Dirent[]>;
  export function mkdir(path: PathInput, options?: { recursive?: boolean }): Promise<void>;
  export function rm(path: PathInput, options?: { recursive?: boolean; force?: boolean }): Promise<void>;
  export function unlink(path: PathInput): Promise<void>;
  export function rename(oldPath: PathInput, newPath: PathInput): Promise<void>;
  export function copyFile(src: PathInput, dest: PathInput, mode?: number): Promise<void>;
  export function realpath(path: PathInput): Promise<string>;
  export function mkdtemp(prefix: string): Promise<string>;
  export const constants: FsConstants;

  const _default: {
    readFile: typeof readFile;
    writeFile: typeof writeFile;
    appendFile: typeof appendFile;
    stat: typeof stat;
    lstat: typeof lstat;
    readdir: typeof readdir;
    mkdir: typeof mkdir;
    rm: typeof rm;
    unlink: typeof unlink;
    rename: typeof rename;
    copyFile: typeof copyFile;
    realpath: typeof realpath;
    mkdtemp: typeof mkdtemp;
    readonly constants: FsConstants;
  };
  export default _default;
}

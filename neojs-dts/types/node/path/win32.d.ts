declare module "node:path/win32" {
  export interface ParsedPath {
    root: string;
    dir: string;
    base: string;
    ext: string;
    name: string;
  }

  export interface FormatInputPathObject {
    root?: string;
    dir?: string;
    base?: string;
    ext?: string;
    name?: string;
  }

  export interface PlatformPath {
    readonly sep: string;
    readonly delimiter: string;
    join(...paths: string[]): string;
    resolve(...paths: string[]): string;
    normalize(path: string): string;
    relative(from: string, to: string): string;
    dirname(path: string): string;
    basename(path: string, suffix?: string): string;
    extname(path: string): string;
    parse(path: string): ParsedPath;
    format(pathObject: FormatInputPathObject): string;
    isAbsolute(path: string): boolean;
    readonly posix: PlatformPath;
    readonly win32: PlatformPath;
  }

  export const sep: "\\";
  export const delimiter: ";";
  export function join(...paths: string[]): string;
  export function resolve(...paths: string[]): string;
  export function normalize(path: string): string;
  export function relative(from: string, to: string): string;
  export function dirname(path: string): string;
  export function basename(path: string, suffix?: string): string;
  export function extname(path: string): string;
  export function parse(path: string): ParsedPath;
  export function format(pathObject: FormatInputPathObject): string;
  export function isAbsolute(path: string): boolean;
  export const posix: PlatformPath;
  export const win32: PlatformPath;

  const _default: PlatformPath;
  export default _default;
}

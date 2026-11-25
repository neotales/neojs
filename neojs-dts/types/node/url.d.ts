declare module "node:url" {
  export class URLSearchParams {
    constructor(init?: string | URLSearchParams | readonly [string, string][] | Iterable<[string, string]> | Record<string, string>);
    readonly size: number;
    append(name: string, value: string): void;
    delete(name: string, value?: string): void;
    get(name: string): string | null;
    getAll(name: string): string[];
    has(name: string, value?: string): boolean;
    set(name: string, value: string): void;
    sort(): void;
    toString(): string;
    forEach(callback: (value: string, name: string, parent: URLSearchParams) => void, thisArg?: unknown): void;
    keys(): IterableIterator<string>;
    values(): IterableIterator<string>;
    entries(): IterableIterator<[string, string]>;
    [Symbol.iterator](): IterableIterator<[string, string]>;
  }

  export class URL {
    constructor(url: string | URL, base?: string | URL);
    href: string;
    readonly origin: string;
    protocol: string;
    username: string;
    password: string;
    host: string;
    hostname: string;
    port: string;
    pathname: string;
    search: string;
    readonly searchParams: URLSearchParams;
    hash: string;
    toString(): string;
    toJSON(): string;
    static canParse(url: string | URL, base?: string | URL): boolean;
    static parse(url: string | URL, base?: string | URL): URL | null;
  }

  export function fileURLToPath(url: string | URL): string;
  export function pathToFileURL(path: string): URL;

  const _default: {
    fileURLToPath: typeof fileURLToPath;
    pathToFileURL: typeof pathToFileURL;
    URL: typeof URL;
    URLSearchParams: typeof URLSearchParams;
  };
  export default _default;
}

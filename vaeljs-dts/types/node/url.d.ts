/**
 * Node-compatible WHATWG `URL`, `URLSearchParams`, and `file://` path conversion helpers.
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:url" {
  /**
   * Mutable URL query string parameter collection.
   *
   * @since 0.0.0-alpha.0
   */
  export class URLSearchParams {
    /** Construct a `URLSearchParams` instance. @since 0.0.0-alpha.0 */
    constructor(init?: string | URLSearchParams | readonly [string, string][] | Iterable<[string, string]> | Record<string, string>);
    /** Total number of search parameter entries. @since 0.0.0-alpha.0 */
    readonly size: number;
    /** Append a specified key/value pair as a new search parameter. @since 0.0.0-alpha.0 */
    append(name: string, value: string): void;
    /** Remove search parameters matching `name` (and optional `value`). @since 0.0.0-alpha.0 */
    delete(name: string, value?: string): void;
    /** Return the first value associated with `name`, or `null`. @since 0.0.0-alpha.0 */
    get(name: string): string | null;
    /** Return all values associated with `name`. @since 0.0.0-alpha.0 */
    getAll(name: string): string[];
    /** Return `true` if a parameter matching `name` (and optional `value`) exists. @since 0.0.0-alpha.0 */
    has(name: string, value?: string): boolean;
    /** Set the value associated with `name` to `value`, removing duplicates. @since 0.0.0-alpha.0 */
    set(name: string, value: string): void;
    /** Sort all key/value pairs in place by key code units. @since 0.0.0-alpha.0 */
    sort(): void;
    /** Serialize parameters as a URL query string without leading `?`. @since 0.0.0-alpha.0 */
    toString(): string;
    /** Invoke `callback` for each key/value pair in insertion order. @since 0.0.0-alpha.0 */
    forEach(callback: (value: string, name: string, parent: URLSearchParams) => void, thisArg?: unknown): void;
    /** Iterate over all parameter names. @since 0.0.0-alpha.0 */
    keys(): IterableIterator<string>;
    /** Iterate over all parameter values. @since 0.0.0-alpha.0 */
    values(): IterableIterator<string>;
    /** Iterate over all `[name, value]` pairs. @since 0.0.0-alpha.0 */
    entries(): IterableIterator<[string, string]>;
    /** Default `[Symbol.iterator]` yielding `[name, value]` pairs. @since 0.0.0-alpha.0 */
    [Symbol.iterator](): IterableIterator<[string, string]>;
  }

  /**
   * WHATWG URL parser and serializer.
   *
   * @since 0.0.0-alpha.0
   */
  export class URL {
    /** Construct a `URL` from `url` and optional `base`. @since 0.0.0-alpha.0 */
    constructor(url: string | URL, base?: string | URL);
    /** @since 0.0.0-alpha.0 */
    href: string;
    /** @since 0.0.0-alpha.0 */
    readonly origin: string;
    /** @since 0.0.0-alpha.0 */
    protocol: string;
    /** @since 0.0.0-alpha.0 */
    username: string;
    /** @since 0.0.0-alpha.0 */
    password: string;
    /** @since 0.0.0-alpha.0 */
    host: string;
    /** @since 0.0.0-alpha.0 */
    hostname: string;
    /** @since 0.0.0-alpha.0 */
    port: string;
    /** @since 0.0.0-alpha.0 */
    pathname: string;
    /** @since 0.0.0-alpha.0 */
    search: string;
    /** @since 0.0.0-alpha.0 */
    readonly searchParams: URLSearchParams;
    /** @since 0.0.0-alpha.0 */
    hash: string;
    /** Return the serialized URL string (`href`). @since 0.0.0-alpha.0 */
    toString(): string;
    /** Return the serialized URL string for JSON serialization. @since 0.0.0-alpha.0 */
    toJSON(): string;
    /** Return `true` if `url` (with optional `base`) parses as a valid URL. @since 0.0.0-alpha.0 */
    static canParse(url: string | URL, base?: string | URL): boolean;
    /** Parse `url` (with optional `base`), returning `null` instead of throwing if invalid. @since 0.0.0-alpha.0 */
    static parse(url: string | URL, base?: string | URL): URL | null;
  }

  /**
   * Convert a `file://` URL string or `URL` object to a platform filesystem path.
   *
   * @since 0.0.0-alpha.0
   */
  export function fileURLToPath(url: string | URL): string;
  /**
   * Convert a filesystem `path` to a `file://` `URL` object.
   *
   * @since 0.0.0-alpha.0
   */
  export function pathToFileURL(path: string): URL;

  /**
   * Default export mirroring `node:url` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    fileURLToPath: typeof fileURLToPath;
    pathToFileURL: typeof pathToFileURL;
    URL: typeof URL;
    URLSearchParams: typeof URLSearchParams;
  };
  /** @since 0.0.0-alpha.0 */
  export default _default;
}

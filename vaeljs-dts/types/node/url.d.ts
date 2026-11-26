/**
 * Node-compatible WHATWG `URL`, `URLSearchParams`, and `file://` path conversion helpers.
 *
 * Available in both `vjs` and `vjs-lite` runtimes. Enabled by default unless
 * `--no-node-compat` is passed.
 *
 * @example
 * ```ts
 * import { URL, URLSearchParams, fileURLToPath, pathToFileURL } from "node:url";
 *
 * const u = new URL("https://example.com/search?q=vaeljs");
 * u.searchParams.append("mode", "fast");
 * console.log(u.toString());
 *
 * const fileUrl = pathToFileURL("/tmp/example.txt");
 * console.log(fileUrl.href, fileURLToPath(fileUrl));
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "node:url" {
  /**
   * Mutable URL query string parameter collection.
   *
   * Supports `application/x-www-form-urlencoded` parsing/serialization,
   * iteration, and live synchronization when accessed via `URL.searchParams`.
   *
   * @example
   * ```ts
   * import { URLSearchParams } from "node:url";
   *
   * const params = new URLSearchParams({ tag: "alpha", count: "2" });
   * params.append("tag", "beta");
   * console.log(params.getAll("tag")); // ["alpha", "beta"]
   * console.log(params.toString());
   * ```
   *
   * @since 0.0.0-alpha.0
   */
  export class URLSearchParams {
    /**
     * Construct a `URLSearchParams` instance.
     *
     * @param init Query string, existing `URLSearchParams`, array/iterable of `[name, value]` tuples, or key/value record.
     * @throws {TypeError} when a sequence or iterable element is not a `[name, value]` pair of length 2.
     * @since 0.0.0-alpha.0
     */
    constructor(init?: string | URLSearchParams | readonly [string, string][] | Iterable<[string, string]> | Record<string, string>);
    /**
     * Total number of search parameter entries.
     *
     * @since 0.0.0-alpha.0
     */
    readonly size: number;
    /**
     * Append a specified key/value pair as a new search parameter.
     *
     * @param name Parameter name.
     * @param value Parameter value.
     * @since 0.0.0-alpha.0
     */
    append(name: string, value: string): void;
    /**
     * Remove search parameters matching `name` (and optional `value`).
     *
     * @param name Parameter name to remove.
     * @param value Optional value; when provided, only entries matching both `name` and `value` are removed.
     * @since 0.0.0-alpha.0
     */
    delete(name: string, value?: string): void;
    /**
     * Return the first value associated with `name`, or `null`.
     *
     * @param name Parameter name to look up.
     * @returns First matching value, or `null` if not present.
     * @since 0.0.0-alpha.0
     */
    get(name: string): string | null;
    /**
     * Return all values associated with `name`.
     *
     * @param name Parameter name to look up.
     * @returns Array of all matching values in insertion order.
     * @since 0.0.0-alpha.0
     */
    getAll(name: string): string[];
    /**
     * Return `true` if a parameter matching `name` (and optional `value`) exists.
     *
     * @param name Parameter name to test.
     * @param value Optional value to match alongside `name`.
     * @returns `true` if a matching parameter exists, otherwise `false`.
     * @since 0.0.0-alpha.0
     */
    has(name: string, value?: string): boolean;
    /**
     * Set the value associated with `name` to `value`, removing duplicates.
     *
     * @param name Parameter name.
     * @param value New parameter value.
     * @since 0.0.0-alpha.0
     */
    set(name: string, value: string): void;
    /**
     * Sort all key/value pairs in place by key code units.
     *
     * @since 0.0.0-alpha.0
     */
    sort(): void;
    /**
     * Serialize parameters as a URL query string without leading `?`.
     *
     * @returns Form-urlencoded query string.
     * @since 0.0.0-alpha.0
     */
    toString(): string;
    /**
     * Invoke `callback` for each key/value pair in insertion order.
     *
     * @param callback Function called with `(value, name, parent)` for each entry.
     * @param thisArg Optional `this` binding passed to `callback`.
     * @throws {TypeError} when `callback` is not a function.
     * @since 0.0.0-alpha.0
     */
    forEach(callback: (value: string, name: string, parent: URLSearchParams) => void, thisArg?: unknown): void;
    /**
     * Iterate over all parameter names.
     *
     * @returns Iterator yielding each parameter name in order.
     * @since 0.0.0-alpha.0
     */
    keys(): IterableIterator<string>;
    /**
     * Iterate over all parameter values.
     *
     * @returns Iterator yielding each parameter value in order.
     * @since 0.0.0-alpha.0
     */
    values(): IterableIterator<string>;
    /**
     * Iterate over all `[name, value]` pairs.
     *
     * @returns Iterator yielding `[name, value]` pairs in order.
     * @since 0.0.0-alpha.0
     */
    entries(): IterableIterator<[string, string]>;
    /**
     * Default `[Symbol.iterator]` yielding `[name, value]` pairs.
     *
     * @returns Iterator yielding `[name, value]` pairs in order.
     * @since 0.0.0-alpha.0
     */
    [Symbol.iterator](): IterableIterator<[string, string]>;
  }

  /**
   * WHATWG URL parser and serializer.
   *
   * Supports inputs up to 64 KiB without embedded NUL bytes.
   *
   * @example
   * ```ts
   * import { URL } from "node:url";
   *
   * const url = new URL("/docs/intro", "https://vaeljs.dev");
   * console.log(url.origin, url.pathname); // "https://vaeljs.dev" "/docs/intro"
   * ```
   *
   * @since 0.0.0-alpha.0
   */
  export class URL {
    /**
     * Construct a `URL` from `url` and optional `base`.
     *
     * @param url Absolute or relative URL string or `URL` instance.
     * @param base Optional base URL used when `url` is relative.
     * @throws {TypeError} with `code = "ERR_INVALID_URL"` when `url` or `base` is invalid or contains NUL, or when called with no arguments.
     * @throws {RangeError} with `code = "ERR_OUT_OF_RANGE"` when `url` or `base` exceeds 64 KiB.
     * @since 0.0.0-alpha.0
     */
    constructor(url: string | URL, base?: string | URL);
    /**
     * Full serialized URL string; assigning a new URL re-parses the entire URL.
     *
     * @since 0.0.0-alpha.0
     */
    href: string;
    /**
     * Read-only ASCII serialization of the URL's origin (`scheme://host[:port]`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly origin: string;
    /**
     * URL scheme including trailing colon (for example `"https:"`).
     *
     * @since 0.0.0-alpha.0
     */
    protocol: string;
    /**
     * Percent-encoded username portion of the URL.
     *
     * @since 0.0.0-alpha.0
     */
    username: string;
    /**
     * Percent-encoded password portion of the URL.
     *
     * @since 0.0.0-alpha.0
     */
    password: string;
    /**
     * Hostname and optional port (for example `"example.com:8080"`).
     *
     * @since 0.0.0-alpha.0
     */
    host: string;
    /**
     * Hostname without port (for example `"example.com"`).
     *
     * @since 0.0.0-alpha.0
     */
    hostname: string;
    /**
     * Decimal port string, or `""` when using the default port or none.
     *
     * @since 0.0.0-alpha.0
     */
    port: string;
    /**
     * Path component starting with `"/"` when hierarchical (for example `"/a/b"`).
     *
     * @since 0.0.0-alpha.0
     */
    pathname: string;
    /**
     * Query component including leading `"?"`, or `""` when empty.
     *
     * @since 0.0.0-alpha.0
     */
    search: string;
    /**
     * Live `URLSearchParams` view synchronized with `search`.
     *
     * @since 0.0.0-alpha.0
     */
    readonly searchParams: URLSearchParams;
    /**
     * Fragment identifier including leading `"#"`, or `""` when empty.
     *
     * @since 0.0.0-alpha.0
     */
    hash: string;
    /**
     * Return the serialized URL string (`href`).
     *
     * @returns Serialized URL string.
     * @since 0.0.0-alpha.0
     */
    toString(): string;
    /**
     * Return the serialized URL string for JSON serialization.
     *
     * @returns Serialized URL string (`href`).
     * @since 0.0.0-alpha.0
     */
    toJSON(): string;
    /**
     * Return `true` if `url` (with optional `base`) parses as a valid URL.
     *
     * @param url Candidate URL string or `URL` instance.
     * @param base Optional base URL string or `URL` instance.
     * @returns `true` if parsing succeeds, otherwise `false`.
     * @example
     * ```ts
     * import { URL } from "node:url";
     *
     * console.log(URL.canParse("https://vaeljs.dev")); // true
     * console.log(URL.canParse("not a url")); // false
     * ```
     * @since 0.0.0-alpha.0
     */
    static canParse(url: string | URL, base?: string | URL): boolean;
    /**
     * Parse `url` (with optional `base`), returning `null` instead of throwing if invalid.
     *
     * @param url Candidate URL string or `URL` instance.
     * @param base Optional base URL string or `URL` instance.
     * @returns Parsed `URL` instance, or `null` if invalid.
     * @example
     * ```ts
     * import { URL } from "node:url";
     *
     * const maybeUrl = URL.parse("/path", "https://vaeljs.dev");
     * console.log(maybeUrl?.href);
     * ```
     * @since 0.0.0-alpha.0
     */
    static parse(url: string | URL, base?: string | URL): URL | null;
  }

  /**
   * Convert a `file://` URL string or `URL` object to a platform filesystem path.
   *
   * Enforces strict `file:` scheme, host, NUL-byte, and percent-encoded
   * separator (`%2f` / `%5c`) validation (max 64 KiB).
   *
   * @param url `file://` URL string or `URL` instance.
   * @returns Platform-native filesystem path string.
   * @throws {TypeError} when `url` is not a string or `URL`.
   * @throws {Error} with `code = "ERR_INVALID_URL"` when `url` is malformed, uses a non-`file:` scheme, specifies an invalid host, or contains encoded path separators.
   * @throws {RangeError} with `code = "ERR_OUT_OF_RANGE"` when `url` exceeds 64 KiB.
   * @example
   * ```ts
   * import { fileURLToPath } from "node:url";
   *
   * console.log(fileURLToPath("file:///tmp/demo.txt"));
   * ```
   * @since 0.0.0-alpha.0
   */
  export function fileURLToPath(url: string | URL): string;
  /**
   * Convert a filesystem `path` to a `file://` `URL` object.
   *
   * @param path Filesystem path string (max 64 KiB).
   * @returns `URL` instance representing the `file://` URL for `path`.
   * @throws {TypeError} when `path` is not a string.
   * @throws {Error} with `code = "EINVAL"` when `path` is invalid or contains a NUL byte.
   * @throws {RangeError} with `code = "ERR_OUT_OF_RANGE"` when `path` exceeds 64 KiB.
   * @example
   * ```ts
   * import { pathToFileURL } from "node:url";
   *
   * const u = pathToFileURL("/tmp/demo.txt");
   * console.log(u.href);
   * ```
   * @since 0.0.0-alpha.0
   */
  export function pathToFileURL(path: string): URL;

  /**
   * Default export mirroring `node:url` named exports.
   *
   * @since 0.0.0-alpha.0
   */
  const _default: {
    /** Convert a `file://` URL to a platform filesystem path. @since 0.0.0-alpha.0 */
    fileURLToPath: typeof fileURLToPath;
    /** Convert a filesystem path to a `file://` `URL`. @since 0.0.0-alpha.0 */
    pathToFileURL: typeof pathToFileURL;
    /** WHATWG `URL` parser and serializer class. @since 0.0.0-alpha.0 */
    URL: typeof URL;
    /** Mutable URL query string parameter collection class. @since 0.0.0-alpha.0 */
    URLSearchParams: typeof URLSearchParams;
  };
  /**
   * Default export object providing `{ fileURLToPath, pathToFileURL, URL, URLSearchParams }`.
   *
   * @since 0.0.0-alpha.0
   */
  export default _default;
}

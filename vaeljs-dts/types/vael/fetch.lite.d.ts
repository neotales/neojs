/**
 * Synchronous SSRF-hardened HTTP/HTTPS client and streaming atomic file
 * downloader (`vjs-lite` profile).
 *
 * Available in the `vjs-lite` (`vaeljs-lite`) runtime. Exposes the synchronous
 * `fetchSync`, `downloadSync`, and `FetchError` exports from `vael:fetch`;
 * worker-thread async APIs are only available in `vjs` (`vaeljs`). Never
 * installed on `globalThis` — import explicitly from `"vael:fetch"`.
 *
 * Enforces `http:`/`https:`-only URLs without embedded credentials, blocks
 * private, loopback, link-local, and cloud-metadata IP ranges unless
 * `allowPrivateIps: true` is specified, validates every redirect hop and strips
 * sensitive credentials across origins, and bounds response/download memory.
 *
 * @example
 * ```ts
 * import { downloadSync, fetchSync } from "vael:fetch";
 *
 * const res = fetchSync("https://example.com/health", {
 *   timeoutMs: 10_000,
 * });
 * if (res.ok) {
 *   console.log(res.status, res.text());
 * }
 *
 * const dl = downloadSync("https://example.com/archive.tar.gz", "./archive.tar.gz", {
 *   overwrite: true,
 * });
 * console.log(dl.bytesWritten, dl.sha256);
 * ```
 *
 * @since 0.0.0-alpha.2
 */
declare module "vael:fetch" {
  /**
   * Stable machine-readable `VAEL_FETCH_*` error codes exposed on
   * `FetchError.code`.
   *
   * @since 0.0.0-alpha.2
   */
  export type FetchErrorCode =
    | "VAEL_FETCH_INVALID_URL"
    | "VAEL_FETCH_INVALID_OPTION"
    | "VAEL_FETCH_SSRF_BLOCKED"
    | "VAEL_FETCH_DNS_TIMEOUT"
    | "VAEL_FETCH_TIMEOUT"
    | "VAEL_FETCH_TOO_MANY_REDIRECTS"
    | "VAEL_FETCH_BODY_TOO_LARGE"
    | "VAEL_FETCH_HTTP_STATUS"
    | "VAEL_FETCH_CHECKSUM_MISMATCH"
    | "VAEL_FETCH_ALREADY_EXISTS"
    | "VAEL_FETCH_NETWORK"
    | "VAEL_FETCH_IO"
    | "VAEL_FETCH_INVALID_UTF8"
    | "VAEL_FETCH_INVALID_JSON";

  /**
   * Supported HTTP request methods accepted by `FetchOptions.method`.
   *
   * @since 0.0.0-alpha.2
   */
  export type HttpMethod = "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS";

  /**
   * Redacted HTTP client or download error carrying a stable `VAEL_FETCH_*`
   * error code.
   *
   * Error messages redact URL query strings, embedded credentials, bearer
   * tokens, and sensitive header values.
   *
   * @since 0.0.0-alpha.2
   */
  export class FetchError extends Error {
    /**
     * Error name (`"FetchError"`).
     *
     * @since 0.0.0-alpha.2
     */
    override name: "FetchError";

    /**
     * Machine-readable `VAEL_FETCH_*` error classification.
     *
     * @since 0.0.0-alpha.2
     */
    code: FetchErrorCode;

    /**
     * HTTP status code when the error resulted from a non-2xx download status
     * (`VAEL_FETCH_HTTP_STATUS`).
     *
     * @since 0.0.0-alpha.2
     */
    readonly status?: number;

    /**
     * Redacted target URL (`scheme://host[:port]/path`) associated with the
     * failure, when applicable.
     *
     * @since 0.0.0-alpha.2
     */
    readonly url?: string;

    /**
     * Construct a `FetchError`.
     *
     * @param code Machine-readable `FetchErrorCode`.
     * @param message Human-readable redacted error message.
     * @since 0.0.0-alpha.2
     */
    constructor(code: FetchErrorCode, message: string);
  }

  /**
   * Request options accepted by `fetchSync`.
   *
   * `body` and `json` are mutually exclusive; specifying both throws
   * `FetchError` with code `"VAEL_FETCH_INVALID_OPTION"`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface FetchOptions {
    /**
     * HTTP request method (case-insensitive, default `"GET"`).
     *
     * @since 0.0.0-alpha.2
     */
    method?: HttpMethod;

    /**
     * Additional HTTP request headers as a plain object or `[name, value]` pairs.
     * Header names must be valid RFC 7230 tokens and values must not contain
     * CR, LF, NUL, or ASCII control characters.
     *
     * @since 0.0.0-alpha.2
     */
    headers?: Record<string, string> | ReadonlyArray<readonly [string, string]>;

    /**
     * Query parameters safely URL-encoded and appended to the request URL.
     *
     * @since 0.0.0-alpha.2
     */
    query?: Record<string, string> | ReadonlyArray<readonly [string, string]>;

    /**
     * Bearer token sent as `Authorization: Bearer <token>` when no explicit
     * `Authorization` header is present. Stripped on cross-origin redirects.
     *
     * @since 0.0.0-alpha.2
     */
    bearerToken?: string;

    /**
     * Raw UTF-8 text or binary request body. Mutually exclusive with `json`,
     * and rejected on `GET` and `HEAD` requests.
     *
     * @since 0.0.0-alpha.2
     */
    body?: string | Uint8Array;

    /**
     * JSON-serializable request payload. Mutually exclusive with `body`,
     * rejected on `GET` and `HEAD` requests, and sets
     * `Content-Type: application/json; charset=utf-8` when absent.
     *
     * @since 0.0.0-alpha.2
     */
    json?: unknown;

    /**
     * Overall request deadline in milliseconds across redirects and body read
     * (default `30_000`, must be `> 0` and `<= 600_000`).
     *
     * @since 0.0.0-alpha.2
     */
    timeoutMs?: number;

    /**
     * Maximum buffered response body size in bytes (default `16_777_216` /
     * 16 MiB, must be `> 0` and `<= 268_435_456` / 256 MiB).
     *
     * @since 0.0.0-alpha.2
     */
    maxResponseBytes?: number;

    /**
     * Maximum number of HTTP redirects (`301`, `302`, `303`, `307`, `308`)
     * to follow (default `10`, maximum `20`).
     *
     * @since 0.0.0-alpha.2
     */
    maxRedirects?: number;

    /**
     * Allow connections to private, loopback, link-local, or cloud-metadata
     * IP addresses (default `false`, SSRF protection active).
     *
     * @since 0.0.0-alpha.2
     */
    allowPrivateIps?: boolean;

    /**
     * Custom `User-Agent` header value, or `null` to omit the default
     * `vaeljs/<version>` header.
     *
     * @since 0.0.0-alpha.2
     */
    userAgent?: string | null;
  }

  /**
   * Buffered HTTP response returned by `fetchSync`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface FetchResponse {
    /**
     * HTTP status code (for example `200` or `404`).
     *
     * @since 0.0.0-alpha.2
     */
    readonly status: number;

    /**
     * HTTP status reason phrase (for example `"OK"`).
     *
     * @since 0.0.0-alpha.2
     */
    readonly statusText: string;

    /**
     * `true` when `status` is in the `200..=299` range.
     *
     * @since 0.0.0-alpha.2
     */
    readonly ok: boolean;

    /**
     * Final resolved URL after following any redirects (without fragment).
     *
     * @since 0.0.0-alpha.2
     */
    readonly url: string;

    /**
     * `true` if at least one HTTP redirect hop was followed.
     *
     * @since 0.0.0-alpha.2
     */
    readonly redirected: boolean;

    /**
     * Lowercased response header map.
     *
     * @since 0.0.0-alpha.2
     */
    readonly headers: Readonly<Record<string, string>>;

    /**
     * Buffered response body bytes (bounded by `maxResponseBytes`).
     *
     * @since 0.0.0-alpha.2
     */
    readonly body: Uint8Array;

    /**
     * Look up the first header value matching `name` case-insensitively.
     *
     * @param name HTTP header field name.
     * @returns Header value, or `undefined` if absent.
     * @since 0.0.0-alpha.2
     */
    header(name: string): string | undefined;

    /**
     * Return all response header values matching `name` case-insensitively in
     * emission order (useful for multi-value headers like `set-cookie`).
     *
     * @param name HTTP header field name.
     * @returns Array of matching header values.
     * @since 0.0.0-alpha.2
     */
    headerValues(name: string): string[];

    /**
     * Decode `body` as strict UTF-8 text.
     *
     * @returns Decoded UTF-8 response body string.
     * @throws {FetchError} with code `"VAEL_FETCH_INVALID_UTF8"` if `body` is not valid UTF-8.
     * @since 0.0.0-alpha.2
     */
    text(): string;

    /**
     * Parse `body` as JSON.
     *
     * @returns Parsed JSON value.
     * @throws {FetchError} with code `"VAEL_FETCH_INVALID_JSON"` if `body` is not valid JSON.
     * @since 0.0.0-alpha.2
     */
    json<T = unknown>(): T;
  }

  /**
   * Incremental streaming download progress snapshot passed to
   * `DownloadOptions.onProgress`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface DownloadProgress {
    /**
     * Total bytes written to the staging temporary file so far.
     *
     * @since 0.0.0-alpha.2
     */
    readonly bytesWritten: number;

    /**
     * Declared `Content-Length` byte count from the server, or `null` if absent.
     *
     * @since 0.0.0-alpha.2
     */
    readonly totalBytes: number | null;
  }

  /**
   * Options accepted by `downloadSync`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface DownloadOptions {
    /**
     * Additional HTTP request headers as a plain object or `[name, value]` pairs.
     *
     * @since 0.0.0-alpha.2
     */
    headers?: Record<string, string> | ReadonlyArray<readonly [string, string]>;

    /**
     * Query parameters safely URL-encoded and appended to the request URL.
     *
     * @since 0.0.0-alpha.2
     */
    query?: Record<string, string> | ReadonlyArray<readonly [string, string]>;

    /**
     * Bearer token sent as `Authorization: Bearer <token>` when no explicit
     * `Authorization` header is present. Stripped on cross-origin redirects.
     *
     * @since 0.0.0-alpha.2
     */
    bearerToken?: string;

    /**
     * Overall download deadline in milliseconds across redirects and disk
     * streaming (default `30_000`, must be `> 0` and `<= 600_000`).
     *
     * @since 0.0.0-alpha.2
     */
    timeoutMs?: number;

    /**
     * Maximum number of HTTP redirects to follow (default `10`, maximum `20`).
     *
     * @since 0.0.0-alpha.2
     */
    maxRedirects?: number;

    /**
     * Allow connections to private, loopback, link-local, or cloud-metadata
     * IP addresses (default `false`, SSRF protection active).
     *
     * @since 0.0.0-alpha.2
     */
    allowPrivateIps?: boolean;

    /**
     * Custom `User-Agent` header value, or `null` to omit the default
     * `vaeljs/<version>` header.
     *
     * @since 0.0.0-alpha.2
     */
    userAgent?: string | null;

    /**
     * Maximum bytes allowed to be written to disk (default `268_435_456` /
     * 256 MiB, must be `> 0` and `<= 4_294_967_296` / 4 GiB).
     *
     * @since 0.0.0-alpha.2
     */
    maxBytes?: number;

    /**
     * Optional 64-character ASCII hexadecimal SHA-256 digest verified in
     * constant time before committing the downloaded file.
     *
     * @since 0.0.0-alpha.2
     */
    expectedSha256?: string;

    /**
     * Atomically replace `destPath` if it already exists (default `false`,
     * which fails with `"VAEL_FETCH_ALREADY_EXISTS"`).
     *
     * @since 0.0.0-alpha.2
     */
    overwrite?: boolean;

    /**
     * Optional callback invoked after each streamed chunk is written to the
     * staging file.
     *
     * @param progress Current written byte count and optional total length.
     * @since 0.0.0-alpha.2
     */
    onProgress?: (progress: DownloadProgress) => void;
  }

  /**
   * Completion metadata returned by `downloadSync`.
   *
   * @since 0.0.0-alpha.2
   */
  export interface DownloadResult {
    /**
     * Destination filesystem path written atomically.
     *
     * @since 0.0.0-alpha.2
     */
    readonly path: string;

    /**
     * HTTP status code (`200..=299`).
     *
     * @since 0.0.0-alpha.2
     */
    readonly status: number;

    /**
     * Final resolved URL after following any redirects (without fragment).
     *
     * @since 0.0.0-alpha.2
     */
    readonly url: string;

    /**
     * Total number of bytes written to `path`.
     *
     * @since 0.0.0-alpha.2
     */
    readonly bytesWritten: number;

    /**
     * Lowercase 64-character hexadecimal SHA-256 digest of the downloaded file.
     *
     * @since 0.0.0-alpha.2
     */
    readonly sha256: string;

    /**
     * Response `Content-Type` header value, or `null` if absent.
     *
     * @since 0.0.0-alpha.2
     */
    readonly contentType: string | null;

    /**
     * Response `ETag` header value, or `null` if absent.
     *
     * @since 0.0.0-alpha.2
     */
    readonly etag: string | null;
  }

  /**
   * Perform a synchronous SSRF-hardened HTTP/HTTPS request and buffer the
   * bounded response.
   *
   * Available in both `vjs` (`vaeljs`) and `vjs-lite` (`vaeljs-lite`).
   *
   * @param url Target `http:` or `https:` URL without embedded userinfo.
   * @param options Optional request method, headers, query, body, and limits.
   * @returns Buffered `FetchResponse`.
   * @throws {TypeError} on wrong argument types.
   * @throws {FetchError} on invalid URL/options, SSRF block, timeout, redirect limit, body limit, or network error.
   * @example
   * ```ts
   * import { fetchSync } from "vael:fetch";
   *
   * const res = fetchSync("https://example.com/health");
   * console.log(res.ok, res.status);
   * ```
   * @since 0.0.0-alpha.2
   */
  export function fetchSync(url: string, options?: FetchOptions): FetchResponse;

  /**
   * Synchronously stream an HTTP/HTTPS `GET` response directly to a sibling
   * temporary file, verify optional SHA-256 integrity, and atomically commit
   * to `destPath`.
   *
   * Non-2xx status codes throw `FetchError` (`"VAEL_FETCH_HTTP_STATUS"`)
   * without creating or modifying `destPath`. Available in both `vjs`
   * (`vaeljs`) and `vjs-lite` (`vaeljs-lite`).
   *
   * @param url Target `http:` or `https:` URL without embedded userinfo.
   * @param destPath Destination regular file path (parent must be a non-symlink directory).
   * @param options Optional headers, timeout, size limit, SHA-256 digest, overwrite, and progress callback.
   * @returns `DownloadResult` metadata.
   * @throws {TypeError} on wrong argument types.
   * @throws {FetchError} on invalid URL/options, SSRF block, non-2xx status, checksum mismatch, existing target, or I/O error.
   * @example
   * ```ts
   * import { downloadSync } from "vael:fetch";
   *
   * const res = downloadSync("https://example.com/release.bin", "./release.bin", {
   *   overwrite: true,
   * });
   * console.log(res.bytesWritten, res.sha256);
   * ```
   * @since 0.0.0-alpha.2
   */
  export function downloadSync(
    url: string,
    destPath: string,
    options?: DownloadOptions,
  ): DownloadResult;
}

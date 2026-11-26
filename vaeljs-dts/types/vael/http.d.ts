/**
 * Go-style HTTP/1.1 server, ServeMux router, middleware, and immutable request/response types.
 *
 * Available in the `vjs` (`vaeljs`) runtime only; absent in `vjs-lite`.
 * Provides Go 1.22+ `ServeMux` pattern routing, composable middleware,
 * immutable `HttpRequest` / `HttpResponse` objects, zero-socket `.fetch()`
 * dispatch, and a `hyper` / `tokio` HTTP/1.1 server via `serve()`.
 *
 * @example
 * ```ts
 * import { createRouter, json, recover, requestId } from "vael:http";
 *
 * const router = createRouter();
 * router.use(recover(), requestId());
 * router.get("/users/{id}", (req) => {
 *   return json({ id: req.param("id") });
 * });
 *
 * const res = await router.fetch("/users/42");
 * console.log(res.status, res.json());
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:http" {
  /**
   * Read-only URL query or `application/x-www-form-urlencoded` parameter bag.
   *
   * @since 0.0.0-alpha.0
   */
  export interface QueryParams {
    /** Number of parameter entries. @since 0.0.0-alpha.0 */
    readonly size: number;

    /**
     * Return the first value associated with `name`, or `null` if absent.
     *
     * @param name Parameter name.
     * @returns First parameter value, or `null`.
     * @since 0.0.0-alpha.0
     */
    get(name: string): string | null;

    /**
     * Return all values associated with `name` in insertion order.
     *
     * @param name Parameter name.
     * @returns Array of matching parameter values.
     * @since 0.0.0-alpha.0
     */
    getAll(name: string): string[];

    /**
     * Return `true` if `name` is present at least once.
     *
     * @param name Parameter name.
     * @returns `true` if `name` exists.
     * @since 0.0.0-alpha.0
     */
    has(name: string): boolean;

    /**
     * Iterate parameter names in insertion order.
     *
     * @returns Iterator over parameter names.
     * @since 0.0.0-alpha.0
     */
    keys(): IterableIterator<string>;

    /**
     * Iterate parameter values in insertion order.
     *
     * @returns Iterator over parameter values.
     * @since 0.0.0-alpha.0
     */
    values(): IterableIterator<string>;

    /**
     * Iterate `[name, value]` pairs in insertion order.
     *
     * @returns Iterator over `[name, value]` entries.
     * @since 0.0.0-alpha.0
     */
    entries(): IterableIterator<[string, string]>;

    /**
     * Default iterator yielding `[name, value]` pairs in insertion order.
     *
     * @returns Iterator over `[name, value]` entries.
     * @since 0.0.0-alpha.0
     */
    [Symbol.iterator](): IterableIterator<[string, string]>;

    /**
     * Invoke `callback` once for each `[name, value]` entry in insertion order.
     *
     * @param callback Callback receiving `(value, key, parent)`.
     * @since 0.0.0-alpha.0
     */
    forEach(callback: (value: string, key: string, parent: QueryParams) => void): void;

    /**
     * Serialize all parameters as a URL-encoded query string.
     *
     * @returns URL-encoded query string without a leading `"?"`.
     * @since 0.0.0-alpha.0
     */
    toString(): string;
  }

  /**
   * Minimal abort-signal contract accepted by `serve({ signal })`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface AbortSignalLike {
    /** Whether the signal has already been aborted. @since 0.0.0-alpha.0 */
    readonly aborted: boolean;

    /**
     * Register an `"abort"` listener.
     *
     * @param type Event type (`"abort"`).
     * @param listener Callback invoked upon abort.
     * @param options Optional listener options (`{ once }`).
     * @since 0.0.0-alpha.0
     */
    addEventListener(type: "abort", listener: () => void, options?: { once?: boolean }): void;

    /**
     * Unregister a previously registered `"abort"` listener.
     *
     * @param type Event type (`"abort"`).
     * @param listener Listener callback to remove.
     * @since 0.0.0-alpha.0
     */
    removeEventListener?(type: "abort", listener: () => void): void;
  }

  /**
   * HTTP header collection input: either a plain object or ordered `[name, value]` pairs.
   *
   * @since 0.0.0-alpha.0
   */
  export type HeadersInit = Record<string, string | undefined> | ReadonlyArray<readonly [string, string]>;

  /**
   * Options accepted by `response`, `json`, `html`, `text`, and `noContent`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface ResponseInit {
    /** HTTP status code (`100..=599`, default `200`). @since 0.0.0-alpha.0 */
    status?: number;
    /** HTTP response headers, validated against RFC 9110 and CRLF-safe rules. @since 0.0.0-alpha.0 */
    headers?: HeadersInit;
  }

  /**
   * Supported HTTP redirect status codes.
   *
   * @since 0.0.0-alpha.0
   */
  export type RedirectStatus = 301 | 302 | 303 | 307 | 308;

  /**
   * `Set-Cookie` `SameSite` policy attribute.
   *
   * @since 0.0.0-alpha.0
   */
  export type SameSitePolicy = "Lax" | "Strict" | "None";

  /**
   * Options accepted by `setCookie` and `formatSetCookie`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface SetCookieOptions {
    /** Cookie name (RFC 6265 token, no whitespace or separators). @since 0.0.0-alpha.0 */
    name: string;
    /** Cookie value (CRLF-free). @since 0.0.0-alpha.0 */
    value: string;
    /** Optional `Path` cookie attribute. @since 0.0.0-alpha.0 */
    path?: string;
    /** Optional `Domain` cookie attribute. @since 0.0.0-alpha.0 */
    domain?: string;
    /** Optional `Max-Age` attribute in whole seconds. @since 0.0.0-alpha.0 */
    maxAge?: number;
    /** Whether to include the `HttpOnly` attribute. @since 0.0.0-alpha.0 */
    httpOnly?: boolean;
    /** Whether to include the `Secure` attribute. @since 0.0.0-alpha.0 */
    secure?: boolean;
    /** Optional `SameSite` attribute (`"Lax"`, `"Strict"`, or `"None"`). @since 0.0.0-alpha.0 */
    sameSite?: SameSitePolicy;
  }

  /**
   * Immutable incoming HTTP request passed to handlers and middleware.
   *
   * @since 0.0.0-alpha.0
   */
  export class HttpRequest {
    /** Uppercase HTTP request method (for example `"GET"` or `"POST"`). @since 0.0.0-alpha.0 */
    readonly method: string;
    /** Raw request URL or path+query string. @since 0.0.0-alpha.0 */
    readonly url: string;
    /** Normalized request pathname without query string. @since 0.0.0-alpha.0 */
    readonly path: string;
    /** Parsed read-only URL query parameters. @since 0.0.0-alpha.0 */
    readonly query: QueryParams;
    /** Lowercased frozen header map. @since 0.0.0-alpha.0 */
    readonly headers: Readonly<Record<string, string>>;
    /** Matched route path parameters (for example `{ id: "42" }`). @since 0.0.0-alpha.0 */
    readonly params: Readonly<Record<string, string>>;
    /** Parsed request `Cookie` header map. @since 0.0.0-alpha.0 */
    readonly cookies: Readonly<Record<string, string>>;
    /** Remote peer address string (for example `"127.0.0.1:54321"`). @since 0.0.0-alpha.0 */
    readonly remoteAddr: string;

    /**
     * Case-insensitive header lookup.
     *
     * @param name Header field name.
     * @returns Header value, or `undefined` if not present.
     * @since 0.0.0-alpha.0
     */
    header(name: string): string | undefined;

    /**
     * Route path parameter lookup.
     *
     * @param name Route wildcard parameter name.
     * @returns Captured parameter string, or `undefined` if absent.
     * @since 0.0.0-alpha.0
     */
    param(name: string): string | undefined;

    /**
     * Return a fresh copy of the raw request body bytes.
     *
     * @returns Fresh `Uint8Array` copy of the request body.
     * @since 0.0.0-alpha.0
     */
    bodyBytes(): Uint8Array;

    /**
     * Decode the request body as strict UTF-8 text.
     *
     * @returns Promise resolving to the UTF-8 request body string.
     * @throws {HttpError} with `status = 400` on invalid UTF-8 bytes.
     * @since 0.0.0-alpha.0
     */
    text(): Promise<string>;

    /**
     * Decode and parse a JSON request body.
     *
     * @returns Promise resolving to the parsed JSON value.
     * @throws {HttpError} with `status = 400` on invalid UTF-8 or malformed JSON.
     * @since 0.0.0-alpha.0
     */
    json<T = unknown>(): Promise<T>;

    /**
     * Decode an `application/x-www-form-urlencoded` body into `QueryParams`.
     *
     * @returns Promise resolving to parsed form parameters.
     * @throws {HttpError} with `status = 400` on invalid UTF-8 or malformed form data.
     * @since 0.0.0-alpha.0
     */
    form(): Promise<QueryParams>;
  }

  /**
   * Immutable HTTP response descriptor returned by handlers and response builders.
   *
   * @since 0.0.0-alpha.0
   */
  export class HttpResponse {
    /** HTTP status code (`100..=599`). @since 0.0.0-alpha.0 */
    readonly status: number;
    /** Lowercased frozen response header map. @since 0.0.0-alpha.0 */
    readonly headers: Readonly<Record<string, string>>;
    /** Immutable response body bytes. @since 0.0.0-alpha.0 */
    readonly body: Uint8Array;

    /**
     * Case-insensitive lookup for the collapsed header value.
     *
     * @param name Header field name.
     * @returns Header value, or `undefined` if absent.
     * @since 0.0.0-alpha.0
     */
    header(name: string): string | undefined;

    /**
     * Case-insensitive lookup returning all header values for `name` (e.g. `set-cookie`).
     *
     * @param name Header field name.
     * @returns Array of header values in emission order.
     * @since 0.0.0-alpha.0
     */
    headerValues(name: string): string[];

    /**
     * Return a fresh copy of the response body bytes.
     *
     * @returns Fresh `Uint8Array` copy of the response body.
     * @since 0.0.0-alpha.0
     */
    bytes(): Uint8Array;

    /**
     * Decode the response body as strict UTF-8 text.
     *
     * @returns UTF-8 decoded response body string.
     * @throws {TypeError} if the response body is not valid UTF-8.
     * @since 0.0.0-alpha.0
     */
    text(): string;

    /**
     * Decode and parse the response body as JSON.
     *
     * @returns Parsed JSON value.
     * @throws {SyntaxError} if the body is not valid JSON.
     * @since 0.0.0-alpha.0
     */
    json<T = unknown>(): T;
  }

  /**
   * Per-request context passed alongside `HttpRequest` to handlers and middleware.
   *
   * @since 0.0.0-alpha.0
   */
  export interface RequestContext {
    /** Matched route path parameters. @since 0.0.0-alpha.0 */
    readonly params: Readonly<Record<string, string>>;
    /** Mutable per-request state bag shared across middleware and handler. @since 0.0.0-alpha.0 */
    readonly state: Record<string, unknown>;
    /** Remote peer address string. @since 0.0.0-alpha.0 */
    readonly remoteAddr: string;
  }

  /**
   * Pure HTTP handler returning an `HttpResponse` (synchronously or as a `Promise`).
   *
   * @since 0.0.0-alpha.0
   */
  export type Handler = (req: HttpRequest, ctx: RequestContext) => HttpResponse | Promise<HttpResponse>;

  /**
   * Composable middleware wrapping a downstream `Handler`.
   *
   * @since 0.0.0-alpha.0
   */
  export type Middleware = (next: Handler) => Handler;

  /**
   * Structured HTTP error converted into an HTTP response by `ServeMux` and `recover()`.
   *
   * @since 0.0.0-alpha.0
   */
  export class HttpError extends Error {
    /** Error name (`"HttpError"`). @since 0.0.0-alpha.0 */
    override name: "HttpError";
    /** HTTP status code (`400..=599`). @since 0.0.0-alpha.0 */
    status: number;
    /** Optional machine-readable error code. @since 0.0.0-alpha.0 */
    code?: string;
    /** Optional structured error details. @since 0.0.0-alpha.0 */
    details?: unknown;
    /** Optional extra response headers (for example `Allow` on `405`). @since 0.0.0-alpha.0 */
    headers?: Readonly<Record<string, string>>;

    /**
     * Construct an `HttpError`.
     *
     * @param status HTTP status code.
     * @param message Optional error message.
     * @param code Optional machine-readable error code.
     * @param details Optional structured error details.
     * @since 0.0.0-alpha.0
     */
    constructor(status: number, message?: string, code?: string, details?: unknown);
  }

  /**
   * Options accepted by `ServeMux.static(urlPrefix, rootDir, options)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface StaticOptions {
    /** Directory index filename (default `"index.html"`, or `null` to disable directory index). @since 0.0.0-alpha.0 */
    index?: string | null;
  }

  /**
   * Request input accepted by `ServeMux.fetch(input, init)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FetchInput {
    /** Request URL or path+query string. @since 0.0.0-alpha.0 */
    url: string;
    /** HTTP method (default `"GET"`). @since 0.0.0-alpha.0 */
    method?: string;
    /** Request headers. @since 0.0.0-alpha.0 */
    headers?: HeadersInit;
    /** Request body string, bytes, or `null`. @since 0.0.0-alpha.0 */
    body?: string | Uint8Array | null;
    /** Synthetic remote address string for testing. @since 0.0.0-alpha.0 */
    remoteAddr?: string;
  }

  /**
   * Request init overrides accepted by `ServeMux.fetch(input, init)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FetchInit {
    /** HTTP method override (default `"GET"`). @since 0.0.0-alpha.0 */
    method?: string;
    /** Request headers override. @since 0.0.0-alpha.0 */
    headers?: HeadersInit;
    /** Request body override. @since 0.0.0-alpha.0 */
    body?: string | Uint8Array | null;
    /** Synthetic remote address override. @since 0.0.0-alpha.0 */
    remoteAddr?: string;
  }

  /**
   * Go 1.22+ `ServeMux`-style HTTP router with middleware, route groups,
   * traversal-safe static file serving, and zero-socket `.fetch()` testing.
   *
   * @since 0.0.0-alpha.0
   */
  export class ServeMux {
    /**
     * Create an empty `ServeMux` router.
     *
     * @since 0.0.0-alpha.0
     */
    constructor();

    /**
     * Register a Go 1.22+ pattern (`"GET /users/{id}"`, `"/files/{path...}"`, etc.).
     *
     * @param pattern Route pattern string (optional method prefix + path with `{param}` or `{rest...}` wildcards).
     * @param handler Route handler function.
     * @returns `this` for chaining.
     * @throws {TypeError} when `pattern` is invalid or conflicts with an existing route.
     * @since 0.0.0-alpha.0
     */
    handle(pattern: string, handler: Handler): ServeMux;

    /**
     * Register a `GET` (and automatic `HEAD`) route.
     *
     * @param path Route path pattern.
     * @param handler Route handler function.
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    get(path: string, handler: Handler): ServeMux;

    /**
     * Register a `POST` route.
     *
     * @param path Route path pattern.
     * @param handler Route handler function.
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    post(path: string, handler: Handler): ServeMux;

    /**
     * Register a `PUT` route.
     *
     * @param path Route path pattern.
     * @param handler Route handler function.
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    put(path: string, handler: Handler): ServeMux;

    /**
     * Register a `PATCH` route.
     *
     * @param path Route path pattern.
     * @param handler Route handler function.
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    patch(path: string, handler: Handler): ServeMux;

    /**
     * Register a `DELETE` route.
     *
     * @param path Route path pattern.
     * @param handler Route handler function.
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    delete(path: string, handler: Handler): ServeMux;

    /**
     * Attach middleware to this router (or current route group).
     *
     * @param middlewares Middleware functions applied in left-to-right order.
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    use(...middlewares: Middleware[]): ServeMux;

    /**
     * Mount a prefixed route group with optional group-local `.use()` middleware.
     *
     * @param prefix Path prefix for all routes registered inside `fn`.
     * @param fn Builder callback receiving the scoped `ServeMux` group.
     * @returns `this` for chaining.
     * @example
     * ```ts
     * import { createRouter, json } from "vael:http";
     *
     * const router = createRouter();
     * router.group("/api", (api) => {
     *   api.get("/health", () => json({ ok: true }));
     * });
     * ```
     * @since 0.0.0-alpha.0
     */
    group(prefix: string, fn: (sub: ServeMux) => void): ServeMux;

    /**
     * Serve static files from `rootDir` under `urlPrefix` with traversal/symlink protection.
     *
     * @param urlPrefix URL mount prefix (for example `"/assets"`).
     * @param rootDir Local filesystem directory root.
     * @param options Optional static serving options (`{ index }`).
     * @returns `this` for chaining.
     * @since 0.0.0-alpha.0
     */
    static(urlPrefix: string, rootDir: string, options?: StaticOptions): ServeMux;

    /**
     * Dispatch a direct in-memory request through the router with zero sockets.
     *
     * @param input Request URL string or `FetchInput` descriptor.
     * @param init Optional request method, headers, body, and remote address overrides.
     * @returns Promise resolving to the produced `HttpResponse`.
     * @example
     * ```ts
     * import { createRouter, text } from "vael:http";
     *
     * const router = createRouter();
     * router.get("/ping", () => text("pong"));
     * const res = await router.fetch("/ping");
     * console.log(res.status, res.text());
     * ```
     * @since 0.0.0-alpha.0
     */
    fetch(input: string | FetchInput, init?: FetchInit): Promise<HttpResponse>;
  }

  /**
   * Create a new `ServeMux` router instance.
   *
   * @returns New empty `ServeMux` router.
   * @example
   * ```ts
   * import { createRouter, json } from "vael:http";
   *
   * const router = createRouter();
   * router.get("/", () => json({ hello: "vael" }));
   * const res = await router.fetch("/");
   * console.log(res.json());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function createRouter(): ServeMux;

  /**
   * Create an immutable `HttpResponse`.
   *
   * @param body Optional response body string, bytes, or `null`.
   * @param init Optional status code and headers.
   * @returns Immutable `HttpResponse`.
   * @throws {TypeError} on invalid status code or unsafe header values.
   * @since 0.0.0-alpha.0
   */
  export function response(body?: string | Uint8Array | null, init?: ResponseInit): HttpResponse;

  /**
   * Create an immutable JSON `HttpResponse` (`content-type: application/json; charset=utf-8`).
   *
   * @param data JSON-serializable value.
   * @param init Optional status code and headers.
   * @returns Immutable JSON `HttpResponse`.
   * @example
   * ```ts
   * import { json } from "vael:http";
   *
   * const res = json({ ok: true }, { status: 201 });
   * console.log(res.status, res.text());
   * ```
   * @since 0.0.0-alpha.0
   */
  export function json(data: unknown, init?: ResponseInit): HttpResponse;

  /**
   * Create an immutable HTML `HttpResponse` (`content-type: text/html; charset=utf-8`).
   *
   * @param body HTML body string.
   * @param init Optional status code and headers.
   * @returns Immutable HTML `HttpResponse`.
   * @since 0.0.0-alpha.0
   */
  export function html(body: string, init?: ResponseInit): HttpResponse;

  /**
   * Create an immutable plain-text `HttpResponse` (`content-type: text/plain; charset=utf-8`).
   *
   * @param body Plain-text body string.
   * @param init Optional status code and headers.
   * @returns Immutable plain-text `HttpResponse`.
   * @since 0.0.0-alpha.0
   */
  export function text(body: string, init?: ResponseInit): HttpResponse;

  /**
   * Create an immutable redirect `HttpResponse` (default `302`).
   *
   * @param location Target `Location` header URL or path (CRLF-free).
   * @param status Redirect status code (`301`, `302`, `303`, `307`, or `308`, default `302`).
   * @returns Immutable redirect `HttpResponse`.
   * @throws {TypeError} on invalid status code or unsafe `location`.
   * @since 0.0.0-alpha.0
   */
  export function redirect(location: string, status?: RedirectStatus): HttpResponse;

  /**
   * Create an immutable `204 No Content` `HttpResponse`.
   *
   * @param init Optional status/headers overrides (defaults to status `204`).
   * @returns Immutable empty `HttpResponse`.
   * @since 0.0.0-alpha.0
   */
  export function noContent(init?: ResponseInit): HttpResponse;

  /**
   * Return a new `HttpResponse` with an RFC 6265 / CRLF-safe `Set-Cookie` header appended.
   *
   * @param res Base immutable `HttpResponse`.
   * @param cookie Cookie name, value, and attributes.
   * @returns New `HttpResponse` with the `Set-Cookie` header appended.
   * @throws {TypeError} when cookie name, value, or attributes fail validation.
   * @since 0.0.0-alpha.0
   */
  export function setCookie(res: HttpResponse, cookie: SetCookieOptions): HttpResponse;

  /**
   * Construct an `HttpError`.
   *
   * @param status HTTP status code (`400..=599`).
   * @param message Optional error message.
   * @param code Optional machine-readable error code.
   * @param details Optional structured error details.
   * @returns New `HttpError` instance.
   * @since 0.0.0-alpha.0
   */
  export function httpError(status: number, message?: string, code?: string, details?: unknown): HttpError;

  /**
   * Construct a `400 Bad Request` `HttpError`.
   *
   * @param message Optional error message.
   * @param code Optional machine-readable error code.
   * @returns `HttpError` with `status = 400`.
   * @since 0.0.0-alpha.0
   */
  export function badRequest(message?: string, code?: string): HttpError;

  /**
   * Construct a `401 Unauthorized` `HttpError`.
   *
   * @param message Optional error message.
   * @param code Optional machine-readable error code.
   * @returns `HttpError` with `status = 401`.
   * @since 0.0.0-alpha.0
   */
  export function unauthorized(message?: string, code?: string): HttpError;

  /**
   * Construct a `403 Forbidden` `HttpError`.
   *
   * @param message Optional error message.
   * @param code Optional machine-readable error code.
   * @returns `HttpError` with `status = 403`.
   * @since 0.0.0-alpha.0
   */
  export function forbidden(message?: string, code?: string): HttpError;

  /**
   * Construct a `404 Not Found` `HttpError`.
   *
   * @param message Optional error message.
   * @param code Optional machine-readable error code.
   * @returns `HttpError` with `status = 404`.
   * @since 0.0.0-alpha.0
   */
  export function notFound(message?: string, code?: string): HttpError;

  /**
   * Construct a `409 Conflict` `HttpError`.
   *
   * @param message Optional error message.
   * @param code Optional machine-readable error code.
   * @returns `HttpError` with `status = 409`.
   * @since 0.0.0-alpha.0
   */
  export function conflict(message?: string, code?: string): HttpError;

  /**
   * Construct a `405 Method Not Allowed` `HttpError` with optional `Allow` methods.
   *
   * @param allow Optional list of allowed HTTP methods for the `Allow` header.
   * @param message Optional error message.
   * @returns `HttpError` with `status = 405`.
   * @since 0.0.0-alpha.0
   */
  export function methodNotAllowed(allow?: string[], message?: string): HttpError;

  /**
   * Options accepted by `recover()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface RecoverOptions {
    /** Optional error observer invoked when a downstream handler throws or rejects. @since 0.0.0-alpha.0 */
    onError?: (error: unknown, req: HttpRequest, ctx: RequestContext) => void;
  }

  /**
   * Middleware that converts `HttpError` to its response and unexpected throws to `500`.
   *
   * @param options Optional `{ onError }` observer.
   * @returns Composable `Middleware` function.
   * @since 0.0.0-alpha.0
   */
  export function recover(options?: RecoverOptions): Middleware;

  /**
   * Options accepted by `requestId()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface RequestIdOptions {
    /** Request/response header name (default `"x-request-id"`). @since 0.0.0-alpha.0 */
    headerName?: string;
    /** Custom ID generator invoked when no valid incoming request ID header is present. @since 0.0.0-alpha.0 */
    generator?: () => string;
  }

  /**
   * Middleware that propagates or generates a safe `x-request-id` on `ctx.state.requestId` and the response.
   *
   * @param options Optional `{ headerName, generator }` configuration.
   * @returns Composable `Middleware` function.
   * @since 0.0.0-alpha.0
   */
  export function requestId(options?: RequestIdOptions): Middleware;

  /**
   * Structured request log record passed to `logger({ sink })`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface LogEntry {
    /** HTTP method. @since 0.0.0-alpha.0 */
    readonly method: string;
    /** Request pathname. @since 0.0.0-alpha.0 */
    readonly path: string;
    /** Response HTTP status code. @since 0.0.0-alpha.0 */
    readonly status: number;
    /** Request handling duration in milliseconds. @since 0.0.0-alpha.0 */
    readonly durationMs: number;
    /** Request ID from `ctx.state.requestId`, if set. @since 0.0.0-alpha.0 */
    readonly requestId: string | undefined;
  }

  /**
   * Options accepted by `logger()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface LoggerOptions {
    /** Custom sink receiving each `LogEntry` (defaults to `console.log`). @since 0.0.0-alpha.0 */
    sink?: (entry: LogEntry) => void;
  }

  /**
   * Middleware that logs request method, path, status, and duration.
   *
   * @param options Optional `{ sink }` configuration.
   * @returns Composable `Middleware` function.
   * @since 0.0.0-alpha.0
   */
  export function logger(options?: LoggerOptions): Middleware;

  /**
   * Options accepted by `securityHeaders()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface SecurityHeadersOptions {
    /** `X-Content-Type-Options` value (default `"nosniff"`, or `false` to omit). @since 0.0.0-alpha.0 */
    contentTypeOptions?: string | false;
    /** `X-Frame-Options` value (default `"DENY"`, or `false` to omit). @since 0.0.0-alpha.0 */
    frameOptions?: string | false;
    /** `Referrer-Policy` value (default `"no-referrer"`, or `false` to omit). @since 0.0.0-alpha.0 */
    referrerPolicy?: string | false;
  }

  /**
   * Middleware that attaches `x-content-type-options`, `x-frame-options`, and `referrer-policy` defaults.
   *
   * @param options Optional header overrides or `false` flags to disable individual headers.
   * @returns Composable `Middleware` function.
   * @since 0.0.0-alpha.0
   */
  export function securityHeaders(options?: SecurityHeadersOptions): Middleware;

  /**
   * Options accepted by `cors()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface CorsOptions {
    /** Allowed origin string, list of origins, or dynamic predicate callback. @since 0.0.0-alpha.0 */
    origin?: string | string[] | ((origin: string, req: HttpRequest) => string | boolean | null | undefined);
    /** Allowed HTTP methods for preflight responses. @since 0.0.0-alpha.0 */
    methods?: string[];
    /** Allowed request headers for preflight responses. @since 0.0.0-alpha.0 */
    allowHeaders?: string[];
    /** Response headers exposed to browser clients. @since 0.0.0-alpha.0 */
    exposeHeaders?: string[];
    /** Whether to emit `Access-Control-Allow-Credentials: true`. @since 0.0.0-alpha.0 */
    credentials?: boolean;
    /** Preflight cache duration (`Access-Control-Max-Age`) in seconds. @since 0.0.0-alpha.0 */
    maxAge?: number;
  }

  /**
   * Middleware that handles CORS preflight requests and `Access-Control-*` response headers.
   *
   * @param options Optional CORS policy configuration.
   * @returns Composable `Middleware` function.
   * @since 0.0.0-alpha.0
   */
  export function cors(options?: CorsOptions): Middleware;

  /**
   * Compose middleware functions left-to-right into a single `Middleware`.
   *
   * @param middlewares Middleware functions to compose.
   * @returns Combined `Middleware` function.
   * @since 0.0.0-alpha.0
   */
  export function compose(...middlewares: Middleware[]): Middleware;

  /**
   * Options accepted by `serve(options, handlerOrMux)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface ServeOptions {
    /** Bind host IP or hostname (default `"127.0.0.1"`). @since 0.0.0-alpha.0 */
    host?: string;
    /** Bind port (`0..=65535`, default `8080`; `0` selects an ephemeral OS port). @since 0.0.0-alpha.0 */
    port?: number;
    /** Maximum accepted request body size in bytes (default `10 MiB`). @since 0.0.0-alpha.0 */
    maxBodyBytes?: number;
    /** Optional abort signal that closes the server when aborted. @since 0.0.0-alpha.0 */
    signal?: AbortSignalLike;
  }

  /**
   * Running HTTP/1.1 server handle returned by `serve()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Server {
    /** Bound host address string. @since 0.0.0-alpha.0 */
    readonly host: string;
    /** Bound TCP port (resolved ephemeral port when started with `port: 0`). @since 0.0.0-alpha.0 */
    readonly port: number;
    /** Bound `host:port` socket string. @since 0.0.0-alpha.0 */
    readonly addr: string;
    /** Base HTTP URL (for example `"http://127.0.0.1:8080"`). @since 0.0.0-alpha.0 */
    readonly url: string;
    /**
     * Gracefully stop accepting connections and close the HTTP server.
     *
     * @returns Promise resolving once shutdown finishes.
     * @since 0.0.0-alpha.0
     */
    close(): Promise<void>;
    /** Promise that resolves once the server has stopped. @since 0.0.0-alpha.0 */
    readonly finished: Promise<void>;
    /**
     * Async disposal hook (`await using`) delegating to `close()`.
     *
     * @returns Promise resolving once the server has closed.
     * @since 0.0.0-alpha.0
     */
    [Symbol.asyncDispose](): Promise<void>;
  }

  /**
   * Start a live HTTP/1.1 server backed by `hyper` and `tokio::net::TcpListener`.
   *
   * @param handlerOrMux Request handler function or `ServeMux` router instance.
   * @returns Running `Server` handle.
   * @throws {TypeError} on invalid handler/router.
   * @throws {Error} on bind failure.
   * @example
   * ```ts
   * import { createRouter, serve, text } from "vael:http";
   *
   * const router = createRouter();
   * router.get("/", () => text("ok"));
   * const server = serve({ host: "127.0.0.1", port: 0 }, router);
   * console.log(server.url);
   * await server.close();
   * ```
   * @since 0.0.0-alpha.0
   */
  export function serve(handlerOrMux: Handler | ServeMux): Server;

  /**
   * Start a live HTTP/1.1 server backed by `hyper` and `tokio::net::TcpListener` with explicit options.
   *
   * @param options Server bind host, port, body limit, and optional abort signal.
   * @param handlerOrMux Request handler function or `ServeMux` router instance.
   * @returns Running `Server` handle.
   * @throws {TypeError} on invalid options or handler/router.
   * @throws {RangeError} when `port` or `maxBodyBytes` is out of range.
   * @throws {Error} on bind failure.
   * @since 0.0.0-alpha.0
   */
  export function serve(options: ServeOptions, handlerOrMux: Handler | ServeMux): Server;

  /**
   * Parse an HTTP `Cookie` request header into `{ name: value }` pairs.
   *
   * @param header Raw `Cookie` header value.
   * @returns Record mapping cookie names to decoded string values.
   * @since 0.0.0-alpha.0
   */
  export function parseCookies(header: string): Record<string, string>;

  /**
   * Format and validate an RFC 6265 / CRLF-safe `Set-Cookie` header value.
   *
   * @param options Cookie name, value, and attributes.
   * @returns Formatted `Set-Cookie` header string.
   * @throws {TypeError} when cookie name, value, or attributes fail validation.
   * @since 0.0.0-alpha.0
   */
  export function formatSetCookie(options: SetCookieOptions): string;
}

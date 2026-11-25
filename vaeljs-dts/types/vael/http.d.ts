/**
 * Go-style HTTP/1.1 server, ServeMux router, middleware, and immutable request/response types.
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
    /** @since 0.0.0-alpha.0 */
    readonly size: number;
    /** @since 0.0.0-alpha.0 */
    get(name: string): string | null;
    /** @since 0.0.0-alpha.0 */
    getAll(name: string): string[];
    /** @since 0.0.0-alpha.0 */
    has(name: string): boolean;
    /** @since 0.0.0-alpha.0 */
    keys(): IterableIterator<string>;
    /** @since 0.0.0-alpha.0 */
    values(): IterableIterator<string>;
    /** @since 0.0.0-alpha.0 */
    entries(): IterableIterator<[string, string]>;
    /** @since 0.0.0-alpha.0 */
    [Symbol.iterator](): IterableIterator<[string, string]>;
    /** @since 0.0.0-alpha.0 */
    forEach(callback: (value: string, key: string, parent: QueryParams) => void): void;
    /** @since 0.0.0-alpha.0 */
    toString(): string;
  }

  /**
   * Minimal abort-signal contract accepted by `serve({ signal })`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface AbortSignalLike {
    /** @since 0.0.0-alpha.0 */
    readonly aborted: boolean;
    /** @since 0.0.0-alpha.0 */
    addEventListener(type: "abort", listener: () => void, options?: { once?: boolean }): void;
    /** @since 0.0.0-alpha.0 */
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
    /** @since 0.0.0-alpha.0 */
    name: string;
    /** @since 0.0.0-alpha.0 */
    value: string;
    /** @since 0.0.0-alpha.0 */
    path?: string;
    /** @since 0.0.0-alpha.0 */
    domain?: string;
    /** @since 0.0.0-alpha.0 */
    maxAge?: number;
    /** @since 0.0.0-alpha.0 */
    httpOnly?: boolean;
    /** @since 0.0.0-alpha.0 */
    secure?: boolean;
    /** @since 0.0.0-alpha.0 */
    sameSite?: SameSitePolicy;
  }

  /**
   * Immutable incoming HTTP request passed to handlers and middleware.
   *
   * @since 0.0.0-alpha.0
   */
  export class HttpRequest {
    /** @since 0.0.0-alpha.0 */
    readonly method: string;
    /** @since 0.0.0-alpha.0 */
    readonly url: string;
    /** @since 0.0.0-alpha.0 */
    readonly path: string;
    /** @since 0.0.0-alpha.0 */
    readonly query: QueryParams;
    /** @since 0.0.0-alpha.0 */
    readonly headers: Readonly<Record<string, string>>;
    /** @since 0.0.0-alpha.0 */
    readonly params: Readonly<Record<string, string>>;
    /** @since 0.0.0-alpha.0 */
    readonly cookies: Readonly<Record<string, string>>;
    /** @since 0.0.0-alpha.0 */
    readonly remoteAddr: string;

    /** Case-insensitive header lookup. @since 0.0.0-alpha.0 */
    header(name: string): string | undefined;
    /** Route path parameter lookup. @since 0.0.0-alpha.0 */
    param(name: string): string | undefined;
    /** Return a fresh copy of the raw request body bytes. @since 0.0.0-alpha.0 */
    bodyBytes(): Uint8Array;
    /** Decode the request body as strict UTF-8 text (throws `HttpError(400)` on invalid UTF-8). @since 0.0.0-alpha.0 */
    text(): Promise<string>;
    /** Decode and parse a JSON request body (throws `HttpError(400)` on invalid UTF-8 or JSON). @since 0.0.0-alpha.0 */
    json<T = unknown>(): Promise<T>;
    /** Decode an `application/x-www-form-urlencoded` body into `QueryParams`. @since 0.0.0-alpha.0 */
    form(): Promise<QueryParams>;
  }

  /**
   * Immutable HTTP response descriptor returned by handlers and response builders.
   *
   * @since 0.0.0-alpha.0
   */
  export class HttpResponse {
    /** @since 0.0.0-alpha.0 */
    readonly status: number;
    /** @since 0.0.0-alpha.0 */
    readonly headers: Readonly<Record<string, string>>;
    /** @since 0.0.0-alpha.0 */
    readonly body: Uint8Array;

    /** Case-insensitive lookup for the collapsed header value. @since 0.0.0-alpha.0 */
    header(name: string): string | undefined;
    /** Case-insensitive lookup returning all header values for `name` (e.g. `set-cookie`). @since 0.0.0-alpha.0 */
    headerValues(name: string): string[];
    /** Return a fresh copy of the response body bytes. @since 0.0.0-alpha.0 */
    bytes(): Uint8Array;
    /** Decode the response body as strict UTF-8 text. @since 0.0.0-alpha.0 */
    text(): string;
    /** Decode and parse the response body as JSON. @since 0.0.0-alpha.0 */
    json<T = unknown>(): T;
  }

  /**
   * Per-request context passed alongside `HttpRequest` to handlers and middleware.
   *
   * @since 0.0.0-alpha.0
   */
  export interface RequestContext {
    /** @since 0.0.0-alpha.0 */
    readonly params: Readonly<Record<string, string>>;
    /** @since 0.0.0-alpha.0 */
    readonly state: Record<string, unknown>;
    /** @since 0.0.0-alpha.0 */
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
    /** @since 0.0.0-alpha.0 */
    override name: "HttpError";
    /** @since 0.0.0-alpha.0 */
    status: number;
    /** @since 0.0.0-alpha.0 */
    code?: string;
    /** @since 0.0.0-alpha.0 */
    details?: unknown;
    /** @since 0.0.0-alpha.0 */
    headers?: Readonly<Record<string, string>>;
    /** @since 0.0.0-alpha.0 */
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
    /** @since 0.0.0-alpha.0 */
    url: string;
    /** @since 0.0.0-alpha.0 */
    method?: string;
    /** @since 0.0.0-alpha.0 */
    headers?: HeadersInit;
    /** @since 0.0.0-alpha.0 */
    body?: string | Uint8Array | null;
    /** @since 0.0.0-alpha.0 */
    remoteAddr?: string;
  }

  /**
   * Request init overrides accepted by `ServeMux.fetch(input, init)`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface FetchInit {
    /** @since 0.0.0-alpha.0 */
    method?: string;
    /** @since 0.0.0-alpha.0 */
    headers?: HeadersInit;
    /** @since 0.0.0-alpha.0 */
    body?: string | Uint8Array | null;
    /** @since 0.0.0-alpha.0 */
    remoteAddr?: string;
  }

  /**
   * Go 1.22+ `ServeMux`-style HTTP router with middleware, route groups,
   * traversal-safe static file serving, and zero-socket `.fetch()` testing.
   *
   * @since 0.0.0-alpha.0
   */
  export class ServeMux {
    /** @since 0.0.0-alpha.0 */
    constructor();
    /** Register a Go 1.22+ pattern (`"GET /users/{id}"`, `"/files/{path...}"`, etc.). @since 0.0.0-alpha.0 */
    handle(pattern: string, handler: Handler): ServeMux;
    /** Register a `GET` (and automatic `HEAD`) route. @since 0.0.0-alpha.0 */
    get(path: string, handler: Handler): ServeMux;
    /** Register a `POST` route. @since 0.0.0-alpha.0 */
    post(path: string, handler: Handler): ServeMux;
    /** Register a `PUT` route. @since 0.0.0-alpha.0 */
    put(path: string, handler: Handler): ServeMux;
    /** Register a `PATCH` route. @since 0.0.0-alpha.0 */
    patch(path: string, handler: Handler): ServeMux;
    /** Register a `DELETE` route. @since 0.0.0-alpha.0 */
    delete(path: string, handler: Handler): ServeMux;
    /** Attach middleware to this router (or current route group). @since 0.0.0-alpha.0 */
    use(...middlewares: Middleware[]): ServeMux;
    /** Mount a prefixed route group with optional group-local `.use()` middleware. @since 0.0.0-alpha.0 */
    group(prefix: string, fn: (sub: ServeMux) => void): ServeMux;
    /** Serve static files from `rootDir` under `urlPrefix` with traversal/symlink protection. @since 0.0.0-alpha.0 */
    static(urlPrefix: string, rootDir: string, options?: StaticOptions): ServeMux;
    /** Dispatch a direct in-memory request through the router with zero sockets. @since 0.0.0-alpha.0 */
    fetch(input: string | FetchInput, init?: FetchInit): Promise<HttpResponse>;
  }

  /**
   * Create a new `ServeMux` router instance.
   *
   * @since 0.0.0-alpha.0
   */
  export function createRouter(): ServeMux;

  /** Create an immutable `HttpResponse`. @since 0.0.0-alpha.0 */
  export function response(body?: string | Uint8Array | null, init?: ResponseInit): HttpResponse;
  /** Create an immutable JSON `HttpResponse` (`content-type: application/json; charset=utf-8`). @since 0.0.0-alpha.0 */
  export function json(data: unknown, init?: ResponseInit): HttpResponse;
  /** Create an immutable HTML `HttpResponse` (`content-type: text/html; charset=utf-8`). @since 0.0.0-alpha.0 */
  export function html(body: string, init?: ResponseInit): HttpResponse;
  /** Create an immutable plain-text `HttpResponse` (`content-type: text/plain; charset=utf-8`). @since 0.0.0-alpha.0 */
  export function text(body: string, init?: ResponseInit): HttpResponse;
  /** Create an immutable redirect `HttpResponse` (default `302`). @since 0.0.0-alpha.0 */
  export function redirect(location: string, status?: RedirectStatus): HttpResponse;
  /** Create an immutable `204 No Content` `HttpResponse`. @since 0.0.0-alpha.0 */
  export function noContent(init?: ResponseInit): HttpResponse;
  /** Return a new `HttpResponse` with an RFC 6265 / CRLF-safe `Set-Cookie` header appended. @since 0.0.0-alpha.0 */
  export function setCookie(res: HttpResponse, cookie: SetCookieOptions): HttpResponse;

  /** Construct an `HttpError`. @since 0.0.0-alpha.0 */
  export function httpError(status: number, message?: string, code?: string, details?: unknown): HttpError;
  /** Construct a `400 Bad Request` `HttpError`. @since 0.0.0-alpha.0 */
  export function badRequest(message?: string, code?: string): HttpError;
  /** Construct a `401 Unauthorized` `HttpError`. @since 0.0.0-alpha.0 */
  export function unauthorized(message?: string, code?: string): HttpError;
  /** Construct a `403 Forbidden` `HttpError`. @since 0.0.0-alpha.0 */
  export function forbidden(message?: string, code?: string): HttpError;
  /** Construct a `404 Not Found` `HttpError`. @since 0.0.0-alpha.0 */
  export function notFound(message?: string, code?: string): HttpError;
  /** Construct a `409 Conflict` `HttpError`. @since 0.0.0-alpha.0 */
  export function conflict(message?: string, code?: string): HttpError;
  /** Construct a `405 Method Not Allowed` `HttpError` with optional `Allow` methods. @since 0.0.0-alpha.0 */
  export function methodNotAllowed(allow?: string[], message?: string): HttpError;

  /** Options accepted by `recover()`. @since 0.0.0-alpha.0 */
  export interface RecoverOptions {
    /** @since 0.0.0-alpha.0 */
    onError?: (error: unknown, req: HttpRequest, ctx: RequestContext) => void;
  }
  /** Middleware that converts `HttpError` to its response and unexpected throws to `500`. @since 0.0.0-alpha.0 */
  export function recover(options?: RecoverOptions): Middleware;

  /** Options accepted by `requestId()`. @since 0.0.0-alpha.0 */
  export interface RequestIdOptions {
    /** @since 0.0.0-alpha.0 */
    headerName?: string;
    /** @since 0.0.0-alpha.0 */
    generator?: () => string;
  }
  /** Middleware that propagates or generates a safe `x-request-id` on `ctx.state.requestId` and the response. @since 0.0.0-alpha.0 */
  export function requestId(options?: RequestIdOptions): Middleware;

  /** Structured request log record passed to `logger({ sink })`. @since 0.0.0-alpha.0 */
  export interface LogEntry {
    /** @since 0.0.0-alpha.0 */
    readonly method: string;
    /** @since 0.0.0-alpha.0 */
    readonly path: string;
    /** @since 0.0.0-alpha.0 */
    readonly status: number;
    /** @since 0.0.0-alpha.0 */
    readonly durationMs: number;
    /** @since 0.0.0-alpha.0 */
    readonly requestId: string | undefined;
  }
  /** Options accepted by `logger()`. @since 0.0.0-alpha.0 */
  export interface LoggerOptions {
    /** @since 0.0.0-alpha.0 */
    sink?: (entry: LogEntry) => void;
  }
  /** Middleware that logs request method, path, status, and duration. @since 0.0.0-alpha.0 */
  export function logger(options?: LoggerOptions): Middleware;

  /** Options accepted by `securityHeaders()`. @since 0.0.0-alpha.0 */
  export interface SecurityHeadersOptions {
    /** @since 0.0.0-alpha.0 */
    contentTypeOptions?: string | false;
    /** @since 0.0.0-alpha.0 */
    frameOptions?: string | false;
    /** @since 0.0.0-alpha.0 */
    referrerPolicy?: string | false;
  }
  /** Middleware that attaches `x-content-type-options`, `x-frame-options`, and `referrer-policy` defaults. @since 0.0.0-alpha.0 */
  export function securityHeaders(options?: SecurityHeadersOptions): Middleware;

  /** Options accepted by `cors()`. @since 0.0.0-alpha.0 */
  export interface CorsOptions {
    /** @since 0.0.0-alpha.0 */
    origin?: string | string[] | ((origin: string, req: HttpRequest) => string | boolean | null | undefined);
    /** @since 0.0.0-alpha.0 */
    methods?: string[];
    /** @since 0.0.0-alpha.0 */
    allowHeaders?: string[];
    /** @since 0.0.0-alpha.0 */
    exposeHeaders?: string[];
    /** @since 0.0.0-alpha.0 */
    credentials?: boolean;
    /** @since 0.0.0-alpha.0 */
    maxAge?: number;
  }
  /** Middleware that handles CORS preflight requests and `Access-Control-*` response headers. @since 0.0.0-alpha.0 */
  export function cors(options?: CorsOptions): Middleware;

  /** Compose middleware functions left-to-right into a single `Middleware`. @since 0.0.0-alpha.0 */
  export function compose(...middlewares: Middleware[]): Middleware;

  /** Options accepted by `serve(options, handlerOrMux)`. @since 0.0.0-alpha.0 */
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

  /** Running HTTP/1.1 server handle returned by `serve()`. @since 0.0.0-alpha.0 */
  export interface Server {
    /** @since 0.0.0-alpha.0 */
    readonly host: string;
    /** @since 0.0.0-alpha.0 */
    readonly port: number;
    /** @since 0.0.0-alpha.0 */
    readonly addr: string;
    /** @since 0.0.0-alpha.0 */
    readonly url: string;
    /** @since 0.0.0-alpha.0 */
    close(): Promise<void>;
    /** @since 0.0.0-alpha.0 */
    readonly finished: Promise<void>;
    /** @since 0.0.0-alpha.0 */
    [Symbol.asyncDispose](): Promise<void>;
  }

  /** Start a live HTTP/1.1 server backed by `hyper` and `tokio::net::TcpListener`. @since 0.0.0-alpha.0 */
  export function serve(handlerOrMux: Handler | ServeMux): Server;
  /** Start a live HTTP/1.1 server backed by `hyper` and `tokio::net::TcpListener` with explicit options. @since 0.0.0-alpha.0 */
  export function serve(options: ServeOptions, handlerOrMux: Handler | ServeMux): Server;

  /** Parse an HTTP `Cookie` request header into `{ name: value }` pairs. @since 0.0.0-alpha.0 */
  export function parseCookies(header: string): Record<string, string>;
  /** Format and validate an RFC 6265 / CRLF-safe `Set-Cookie` header value. @since 0.0.0-alpha.0 */
  export function formatSetCookie(options: SetCookieOptions): string;
}

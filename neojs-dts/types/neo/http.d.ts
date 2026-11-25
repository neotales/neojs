declare module "neo:http" {
  /**
   * Read-only URL query or `application/x-www-form-urlencoded` parameter bag.
   */
  export interface QueryParams {
    readonly size: number;
    get(name: string): string | null;
    getAll(name: string): string[];
    has(name: string): boolean;
    keys(): IterableIterator<string>;
    values(): IterableIterator<string>;
    entries(): IterableIterator<[string, string]>;
    [Symbol.iterator](): IterableIterator<[string, string]>;
    forEach(callback: (value: string, key: string, parent: QueryParams) => void): void;
    toString(): string;
  }

  /**
   * Minimal abort-signal contract accepted by `serve({ signal })`.
   */
  export interface AbortSignalLike {
    readonly aborted: boolean;
    addEventListener(type: "abort", listener: () => void, options?: { once?: boolean }): void;
    removeEventListener?(type: "abort", listener: () => void): void;
  }

  /**
   * HTTP header collection input: either a plain object or ordered `[name, value]` pairs.
   */
  export type HeadersInit = Record<string, string | undefined> | ReadonlyArray<readonly [string, string]>;

  /**
   * Options accepted by `response`, `json`, `html`, `text`, and `noContent`.
   */
  export interface ResponseInit {
    /** HTTP status code (`100..=599`, default `200`). */
    status?: number;
    /** HTTP response headers, validated against RFC 9110 and CRLF-safe rules. */
    headers?: HeadersInit;
  }

  /**
   * Supported HTTP redirect status codes.
   */
  export type RedirectStatus = 301 | 302 | 303 | 307 | 308;

  /**
   * `Set-Cookie` `SameSite` policy attribute.
   */
  export type SameSitePolicy = "Lax" | "Strict" | "None";

  /**
   * Options accepted by `setCookie` and `formatSetCookie`.
   */
  export interface SetCookieOptions {
    name: string;
    value: string;
    path?: string;
    domain?: string;
    maxAge?: number;
    httpOnly?: boolean;
    secure?: boolean;
    sameSite?: SameSitePolicy;
  }

  /**
   * Immutable incoming HTTP request passed to handlers and middleware.
   */
  export class HttpRequest {
    readonly method: string;
    readonly url: string;
    readonly path: string;
    readonly query: QueryParams;
    readonly headers: Readonly<Record<string, string>>;
    readonly params: Readonly<Record<string, string>>;
    readonly cookies: Readonly<Record<string, string>>;
    readonly remoteAddr: string;

    /** Case-insensitive header lookup. */
    header(name: string): string | undefined;
    /** Route path parameter lookup. */
    param(name: string): string | undefined;
    /** Return a fresh copy of the raw request body bytes. */
    bodyBytes(): Uint8Array;
    /** Decode the request body as strict UTF-8 text (throws `HttpError(400)` on invalid UTF-8). */
    text(): Promise<string>;
    /** Decode and parse a JSON request body (throws `HttpError(400)` on invalid UTF-8 or JSON). */
    json<T = unknown>(): Promise<T>;
    /** Decode an `application/x-www-form-urlencoded` body into `QueryParams`. */
    form(): Promise<QueryParams>;
  }

  /**
   * Immutable HTTP response descriptor returned by handlers and response builders.
   */
  export class HttpResponse {
    readonly status: number;
    readonly headers: Readonly<Record<string, string>>;
    readonly body: Uint8Array;

    /** Case-insensitive lookup for the collapsed header value. */
    header(name: string): string | undefined;
    /** Case-insensitive lookup returning all header values for `name` (e.g. `set-cookie`). */
    headerValues(name: string): string[];
    /** Return a fresh copy of the response body bytes. */
    bytes(): Uint8Array;
    /** Decode the response body as strict UTF-8 text. */
    text(): string;
    /** Decode and parse the response body as JSON. */
    json<T = unknown>(): T;
  }

  /**
   * Per-request context passed alongside `HttpRequest` to handlers and middleware.
   */
  export interface RequestContext {
    readonly params: Readonly<Record<string, string>>;
    readonly state: Record<string, unknown>;
    readonly remoteAddr: string;
  }

  /**
   * Pure HTTP handler returning an `HttpResponse` (synchronously or as a `Promise`).
   */
  export type Handler = (req: HttpRequest, ctx: RequestContext) => HttpResponse | Promise<HttpResponse>;

  /**
   * Composable middleware wrapping a downstream `Handler`.
   */
  export type Middleware = (next: Handler) => Handler;

  /**
   * Structured HTTP error converted into an HTTP response by `ServeMux` and `recover()`.
   */
  export class HttpError extends Error {
    override name: "HttpError";
    status: number;
    code?: string;
    details?: unknown;
    headers?: Readonly<Record<string, string>>;
    constructor(status: number, message?: string, code?: string, details?: unknown);
  }

  /**
   * Options accepted by `ServeMux.static(urlPrefix, rootDir, options)`.
   */
  export interface StaticOptions {
    /** Directory index filename (default `"index.html"`, or `null` to disable directory index). */
    index?: string | null;
  }

  /**
   * Request input accepted by `ServeMux.fetch(input, init)`.
   */
  export interface FetchInput {
    url: string;
    method?: string;
    headers?: HeadersInit;
    body?: string | Uint8Array | null;
    remoteAddr?: string;
  }

  /**
   * Request init overrides accepted by `ServeMux.fetch(input, init)`.
   */
  export interface FetchInit {
    method?: string;
    headers?: HeadersInit;
    body?: string | Uint8Array | null;
    remoteAddr?: string;
  }

  /**
   * Go 1.22+ `ServeMux`-style HTTP router with middleware, route groups,
   * traversal-safe static file serving, and zero-socket `.fetch()` testing.
   */
  export class ServeMux {
    constructor();
    /** Register a Go 1.22+ pattern (`"GET /users/{id}"`, `"/files/{path...}"`, etc.). */
    handle(pattern: string, handler: Handler): ServeMux;
    /** Register a `GET` (and automatic `HEAD`) route. */
    get(path: string, handler: Handler): ServeMux;
    /** Register a `POST` route. */
    post(path: string, handler: Handler): ServeMux;
    /** Register a `PUT` route. */
    put(path: string, handler: Handler): ServeMux;
    /** Register a `PATCH` route. */
    patch(path: string, handler: Handler): ServeMux;
    /** Register a `DELETE` route. */
    delete(path: string, handler: Handler): ServeMux;
    /** Attach middleware to this router (or current route group). */
    use(...middlewares: Middleware[]): ServeMux;
    /** Mount a prefixed route group with optional group-local `.use()` middleware. */
    group(prefix: string, fn: (sub: ServeMux) => void): ServeMux;
    /** Serve static files from `rootDir` under `urlPrefix` with traversal/symlink protection. */
    static(urlPrefix: string, rootDir: string, options?: StaticOptions): ServeMux;
    /** Dispatch a direct in-memory request through the router with zero sockets. */
    fetch(input: string | FetchInput, init?: FetchInit): Promise<HttpResponse>;
  }

  /**
   * Create a new `ServeMux` router instance.
   */
  export function createRouter(): ServeMux;

  /** Create an immutable `HttpResponse`. */
  export function response(body?: string | Uint8Array | null, init?: ResponseInit): HttpResponse;
  /** Create an immutable JSON `HttpResponse` (`content-type: application/json; charset=utf-8`). */
  export function json(data: unknown, init?: ResponseInit): HttpResponse;
  /** Create an immutable HTML `HttpResponse` (`content-type: text/html; charset=utf-8`). */
  export function html(body: string, init?: ResponseInit): HttpResponse;
  /** Create an immutable plain-text `HttpResponse` (`content-type: text/plain; charset=utf-8`). */
  export function text(body: string, init?: ResponseInit): HttpResponse;
  /** Create an immutable redirect `HttpResponse` (default `302`). */
  export function redirect(location: string, status?: RedirectStatus): HttpResponse;
  /** Create an immutable `204 No Content` `HttpResponse`. */
  export function noContent(init?: ResponseInit): HttpResponse;
  /** Return a new `HttpResponse` with an RFC 6265 / CRLF-safe `Set-Cookie` header appended. */
  export function setCookie(res: HttpResponse, cookie: SetCookieOptions): HttpResponse;

  /** Construct an `HttpError`. */
  export function httpError(status: number, message?: string, code?: string, details?: unknown): HttpError;
  /** Construct a `400 Bad Request` `HttpError`. */
  export function badRequest(message?: string, code?: string): HttpError;
  /** Construct a `401 Unauthorized` `HttpError`. */
  export function unauthorized(message?: string, code?: string): HttpError;
  /** Construct a `403 Forbidden` `HttpError`. */
  export function forbidden(message?: string, code?: string): HttpError;
  /** Construct a `404 Not Found` `HttpError`. */
  export function notFound(message?: string, code?: string): HttpError;
  /** Construct a `409 Conflict` `HttpError`. */
  export function conflict(message?: string, code?: string): HttpError;
  /** Construct a `405 Method Not Allowed` `HttpError` with optional `Allow` methods. */
  export function methodNotAllowed(allow?: string[], message?: string): HttpError;

  /** Options accepted by `recover()`. */
  export interface RecoverOptions {
    onError?: (error: unknown, req: HttpRequest, ctx: RequestContext) => void;
  }
  /** Middleware that converts `HttpError` to its response and unexpected throws to `500`. */
  export function recover(options?: RecoverOptions): Middleware;

  /** Options accepted by `requestId()`. */
  export interface RequestIdOptions {
    headerName?: string;
    generator?: () => string;
  }
  /** Middleware that propagates or generates a safe `x-request-id` on `ctx.state.requestId` and the response. */
  export function requestId(options?: RequestIdOptions): Middleware;

  /** Structured request log record passed to `logger({ sink })`. */
  export interface LogEntry {
    readonly method: string;
    readonly path: string;
    readonly status: number;
    readonly durationMs: number;
    readonly requestId: string | undefined;
  }
  /** Options accepted by `logger()`. */
  export interface LoggerOptions {
    sink?: (entry: LogEntry) => void;
  }
  /** Middleware that logs request method, path, status, and duration. */
  export function logger(options?: LoggerOptions): Middleware;

  /** Options accepted by `securityHeaders()`. */
  export interface SecurityHeadersOptions {
    contentTypeOptions?: string | false;
    frameOptions?: string | false;
    referrerPolicy?: string | false;
  }
  /** Middleware that attaches `x-content-type-options`, `x-frame-options`, and `referrer-policy` defaults. */
  export function securityHeaders(options?: SecurityHeadersOptions): Middleware;

  /** Options accepted by `cors()`. */
  export interface CorsOptions {
    origin?: string | string[] | ((origin: string, req: HttpRequest) => string | boolean | null | undefined);
    methods?: string[];
    allowHeaders?: string[];
    exposeHeaders?: string[];
    credentials?: boolean;
    maxAge?: number;
  }
  /** Middleware that handles CORS preflight requests and `Access-Control-*` response headers. */
  export function cors(options?: CorsOptions): Middleware;

  /** Compose middleware functions left-to-right into a single `Middleware`. */
  export function compose(...middlewares: Middleware[]): Middleware;

  /** Options accepted by `serve(options, handlerOrMux)`. */
  export interface ServeOptions {
    /** Bind host IP or hostname (default `"127.0.0.1"`). */
    host?: string;
    /** Bind port (`0..=65535`, default `8080`; `0` selects an ephemeral OS port). */
    port?: number;
    /** Maximum accepted request body size in bytes (default `10 MiB`). */
    maxBodyBytes?: number;
    /** Optional abort signal that closes the server when aborted. */
    signal?: AbortSignalLike;
  }

  /** Running HTTP/1.1 server handle returned by `serve()`. */
  export interface Server {
    readonly host: string;
    readonly port: number;
    readonly addr: string;
    readonly url: string;
    close(): Promise<void>;
    readonly finished: Promise<void>;
    [Symbol.asyncDispose](): Promise<void>;
  }

  /** Start a live HTTP/1.1 server backed by `hyper` and `tokio::net::TcpListener`. */
  export function serve(handlerOrMux: Handler | ServeMux): Server;
  export function serve(options: ServeOptions, handlerOrMux: Handler | ServeMux): Server;

  /** Parse an HTTP `Cookie` request header into `{ name: value }` pairs. */
  export function parseCookies(header: string): Record<string, string>;
  /** Format and validate an RFC 6265 / CRLF-safe `Set-Cookie` header value. */
  export function formatSetCookie(options: SetCookieOptions): string;
}

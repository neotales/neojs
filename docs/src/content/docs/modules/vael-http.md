---
title: vael:http
description: Go 1.22+ ServeMux HTTP router, middleware, traversal-safe static file serving, cookies, and live HTTP/1.1 server.
---

`vael:http` provides a Go 1.22+ `ServeMux`-style router, immutable request/response types, composable middleware, traversal-safe static file serving, RFC 6265 cookies, zero-socket `.fetch()` testing, and a live HTTP/1.1 server backed by `hyper` and `tokio`.

:::note[Availability]
`vael:http` is available in **`vjs` (V8) only** when built with the `vael-http` Cargo feature (enabled by default; opt-outable with `--no-default-features`). It is **not** available in `vjs-lite`.
:::

## Routing & Go 1.22+ Patterns

Create a router with `createRouter()` or `new ServeMux()`. Patterns follow Go 1.22+ `ServeMux` syntax:

- **Method prefix** (optional): `"GET /users/{id}"`, `"POST /items"`. Omitting the method matches any valid HTTP method.
- **Single-segment parameter**: `"/users/{id}"` matches one non-empty path segment (percent-decoded into `ctx.params.id` and `req.param("id")`).
- **Trailing wildcard**: `"/files/{path...}"` matches the remaining path segments as a single parameter.

Precedence is deterministic left-to-right (`literal > {param} > {wildcard...}`), then method-specific over any-method. `GET` routes automatically satisfy `HEAD` requests unless a `HEAD` route matches with equal or higher specificity; matching a path with an unsupported method returns `405 Method Not Allowed` with an `Allow` header.

### `ServeMux` Methods

| Method | Signature | Description |
| --- | --- | --- |
| `handle` | `(pattern: string, handler: Handler) => ServeMux` | Register a Go 1.22+ pattern. |
| `get` | `(path: string, handler: Handler) => ServeMux` | Register a `GET` (and automatic `HEAD`) route. |
| `post` | `(path: string, handler: Handler) => ServeMux` | Register a `POST` route. |
| `put` | `(path: string, handler: Handler) => ServeMux` | Register a `PUT` route. |
| `patch` | `(path: string, handler: Handler) => ServeMux` | Register a `PATCH` route. |
| `delete` | `(path: string, handler: Handler) => ServeMux` | Register a `DELETE` route. |
| `use` | `(...middlewares: Middleware[]) => ServeMux` | Attach middleware to this router or active `.group()`. |
| `group` | `(prefix: string, fn: (sub: ServeMux) => void) => ServeMux` | Mount a prefixed route group with group-local `.use()`. |
| `static` | `(urlPrefix: string, rootDir: string, options?: StaticOptions) => ServeMux` | Serve files from `rootDir` under `urlPrefix`. |
| `fetch` | `(input: string \| FetchInput, init?: FetchInit) => Promise<HttpResponse>` | Dispatch an in-memory request with zero sockets. |

`static()` rejects `..` traversal, `%2e%2e`, backslashes, NUL/control bytes, colons, encoded slashes (`%2f`), and symlinks escaping `rootDir`. `StaticOptions` accepts `{ index?: string | null }` (defaults to `"index.html"`; `null` disables directory index resolution).

## Requests & Responses

Handlers receive `(req: HttpRequest, ctx: RequestContext)` and return `HttpResponse | Promise<HttpResponse>`. `ctx` exposes `{ params, state, remoteAddr }`.

### `HttpRequest`

| Member | Type / Signature | Description |
| --- | --- | --- |
| `method`, `url`, `path`, `remoteAddr` | `readonly string` | Request method, full URL, path, and peer address. |
| `query` | `readonly QueryParams` | Read-only parameter bag (`size`, `get`, `getAll`, `has`, `keys`, `values`, `entries`, `forEach`, `toString`). |
| `headers`, `params`, `cookies` | `readonly Readonly<Record<string, string>>` | Collapsed headers, route parameters, and parsed cookies. |
| `header(name)` / `param(name)` | `(name: string) => string \| undefined` | Case-insensitive header lookup and route parameter lookup. |
| `bodyBytes()` | `() => Uint8Array` | Fresh copy of the raw request body bytes. |
| `text()` | `() => Promise<string>` | Strict UTF-8 body decode (throws `HttpError(400)` on invalid UTF-8). |
| `json<T>()` | `<T = unknown>() => Promise<T>` | Parse JSON body (throws `HttpError(400)` on invalid UTF-8 or JSON). |
| `form()` | `() => Promise<QueryParams>` | Decode `application/x-www-form-urlencoded` body. |

### `HttpResponse` & Response Builders

`HttpResponse` exposes `status`, `headers`, `body`, `header(name)`, `headerValues(name)`, `bytes()`, `text()`, and `json<T>()`.

| Export | Signature | Description |
| --- | --- | --- |
| `response` | `(body?: string \| Uint8Array \| null, init?: ResponseInit) => HttpResponse` | Construct an immutable response. |
| `json` | `(data: unknown, init?: ResponseInit) => HttpResponse` | `content-type: application/json; charset=utf-8`. |
| `html` | `(body: string, init?: ResponseInit) => HttpResponse` | `content-type: text/html; charset=utf-8`. |
| `text` | `(body: string, init?: ResponseInit) => HttpResponse` | `content-type: text/plain; charset=utf-8`. |
| `redirect` | `(location: string, status?: RedirectStatus) => HttpResponse` | Redirect (`301 \| 302 \| 303 \| 307 \| 308`, default `302`). |
| `noContent` | `(init?: ResponseInit) => HttpResponse` | `204 No Content`. |
| `setCookie` | `(res: HttpResponse, cookie: SetCookieOptions) => HttpResponse` | Return a new `HttpResponse` with a CRLF-safe `Set-Cookie` header appended. |

`ResponseInit` accepts `{ status?: number; headers?: HeadersInit }` where `HeadersInit` is a plain object or `[name, value]` pairs validated against RFC 9110 and CRLF-safe rules.

## Errors, Middleware & Cookies

`ServeMux` and `recover()` convert thrown `HttpError` instances into responses: JSON (`{ error, status, code?, details? }`) when the `Accept` header includes `application/json`, or plain text otherwise.

| Category | Export | Signature |
| --- | --- | --- |
| Error class | `HttpError` | `new HttpError(status, message?, code?, details?)` (with optional `headers`) |
| Error helpers | `httpError` | `(status, message?, code?, details?) => HttpError` |
| Error helpers | `badRequest`, `unauthorized`, `forbidden`, `notFound`, `conflict` | `(message?: string, code?: string) => HttpError` |
| Error helper | `methodNotAllowed` | `(allow?: string[], message?: string) => HttpError` |
| Middleware | `recover` | `(options?: { onError?: (error, req, ctx) => void }) => Middleware` |
| Middleware | `requestId` | `(options?: { headerName?: string; generator?: () => string }) => Middleware` |
| Middleware | `logger` | `(options?: { sink?: (entry: LogEntry) => void }) => Middleware` |
| Middleware | `securityHeaders` | `(options?: { contentTypeOptions?: string \| false; frameOptions?: string \| false; referrerPolicy?: string \| false }) => Middleware` |
| Middleware | `cors` | `(options?: CorsOptions) => Middleware` |
| Middleware | `compose` | `(...middlewares: Middleware[]) => Middleware` |
| Cookies | `parseCookies` | `(header: string) => Record<string, string>` |
| Cookies | `formatSetCookie` | `(options: SetCookieOptions) => string` |

`SetCookieOptions` accepts `{ name, value, path?, domain?, maxAge?, httpOnly?, secure?, sameSite?: "Lax" | "Strict" | "None" }`. `CorsOptions` accepts `{ origin?, methods?, allowHeaders?, exposeHeaders?, credentials?, maxAge? }`.

## Live Server (`serve`)

`serve(handlerOrMux)` or `serve(options, handlerOrMux)` starts an HTTP/1.1 server backed by `hyper` and `tokio::net::TcpListener`, returning a `Server` handle.

- **`ServeOptions`**: `host` (default `"127.0.0.1"`), `port` (default `8080`; `0` binds an ephemeral port), `maxBodyBytes` (default `10 MiB`, hard cap `256 MiB`), `signal` (`AbortSignalLike`).
- **`Server` handle**: `host`, `port`, `addr`, `url`, `close(): Promise<void>`, `finished: Promise<void>`, `[Symbol.asyncDispose](): Promise<void>`.

## Example

```ts
import {
  badRequest,
  createRouter,
  json,
  logger,
  recover,
  requestId,
  serve,
} from "vael:http";

const app = createRouter();
app.use(recover(), requestId(), logger());

app.get("/health", () => json({ ok: true }));

app.group("/api", (api) => {
  api.get("/users/{id}", (req, ctx) => {
    const id = req.param("id");
    if (!id || id === "0") throw badRequest("invalid user id", "INVALID_ID");
    return json({ id, requestId: ctx.state.requestId });
  });
});

// Zero-socket in-memory request test
const res = await app.fetch("/api/users/42");
console.log(res.status, res.json());

// Live HTTP/1.1 server on an ephemeral port
await using server = serve({ port: 0 }, app);
console.log(`Listening on ${server.url}`);
```

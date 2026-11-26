# VaelJS Go-Style Web Application Prototype

A complete, zero-framework web application built purely with VaelJS built-in modules:

- `vael:http` — Go 1.22+ `ServeMux` router, composable middleware (`recover`, `requestId`, `securityHeaders`, `logger`), traversal-safe static file serving, RFC 6265 cookies, and `hyper`/`tokio` HTTP/1.1 `serve()`.
- `vael:db` — SQLite persistence (`openSqlite`), named (`@id`) and positional (`?`) parameter binding, and atomic `db.transaction(...)`.
- `vael:template` — Go `text/template` layouts (`define`, `block`, `template`) with Sprig helpers (`title`, `upper`, `trim`, `default`) and automatic HTML XSS escaping.
- `vael:uuid` — Time-ordered RFC 9562 UUIDv7 primary keys (`v7()`).
- `vael:crypto` — CSPRNG key generation (`randomBytes`) and HMAC-SHA256 (`createHmac`) for tamper-proof flash cookies.
- `vael:test` & `vael:assert` — Zero-socket `router.fetch(...)` and live loopback server integration tests.

## Running

```sh
vjs examples/webapp/main.ts
```

Environment variables (read via `node:process`):

- `HOST` (default `127.0.0.1`)
- `PORT` (default `8080`)
- `DB_PATH` (default `:memory:`)

Example with a file-backed SQLite database:

```sh
PORT=3000 DB_PATH=./tasks.sqlite vjs examples/webapp/main.ts
```

## Testing

```sh
vjs test examples/webapp/app.test.ts
```

## Architecture (Go-Style, Without Go Boilerplate)

- **Plain dependency injection**: `createApp({ db, staticDir, logSink })` wires handlers and returns `{ router }`, with no decorator magic, DI containers, or global singletons.
- **No `ResponseWriter` mutability**: Handlers receive an immutable `HttpRequest` and return an immutable `HttpResponse` (`html(...)`, `json(...)`, `redirect(...)`, `noContent()`), avoiding Go's partial-write / header-after-body hazards.
- **No `if err != nil` clutter**: Handlers simply `throw badRequest(...)` or `throw notFound(...)` (`HttpError`), and `ServeMux` + `recover()` translate errors cleanly while masking unexpected internal exceptions as `500`.
- **Fast zero-socket tests**: `router.fetch(...)` runs the full router, middleware stack, templates, and SQLite queries in memory without opening TCP sockets.

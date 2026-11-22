# neojs database API

`neojs-dbapi` is a new workspace crate. It gives Rust and a future JavaScript surface one SQL
shape: strict JSON configuration, named `@name` or positional `?` parameters, bound values, and
driver-neutral rows.

The crate is asynchronous and does not create a runtime. The SQLite provider uses a Tokio
runtime, so an async caller must already have one.

## What exists

- **Configuration.** A strict JSON parser for SQLite, PostgreSQL, MySQL, and Microsoft SQL Server.
  Unknown fields, wrong types, empty required values, and out-of-range numbers are errors. Error
  values keep a fixed description and never echo a field value.
- **Binding compiler.** A lexical scanner rewrites `@name` or `?` to the target form and builds
  the ordered parameter list. It handles quoted strings and identifiers, comments, PostgreSQL
  dollar quoting, and operator sequences containing `@` or `?`. It rejects forms it cannot
  classify unambiguously.
- **SQLite provider.** Real end-to-end execution through the published
  `neotales-dbapi-sqlite` 0.0.0-alpha.0 crate. Tests use isolated `tempfile` databases.
- **Facade.** `Db`, `Connection`, and `Transaction` with execute, bounded query, cardinality-one
  query, and explicit commit/rollback.

## What does not exist

- **No PostgreSQL, MySQL, or Microsoft SQL Server provider.** Their configuration and
  placeholder forms are implemented and unit-tested, but `Db::open` returns
  `ErrorKind::DriverUnavailable` and opens no socket. A configuration name is not a supported
  driver.
- **No JavaScript/Node adapter.** This was deliberately not stubbed. The neojs built-in surface
  is a synchronous extension loaded into the V8 realm; database calls are asynchronous and need a
  Promise bridge and a lifecycle that survives the isolate. A half-built binding would be a
  provider that appears supported and is not. The next step is a separate crate that depends on
  `neojs-core` and `neojs-dbapi`, exposes JSON in/out, and adds no Node dependency to this crate's
  default graph.

## Why the upstream contract is reused

`neotales-dbapi` 0.0.0-alpha.0 and `neotales-dbapi-sqlite` 0.0.0-alpha.0 are published from
`neotales/rs-db` under MIT. The contract already provides sanitized errors, bounded row access,
transactions, and a real SQLite pool. Reusing it avoids a second row/value model and a second
SQLite implementation. The external repository was read, not modified. The new crate adds the
portable binding layer and the strict configuration facade, which the upstream contract
deliberately does not provide (it requires native placeholder syntax and does not rewrite SQL).

## Security model

- **Values are never interpolated.** The compiler produces SQL text and a separate parameter
  vector. A value cannot change the statement's structure. SQL text itself is still code;
  placeholders do not make an identifier or a statement fragment safe.
- **No regular expressions.** The scanner is a hand-written byte lexer over UTF-8 with ASCII
  delimiters.
- **Ambiguity fails closed.** `?NNN`, `:name`, `$name`, `??`, a bare `@`, mixed named and
  positional forms, multiple statements, and trigger bodies are errors. A `;` is accepted only when
  the remainder is whitespace, comments, and further semicolons.
- **Dialect-aware literals.** Single and double quotes, backticks (SQLite/MySQL), brackets
  (SQLite/SQL Server), line and block comments, nested block comments (PostgreSQL/SQL Server),
  PostgreSQL dollar quoting, and MySQL/PostgreSQL backslash-escape rules are handled. A
  placeholder inside a string, quoted identifier, or comment is not a placeholder. MySQL
  executable comments (`/*! ... */`) are rejected because MySQL executes their contents.
- **Configuration errors are static.** The error type holds a category and a fixed detail string.
  It never contains a path, host, user, password, SQL text, or driver message. `Debug` for a
  network configuration redacts host, database, user, and password.
- **No logging.** The crate has no logger and emits no output. The SQLite provider disables SQL
  statement logging.
- **Credentials.** No credential is read from the environment, a file, or a secret store. A
  network configuration may carry a password, but it is only stored behind a redacted type and no
  provider reads it because no network provider is linked. No real server is contacted by tests.

## Known limits

- The scanner is not a SQL parser. It cannot tell a T-SQL local variable (`DECLARE @x`) from a
  named parameter, or a MySQL user variable (`@var`) from one, so such SQL is outside portable
  mode. PostgreSQL's bare `?` jsonb operator conflicts with a positional placeholder, so
  PostgreSQL requires named parameters and a bare `?` is rejected. Those drivers are not linked,
  and the limitation is documented rather than guessed around.
- SQLite has dynamic typing. JSON `null` binds SQL NULL with a declared `TEXT` type; use
  `Param::TypedNull` when a provider needs an explicit type.
- JSON parameters do not encode bytes. `Param::Bytes` is available from Rust only.
- String equality of names is exact and case-sensitive. SQL Server treats parameter names
  case-insensitively, so the compiler rewrites named parameters to `@p1`, `@p2` rather than
  preserving case-sensitive names.
- `query_one` materializes the bounded batch before deciding cardinality; use paging for large
  result sets.

## Verification

```sh
cargo fmt --check
cargo clippy -p neojs-dbapi --all-targets --all-features -- -D warnings
cargo test -p neojs-dbapi
cargo test -p neojs-dbapi --no-default-features
cargo tree -p neojs-dbapi --no-default-features   # no Node/V8 dependency
```

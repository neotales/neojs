# neojs-dbapi

One SQL API for SQLite, PostgreSQL, MySQL, and Microsoft SQL Server, with strict JSON
configuration and portable named or positional parameters.

By [NeoTales](https://github.com/neotales).

## Status

| Driver | Configuration | Binding compiler | Provider | End-to-end test |
| --- | --- | --- | --- | --- |
| SQLite | yes | yes | `neotales-dbapi-sqlite` | yes, real temp databases |
| PostgreSQL | yes | yes | none | no |
| MySQL | yes | yes | none | no |
| Microsoft SQL Server | yes | yes | none | no |

Only SQLite is implemented end to end. A PostgreSQL, MySQL, or Microsoft SQL Server
configuration parses, but `Db::open` returns `DriverUnavailable` without opening a socket.
There is no placeholder provider behind those names.

There is no JavaScript or Node adapter in this crate yet. The default dependency graph contains
no V8, `deno_core`, or `neojs-core` dependency. See [`docs/db-api.md`](../docs/db-api.md) for the
deferred boundary and the reason.

## Install

```sh
cargo add neojs-dbapi
```

The default feature is `sqlite`. Build the compiler and configuration layer without a driver:

```sh
cargo add neojs-dbapi --no-default-features
```

The API is asynchronous and does not create a runtime. The SQLite provider runs on Tokio, so an
async caller must provide a Tokio runtime (for example `#[tokio::main]` or `#[tokio::test]`).

## Quick start

```rust
use neojs_dbapi::{Db, Param, Params};

# async fn run() -> Result<(), neojs_dbapi::Error> {
let db = Db::open_json(r#"{"driver":"sqlite","database":"app.sqlite"}"#).await?;
db.execute(
    "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
    &Params::none(),
)
.await?;

let insert = Params::named([
    ("id", Param::I64(7)),
    ("name", Param::Text("Ada".to_owned())),
]);
db.execute("INSERT INTO users (id, name) VALUES (@id, @name)", &insert)
    .await?;

let rows = db
    .query(
        "SELECT name FROM users WHERE id = @id",
        &Params::named([("id", Param::I64(7))]),
    )
    .await?;
assert_eq!(rows[0].read::<String>("name")?, "Ada");
# Ok(())
# }
```

## Configuration

The document must be a JSON object with an exact lower-case `driver`. Unknown fields, wrong
types, empty required strings, and out-of-range numbers are errors. Error text never repeats a
configuration value.

SQLite:

```json
{
  "driver": "sqlite",
  "database": "/var/lib/app/app.sqlite",
  "busy_timeout_ms": 5000,
  "max_connections": 5
}
```

| Field | Required | Range |
| --- | --- | --- |
| `database` | yes | non-empty, no NUL |
| `busy_timeout_ms` | no | 1 to 2147483647, default 5000 |
| `max_connections` | no | 1 to 1024, default 5 |

Network drivers accept `host`, `port`, `database`, `user`, optional `password`, and optional
`tls`. Defaults are 5432, 3306, and 1433. They are parsed for a uniform schema but are unavailable
in this build. `password` is redacted from `Debug` and has no public accessor.

## Parameters

Named placeholders are `@name`; positional placeholders are `?`. A statement uses one form or the
other, never both. PostgreSQL requires the named form because a bare `?` is its jsonb operator;
that case is rejected rather than guessed. Names are case-sensitive and limited to ASCII letters,
digits, and underscores, starting with a letter or underscore.

```rust
use neojs_dbapi::{Param, Params};

let named = Params::named([("id", Param::I64(7)), ("name", Param::Text("Ada".to_owned()))]);
let positional = Params::positional([Param::I64(7), Param::Text("Ada".to_owned())]);
# let _ = (named, positional);
```

JSON parameters are an array for positional values or an object for named values:

```json
[7, "Ada", null, true, 1.5]
```

```json
{"id": 7, "name": "Ada"}
```

Values are always bound. The compiler emits `?` for SQLite and MySQL, `$1` for PostgreSQL named
parameters, and `@p1` for Microsoft SQL Server. It never interpolates a value into SQL and never
uses a regular expression. SQL text is still code: placeholders protect values, not identifiers or
SQL structure, so do not build the statement itself from untrusted input.

The SQL scanner understands strings, quoted identifiers, comments, PostgreSQL dollar quoting, and
operators that contain `@` or `?`. It rejects ambiguity instead of guessing: `?NNN`, `:name`,
`$name`, `??`, bare `@`, mixed forms, multiple statements, and trigger bodies are errors. See the
crate documentation for the full portable subset.

## Rows and limits

Rows are the published [`neotales_dbapi`](https://crates.io/crates/neotales-dbapi) `Row` type.
`read`/`read_at` convert exactly; `Option<T>` is a matching typed SQL null. `query` is bounded by
`RowLimit::default()` (1,000 rows, 1 MiB); `query_with_limit` takes an explicit bound. A limit is a
hard failure, never a silent truncation.

## Transactions

```rust
use neojs_dbapi::{Db, Param, Params};

# async fn run(db: &Db) -> Result<(), neojs_dbapi::Error> {
let mut connection = db.connection().await?;
let mut transaction = connection.begin().await?;
transaction
    .execute(
        "INSERT INTO users (id, name) VALUES (@id, @name)",
        &Params::named([("id", Param::I64(8)), ("name", Param::Text("Grace".to_owned()))]),
    )
    .await?;
transaction.commit().await?;
# Ok(())
# }
```

Dropping a transaction without commit or rollback leaves cleanup to the provider, which never
returns a possibly-active connection to the pool.

## License

MIT. Direct dependencies are MIT or MIT/Apache-2.0:

- `neotales-dbapi`, `neotales-dbapi-sqlite` — MIT
- `serde_json` — MIT OR Apache-2.0
- `tokio` (dev only) — MIT
- `tempfile` (dev only) — MIT OR Apache-2.0

The SQLite provider’s transitive dependencies are permissive as well: MIT, Apache-2.0,
Unicode-3.0, Unlicense OR MIT, and Zlib. No copyleft license appears in the default graph.

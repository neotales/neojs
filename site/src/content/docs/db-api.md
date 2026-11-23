---
title: neojs-dbapi
description: One SQL API for SQLite, PostgreSQL, MySQL, and SQL Server, with SQLite implemented end to end and no JavaScript adapter yet.
---

`neojs-dbapi` is the SQLite-only database foundation in the workspace. It is a Rust crate, not a
NeoJS JavaScript built-in. There is no JavaScript or Node adapter yet.

:::warning
A PostgreSQL, MySQL, or Microsoft SQL Server configuration parses, but `Db::open` returns
`DriverUnavailable` without opening a socket. There is no placeholder provider behind those names.
Only SQLite is implemented end to end.
:::

## Status

| Driver | Configuration | Binding compiler | Provider | End-to-end test |
| --- | --- | --- | --- | --- |
| SQLite | Yes | Yes | `neotales-dbapi-sqlite` | Yes, real temp databases |
| PostgreSQL | Yes | Yes | None | No |
| MySQL | Yes | Yes | None | No |
| Microsoft SQL Server | Yes | Yes | None | No |

## Install

```sh
cargo add neojs-dbapi
```

The default feature is `sqlite`. Build the compiler and configuration layer without a driver:

```sh
cargo add neojs-dbapi --no-default-features
```

The API is asynchronous and does not create a runtime. The SQLite provider runs on Tokio, so an
async caller must provide a Tokio runtime.

## Quick start

```rust
use neojs_dbapi::{Db, Param, Params};

#[tokio::main]
async fn main() -> Result<(), neojs_dbapi::Error> {
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
    Ok(())
}
```

## Configuration

The document must be a JSON object with an exact lower-case `driver`. Unknown fields, wrong types,
empty required strings, and out-of-range numbers are errors. Error text never repeats a
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
| `database` | Yes | Non-empty, no NUL |
| `busy_timeout_ms` | No | 1 to 2147483647, default 5000 |
| `max_connections` | No | 1 to 1024, default 5 |

Network drivers accept `host`, `port`, `database`, `user`, optional `password`, and optional `tls`.
Defaults are 5432, 3306, and 1433. They are parsed for a uniform schema but are unavailable in this
build. `password` is redacted from `Debug` and has no public accessor.

## Parameters

Named placeholders are `@name`; positional placeholders are `?`. A statement uses one form or the
other, never both. PostgreSQL requires the named form because a bare `?` is its jsonb operator; that
case is rejected rather than guessed. Names are case-sensitive and limited to ASCII letters, digits,
and underscores, starting with a letter or underscore.

```rust
use neojs_dbapi::{Param, Params};

let named = Params::named([("id", Param::I64(7)), ("name", Param::Text("Ada".to_owned()))]);
let positional = Params::positional([Param::I64(7), Param::Text("Ada".to_owned())]);
```

JSON parameters are an array for positional values or an object for named values:

```json
[7, "Ada", null, true, 1.5]
```

```json
{"id": 7, "name": "Ada"}
```

Values are always bound. The compiler emits `?` for SQLite and MySQL, `$1` for PostgreSQL named
parameters, and `@p1` for Microsoft SQL Server. It never interpolates a value into SQL and never uses
a regular expression.

:::caution
SQL text is still code. Placeholders protect values, not identifiers or SQL structure, so do not
build the statement itself from untrusted input.
:::

The scanner understands strings, quoted identifiers, comments, PostgreSQL dollar quoting, and
operators that contain `@` or `?`. It rejects ambiguity instead of guessing: `?NNN`, `:name`,
`$name`, `??`, bare `@`, mixed forms, multiple statements, and trigger bodies are errors. MySQL `--`
starts a comment only when the next character is whitespace or a control character, matching the
server rule; `SELECT 1--2` is not a comment. MySQL `/*! ... */` and MariaDB `/*M! ... */` executable
comments are rejected. A string whose closing boundary depends on a server escape mode (MySQL
`NO_BACKSLASH_ESCAPES` or PostgreSQL `standard_conforming_strings`) is rejected rather than guessed;
use an explicit PostgreSQL `E'...'`/`U&'...'` string or bind the value as a parameter. No
configuration flag forces a server mode.

## Rows and limits

Rows are the published `neotales_dbapi` `Row` type. `read`/`read_at` convert exactly; `Option<T>` is a
matching typed SQL null. `query` is bounded by `RowLimit::default()` (1,000 rows, 1 MiB);
`query_with_limit` takes an explicit bound. A limit is a hard failure, never a silent truncation.

## Transactions

```rust
use neojs_dbapi::{Db, Param, Params};

async fn insert_user(db: &Db) -> Result<(), neojs_dbapi::Error> {
    let mut connection = db.connection().await?;
    let mut transaction = connection.begin().await?;
    transaction
        .execute(
            "INSERT INTO users (id, name) VALUES (@id, @name)",
            &Params::named([("id", Param::I64(8)), ("name", Param::Text("Grace".to_owned()))]),
        )
        .await?;
    transaction.commit().await?;
    Ok(())
}
```

Dropping a transaction without commit or rollback leaves cleanup to the provider, which never
returns a possibly-active connection to the pool.

## Cancellation and uncertain outcomes

The API creates no task and no timeout. Dropping an in-flight future is the cancellation signal: no
result is returned. With SQLite, the provider's progress handler observes a cancellation flag that
the drop sets, and an interrupted dispatched statement discards that connection. The handler cannot
interrupt a blocked user-defined function, a VFS call, or a busy handler; `busy_timeout` bounds lock
waits but is not a prompt-cancellation guarantee. Effects of a dropped future are unobservable to
that call and must be treated as possibly applied, not as proof that nothing happened. Validation
failures detected before dispatch leave the session usable.

A returned error is different. The SQLite provider maps a standalone write with an unknown dispatch
outcome to `WriteOutcomeUnknown`, and a dispatched `COMMIT` error to `CommitOutcomeUnknown`. That
write mapping is narrower than "any error after a write": a fetch can run `INSERT ... RETURNING`,
and `Overflow`, `Decode`, `Constraint`, or another returned category after dispatch does not prove the
write had no effect. Transaction statements use the provider's general `map_error` and never produce
`WriteOutcomeUnknown`, but the same uncertainty can apply.

**Do not retry any post-dispatch error automatically.** Acquire a new connection and verify the
data. Use `Error::database_kind()` to distinguish the categories.

## Security model

- **Values are never interpolated.** The compiler produces SQL text and a separate parameter vector.
- **No regular expressions.** The scanner is a hand-written byte lexer over UTF-8 with ASCII
  delimiters.
- **Ambiguity fails closed.**
- **Configuration errors are static.** The error type holds a category and a fixed detail string. It
  never contains a path, host, user, password, SQL text, or driver message. `Debug` for a network
  configuration redacts host, database, user, and password.
- **No logging.** The crate has no logger and emits no output. The SQLite provider disables SQL
  statement logging.
- **No credentials from the environment, a file, or a secret store.** A network configuration may
  carry a password, but it is stored behind a redacted type and no provider reads it because no
  network provider is linked.

## Scope

There is no JavaScript or Node adapter in this crate yet. The default dependency graph contains no
V8, `deno_core`, or `neojs-core` dependency. The neojs built-in surface is a synchronous extension
loaded into the V8 realm; database calls are asynchronous and need a Promise bridge and a lifecycle
that survives the isolate.

The upstream contract is `neotales-dbapi` 0.0.0-alpha.0 and `neotales-dbapi-sqlite`
0.0.0-alpha.0 (MIT). This crate adds the portable binding layer and the strict configuration facade.

See `neojs-dbapi/README.md` and `docs/db-api.md` for the fuller design notes.

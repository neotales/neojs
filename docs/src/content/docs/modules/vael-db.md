---
title: vael:db
description: Async SQLite and driver-neutral SQL API with strict JSON config, uniform @name / ? binding, transactions, and redacted errors.
---

`vael:db` provides an asynchronous, SQLite-backed database API with strict JSON configuration, a driver-neutral `@name` / `?` parameter compiler, explicit single-connection transactions, and redacted error reporting.

:::note[Availability]
`vael:db` is available in **`vjs` (V8) only** when built with the `vael-db` Cargo feature (enabled by default; opt-outable with `--no-default-features`). It is **not** available in `vjs-lite`.
:::

## Exports

| Export | Signature | Description |
| --- | --- | --- |
| `openSqlite` | `(database?: string) => Promise<Database>` | Open a SQLite pool (defaults to `":memory:"`). |
| `open` | `(config: DbConfig \| string) => Promise<Database>` | Open a database from a strict config object or JSON string. |
| `DbError` | `new DbError(code: DbErrorCode, message: string, databaseKind?: string)` | Redacted error class (`name = "DbError"`). |

## Configuration & `:memory:` Behavior

`open(config)` accepts either a strict `DbConfig` object or a JSON string with no unknown fields:

- **`SqliteDbConfig`**: `{ driver: "sqlite", database: string, busy_timeout_ms?: number, max_connections?: number }`.
- **`NetworkDbConfig`**: `{ driver: "postgres" | "mysql" | "mssql", host: string, port?: number, database: string, user: string, password?: string, tls?: boolean }`.

Network configs are parsed strictly, but no network driver is linked in this build — calling `open()` with `"postgres"`, `"mysql"`, or `"mssql"` rejects with `DbError` (`code === "DRIVER_UNAVAILABLE"`) without opening a socket.

:::caution[SQLite `:memory:` Connection Pool]
SQLite `:memory:` databases opened via `openSqlite()` (or `open()` without an explicit `max_connections`) default to `max_connections: 1` so all queries share the same in-memory database rather than opening separate empty databases per pooled connection.
:::

## `Database` & Transactions

| Method | Signature | Behavior |
| --- | --- | --- |
| `execute` | `(sql: string, params?: DbParams) => Promise<{ rowsAffected: number }>` | Run a statement and return affected row count. |
| `query` | `(sql: string, params?: DbParams) => Promise<Record<string, unknown>[]>` | Return rows as plain objects keyed by column name. |
| `queryOne` | `(sql: string, params?: DbParams) => Promise<Record<string, unknown> \| null>` | Return at most one row, or `null`. Rejects with `"CARDINALITY"` if more than one row matches. |
| `transaction` | `<T>(fn: (tx: Database) => Promise<T> \| T) => Promise<T>` | Run `fn` inside a transaction (`COMMIT` on resolve, `ROLLBACK` on throw/reject). |
| `close` | `() => Promise<void>` | Close the pool handle. Idempotent; further calls reject with `"CLOSED"`. |
| `[Symbol.asyncDispose]` | `() => Promise<void>` | Async disposal hook (`await using`) delegating to `close()`. |

Inside `transaction(fn)`, always run statements against the `tx` handle passed to `fn` rather than the outer `db` pool. Calling `tx.transaction()` (nested transactions) or `tx.close()` directly rejects with `code === "SQL"`, and using `tx` after `transaction()` settles rejects with `code === "CLOSED"`.

## Parameter Binding (`@name` and `?`)

`DbParams` accepts `undefined`, `null`, an array for positional `?` placeholders, or a plain object for named `@name` placeholders (case-sensitive). Values must be `null | boolean | number | bigint | string | Uint8Array` (`DbParamValue`).

The lexical binding compiler rewrites placeholders outside string literals, quoted identifiers, and comments, and fails closed on ambiguous syntax:

- Rejected forms: `?NNN`, `:name`, `$name`, `??`, bare `@`, mixing named and positional placeholders in one statement, and multiple SQL statements.
- Rejected parameter values: `NaN`, `Infinity`, `bigint` outside signed 64-bit integer range, `Proxy` objects, accessor getters, and non-plain objects (reject with `"INVALID_PARAM"`).
- Integer decoding: signed 64-bit integers (`i64`) return `number` inside `[Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER]` and `bigint` outside it.

## Limits & Error Redaction

- **Row & byte limits**: `query()` and `queryOne()` enforce `RowLimit::default()` (`1,000` rows, `1 MiB` total). Exceeding the limit rejects with `DbError` (`code === "DATABASE"`, `databaseKind === "Overflow"`).
- **Parameter cap**: At most `4,096` parameters per statement (`"INVALID_PARAM"`).
- **Redaction**: `DbError.message` uses static redacted descriptions and never echoes SQL text, filesystem paths, hosts, usernames, or passwords.

| `DbErrorCode` | Meaning |
| --- | --- |
| `"CONFIG"` | Invalid or unserializable configuration object/JSON. |
| `"DRIVER_UNAVAILABLE"` | Valid network config whose driver is not linked (`postgres`, `mysql`, `mssql`). |
| `"SQL"` | Invalid SQL syntax, disallowed multi-statement, or invalid `tx.close()` / nested `tx.transaction()`. |
| `"PARAMETER"` | Missing, unused, or mismatched `@name` / `?` placeholder binding. |
| `"CARDINALITY"` | `queryOne()` matched more than one row. |
| `"DATABASE"` | Provider/SQLite execution error (check `err.databaseKind`, e.g. `"Constraint"`, `"Overflow"`). |
| `"CLOSED"` | Operation attempted on a closed `Database` or settled `tx` handle. |
| `"INVALID_PARAM"` | Unsupported JS value type, `NaN`/`Infinity`, out-of-range `bigint`, or > `4,096` parameters. |

## Example

```ts
import { openSqlite } from "vael:db";

await using db = await openSqlite(":memory:");

await db.execute(
  "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE)",
);

await db.transaction(async (tx) => {
  await tx.execute("INSERT INTO users (email) VALUES (@email)", {
    email: "ada@example.com",
  });
});

const user = await db.queryOne("SELECT id, email FROM users WHERE id = ?", [1]);
console.log(user); // { id: 1, email: "ada@example.com" }
```

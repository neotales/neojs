/**
 * Bounded SQLite database queries, transactions, and strict JSON configuration.
 *
 * Available in the `vjs` (`vaeljs`) runtime only; absent in `vjs-lite`.
 * Provides async SQLite queries, parameterized execution, and transactions with
 * bounded row limits and automatic `COMMIT`/`ROLLBACK` handling.
 *
 * @example
 * ```ts
 * import { openSqlite } from "vael:db";
 *
 * const db = await openSqlite(":memory:");
 * try {
 *   await db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)");
 *   await db.execute("INSERT INTO users (name) VALUES (?)", ["Ada"]);
 *   const rows = await db.query("SELECT id, name FROM users");
 *   console.log(rows);
 * } finally {
 *   await db.close();
 * }
 * ```
 *
 * @since 0.0.0-alpha.0
 */
declare module "vael:db" {
  /**
   * Stable screaming-snake-case error codes reported on `DbError.code`.
   *
   * @since 0.0.0-alpha.0
   */
  export type DbErrorCode =
    | "CONFIG"
    | "DRIVER_UNAVAILABLE"
    | "SQL"
    | "PARAMETER"
    | "CARDINALITY"
    | "DATABASE"
    | "CLOSED"
    | "INVALID_PARAM";

  /**
   * Redacted database API error with a stable `code`.
   *
   * @since 0.0.0-alpha.0
   */
  export class DbError extends Error {
    /**
     * Error name (`"DbError"`).
     *
     * @since 0.0.0-alpha.0
     */
    override name: "DbError";

    /**
     * Machine-readable database error code.
     *
     * @since 0.0.0-alpha.0
     */
    code: DbErrorCode;

    /**
     * Optional underlying provider error category (for example `"Overflow"`).
     *
     * @since 0.0.0-alpha.0
     */
    readonly databaseKind?: string;

    /**
     * Construct a `DbError`.
     *
     * @param code Machine-readable `DbErrorCode`.
     * @param message Human-readable redacted error message.
     * @param databaseKind Optional provider error category.
     * @since 0.0.0-alpha.0
     */
    constructor(code: DbErrorCode, message: string, databaseKind?: string);
  }

  /**
   * Strict SQLite database configuration.
   *
   * @since 0.0.0-alpha.0
   */
  export interface SqliteDbConfig {
    /** Driver identifier (`"sqlite"`). @since 0.0.0-alpha.0 */
    driver: "sqlite";
    /** File path or `":memory:"` for an in-memory SQLite database. @since 0.0.0-alpha.0 */
    database: string;
    /** SQLite busy timeout in milliseconds. @since 0.0.0-alpha.0 */
    busy_timeout_ms?: number;
    /** Maximum connection pool size (defaults to `1` for `":memory:"`). @since 0.0.0-alpha.0 */
    max_connections?: number;
  }

  /**
   * Network database configuration (parsed strictly; no network provider is linked in this build).
   *
   * @since 0.0.0-alpha.0
   */
  export interface NetworkDbConfig {
    /** Network SQL driver name. @since 0.0.0-alpha.0 */
    driver: "postgres" | "mysql" | "mssql";
    /** Database server hostname or IP address. @since 0.0.0-alpha.0 */
    host: string;
    /** Database server TCP port. @since 0.0.0-alpha.0 */
    port?: number;
    /** Database or catalog name. @since 0.0.0-alpha.0 */
    database: string;
    /** Username for authentication. @since 0.0.0-alpha.0 */
    user: string;
    /** Optional password credential. @since 0.0.0-alpha.0 */
    password?: string;
    /** Whether to enable TLS for the connection. @since 0.0.0-alpha.0 */
    tls?: boolean;
  }

  /**
   * Strict JSON configuration accepted by `open(config)`.
   *
   * @since 0.0.0-alpha.0
   */
  export type DbConfig = SqliteDbConfig | NetworkDbConfig;

  /**
   * Single scalar or binary SQL parameter value.
   *
   * @since 0.0.0-alpha.0
   */
  export type DbParamValue = null | boolean | number | bigint | string | Uint8Array;

  /**
   * Positional (`?`) or named (`@name`) parameters for a SQL statement.
   *
   * @since 0.0.0-alpha.0
   */
  export type DbParams =
    | undefined
    | null
    | readonly DbParamValue[]
    | Record<string, DbParamValue>;

  /**
   * Result returned by `Database.execute()`.
   *
   * @since 0.0.0-alpha.0
   */
  export interface DbExecuteResult {
    /**
     * Number of rows inserted, updated, or deleted by the statement.
     *
     * @since 0.0.0-alpha.0
     */
    rowsAffected: number;
  }

  /**
   * Handle to an open database pool or active single-connection transaction.
   *
   * Note: SQLite `:memory:` databases opened via `openSqlite()` (or `open()` without
   * an explicit `max_connections`) default to `max_connections: 1` so queries share
   * the same in-memory database. Inside `transaction(fn)`, always use the `tx` handle
   * passed to `fn` rather than the outer `db` handle.
   *
   * @since 0.0.0-alpha.0
   */
  export interface Database {
    /**
     * Execute a statement with optional positional or named parameters.
     *
     * @param sql SQL statement to execute.
     * @param params Optional positional array (`?`) or named parameter record (`@name`).
     * @returns Promise resolving to `{ rowsAffected }`.
     * @throws {TypeError} on wrong argument types.
     * @throws {DbError} with `code` `"SQL"`, `"PARAMETER"`, `"DATABASE"`, `"CLOSED"`, or `"INVALID_PARAM"`.
     * @example
     * ```ts
     * import { openSqlite } from "vael:db";
     *
     * const db = await openSqlite();
     * await db.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT)");
     * const res = await db.execute("INSERT INTO items (name) VALUES (@name)", { name: "book" });
     * console.log(res.rowsAffected); // 1
     * await db.close();
     * ```
     * @since 0.0.0-alpha.0
     */
    execute(sql: string, params?: DbParams): Promise<DbExecuteResult>;

    /**
     * Query a bounded batch of rows as plain objects keyed by column name.
     *
     * Bounded by `RowLimit::default()` (1,000 rows, 1 MiB); exceeding the limit
     * rejects with `DbError` (`code === "DATABASE"`, `databaseKind === "Overflow"`).
     * Signed 64-bit integer columns (`i64`) return `number` within
     * `[Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER]` and `bigint` outside it.
     *
     * @param sql SQL query to execute.
     * @param params Optional positional array or named parameter record.
     * @returns Promise resolving to an array of row objects keyed by column name.
     * @throws {TypeError} on wrong argument types.
     * @throws {DbError} with `code` `"SQL"`, `"PARAMETER"`, `"DATABASE"`, `"CLOSED"`, or `"INVALID_PARAM"`.
     * @example
     * ```ts
     * import { openSqlite } from "vael:db";
     *
     * const db = await openSqlite();
     * const rows = await db.query("SELECT 1 + ? AS sum", [41]);
     * console.log(rows[0].sum); // 42
     * await db.close();
     * ```
     * @since 0.0.0-alpha.0
     */
    query(sql: string, params?: DbParams): Promise<Record<string, unknown>[]>;

    /**
     * Query at most one row, or `null` when no row matched.
     * Rejects with `code === "CARDINALITY"` if more than one row matched.
     * Subject to the same `RowLimit::default()` (1,000 rows, 1 MiB) and
     * `i64` -> `number` | `bigint` conversion rules as `query()`.
     *
     * @param sql SQL query expected to return zero or one row.
     * @param params Optional positional array or named parameter record.
     * @returns Promise resolving to the single matching row object, or `null`.
     * @throws {TypeError} on wrong argument types.
     * @throws {DbError} with `code` `"CARDINALITY"`, `"SQL"`, `"PARAMETER"`, `"DATABASE"`, `"CLOSED"`, or `"INVALID_PARAM"`.
     * @since 0.0.0-alpha.0
     */
    queryOne(sql: string, params?: DbParams): Promise<Record<string, unknown> | null>;

    /**
     * Execute `fn` inside a database transaction (`COMMIT` on resolve, `ROLLBACK` on reject).
     *
     * Calling `tx.transaction()` (nested transactions) or `tx.close()` directly on the
     * transaction handle rejects with `DbError` (`code === "SQL"`). Any use of `tx` after
     * `transaction()` settles rejects with `DbError` (`code === "CLOSED"`).
     *
     * @param fn Async or synchronous callback receiving the single-connection transaction handle `tx`.
     * @returns Promise resolving to the return value of `fn`.
     * @throws {TypeError} when `fn` is not a function.
     * @throws {DbError} with `code` `"SQL"`, `"DATABASE"`, or `"CLOSED"`.
     * @example
     * ```ts
     * import { openSqlite } from "vael:db";
     *
     * const db = await openSqlite();
     * await db.execute("CREATE TABLE counts (n INTEGER)");
     * await db.transaction(async (tx) => {
     *   await tx.execute("INSERT INTO counts (n) VALUES (?)", [1]);
     * });
     * await db.close();
     * ```
     * @since 0.0.0-alpha.0
     */
    transaction<T>(fn: (tx: Database) => Promise<T> | T): Promise<T>;

    /**
     * Close the handle and release underlying resources. Idempotent.
     * Subsequent operations reject with `code === "CLOSED"`.
     * Calling `close()` on a transaction handle rejects with `code === "SQL"`.
     *
     * @returns Promise resolving once resources are released.
     * @throws {DbError} with `code === "SQL"` if called on an active transaction handle.
     * @since 0.0.0-alpha.0
     */
    close(): Promise<void>;

    /**
     * Async disposal hook (`await using`) delegating to `close()`.
     *
     * @returns Promise resolving once the database handle is closed.
     * @since 0.0.0-alpha.0
     */
    [Symbol.asyncDispose](): Promise<void>;
  }

  /**
   * Open a database from a strict configuration object or JSON configuration string.
   *
   * @param config Strict `DbConfig` object or JSON configuration string.
   * @returns Promise resolving to an open `Database` handle.
   * @throws {TypeError} on invalid argument type.
   * @throws {DbError} with `code` `"CONFIG"`, `"DRIVER_UNAVAILABLE"`, or `"DATABASE"`.
   * @example
   * ```ts
   * import { open } from "vael:db";
   *
   * const db = await open({ driver: "sqlite", database: ":memory:" });
   * await db.close();
   * ```
   * @since 0.0.0-alpha.0
   */
  export function open(config: DbConfig | string): Promise<Database>;

  /**
   * Open a SQLite database (defaults to `":memory:"`).
   *
   * @param database File path or `":memory:"` (defaults to `":memory:"`).
   * @returns Promise resolving to an open `Database` handle.
   * @throws {TypeError} when `database` is defined and not a string.
   * @throws {DbError} with `code` `"CONFIG"` or `"DATABASE"`.
   * @example
   * ```ts
   * import { openSqlite } from "vael:db";
   *
   * const db = await openSqlite(":memory:");
   * const row = await db.queryOne("SELECT 1 AS ok");
   * console.log(row?.ok); // 1
   * await db.close();
   * ```
   * @since 0.0.0-alpha.0
   */
  export function openSqlite(database?: string): Promise<Database>;
}

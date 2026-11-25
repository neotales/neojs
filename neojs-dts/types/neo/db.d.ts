declare module "neo:db" {
  /**
   * Stable screaming-snake-case error codes reported on `DbError.code`.
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
   */
  export class DbError extends Error {
    override name: "DbError";
    code: DbErrorCode;
    readonly databaseKind?: string;
    constructor(code: DbErrorCode, message: string, databaseKind?: string);
  }

  /**
   * Strict SQLite database configuration.
   */
  export interface SqliteDbConfig {
    driver: "sqlite";
    database: string;
    busy_timeout_ms?: number;
    max_connections?: number;
  }

  /**
   * Network database configuration (parsed strictly; no network provider is linked in this build).
   */
  export interface NetworkDbConfig {
    driver: "postgres" | "mysql" | "mssql";
    host: string;
    port?: number;
    database: string;
    user: string;
    password?: string;
    tls?: boolean;
  }

  /**
   * Strict JSON configuration accepted by `open(config)`.
   */
  export type DbConfig = SqliteDbConfig | NetworkDbConfig;

  /**
   * Single scalar or binary SQL parameter value.
   */
  export type DbParamValue = null | boolean | number | bigint | string | Uint8Array;

  /**
   * Positional (`?`) or named (`@name`) parameters for a SQL statement.
   */
  export type DbParams =
    | undefined
    | null
    | readonly DbParamValue[]
    | Record<string, DbParamValue>;

  /**
   * Result returned by `Database.execute()`.
   */
  export interface DbExecuteResult {
    rowsAffected: number;
  }

  /**
   * Handle to an open database pool or active single-connection transaction.
   *
   * Note: SQLite `:memory:` databases opened via `openSqlite()` (or `open()` without
   * an explicit `max_connections`) default to `max_connections: 1` so queries share
   * the same in-memory database. Inside `transaction(fn)`, always use the `tx` handle
   * passed to `fn` rather than the outer `db` handle.
   */
  export interface Database {
    /**
     * Execute a statement with optional positional or named parameters.
     */
    execute(sql: string, params?: DbParams): Promise<DbExecuteResult>;

    /**
     * Query a bounded batch of rows as plain objects keyed by column name.
     *
     * Bounded by `RowLimit::default()` (1,000 rows, 1 MiB); exceeding the limit
     * rejects with `DbError` (`code === "DATABASE"`, `databaseKind === "Overflow"`).
     * Signed 64-bit integer columns (`i64`) return `number` within
     * `[Number.MIN_SAFE_INTEGER, Number.MAX_SAFE_INTEGER]` and `bigint` outside it.
     */
    query(sql: string, params?: DbParams): Promise<Record<string, unknown>[]>;

    /**
     * Query at most one row, or `null` when no row matched.
     * Rejects with `code === "CARDINALITY"` if more than one row matched.
     * Subject to the same `RowLimit::default()` (1,000 rows, 1 MiB) and
     * `i64` -> `number` | `bigint` conversion rules as `query()`.
     */
    queryOne(sql: string, params?: DbParams): Promise<Record<string, unknown> | null>;

    /**
     * Execute `fn` inside a database transaction (`COMMIT` on resolve, `ROLLBACK` on reject).
     *
     * Calling `tx.transaction()` (nested transactions) or `tx.close()` directly on the
     * transaction handle rejects with `DbError` (`code === "SQL"`). Any use of `tx` after
     * `transaction()` settles rejects with `DbError` (`code === "CLOSED"`).
     */
    transaction<T>(fn: (tx: Database) => Promise<T> | T): Promise<T>;

    /**
     * Close the handle and release underlying resources. Idempotent.
     * Subsequent operations reject with `code === "CLOSED"`.
     * Calling `close()` on a transaction handle rejects with `code === "SQL"`.
     */
    close(): Promise<void>;

    /**
     * Async disposal hook (`await using`) delegating to `close()`.
     */
    [Symbol.asyncDispose](): Promise<void>;
  }

  /**
   * Open a database from a strict configuration object or JSON configuration string.
   */
  export function open(config: DbConfig | string): Promise<Database>;

  /**
   * Open a SQLite database (defaults to `":memory:"`).
   */
  export function openSqlite(database?: string): Promise<Database>;
}

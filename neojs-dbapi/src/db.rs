//! The connection facade.
//!
//! [`Db`] owns a provider pool. [`Connection`] borrows one exclusive session
//! from it, and [`Transaction`] borrows a connection. Rows and values are the
//! published [`neotales_dbapi`] types, so provider-neutral code does not need a
//! second value model.

use std::fmt;

#[cfg(feature = "sqlite")]
use neotales_dbapi::Pool as _;
use neotales_dbapi::{
    QueryOptions, Row, RowLimit, Session as DbSession, Transaction as DbTransaction, TxOptions,
};

use crate::Params;
use crate::bind::{CompiledSql, Dialect, compile};
use crate::config::{DbConfig, Driver};
use crate::error::Error;

#[cfg(feature = "sqlite")]
use neotales_dbapi_sqlite::{SqliteConfig as ProviderSqliteConfig, SqlitePoolProvider};

/// A provider pool with one configured driver.
///
/// Only SQLite is linked in this build. Opening a PostgreSQL, MySQL, or
/// Microsoft SQL Server configuration returns
/// [`crate::ErrorKind::DriverUnavailable`] without attempting a connection.
///
/// # Cancellation and uncertain outcomes
///
/// This type creates no task or timeout. Dropping an in-flight future is the
/// cancellation signal: no `Result` is returned, the provider's progress
/// handler observes the cancellation flag that the drop sets, and an
/// interrupted dispatched statement discards the connection. The handler
/// interrupts SQLite VM execution but cannot interrupt a blocked user-defined
/// function, a VFS call, or a busy handler; `busy_timeout` bounds a lock wait
/// and is not a prompt cancellation guarantee. Effects of a dropped future are
/// unobservable to that call and must be treated as possibly applied, not as
/// proof that nothing happened. A validation failure detected before dispatch
/// leaves the session usable.
///
/// A returned error is different from a dropped future. The SQLite provider
/// maps a standalone write whose dispatch outcome is unknown to
/// `WriteOutcomeUnknown`, and a dispatched `COMMIT` error to
/// `CommitOutcomeUnknown`. Those mappings are narrower than "any error after a
/// write": a fetch can run `INSERT ... RETURNING`, and an `Overflow`, `Decode`,
/// `Constraint`, or other returned category after dispatch does not prove the
/// write had no effect. Do not retry any post-dispatch error automatically;
/// verify on a new connection. Neither category is returned when the future is
/// dropped, because there is no result to return. Use
/// [`Error::database_kind`].
pub struct Db {
    driver: Driver,
    dialect: Dialect,
    #[cfg(feature = "sqlite")]
    backend: Backend,
}

#[cfg(feature = "sqlite")]
enum Backend {
    Sqlite(SqlitePoolProvider),
}

impl Db {
    /// Opens a pool from validated configuration.
    ///
    /// # Errors
    /// Returns [`crate::ErrorKind::Config`] for invalid values,
    /// [`crate::ErrorKind::DriverUnavailable`] when the driver has no linked
    /// provider, or [`crate::ErrorKind::Database`] when the provider cannot
    /// open the database.
    pub async fn open(config: &DbConfig) -> Result<Self, Error> {
        config.validate()?;
        match config {
            DbConfig::Sqlite(options) => {
                #[cfg(feature = "sqlite")]
                {
                    let provider = SqlitePoolProvider::connect(
                        ProviderSqliteConfig::new(options.database())
                            .busy_timeout(options.busy_timeout())
                            .max_connections(options.max_connections()),
                    )
                    .await
                    .map_err(Error::from)?;
                    Ok(Self {
                        driver: Driver::Sqlite,
                        dialect: Dialect::Sqlite,
                        backend: Backend::Sqlite(provider),
                    })
                }
                #[cfg(not(feature = "sqlite"))]
                {
                    let _ = options;
                    Err(Error::driver_unavailable())
                }
            }
            DbConfig::Postgres(_) | DbConfig::MySql(_) | DbConfig::Mssql(_) => {
                Err(Error::driver_unavailable())
            }
        }
    }

    /// Parses strict JSON configuration and opens a pool.
    ///
    /// # Errors
    /// Returns the same errors as [`DbConfig::from_json`] and [`Db::open`].
    pub async fn open_json(json: &str) -> Result<Self, Error> {
        Self::open(&DbConfig::from_json(json)?).await
    }

    /// Returns the configured driver.
    #[must_use]
    pub const fn driver(&self) -> Driver {
        self.driver
    }

    /// Returns the dialect used to compile portable placeholders.
    #[must_use]
    pub const fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Executes one statement and returns the affected-row count.
    ///
    /// A standalone write that returns with an unknown dispatch outcome is
    /// reported as `WriteOutcomeUnknown`. Dropping the future returns no error
    /// and has the unobservable effects described on [`Db`]. Check
    /// [`Error::database_kind`] before retrying; this facade does not infer
    /// replay safety from SQL shape.
    ///
    /// # Errors
    /// Returns a SQL, parameter, or provider error.
    pub async fn execute(&self, sql: &str, params: &Params) -> Result<u64, Error> {
        let compiled = compile(sql, params, self.dialect)?;
        #[cfg(feature = "sqlite")]
        {
            let Backend::Sqlite(provider) = &self.backend;
            let options = QueryOptions::new().bind_all(compiled.parameters());
            provider
                .execute_async(compiled.sql(), options)
                .await
                .map_err(Error::from)
        }
        #[cfg(not(feature = "sqlite"))]
        {
            let _ = compiled;
            Err(Error::driver_unavailable())
        }
    }

    /// Fetches one bounded batch using the default [`RowLimit`].
    ///
    /// Fetching does not imply read-only SQL: `INSERT ... RETURNING` can write.
    /// The SQLite provider maps a standalone fetch row-stream `Connection`,
    /// `Timeout`, or `Cancelled` error to `WriteOutcomeUnknown`, but `Overflow`
    /// from a row or byte limit, `Decode`, and other categories are returned as
    /// themselves. Any error after the statement was dispatched can still leave
    /// the write's effects uncertain, even when the category is not
    /// `WriteOutcomeUnknown`. Do not retry automatically; verify the data.
    ///
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query(&self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        self.query_with_limit(sql, params, RowLimit::default())
            .await
    }

    /// Fetches one bounded batch with an explicit limit.
    ///
    /// A limit is enforced after the statement is dispatched, so an `Overflow`
    /// after `INSERT ... RETURNING` does not mean the write did not happen. See
    /// [`Db::query`] for the post-dispatch error rules.
    ///
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query_with_limit(
        &self,
        sql: &str,
        params: &Params,
        limit: RowLimit,
    ) -> Result<Vec<Row>, Error> {
        let compiled = compile(sql, params, self.dialect)?;
        self.fetch(&compiled, limit).await
    }

    /// Fetches a statement whose result set must contain at most one row.
    ///
    /// The whole bounded batch is drained before the cardinality is decided. A
    /// statement such as `INSERT ... RETURNING` can write, and any post-dispatch
    /// error, including `Overflow` or `Decode`, can leave its effects uncertain;
    /// see [`Db::query`].
    ///
    /// # Errors
    /// Returns [`crate::ErrorKind::Cardinality`] for more than one row, plus
    /// the errors of [`Db::query`].
    pub async fn query_one(&self, sql: &str, params: &Params) -> Result<Option<Row>, Error> {
        let mut rows = self.query(sql, params).await?;
        match rows.len() {
            0 => Ok(None),
            1 => Ok(rows.pop()),
            _ => Err(Error::cardinality()),
        }
    }

    /// Acquires one exclusive connection from the pool.
    ///
    /// The connection is discarded, not returned to the pool, if a dispatched
    /// statement fails or is cancelled.
    ///
    /// # Errors
    /// Returns [`crate::ErrorKind::DriverUnavailable`] when no provider is
    /// linked, or [`crate::ErrorKind::Database`] when the pool cannot acquire
    /// a connection.
    pub async fn connection(&self) -> Result<Connection, Error> {
        #[cfg(feature = "sqlite")]
        {
            let Backend::Sqlite(provider) = &self.backend;
            let session = provider.acquire_async().await.map_err(Error::from)?;
            Ok(Connection {
                inner: session,
                dialect: self.dialect,
            })
        }
        #[cfg(not(feature = "sqlite"))]
        {
            Err(Error::driver_unavailable())
        }
    }

    #[cfg(feature = "sqlite")]
    async fn fetch(&self, compiled: &CompiledSql, limit: RowLimit) -> Result<Vec<Row>, Error> {
        let Backend::Sqlite(provider) = &self.backend;
        let options = QueryOptions::new()
            .bind_all(compiled.parameters())
            .limit(limit);
        provider
            .fetch_all_async(compiled.sql(), options)
            .await
            .map_err(Error::from)
    }

    #[cfg(not(feature = "sqlite"))]
    async fn fetch(&self, _compiled: &CompiledSql, _limit: RowLimit) -> Result<Vec<Row>, Error> {
        Err(Error::driver_unavailable())
    }
}

impl fmt::Debug for Db {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Db")
            .field("driver", &self.driver)
            .finish_non_exhaustive()
    }
}

/// An exclusive session acquired from [`Db`].
///
/// Dropping a connection returns it to the pool. A session that a failed or
/// cancelled dispatched statement invalidated is discarded by the provider
/// instead. Dropping an in-flight future on this connection is the cancellation
/// signal; the SQLite progress handler cannot interrupt a blocked user
/// function, VFS call, or busy handler, and `busy_timeout` bounds lock waits
/// rather than promising prompt cancellation.
pub struct Connection {
    inner: Box<dyn DbSession>,
    dialect: Dialect,
}

impl Connection {
    /// Executes one statement on this connection.
    ///
    /// A standalone write that returns with an unknown dispatch outcome is
    /// reported as `WriteOutcomeUnknown`. Dropping the future returns no error
    /// and has the unobservable effects described on [`Db`]. A dispatched
    /// failure or interruption discards this connection; acquire a new one
    /// rather than assuming the session is usable.
    ///
    /// # Errors
    /// Returns a SQL, parameter, or provider error.
    pub async fn execute(&mut self, sql: &str, params: &Params) -> Result<u64, Error> {
        let compiled = compile(sql, params, self.dialect)?;
        let options = QueryOptions::new().bind_all(compiled.parameters());
        self.inner
            .execute_async(compiled.sql(), options)
            .await
            .map_err(Error::from)
    }

    /// Fetches one bounded batch on this connection.
    ///
    /// Fetching does not imply read-only SQL. For a standalone fetch, the SQLite
    /// provider maps row-stream `Connection`, `Timeout`, and `Cancelled` errors
    /// to `WriteOutcomeUnknown`, but `Overflow`, `Decode`, and other returned
    /// categories are not that mapping. Any error after dispatch can still leave
    /// an `INSERT ... RETURNING` write uncertain; do not retry automatically.
    ///
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query(&mut self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        self.query_with_limit(sql, params, RowLimit::default())
            .await
    }

    /// Fetches one bounded batch with an explicit limit.
    ///
    /// A limit overflow can follow a dispatched `INSERT ... RETURNING` write.
    /// See [`Connection::query`] for the post-dispatch error rules.
    ///
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query_with_limit(
        &mut self,
        sql: &str,
        params: &Params,
        limit: RowLimit,
    ) -> Result<Vec<Row>, Error> {
        let compiled = compile(sql, params, self.dialect)?;
        let options = QueryOptions::new()
            .bind_all(compiled.parameters())
            .limit(limit);
        self.inner
            .fetch_all_async(compiled.sql(), options)
            .await
            .map_err(Error::from)
    }

    /// Fetches at most one row and drains the bounded batch.
    ///
    /// A writing fetch can fail after dispatch with `Overflow`, `Decode`, or
    /// another category; see [`Connection::query`]. Automatic retry is not safe.
    ///
    /// # Errors
    /// Returns [`crate::ErrorKind::Cardinality`] for more than one row, plus
    /// the errors of [`Connection::query`].
    pub async fn query_one(&mut self, sql: &str, params: &Params) -> Result<Option<Row>, Error> {
        let mut rows = self.query(sql, params).await?;
        match rows.len() {
            0 => Ok(None),
            1 => Ok(rows.pop()),
            _ => Err(Error::cardinality()),
        }
    }

    /// Begins a transaction that borrows this connection.
    ///
    /// Dropping an in-flight begin future is a cancellation. The provider must
    /// discard the connection rather than return one whose transaction state is
    /// unknown.
    ///
    /// # Errors
    /// Returns a provider error. SQLite does not accept isolation-level or
    /// read-only requests here; this method requests neither.
    pub async fn begin(&mut self) -> Result<Transaction<'_>, Error> {
        let inner = self
            .inner
            .begin_async(TxOptions::default())
            .await
            .map_err(Error::from)?;
        Ok(Transaction {
            inner,
            dialect: self.dialect,
        })
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Connection")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

/// A transaction borrowed from a [`Connection`].
///
/// Dropping without [`Transaction::commit`] or [`Transaction::rollback`] leaves
/// cleanup to the provider, which must not return a possibly-active connection
/// to the pool. A dispatched statement failure or an interrupted future may
/// invalidate the borrowed connection; acquire a new one rather than assuming
/// the transaction is still usable. Dropping an in-flight future is the
/// cancellation signal, and the SQLite progress handler has the limits
/// described on [`Connection`].
///
/// A statement inside the transaction can write. Transaction statements are
/// mapped with `map_error`, not the standalone write mapper, so none of the
/// returned categories is `WriteOutcomeUnknown`. Any post-dispatch error, and in
/// particular any error from a fetch that can write, may still leave the write's
/// effects uncertain; it is not proof of no effect.
pub struct Transaction<'a> {
    inner: Box<dyn DbTransaction<'a> + 'a>,
    dialect: Dialect,
}

impl Transaction<'_> {
    /// Executes one statement inside this transaction.
    ///
    /// The SQLite provider maps a transaction statement error with its general
    /// mapper, not the standalone write mapper. A returned `Connection`,
    /// `Timeout`, or `Cancelled` error does not mean the statement had no
    /// effect, and `WriteOutcomeUnknown` is not produced here. The connection is
    /// discarded after a dispatched failure; do not reuse the transaction.
    ///
    /// # Errors
    /// Returns a SQL, parameter, or provider error.
    pub async fn execute(&mut self, sql: &str, params: &Params) -> Result<u64, Error> {
        let compiled = compile(sql, params, self.dialect)?;
        let options = QueryOptions::new().bind_all(compiled.parameters());
        self.inner
            .execute_async(compiled.sql(), options)
            .await
            .map_err(Error::from)
    }

    /// Fetches one bounded batch inside this transaction.
    ///
    /// Fetching does not imply read-only SQL. Transaction fetches use the
    /// provider's general `map_error`, so `Connection`, `Timeout`, `Cancelled`,
    /// `Overflow`, and `Decode` are returned as themselves and never as
    /// `WriteOutcomeUnknown`. Any error after dispatch can still leave an
    /// `INSERT ... RETURNING` write uncertain. Do not retry automatically.
    ///
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query(&mut self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        self.query_with_limit(sql, params, RowLimit::default())
            .await
    }

    /// Fetches one bounded batch with an explicit limit.
    ///
    /// A limit overflow can follow a dispatched `INSERT ... RETURNING` write.
    /// See [`Connection::query`] for the post-dispatch error rules.
    ///
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query_with_limit(
        &mut self,
        sql: &str,
        params: &Params,
        limit: RowLimit,
    ) -> Result<Vec<Row>, Error> {
        let compiled = compile(sql, params, self.dialect)?;
        let options = QueryOptions::new()
            .bind_all(compiled.parameters())
            .limit(limit);
        self.inner
            .fetch_all_async(compiled.sql(), options)
            .await
            .map_err(Error::from)
    }

    /// Fetches at most one row and drains the bounded batch.
    ///
    /// A writing fetch can fail after dispatch; see [`Transaction::query`].
    /// Automatic retry is not safe.
    ///
    /// # Errors
    /// Returns [`crate::ErrorKind::Cardinality`] for more than one row, plus
    /// the errors of [`Transaction::query`].
    pub async fn query_one(&mut self, sql: &str, params: &Params) -> Result<Option<Row>, Error> {
        let mut rows = self.query(sql, params).await?;
        match rows.len() {
            0 => Ok(None),
            1 => Ok(rows.pop()),
            _ => Err(Error::cardinality()),
        }
    }

    /// Commits the transaction and returns the borrowed connection.
    ///
    /// If `COMMIT` was dispatched and returned an error, the SQLite provider
    /// reports `CommitOutcomeUnknown` and discards the connection. That category
    /// means the commit may have succeeded; it is not a rollback. If the future
    /// is dropped instead, no category is returned and success is unobservable.
    /// In either case, do not retry the commit on this transaction. Acquire a new
    /// connection and verify the data instead.
    ///
    /// # Errors
    /// Returns a provider error, including `CommitOutcomeUnknown`.
    pub async fn commit(self) -> Result<(), Error> {
        self.inner.commit_async().await.map_err(Error::from)
    }

    /// Rolls the transaction back and returns the borrowed connection.
    ///
    /// A returned rollback error is reported as `Connection` after the provider
    /// discards the connection; the rollback is not confirmed. Dropping an
    /// in-flight rollback future returns no error and leaves an unobservable
    /// outcome; the connection is discarded if the drop interrupts a dispatched
    /// rollback.
    ///
    /// # Errors
    /// Returns a provider error.
    pub async fn rollback(self) -> Result<(), Error> {
        self.inner.rollback_async().await.map_err(Error::from)
    }
}

impl fmt::Debug for Transaction<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Transaction")
            .field("dialect", &self.dialect)
            .finish_non_exhaustive()
    }
}

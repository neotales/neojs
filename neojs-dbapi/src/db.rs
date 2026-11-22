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
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query(&self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        self.query_with_limit(sql, params, RowLimit::default())
            .await
    }

    /// Fetches one bounded batch with an explicit limit.
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
    /// The whole bounded batch is drained before the cardinality is decided.
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
/// Dropping a connection returns it to the pool. A session that a failed
/// statement invalidated is discarded by the provider instead.
pub struct Connection {
    inner: Box<dyn DbSession>,
    dialect: Dialect,
}

impl Connection {
    /// Executes one statement on this connection.
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
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query(&mut self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        self.query_with_limit(sql, params, RowLimit::default())
            .await
    }

    /// Fetches one bounded batch with an explicit limit.
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
/// to the pool. A dispatched statement failure may invalidate the borrowed
/// connection; acquire a new one rather than assuming the transaction is still
/// usable.
pub struct Transaction<'a> {
    inner: Box<dyn DbTransaction<'a> + 'a>,
    dialect: Dialect,
}

impl Transaction<'_> {
    /// Executes one statement inside this transaction.
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
    /// # Errors
    /// Returns a SQL, parameter, limit, or provider error.
    pub async fn query(&mut self, sql: &str, params: &Params) -> Result<Vec<Row>, Error> {
        self.query_with_limit(sql, params, RowLimit::default())
            .await
    }

    /// Fetches one bounded batch with an explicit limit.
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
    /// # Errors
    /// Returns a provider error, including an unconfirmed commit outcome.
    pub async fn commit(self) -> Result<(), Error> {
        self.inner.commit_async().await.map_err(Error::from)
    }

    /// Rolls the transaction back and returns the borrowed connection.
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

//! A uniform SQL API with strict JSON configuration and portable parameters.
//!
//! The crate gives Rust and a future JavaScript adapter one shape: open a
//! database from a JSON document, execute or query with named `@name` or
//! positional `?` parameters, and read rows as the published
//! [`neotales_dbapi`] values. Values are always bound; the compiler rewrites
//! placeholder syntax and never interpolates a value into SQL.
//!
//! # Status
//!
//! SQLite is implemented end to end through the published
//! `neotales-dbapi-sqlite` provider. PostgreSQL, MySQL, and Microsoft SQL
//! Server configurations are parsed, and their placeholder forms are
//! implemented and unit-tested, but **no network provider is linked**. Opening
//! one returns [`ErrorKind::DriverUnavailable`]; it does not label a stub as a
//! working driver.
//!
//! A JavaScript/Node adapter is deliberately not part of this crate. The
//! default dependency graph contains no V8, `deno_core`, or `vaeljs-core`
//! dependency. See <https://vaeljs.com/architecture/db-api/> for the boundary and the
//! reason it is deferred.
//!
//! # Example
//!
//! ```no_run
//! use vaeljs_dbapi::{Db, Param, Params};
//!
//! # async fn run() -> Result<(), vaeljs_dbapi::Error> {
//! let db = Db::open_json(
//!     r#"{"driver":"sqlite","database":"app.sqlite"}"#,
//! )
//! .await?;
//! db.execute(
//!     "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
//!     &Params::none(),
//! )
//! .await?;
//! let insert = Params::named([
//!     ("id", Param::I64(7)),
//!     ("name", Param::Text("Ada".to_owned())),
//! ]);
//! db.execute("INSERT INTO users (id, name) VALUES (@id, @name)", &insert)
//!     .await?;
//! let rows = db
//!     .query(
//!         "SELECT name FROM users WHERE id = @id",
//!         &Params::named([("id", Param::I64(7))]),
//!     )
//!     .await?;
//! assert_eq!(rows[0].read::<String>("name")?, "Ada");
//! # Ok(())
//! # }
//! ```
//!
//! # Binding rules
//!
//! A statement uses either named or positional placeholders, never both.
//! Named parameters are supplied as a map and are matched case-sensitively.
//! Every supplied parameter must be used and every placeholder must have a
//! value. SQLite and MySQL receive `?`; PostgreSQL receives `$1`, `$2`;
//! Microsoft SQL Server receives `@p1`, `@p2`. Repeated named placeholders
//! reuse one parameter on PostgreSQL and SQL Server, and repeat the value on
//! SQLite and MySQL.
//!
//! The scanner is lexical, not a SQL parser. It handles quoted strings,
//! quoted identifiers, comments, PostgreSQL dollar quoting, and the operator
//! sequences containing `@` or `?`, and it rejects anything ambiguous. It
//! rejects `?NNN`, `:name`, and `$name`, multi-statement SQL, and trigger
//! bodies. These limits are part of the API, not an implementation detail.
//!
//! # Security
//!
//! Configuration and provider errors are fixed descriptions. They never echo a
//! path, host, user name, password, SQL string, or parameter value. The exact
//! SQL text may still be sensitive when a caller chooses to log it; this crate
//! does not log.

#![forbid(unsafe_code)]

mod bind;
mod config;
mod db;
mod error;

pub use bind::{CompiledSql, Dialect, Param, Params, compile};
pub use config::{DbConfig, Driver, NetworkConfig, SqliteConfig};
pub use db::{Connection, Db, Transaction};
pub use error::{Error, ErrorKind};
pub use neotales_dbapi::ErrorKind as ProviderErrorKind;
pub use neotales_dbapi::{DbType, FromValue, Parameter, Row, RowLimit, Value};

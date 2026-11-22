//! Strict JSON configuration.
//!
//! The parser is intentionally conservative: the top-level document must be an
//! object, `driver` must be one of four exact lower-case names, every field is
//! type-checked, out-of-range numbers are rejected, and unknown fields are an
//! error rather than being ignored. Error values contain a fixed description
//! and never echo a configuration value.
//!
//! Only SQLite has a linked provider in this build. PostgreSQL, MySQL, and
//! Microsoft SQL Server configurations are parsed so the JSON shape is uniform,
//! but [`crate::Db::open`] returns [`crate::ErrorKind::DriverUnavailable`] for
//! them without opening a socket.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Map, Value as JsonValue};

use crate::error::Error;

/// Maximum accepted SQLite connection count.
const MAX_SQLITE_CONNECTIONS: u32 = 1_024;

/// Default SQLite lock wait in milliseconds, matching the provider default.
const DEFAULT_BUSY_TIMEOUT_MS: u64 = 5_000;

/// Default SQLite pool size, matching the provider default.
const DEFAULT_MAX_CONNECTIONS: u32 = 5;

/// A database family understood by the configuration schema.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Driver {
    /// SQLite, the only provider linked by default.
    Sqlite,
    /// PostgreSQL.
    Postgres,
    /// MySQL (and MariaDB, which shares the wire protocol).
    MySql,
    /// Microsoft SQL Server.
    Mssql,
}

impl Driver {
    /// Returns the exact lower-case name used in JSON.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
            Self::MySql => "mysql",
            Self::Mssql => "mssql",
        }
    }

    /// Returns the conventional port for a network driver.
    ///
    /// SQLite has no network port; this returns `None`.
    #[must_use]
    pub const fn default_port(self) -> Option<u16> {
        match self {
            Self::Sqlite => None,
            Self::Postgres => Some(5_432),
            Self::MySql => Some(3_306),
            Self::Mssql => Some(1_433),
        }
    }
}

impl fmt::Display for Driver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A secret string whose `Debug` output is redacted.
///
/// There is deliberately no `Display`, `Serialize`, or public accessor. A
/// provider in this crate may read the value; callers outside it cannot. The
/// bytes are not zeroized on drop, and this type makes no such promise.
#[derive(Clone)]
struct Secret(String);

impl Secret {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }

    #[cfg(test)]
    pub(crate) fn as_str_for_test(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

/// Configuration for a file-backed SQLite pool.
#[derive(Clone)]
pub struct SqliteConfig {
    database: PathBuf,
    busy_timeout: Duration,
    max_connections: u32,
}

impl SqliteConfig {
    /// Creates configuration with the provider defaults: a five-second lock
    /// wait and five pooled connections.
    #[must_use]
    pub fn new(database: impl Into<PathBuf>) -> Self {
        Self {
            database: database.into(),
            busy_timeout: Duration::from_millis(DEFAULT_BUSY_TIMEOUT_MS),
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }

    /// Replaces the SQLite busy-handler wait bound.
    ///
    /// Zero and values above SQLite's supported millisecond range are rejected
    /// by [`crate::Db::open`].
    #[must_use]
    pub fn with_busy_timeout(mut self, timeout: Duration) -> Self {
        self.busy_timeout = timeout;
        self
    }

    /// Replaces the maximum pooled connection count.
    ///
    /// Zero and values above 1,024 are rejected by [`crate::Db::open`].
    #[must_use]
    pub fn with_max_connections(mut self, count: u32) -> Self {
        self.max_connections = count;
        self
    }

    /// Returns the database path.
    #[must_use]
    pub fn database(&self) -> &Path {
        &self.database
    }

    /// Returns the configured busy-handler wait bound.
    #[must_use]
    pub const fn busy_timeout(&self) -> Duration {
        self.busy_timeout
    }

    /// Returns the configured maximum connection count.
    #[must_use]
    pub const fn max_connections(&self) -> u32 {
        self.max_connections
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.database.as_os_str().is_empty()
            || self.database.as_os_str().as_encoded_bytes().contains(&0)
        {
            return Err(Error::config(
                "database must be a non-empty path without NUL",
            ));
        }
        let timeout_ms = self.busy_timeout.as_millis();
        if timeout_ms == 0 || timeout_ms > i32::MAX as u128 {
            return Err(Error::config(
                "busy_timeout must be between 1 millisecond and the SQLite maximum",
            ));
        }
        if self.max_connections == 0 || self.max_connections > MAX_SQLITE_CONNECTIONS {
            return Err(Error::config("max_connections must be between 1 and 1024"));
        }
        Ok(())
    }
}

impl fmt::Debug for SqliteConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SqliteConfig")
            .field("database", &"[redacted]")
            .field("busy_timeout", &self.busy_timeout)
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

/// Configuration for a network database driver.
///
/// This build parses the shape but links no network provider. No connection is
/// attempted, and the password is never displayed or exposed by a public
/// accessor.
#[derive(Clone)]
pub struct NetworkConfig {
    driver: Driver,
    host: String,
    port: u16,
    database: String,
    user: String,
    password: Option<Secret>,
    tls: bool,
}

impl NetworkConfig {
    /// Creates network configuration for a network driver.
    ///
    /// `driver` must be [`Driver::Postgres`], [`Driver::MySql`], or
    /// [`Driver::Mssql`]. A SQLite driver is rejected by [`crate::Db::open`].
    #[must_use]
    pub fn new(
        driver: Driver,
        host: impl Into<String>,
        database: impl Into<String>,
        user: impl Into<String>,
    ) -> Self {
        Self {
            driver,
            host: host.into(),
            port: driver.default_port().unwrap_or(0),
            database: database.into(),
            user: user.into(),
            password: None,
            tls: false,
        }
    }

    /// Sets the password. Its value is redacted from `Debug`.
    #[must_use]
    pub fn password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(Secret(password.into()));
        self
    }

    /// Requests or disables transport security.
    #[must_use]
    pub const fn tls(mut self, enabled: bool) -> Self {
        self.tls = enabled;
        self
    }

    /// Returns the configured driver.
    #[must_use]
    pub const fn driver(&self) -> Driver {
        self.driver
    }

    /// Returns the host name.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Returns the database name.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.database
    }

    /// Returns the user name.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    /// Reports whether a password was supplied without returning it.
    #[must_use]
    pub const fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// Returns whether transport security was requested.
    #[must_use]
    pub const fn tls_enabled(&self) -> bool {
        self.tls
    }

    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.driver == Driver::Sqlite {
            return Err(Error::config(
                "a network configuration requires a network driver",
            ));
        }
        if !valid_text(&self.host) {
            return Err(Error::config("host must be a non-empty string without NUL"));
        }
        if self.port == 0 {
            return Err(Error::config("port must be between 1 and 65535"));
        }
        if !valid_text(&self.database) {
            return Err(Error::config(
                "database must be a non-empty string without NUL",
            ));
        }
        if !valid_text(&self.user) {
            return Err(Error::config("user must be a non-empty string without NUL"));
        }
        if let Some(password) = &self.password
            && password.expose().contains('\0')
        {
            return Err(Error::config("password must not contain NUL"));
        }
        Ok(())
    }
}

impl fmt::Debug for NetworkConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkConfig")
            .field("driver", &self.driver)
            .field("host", &"[redacted]")
            .field("port", &self.port)
            .field("database", &"[redacted]")
            .field("user", &"[redacted]")
            .field("has_password", &self.has_password())
            .field("tls", &self.tls)
            .finish()
    }
}

/// Validated configuration for one database.
#[derive(Clone)]
pub enum DbConfig {
    /// File-backed SQLite.
    Sqlite(SqliteConfig),
    /// PostgreSQL.
    Postgres(NetworkConfig),
    /// MySQL or MariaDB.
    MySql(NetworkConfig),
    /// Microsoft SQL Server.
    Mssql(NetworkConfig),
}

impl DbConfig {
    /// Creates SQLite configuration.
    #[must_use]
    pub const fn sqlite(config: SqliteConfig) -> Self {
        Self::Sqlite(config)
    }

    /// Creates PostgreSQL configuration.
    #[must_use]
    pub const fn postgres(config: NetworkConfig) -> Self {
        Self::Postgres(config)
    }

    /// Creates MySQL configuration.
    #[must_use]
    pub const fn mysql(config: NetworkConfig) -> Self {
        Self::MySql(config)
    }

    /// Creates Microsoft SQL Server configuration.
    #[must_use]
    pub const fn mssql(config: NetworkConfig) -> Self {
        Self::Mssql(config)
    }

    /// Returns the configured driver.
    #[must_use]
    pub const fn driver(&self) -> Driver {
        match self {
            Self::Sqlite(_) => Driver::Sqlite,
            Self::Postgres(_) => Driver::Postgres,
            Self::MySql(_) => Driver::MySql,
            Self::Mssql(_) => Driver::Mssql,
        }
    }

    /// Returns the SQLite options, if this is a SQLite configuration.
    #[must_use]
    pub const fn sqlite_options(&self) -> Option<&SqliteConfig> {
        match self {
            Self::Sqlite(config) => Some(config),
            Self::Postgres(_) | Self::MySql(_) | Self::Mssql(_) => None,
        }
    }

    /// Returns the network options, if this is a network configuration.
    #[must_use]
    pub const fn network_options(&self) -> Option<&NetworkConfig> {
        match self {
            Self::Sqlite(_) => None,
            Self::Postgres(config) | Self::MySql(config) | Self::Mssql(config) => Some(config),
        }
    }

    /// Parses a strict JSON configuration document.
    ///
    /// # Errors
    /// Returns [`crate::ErrorKind::Config`] for malformed JSON, a non-object
    /// document, a missing or unknown driver, an unknown field, a wrong field
    /// type, or a value outside its accepted range. The error never contains a
    /// configuration value.
    pub fn from_json(json: &str) -> Result<Self, Error> {
        let value: JsonValue = serde_json::from_str(json)
            .map_err(|_| Error::config("configuration is not valid JSON"))?;
        let object = value
            .as_object()
            .ok_or_else(|| Error::config("configuration must be a JSON object"))?;
        let driver = object
            .get("driver")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| Error::config("configuration requires a string driver"))?;
        let config = match driver {
            "sqlite" => Self::parse_sqlite(object)?,
            "postgres" => Self::parse_network(object, Driver::Postgres)?,
            "mysql" => Self::parse_network(object, Driver::MySql)?,
            "mssql" => Self::parse_network(object, Driver::Mssql)?,
            _ => return Err(Error::config("driver is not recognized")),
        };
        config.validate()?;
        Ok(config)
    }

    /// Validates this configuration without opening a connection.
    ///
    /// # Errors
    /// Returns [`crate::ErrorKind::Config`] when a value is out of range.
    pub fn validate(&self) -> Result<(), Error> {
        match self {
            Self::Sqlite(config) => config.validate(),
            Self::Postgres(config) | Self::MySql(config) | Self::Mssql(config) => config.validate(),
        }
    }

    fn parse_sqlite(object: &Map<String, JsonValue>) -> Result<Self, Error> {
        ensure_only(
            object,
            &["driver", "database", "busy_timeout_ms", "max_connections"],
        )?;
        let database = required_text(object, "database")?;
        let busy_timeout_ms =
            optional_u64(object, "busy_timeout_ms")?.unwrap_or(DEFAULT_BUSY_TIMEOUT_MS);
        let max_connections =
            optional_u64(object, "max_connections")?.unwrap_or(u64::from(DEFAULT_MAX_CONNECTIONS));
        let max_connections = u32::try_from(max_connections)
            .map_err(|_| Error::config("max_connections must fit in 32 bits"))?;
        Ok(Self::Sqlite(
            SqliteConfig::new(database)
                .with_busy_timeout(Duration::from_millis(busy_timeout_ms))
                .with_max_connections(max_connections),
        ))
    }

    fn parse_network(object: &Map<String, JsonValue>, driver: Driver) -> Result<Self, Error> {
        ensure_only(
            object,
            &[
                "driver", "host", "port", "database", "user", "password", "tls",
            ],
        )?;
        let host = required_text(object, "host")?;
        let database = required_text(object, "database")?;
        let user = required_text(object, "user")?;
        let default_port = driver
            .default_port()
            .ok_or_else(|| Error::config("driver has no network port"))?;
        let port = match optional_u64(object, "port")? {
            Some(port) => u16::try_from(port)
                .map_err(|_| Error::config("port must be between 1 and 65535"))?,
            None => default_port,
        };
        let password = match object.get("password") {
            Some(JsonValue::String(value)) => Some(Secret(value.clone())),
            Some(_) => return Err(Error::config("password must be a string")),
            None => None,
        };
        let tls = match object.get("tls") {
            Some(JsonValue::Bool(value)) => *value,
            Some(_) => return Err(Error::config("tls must be a boolean")),
            None => false,
        };
        let mut config = NetworkConfig::new(driver, host, database, user);
        config.port = port;
        config.password = password;
        config.tls = tls;
        let config = match driver {
            Driver::Postgres => Self::Postgres(config),
            Driver::MySql => Self::MySql(config),
            Driver::Mssql => Self::Mssql(config),
            Driver::Sqlite => return Err(Error::config("driver has no network port")),
        };
        Ok(config)
    }
}

impl fmt::Debug for DbConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DbConfig")
            .field("driver", &self.driver())
            .finish_non_exhaustive()
    }
}

fn ensure_only(object: &Map<String, JsonValue>, allowed: &[&str]) -> Result<(), Error> {
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::config("configuration contains an unknown field"));
    }
    Ok(())
}

fn required_text(object: &Map<String, JsonValue>, key: &'static str) -> Result<String, Error> {
    match object.get(key) {
        Some(JsonValue::String(value)) if valid_text(value) => Ok(value.clone()),
        Some(JsonValue::String(_)) | None => Err(Error::config(match key {
            "database" => "database must be a non-empty string without NUL",
            "host" => "host must be a non-empty string without NUL",
            "user" => "user must be a non-empty string without NUL",
            _ => "configuration field must be a non-empty string without NUL",
        })),
        Some(_) => Err(Error::config("configuration field has the wrong type")),
    }
}

fn optional_u64(object: &Map<String, JsonValue>, key: &'static str) -> Result<Option<u64>, Error> {
    match object.get(key) {
        Some(JsonValue::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| Error::config("configuration number must be an unsigned integer")),
        Some(_) => Err(Error::config("configuration field has the wrong type")),
        None => Ok(None),
    }
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && !value.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_is_redacted() {
        let secret = Secret("hunter2".to_owned());
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(secret.as_str_for_test(), "hunter2");
    }
}

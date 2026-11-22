//! Sanitized errors for configuration, binding, and provider failures.

use std::error::Error as StdError;
use std::fmt;

use neotales_dbapi::ErrorKind as ProviderErrorKind;

/// Stable error category, safe to match on and to use as a bounded metric label.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    /// The JSON configuration document is not a valid, strict configuration.
    Config,
    /// The configured driver has no provider linked in this build.
    DriverUnavailable,
    /// The SQL text is outside the supported portable subset.
    Sql,
    /// A parameter is missing, unused, mistyped, or the two sides disagree.
    Parameter,
    /// A cardinality-one operation found more than one row.
    Cardinality,
    /// The provider rejected or failed the operation.
    Database,
}

/// A sanitized database API error.
///
/// The display text is a fixed description. It never contains SQL, parameter
/// values, configuration values, file paths, host names, user names, or driver
/// error text. When the failure came from the provider, [`Error::database_kind`]
/// exposes the provider's stable category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    kind: ErrorKind,
    detail: &'static str,
    database_kind: Option<ProviderErrorKind>,
}

impl Error {
    /// Creates an error with a fixed, non-sensitive detail string.
    #[must_use]
    pub(crate) const fn new(kind: ErrorKind, detail: &'static str) -> Self {
        Self {
            kind,
            detail,
            database_kind: None,
        }
    }

    /// Creates a configuration error without retaining any configuration value.
    #[must_use]
    pub(crate) const fn config(detail: &'static str) -> Self {
        Self::new(ErrorKind::Config, detail)
    }

    /// Creates a SQL/placeholder-compilation error.
    #[must_use]
    pub(crate) const fn sql(detail: &'static str) -> Self {
        Self::new(ErrorKind::Sql, detail)
    }

    /// Creates a parameter error.
    #[must_use]
    pub(crate) const fn parameter(detail: &'static str) -> Self {
        Self::new(ErrorKind::Parameter, detail)
    }

    /// Creates a cardinality error.
    #[must_use]
    pub(crate) const fn cardinality() -> Self {
        Self::new(ErrorKind::Cardinality, "more than one row matched")
    }

    /// Creates the error returned when a configured driver is not linked.
    #[must_use]
    pub(crate) const fn driver_unavailable() -> Self {
        Self::new(
            ErrorKind::DriverUnavailable,
            "the configured driver has no provider in this build",
        )
    }

    /// Returns the stable error category.
    #[must_use]
    pub const fn kind(self) -> ErrorKind {
        self.kind
    }

    /// Returns the provider's stable category when the failure came from it.
    ///
    /// `WriteOutcomeUnknown` means a dispatched write may or may not have
    /// happened; `CommitOutcomeUnknown` means a dispatched `COMMIT` may have
    /// succeeded. Both require caller-owned verification, not a blind retry.
    #[must_use]
    pub const fn database_kind(self) -> Option<ProviderErrorKind> {
        self.database_kind
    }

    /// Returns the fixed description. It contains no user data.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        self.detail
    }
}

impl From<neotales_dbapi::DbError> for Error {
    fn from(error: neotales_dbapi::DbError) -> Self {
        Self {
            kind: ErrorKind::Database,
            detail: "the database provider rejected the operation",
            database_kind: Some(error.kind()),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "database API error ({:?}): {}",
            self.kind, self.detail
        )
    }
}

impl StdError for Error {}

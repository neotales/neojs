//! Portable parameter binding.
//!
//! Callers write either named `@name` placeholders or anonymous `?`
//! placeholders. [`compile`] scans the SQL with a lexical scanner, replaces
//! placeholders with the target driver's positional form, and produces the
//! ordered [`Parameter`] list. It never interpolates a value into SQL and it
//! never uses a regular expression.
//!
//! The scanner understands single-quoted strings, double-quoted identifiers,
//! backtick and bracket identifiers, line and block comments, PostgreSQL dollar
//! quoting, and the operator sequences that contain `@` or `?`. It is not a SQL
//! parser: anything it cannot classify unambiguously is rejected rather than
//! guessed.
//!
//! # Portable subset
//!
//! * Named placeholders are `@` followed by an ASCII letter or `_`, then ASCII
//!   letters, digits, or `_`. Names are case-sensitive.
//! * Positional placeholders are `?` with nothing after them. PostgreSQL does
//!   not accept this form because a bare `?` is its jsonb operator; use named
//!   placeholders there.
//! * Named and positional forms must not be mixed in one statement.
//! * `?NNN`, `:name`, and `$name` are rejected. Only PostgreSQL accepts a
//!   dollar-quoted string such as `$tag$ ... $tag$`.
//! * A statement separator `;` is accepted only when the rest of the text is
//!   whitespace, comments, and further semicolons. Multi-statement SQL and
//!   trigger bodies are rejected.
//! * `@>`, `<@`, `@@`, `@?`, `@-@`, `?|`, and `?&` are preserved as operators.
//!   `??` is rejected as ambiguous. A bare `@` is rejected.
//! * MySQL `/*! ... */` and MariaDB `/*M! ... */` executable comments are
//!   rejected in both the statement and the trailing-terminator scan.
//!
//! PostgreSQL uses `$1`, `$2`; MySQL and SQLite use `?`; Microsoft SQL Server
//! uses `@p1`, `@p2`. Only the SQLite output is wired to a provider in this
//! build.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use neotales_dbapi::{DbType, Parameter, Value};

use crate::config::Driver;
use crate::error::Error;

/// SQL dialect used for lexical rules and the generated placeholder form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dialect {
    /// SQLite.
    Sqlite,
    /// PostgreSQL.
    Postgres,
    /// MySQL or MariaDB.
    MySql,
    /// Microsoft SQL Server.
    Mssql,
}

impl Dialect {
    /// Returns the configuration driver for this dialect.
    ///
    /// PostgreSQL requires named `@name` parameters: a bare `?` is the jsonb
    /// existence operator there and is rejected as ambiguous.
    #[must_use]
    pub const fn driver(self) -> Driver {
        match self {
            Self::Sqlite => Driver::Sqlite,
            Self::Postgres => Driver::Postgres,
            Self::MySql => Driver::MySql,
            Self::Mssql => Driver::Mssql,
        }
    }
}

impl From<Driver> for Dialect {
    fn from(driver: Driver) -> Self {
        match driver {
            Driver::Sqlite => Self::Sqlite,
            Driver::Postgres => Self::Postgres,
            Driver::MySql => Self::MySql,
            Driver::Mssql => Self::Mssql,
        }
    }
}

/// One caller-supplied parameter value.
///
/// JSON parameters use `null`, booleans, signed 64-bit integers, finite
/// floating-point numbers, and strings. Bytes are available from the Rust API
/// only; JSON has no native byte sequence and this crate deliberately does not
/// invent a base64 encoding.
#[derive(Clone, Debug, PartialEq)]
pub enum Param {
    /// An untyped SQL null. SQLite binds SQL NULL; the declared type is `TEXT`.
    Null,
    /// A typed SQL null for providers that require one.
    TypedNull(DbType),
    /// A boolean.
    Bool(bool),
    /// A signed 64-bit integer.
    I64(i64),
    /// A finite 64-bit floating-point number.
    F64(f64),
    /// UTF-8 text.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
}

impl Param {
    fn to_parameter(&self) -> Result<Parameter, Error> {
        let (db_type, value) = match self {
            Self::Null => (DbType::Text, Value::Null(DbType::Text)),
            Self::TypedNull(db_type) => (*db_type, Value::Null(*db_type)),
            Self::Bool(value) => (DbType::Bool, Value::Bool(*value)),
            Self::I64(value) => (DbType::I64, Value::I64(*value)),
            Self::F64(value) => (DbType::F64, Value::F64(*value)),
            Self::Text(value) => (DbType::Text, Value::Text(value.clone())),
            Self::Bytes(value) => (DbType::Bytes, Value::Bytes(value.clone())),
        };
        Parameter::new(db_type, value)
            .map_err(|_| Error::parameter("parameter value is not supported"))
    }

    fn from_json(value: &serde_json::Value) -> Result<Self, Error> {
        match value {
            serde_json::Value::Null => Ok(Self::Null),
            serde_json::Value::Bool(value) => Ok(Self::Bool(*value)),
            serde_json::Value::Number(number) => {
                if let Some(value) = number.as_i64() {
                    Ok(Self::I64(value))
                } else if number.as_u64().is_some() {
                    Err(Error::parameter(
                        "integer parameter is outside the signed 64-bit range",
                    ))
                } else if let Some(value) = number.as_f64() {
                    if value.is_finite() {
                        Ok(Self::F64(value))
                    } else {
                        Err(Error::parameter("float parameter must be finite"))
                    }
                } else {
                    Err(Error::parameter("number parameter is not supported"))
                }
            }
            serde_json::Value::String(value) => Ok(Self::Text(value.clone())),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => Err(Error::parameter(
                "JSON parameter must be null, boolean, number, or string",
            )),
        }
    }
}

/// Ordered parameters for one statement.
#[derive(Clone, Debug, PartialEq)]
pub enum Params {
    /// Parameters bound to `?` placeholders in source order.
    Positional(Vec<Param>),
    /// Parameters bound to `@name` placeholders by exact name.
    Named(BTreeMap<String, Param>),
}

impl Default for Params {
    fn default() -> Self {
        Self::none()
    }
}

impl Params {
    /// Creates positional parameters in source order.
    #[must_use]
    pub fn positional(values: impl IntoIterator<Item = Param>) -> Self {
        Self::Positional(values.into_iter().collect())
    }

    /// Creates named parameters. A repeated name keeps the last value.
    #[must_use]
    pub fn named<I, K>(values: I) -> Self
    where
        I: IntoIterator<Item = (K, Param)>,
        K: Into<String>,
    {
        Self::Named(
            values
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
        )
    }

    /// Creates an empty positional parameter list.
    #[must_use]
    pub const fn none() -> Self {
        Self::Positional(Vec::new())
    }

    /// Parses a JSON array as positional parameters or a JSON object as named
    /// parameters.
    ///
    /// # Errors
    /// Returns [`crate::ErrorKind::Parameter`] for malformed JSON, a top-level
    /// scalar, or a nested array/object value. Duplicate object keys follow
    /// `serde_json`'s last-value behavior.
    pub fn from_json(json: &str) -> Result<Self, Error> {
        let value: serde_json::Value = serde_json::from_str(json)
            .map_err(|_| Error::parameter("parameters are not valid JSON"))?;
        match value {
            serde_json::Value::Array(values) => {
                let mut parameters = Vec::with_capacity(values.len());
                for value in &values {
                    parameters.push(Param::from_json(value)?);
                }
                Ok(Self::Positional(parameters))
            }
            serde_json::Value::Object(values) => {
                let mut parameters = BTreeMap::new();
                for (name, value) in &values {
                    parameters.insert(name.clone(), Param::from_json(value)?);
                }
                Ok(Self::Named(parameters))
            }
            _ => Err(Error::parameter(
                "parameters must be a JSON array or object",
            )),
        }
    }

    /// Returns the number of supplied values.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Positional(values) => values.len(),
            Self::Named(values) => values.len(),
        }
    }

    /// Returns whether no values were supplied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// SQL rewritten for one driver and its ordered parameters.
#[derive(Clone)]
pub struct CompiledSql {
    sql: String,
    parameters: Vec<Parameter>,
}

impl CompiledSql {
    /// Returns the rewritten SQL.
    #[must_use]
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Returns the parameters in the order the generated placeholders expect.
    #[must_use]
    pub fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    /// Consumes this value and returns the SQL and its parameters.
    #[must_use]
    pub fn into_parts(self) -> (String, Vec<Parameter>) {
        (self.sql, self.parameters)
    }
}

impl fmt::Debug for CompiledSql {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompiledSql")
            .field("sql_len", &self.sql.len())
            .field("parameter_count", &self.parameters.len())
            .finish_non_exhaustive()
    }
}

/// Compiles portable placeholders into a driver-specific statement.
///
/// # Errors
/// Returns [`crate::ErrorKind::Sql`] for text outside the portable subset and
/// [`crate::ErrorKind::Parameter`] for a missing, unused, or invalid parameter.
/// No value is ever inserted into the SQL.
pub fn compile(sql: &str, params: &Params, dialect: Dialect) -> Result<CompiledSql, Error> {
    if sql.as_bytes().contains(&0) {
        return Err(Error::sql("SQL must not contain NUL"));
    }
    let mut compiler = Compiler {
        sql,
        bytes: sql.as_bytes(),
        dialect,
        params,
        out: String::with_capacity(sql.len()),
        parameters: Vec::new(),
        named_indices: BTreeMap::new(),
        used_names: BTreeSet::new(),
        next_positional: 0,
        has_code: false,
    };
    compiler.run()?;
    compiler.finish()?;
    Ok(CompiledSql {
        sql: compiler.out,
        parameters: compiler.parameters,
    })
}

struct Compiler<'a> {
    sql: &'a str,
    bytes: &'a [u8],
    dialect: Dialect,
    params: &'a Params,
    out: String,
    parameters: Vec<Parameter>,
    named_indices: BTreeMap<String, usize>,
    used_names: BTreeSet<String>,
    next_positional: usize,
    has_code: bool,
}

/// How a quoted string interprets backslashes.
#[derive(Clone, Copy)]
enum StringEscape {
    /// Backslash is an ordinary character.
    Literal,
    /// Backslash always escapes the next character.
    Escape,
    /// The server setting decides; scan both ways and require the same end.
    ServerDependent,
}

impl Compiler<'_> {
    fn run(&mut self) -> Result<(), Error> {
        let mut index = 0;
        while index < self.bytes.len() {
            let byte = self.bytes[index];
            match byte {
                b'\'' => {
                    let escape = self.string_escape(index, b'\'');
                    index = self.copy_string(index, b'\'', escape)?;
                }
                b'"' => {
                    if self.dialect == Dialect::Postgres && self.unicode_escape_prefix(index) {
                        return Err(Error::sql(
                            "PostgreSQL U& quoted identifiers are not supported",
                        ));
                    }
                    let escape = self.string_escape(index, b'"');
                    index = self.copy_string(index, b'"', escape)?;
                }
                b'`' => {
                    if matches!(self.dialect, Dialect::Sqlite | Dialect::MySql) {
                        index = self.copy_string(index, b'`', StringEscape::Literal)?;
                    } else {
                        return Err(Error::sql("backtick quoting is not valid for this dialect"));
                    }
                }
                b'[' => {
                    if matches!(self.dialect, Dialect::Sqlite | Dialect::Mssql) {
                        index = self.copy_bracket(index)?;
                    } else {
                        self.out.push('[');
                        self.has_code = true;
                        index += 1;
                    }
                }
                b'-' if self.bytes.get(index + 1) == Some(&b'-')
                    && self.mysql_dash_starts_comment(index) =>
                {
                    index = self.copy_line_comment(index);
                }
                b'/' if self.bytes.get(index + 1) == Some(&b'*') => {
                    index = self.copy_block_comment(index)?;
                }
                b'#' if self.dialect == Dialect::MySql => {
                    index = self.copy_line_comment(index);
                }
                b'$' => index = self.copy_dollar(index)?,
                b'<' if self.bytes.get(index + 1) == Some(&b'@') => {
                    self.out.push_str("<@");
                    self.has_code = true;
                    index += 2;
                }
                b':' => {
                    if self.bytes.get(index + 1) == Some(&b':') {
                        self.out.push_str("::");
                        self.has_code = true;
                        index += 2;
                    } else {
                        return Err(Error::sql(":name parameters are not portable"));
                    }
                }
                b'@' => index = self.handle_at(index)?,
                b'?' => index = self.handle_question(index)?,
                b';' => {
                    if self.rest_is_terminator(index + 1)? {
                        self.out.push_str(&self.sql[index..]);
                        return Ok(());
                    }
                    return Err(Error::sql("multiple SQL statements are not supported"));
                }
                _ => {
                    let start = index;
                    while index < self.bytes.len() && !is_special(self.bytes[index]) {
                        index += 1;
                    }
                    if index == start {
                        index += 1;
                    }
                    self.push_code(&self.sql[start..index]);
                }
            }
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), Error> {
        if !self.has_code {
            return Err(Error::sql("SQL contains no statement"));
        }
        match self.params {
            Params::Positional(values) if self.next_positional < values.len() => {
                Err(Error::parameter("not all positional parameters were used"))
            }
            Params::Named(values) if values.keys().any(|name| !self.used_names.contains(name)) => {
                Err(Error::parameter("not all named parameters were used"))
            }
            Params::Positional(_) | Params::Named(_) => Ok(()),
        }
    }

    fn push_code(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if text.chars().any(|character| !character.is_whitespace()) {
            self.has_code = true;
        }
        self.out.push_str(text);
    }

    fn string_escape(&self, index: usize, delimiter: u8) -> StringEscape {
        match (self.dialect, delimiter) {
            (Dialect::MySql, b'\'' | b'"') => StringEscape::ServerDependent,
            (Dialect::Postgres, b'\'') if self.explicit_escape_prefix(index) => {
                StringEscape::Escape
            }
            (Dialect::Postgres, b'\'') => StringEscape::ServerDependent,
            (Dialect::Postgres, b'"') | (Dialect::Sqlite | Dialect::Mssql, _) => {
                StringEscape::Literal
            }
            (Dialect::MySql, _) | (Dialect::Postgres, _) => StringEscape::Literal,
        }
    }

    fn explicit_escape_prefix(&self, index: usize) -> bool {
        self.e_string_prefix(index) || self.unicode_escape_prefix(index)
    }

    fn e_string_prefix(&self, index: usize) -> bool {
        if self.dialect != Dialect::Postgres {
            return false;
        }
        let previous = index.checked_sub(1).and_then(|value| self.bytes.get(value));
        let before_previous = index.checked_sub(2).and_then(|value| self.bytes.get(value));
        matches!(previous, Some(b'E' | b'e'))
            && before_previous.is_none_or(|byte| !is_ident_byte(*byte))
    }

    fn unicode_escape_prefix(&self, index: usize) -> bool {
        if self.dialect != Dialect::Postgres {
            return false;
        }
        let previous = index.checked_sub(1).and_then(|value| self.bytes.get(value));
        let before_previous = index.checked_sub(2).and_then(|value| self.bytes.get(value));
        matches!(previous, Some(b'&'))
            && matches!(before_previous, Some(b'U' | b'u'))
            && index
                .checked_sub(3)
                .and_then(|value| self.bytes.get(value))
                .is_none_or(|byte| !is_ident_byte(*byte))
    }

    fn copy_string(
        &mut self,
        start: usize,
        delimiter: u8,
        escape: StringEscape,
    ) -> Result<usize, Error> {
        let end = match escape {
            StringEscape::Literal => self.scan_string(start, delimiter, false)?,
            StringEscape::Escape => self.scan_string(start, delimiter, true)?,
            StringEscape::ServerDependent => {
                let literal = self.scan_string(start, delimiter, false);
                let escaped = self.scan_string(start, delimiter, true);
                match (literal, escaped) {
                    (Ok(literal), Ok(escaped)) if literal == escaped => literal,
                    (Ok(_), Ok(_)) | (Ok(_), Err(_)) | (Err(_), Ok(_)) => {
                        return Err(Error::sql(
                            "string boundary depends on the server escape mode; use an explicit escape form or bind the value as a parameter",
                        ));
                    }
                    (Err(_), Err(_)) => {
                        return Err(Error::sql("unterminated SQL string"));
                    }
                }
            }
        };
        self.out.push_str(&self.sql[start..end]);
        self.has_code = true;
        Ok(end)
    }

    fn scan_string(
        &self,
        start: usize,
        delimiter: u8,
        backslash_escape: bool,
    ) -> Result<usize, Error> {
        let mut index = start + 1;
        while index < self.bytes.len() {
            let byte = self.bytes[index];
            if backslash_escape && byte == b'\\' {
                index += 2;
                if index > self.bytes.len() {
                    return Err(Error::sql("unterminated SQL string"));
                }
            } else if byte == delimiter {
                if self.bytes.get(index + 1) == Some(&delimiter) {
                    index += 2;
                } else {
                    return Ok(index + 1);
                }
            } else {
                index += 1;
            }
        }
        Err(Error::sql("unterminated SQL string"))
    }

    fn copy_bracket(&mut self, start: usize) -> Result<usize, Error> {
        let mut index = start + 1;
        while index < self.bytes.len() {
            if self.bytes[index] == b']' {
                if self.dialect == Dialect::Mssql && self.bytes.get(index + 1) == Some(&b']') {
                    index += 2;
                } else {
                    let end = index + 1;
                    self.out.push_str(&self.sql[start..end]);
                    self.has_code = true;
                    return Ok(end);
                }
            } else {
                index += 1;
            }
        }
        Err(Error::sql("unterminated quoted identifier"))
    }

    fn copy_line_comment(&mut self, start: usize) -> usize {
        let end = self.bytes[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(self.bytes.len(), |offset| start + offset);
        self.out.push_str(&self.sql[start..end]);
        end
    }

    fn copy_block_comment(&mut self, start: usize) -> Result<usize, Error> {
        if self.executable_comment(start) {
            return Err(Error::sql(
                "MySQL/MariaDB executable comments are not supported",
            ));
        }
        let mut depth = 1usize;
        let mut index = start + 2;
        while index < self.bytes.len() {
            if self.bytes[index] == b'*' && self.bytes.get(index + 1) == Some(&b'/') {
                depth -= 1;
                index += 2;
                if depth == 0 {
                    self.out.push_str(&self.sql[start..index]);
                    return Ok(index);
                }
            } else if matches!(self.dialect, Dialect::Postgres | Dialect::Mssql)
                && self.bytes[index] == b'/'
                && self.bytes.get(index + 1) == Some(&b'*')
            {
                depth += 1;
                index += 2;
            } else {
                index += 1;
            }
        }
        Err(Error::sql("unterminated SQL comment"))
    }

    fn copy_dollar(&mut self, start: usize) -> Result<usize, Error> {
        if self.dialect != Dialect::Postgres {
            return Err(Error::sql("$name parameters are not portable"));
        }
        let mut index = start + 1;
        let tag = if self.bytes.get(index) == Some(&b'$') {
            index += 1;
            "$$"
        } else {
            let Some(first) = self.bytes.get(index).copied() else {
                return Err(Error::sql("dollar-quoted string is unterminated"));
            };
            if !is_ident_start(first) {
                return Err(Error::sql("$name parameters are not portable"));
            }
            while index < self.bytes.len() && is_ident_byte(self.bytes[index]) {
                index += 1;
            }
            if self.bytes.get(index) != Some(&b'$') {
                return Err(Error::sql("$name parameters are not portable"));
            }
            index += 1;
            &self.sql[start..index]
        };
        match self.sql[index..].find(tag) {
            Some(offset) => {
                let end = index + offset + tag.len();
                self.out.push_str(&self.sql[start..end]);
                self.has_code = true;
                Ok(end)
            }
            None => Err(Error::sql("dollar-quoted string is unterminated")),
        }
    }

    fn handle_at(&mut self, index: usize) -> Result<usize, Error> {
        match self.bytes.get(index + 1).copied() {
            Some(b'>' | b'@' | b'?') => {
                self.out.push_str(&self.sql[index..index + 2]);
                self.has_code = true;
                Ok(index + 2)
            }
            Some(b'-') if self.bytes.get(index + 2) == Some(&b'@') => {
                self.out.push_str(&self.sql[index..index + 3]);
                self.has_code = true;
                Ok(index + 3)
            }
            Some(byte) if is_ident_start(byte) => {
                let name_start = index + 1;
                let mut name_end = name_start;
                while name_end < self.bytes.len() && is_ident_byte(self.bytes[name_end]) {
                    name_end += 1;
                }
                if let Some(character) = self.sql[name_end..].chars().next()
                    && (character.is_alphanumeric() || character == '_' || character == '$')
                {
                    return Err(Error::sql(
                        "named parameter names must contain only ASCII letters, digits, and underscores",
                    ));
                }
                self.bind_named(&self.sql[name_start..name_end])?;
                Ok(name_end)
            }
            Some(_) | None => Err(Error::sql("bare @ is ambiguous")),
        }
    }

    fn handle_question(&mut self, index: usize) -> Result<usize, Error> {
        match self.bytes.get(index + 1).copied() {
            Some(b'|' | b'&') if self.dialect == Dialect::Postgres => {
                self.out.push_str(&self.sql[index..index + 2]);
                self.has_code = true;
                Ok(index + 2)
            }
            Some(b'?' | b'|' | b'&') => {
                Err(Error::sql("? operator or escaped placeholder is ambiguous"))
            }
            Some(byte) if byte.is_ascii_digit() => {
                Err(Error::sql("?NNN placeholders are not portable"))
            }
            _ => {
                // PostgreSQL has a bare `?` jsonb operator, so a bare `?`
                // cannot be unambiguously read as a positional placeholder.
                if self.dialect == Dialect::Postgres {
                    return Err(Error::sql(
                        "bare ? is ambiguous for PostgreSQL; use named @parameters",
                    ));
                }
                self.bind_positional()?;
                Ok(index + 1)
            }
        }
    }

    fn bind_named(&mut self, name: &str) -> Result<(), Error> {
        let Params::Named(values) = self.params else {
            return Err(Error::parameter(
                "SQL uses named placeholders but positional parameters were supplied",
            ));
        };
        let value = values
            .get(name)
            .ok_or_else(|| Error::parameter("a named placeholder has no supplied value"))?;
        let parameter = value.to_parameter()?;
        match self.dialect {
            Dialect::Sqlite | Dialect::MySql => {
                self.out.push('?');
                self.parameters.push(parameter);
            }
            Dialect::Postgres | Dialect::Mssql => {
                let next_index = self.parameters.len() + 1;
                let index = *self
                    .named_indices
                    .entry(name.to_owned())
                    .or_insert(next_index);
                push_indexed(&mut self.out, self.dialect, index);
                if index == next_index {
                    self.parameters.push(parameter);
                }
            }
        }
        self.used_names.insert(name.to_owned());
        Ok(())
    }

    fn bind_positional(&mut self) -> Result<(), Error> {
        let Params::Positional(values) = self.params else {
            return Err(Error::parameter(
                "SQL uses positional placeholders but named parameters were supplied",
            ));
        };
        let value = values.get(self.next_positional).ok_or_else(|| {
            Error::parameter("SQL contains more placeholders than supplied parameters")
        })?;
        self.next_positional += 1;
        let parameter = value.to_parameter()?;
        match self.dialect {
            Dialect::Sqlite | Dialect::MySql => self.out.push('?'),
            Dialect::Postgres | Dialect::Mssql => {
                push_indexed(&mut self.out, self.dialect, self.parameters.len() + 1);
            }
        }
        self.parameters.push(parameter);
        Ok(())
    }

    fn rest_is_terminator(&self, start: usize) -> Result<bool, Error> {
        let mut index = start;
        while index < self.bytes.len() {
            match self.bytes[index] {
                b';' => index += 1,
                byte if byte.is_ascii_whitespace() => index += 1,
                b'-' if self.bytes.get(index + 1) == Some(&b'-')
                    && self.mysql_dash_starts_comment(index) =>
                {
                    index = self.bytes[index..]
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or(self.bytes.len(), |offset| index + offset);
                }
                b'/' if self.bytes.get(index + 1) == Some(&b'*') => {
                    if self.executable_comment(index) {
                        return Err(Error::sql(
                            "MySQL/MariaDB executable comments are not supported",
                        ));
                    }
                    let Some(offset) = self.bytes[index + 2..]
                        .windows(2)
                        .position(|window| window == b"*/")
                    else {
                        return Err(Error::sql("unterminated SQL comment"));
                    };
                    index += offset + 4;
                }
                b'#' if self.dialect == Dialect::MySql => {
                    index = self.bytes[index..]
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or(self.bytes.len(), |offset| index + offset);
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    fn executable_comment(&self, index: usize) -> bool {
        if self.dialect != Dialect::MySql {
            return false;
        }
        self.bytes.get(index + 2) == Some(&b'!')
            || (self.bytes.get(index + 2) == Some(&b'M')
                && self.bytes.get(index + 3) == Some(&b'!'))
    }

    fn mysql_dash_starts_comment(&self, index: usize) -> bool {
        if self.dialect != Dialect::MySql {
            return true;
        }
        matches!(
            self.bytes.get(index + 2),
            Some(byte) if byte.is_ascii_whitespace() || *byte < 0x20 || *byte == 0x7f
        )
    }
}

fn push_indexed(out: &mut String, dialect: Dialect, index: usize) {
    match dialect {
        Dialect::Postgres => {
            out.push('$');
            out.push_str(&index.to_string());
        }
        Dialect::Mssql => {
            out.push_str("@p");
            out.push_str(&index.to_string());
        }
        Dialect::Sqlite | Dialect::MySql => out.push('?'),
    }
}

fn is_special(byte: u8) -> bool {
    matches!(
        byte,
        b'\'' | b'"' | b'`' | b'[' | b'<' | b'-' | b'/' | b'#' | b'$' | b':' | b'@' | b'?' | b';'
    )
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

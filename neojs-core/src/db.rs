//! Async SQLite and driver-neutral database ops for `neo:db` on V8.
//!
//! All strict JSON configuration parsing, portable `@name`/`?` placeholder
//! compilation, SQLite pooling, transaction management, and error redaction
//! live in the engine-neutral [`neojs_dbapi`] crate. This module validates V8
//! values, manages per-isolate database and transaction handles in
//! `OpState.resource_table`, converts [`neojs_dbapi::Params`] and
//! [`neotales_dbapi::Row`] values, and builds null-prototype result objects for
//! [`crate::builtins`].

use std::borrow::Cow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use deno_core::FromV8;
use deno_core::OpState;
use deno_core::Resource;
use deno_core::ResourceId;
use deno_core::ToV8;
use deno_core::op2;
use deno_core::v8;
use deno_error::JsErrorBox;
use neojs_dbapi::{Db, Error as DbError, ErrorKind, Param, Params, ProviderErrorKind, Row, Value};
use tokio::sync::{mpsc, oneshot};

/// Maximum JavaScript safe integer (`2^53 - 1`), used for `i64` <-> `number`/`bigint`.
const MAX_SAFE_INTEGER_I64: i64 = 9_007_199_254_740_991;
const MAX_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;

/// Stable screaming-snake-case error codes exposed on `DbError.code`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DbErrorCode {
    Config,
    DriverUnavailable,
    Sql,
    Parameter,
    Cardinality,
    Database,
    Closed,
    InvalidParam,
}

impl DbErrorCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Config => "CONFIG",
            Self::DriverUnavailable => "DRIVER_UNAVAILABLE",
            Self::Sql => "SQL",
            Self::Parameter => "PARAMETER",
            Self::Cardinality => "CARDINALITY",
            Self::Database => "DATABASE",
            Self::Closed => "CLOSED",
            Self::InvalidParam => "INVALID_PARAM",
        }
    }
}

/// Redacted error payload carried back to JavaScript via null-prototype objects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DbOpError {
    code: DbErrorCode,
    message: &'static str,
    database_kind: Option<&'static str>,
}

impl DbOpError {
    const fn new(code: DbErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            database_kind: None,
        }
    }

    const fn closed() -> Self {
        Self::new(DbErrorCode::Closed, "database handle is closed")
    }

    const fn invalid_param(message: &'static str) -> Self {
        Self::new(DbErrorCode::InvalidParam, message)
    }
}

const fn provider_kind_str(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::InvalidArgument => "InvalidArgument",
        ProviderErrorKind::Constraint => "Constraint",
        ProviderErrorKind::Busy => "Busy",
        ProviderErrorKind::Timeout => "Timeout",
        ProviderErrorKind::Cancelled => "Cancelled",
        ProviderErrorKind::Unsupported => "Unsupported",
        ProviderErrorKind::Reentrant => "Reentrant",
        ProviderErrorKind::Decode => "Decode",
        ProviderErrorKind::Connection => "Connection",
        ProviderErrorKind::WriteOutcomeUnknown => "WriteOutcomeUnknown",
        ProviderErrorKind::CommitOutcomeUnknown => "CommitOutcomeUnknown",
        ProviderErrorKind::Missing => "Missing",
        ProviderErrorKind::NoRows => "NoRows",
        ProviderErrorKind::TooManyRows => "TooManyRows",
        ProviderErrorKind::Ambiguous => "Ambiguous",
        ProviderErrorKind::Null => "Null",
        ProviderErrorKind::Overflow => "Overflow",
        ProviderErrorKind::WrongType => "WrongType",
    }
}

impl From<DbError> for DbOpError {
    fn from(error: DbError) -> Self {
        let code = match error.kind() {
            ErrorKind::Config => DbErrorCode::Config,
            ErrorKind::DriverUnavailable => DbErrorCode::DriverUnavailable,
            ErrorKind::Sql => DbErrorCode::Sql,
            ErrorKind::Parameter => DbErrorCode::Parameter,
            ErrorKind::Cardinality => DbErrorCode::Cardinality,
            ErrorKind::Database => DbErrorCode::Database,
        };
        Self {
            code,
            message: error.detail(),
            database_kind: error.database_kind().map(provider_kind_str),
        }
    }
}

/// Resource storing an open [`neojs_dbapi::Db`] pool handle.
struct DbHandleResource {
    db: Arc<Db>,
}

impl Resource for DbHandleResource {
    fn name(&self) -> Cow<'_, str> {
        "neo::db::Database".into()
    }
}

/// Commands serialized to the single-connection transaction task.
enum TxCommand {
    Execute {
        sql: String,
        params: Params,
        reply: oneshot::Sender<Result<u64, DbError>>,
    },
    Query {
        sql: String,
        params: Params,
        reply: oneshot::Sender<Result<Vec<Row>, DbError>>,
    },
    QueryOne {
        sql: String,
        params: Params,
        reply: oneshot::Sender<Result<Option<Row>, DbError>>,
    },
    Commit {
        reply: oneshot::Sender<Result<(), DbError>>,
    },
    Rollback {
        reply: oneshot::Sender<Result<(), DbError>>,
    },
}

/// Resource storing a sender to an active single-connection transaction task.
struct TxHandleResource {
    sender: mpsc::Sender<TxCommand>,
}

impl Resource for TxHandleResource {
    fn name(&self) -> Cow<'_, str> {
        "neo::db::Transaction".into()
    }
}

/// Strict UTF-8 string argument parsed synchronously from V8 before an async op awaits.
struct DbTextInput(Result<String, DbOpError>);

impl<'a> FromV8<'a> for DbTextInput {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        let Ok(string) = v8::Local::<v8::String>::try_from(value) else {
            return Ok(Self(Err(DbOpError::new(
                DbErrorCode::Sql,
                "expected a string argument",
            ))));
        };
        Ok(Self(copy_v8_string(
            scope,
            string,
            DbErrorCode::Sql,
            "string argument is not valid UTF-8",
        )))
    }
}

/// Parameters parsed synchronously from V8 before an async op awaits.
struct DbParamsInput(Result<Params, DbOpError>);

impl<'a> FromV8<'a> for DbParamsInput {
    type Error = JsErrorBox;

    fn from_v8<'i>(
        scope: &mut v8::PinScope<'a, 'i>,
        value: v8::Local<'a, v8::Value>,
    ) -> Result<Self, Self::Error> {
        Ok(Self(parse_params(scope, value)))
    }
}

/// Async op result converted back to a null-prototype V8 object via [`ToV8`].
enum DbOpOutcome {
    Id(ResourceId),
    Unit,
    Execute { rows_affected: u64 },
    Rows(Vec<Row>),
    Row(Option<Row>),
    Err(DbOpError),
}

impl<'a> ToV8<'a> for DbOpOutcome {
    type Error = JsErrorBox;

    fn to_v8<'i>(
        self,
        scope: &mut v8::PinScope<'a, 'i>,
    ) -> Result<v8::Local<'a, v8::Value>, Self::Error> {
        let object = response(scope);
        match self {
            Self::Id(id) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(
                    scope,
                    object,
                    "id",
                    v8::Number::new(scope, f64::from(id)).into(),
                );
            }
            Self::Unit => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
            }
            Self::Execute { rows_affected } => {
                let value = u64_to_v8(scope, rows_affected);
                let result_obj = v8::Object::new(scope);
                set(scope, result_obj, "rowsAffected", value);
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", result_obj.into());
            }
            Self::Rows(rows) => {
                let mut js_rows = Vec::with_capacity(rows.len());
                for row in &rows {
                    let Some(js_row) = row_to_v8(scope, row) else {
                        set_failure(
                            scope,
                            object,
                            DbOpError::new(
                                DbErrorCode::Database,
                                "database row value could not be converted to JS",
                            ),
                        );
                        return Ok(object.into());
                    };
                    js_rows.push(js_row.into());
                }
                let array = v8::Array::new_with_elements(scope, &js_rows);
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", array.into());
            }
            Self::Row(Some(row)) => {
                let Some(js_row) = row_to_v8(scope, &row) else {
                    set_failure(
                        scope,
                        object,
                        DbOpError::new(
                            DbErrorCode::Database,
                            "database row value could not be converted to JS",
                        ),
                    );
                    return Ok(object.into());
                };
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", js_row.into());
            }
            Self::Row(None) => {
                set(scope, object, "ok", v8::Boolean::new(scope, true).into());
                set(scope, object, "value", v8::null(scope).into());
            }
            Self::Err(error) => {
                set_failure(scope, object, error);
            }
        }
        Ok(object.into())
    }
}

fn response<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    let prototype = v8::null(scope);
    v8::Object::with_prototype_and_properties(scope, prototype.into(), &[], &[])
}

fn set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) {
    let Some(key) = v8::String::new(scope, name) else {
        return;
    };
    object.create_data_property(scope, key.into(), value);
}

fn set_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: &str,
) {
    let Some(value) = v8::String::new(scope, value) else {
        return;
    };
    set(scope, object, name, value.into());
}

fn set_failure<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    error: DbOpError,
) {
    set(scope, object, "ok", v8::Boolean::new(scope, false).into());
    set_string(scope, object, "name", "DbError");
    set_string(scope, object, "code", error.code.as_str());
    set_string(scope, object, "message", error.message);
    if let Some(kind) = error.database_kind {
        set_string(scope, object, "databaseKind", kind);
    }
}

fn copy_v8_string(
    scope: &v8::PinScope<'_, '_>,
    string: v8::Local<'_, v8::String>,
    err_code: DbErrorCode,
    err_message: &'static str,
) -> Result<String, DbOpError> {
    let length = string.utf8_length(scope);
    let Some(capacity) = length.checked_add(4) else {
        return Err(DbOpError::new(err_code, err_message));
    };
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(capacity)
        .map_err(|_| DbOpError::new(err_code, err_message))?;
    buffer.resize(capacity, 0);
    let written = string.write_utf8_v2(scope, &mut buffer, v8::WriteFlags::default(), None);
    buffer.truncate(written);
    String::from_utf8(buffer).map_err(|_| DbOpError::new(err_code, err_message))
}

fn read_uint8_array(
    scope: &v8::PinScope<'_, '_>,
    value: v8::Local<'_, v8::Value>,
) -> Result<Vec<u8>, DbOpError> {
    let Ok(view) = v8::Local::<v8::Uint8Array>::try_from(value) else {
        return Err(DbOpError::invalid_param("parameter must be a Uint8Array"));
    };
    let Some(buffer) = view.buffer(scope) else {
        return Err(DbOpError::invalid_param(
            "Uint8Array parameter buffer is detached",
        ));
    };
    if buffer.was_detached() {
        return Err(DbOpError::invalid_param(
            "Uint8Array parameter buffer is detached",
        ));
    }
    let length = view.byte_length();
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| DbOpError::invalid_param("Uint8Array parameter allocation failed"))?;
    bytes.resize(length, 0);
    if length <= i32::MAX as usize {
        if view.copy_contents(&mut bytes) != length {
            return Err(DbOpError::invalid_param(
                "Uint8Array parameter buffer is detached or out of bounds",
            ));
        }
    } else {
        // `v8::ArrayBufferView::copy_contents` panics when `dest.len() > i32::MAX`.
        let store = buffer.get_backing_store();
        let offset = view.byte_offset();
        let end = offset.checked_add(length).ok_or_else(|| {
            DbOpError::invalid_param("Uint8Array parameter buffer is detached or out of bounds")
        })?;
        let slice = store.get(offset..end).ok_or_else(|| {
            DbOpError::invalid_param("Uint8Array parameter buffer is detached or out of bounds")
        })?;
        for (dst, src) in bytes.iter_mut().zip(slice.iter()) {
            *dst = src.get();
        }
    }
    Ok(bytes)
}

fn js_value_to_param<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Param, DbOpError> {
    if value.is_null() {
        return Ok(Param::Null);
    }
    if value.is_boolean() {
        return Ok(Param::Bool(value.boolean_value(scope)));
    }
    if value.is_number() {
        let number = value
            .number_value(scope)
            .ok_or_else(|| DbOpError::invalid_param("numeric parameter could not be read"))?;
        if !number.is_finite() {
            return Err(DbOpError::invalid_param(
                "float parameter must be finite (NaN and Infinity are not supported)",
            ));
        }
        if number.fract() == 0.0 && (-MAX_SAFE_INTEGER_F64..=MAX_SAFE_INTEGER_F64).contains(&number)
        {
            return Ok(Param::I64(number as i64));
        }
        return Ok(Param::F64(number));
    }
    if value.is_big_int() {
        let bigint = v8::Local::<v8::BigInt>::try_from(value)
            .map_err(|_| DbOpError::invalid_param("bigint parameter could not be read"))?;
        let (n, lossless) = bigint.i64_value();
        if !lossless {
            return Err(DbOpError::invalid_param(
                "bigint parameter is outside the signed 64-bit integer range",
            ));
        }
        return Ok(Param::I64(n));
    }
    if value.is_string() {
        let string = v8::Local::<v8::String>::try_from(value)
            .map_err(|_| DbOpError::invalid_param("string parameter could not be read"))?;
        let text = copy_v8_string(
            scope,
            string,
            DbErrorCode::InvalidParam,
            "string parameter must be valid UTF-8",
        )?;
        return Ok(Param::Text(text));
    }
    if value.is_uint8_array() {
        return Ok(Param::Bytes(read_uint8_array(scope, value)?));
    }
    Err(DbOpError::invalid_param(
        "parameter values must be null, boolean, finite number, i64 bigint, string, or Uint8Array",
    ))
}

fn is_plain_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) -> bool {
    if value.is_array()
        || value.is_typed_array()
        || value.is_array_buffer()
        || value.is_array_buffer_view()
        || value.is_date()
        || value.is_map()
        || value.is_set()
        || value.is_reg_exp()
        || value.is_promise()
        || value.is_function()
        || value.is_proxy()
    {
        return false;
    }
    let Some(proto) = object.get_prototype(scope) else {
        return false;
    };
    if proto.is_null() {
        return true;
    }
    let plain = v8::Object::new(scope);
    plain
        .get_prototype(scope)
        .is_some_and(|object_proto| proto.strict_equals(object_proto))
}

fn parse_params<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Params, DbOpError> {
    if value.is_undefined() || value.is_null() {
        return Ok(Params::none());
    }
    if value.is_array() {
        let array = v8::Local::<v8::Array>::try_from(value)
            .map_err(|_| DbOpError::invalid_param("positional parameters must be an array"))?;
        let len = array.length();
        let mut out = Vec::with_capacity(len as usize);
        v8::tc_scope!(let tc_scope, scope);
        for index in 0..len {
            let Some(item) = array.get_index(tc_scope, index) else {
                return Err(DbOpError::invalid_param(
                    "failed to read positional parameter element",
                ));
            };
            if tc_scope.has_caught() {
                return Err(DbOpError::invalid_param(
                    "positional parameter getter threw an error",
                ));
            }
            out.push(js_value_to_param(tc_scope, item)?);
        }
        return Ok(Params::positional(out));
    }
    if value.is_object() {
        let object = v8::Local::<v8::Object>::try_from(value)
            .map_err(|_| DbOpError::invalid_param("named parameters must be a plain object"))?;
        if !is_plain_object(scope, object, value) {
            return Err(DbOpError::invalid_param(
                "parameters must be undefined, null, an array, or a plain object",
            ));
        }
        v8::tc_scope!(let tc_scope, scope);
        let Some(keys) = object.get_own_property_names(
            tc_scope,
            v8::GetPropertyNamesArgs {
                mode: v8::KeyCollectionMode::OwnOnly,
                property_filter: v8::PropertyFilter::ONLY_ENUMERABLE
                    | v8::PropertyFilter::SKIP_SYMBOLS,
                ..Default::default()
            },
        ) else {
            return Err(DbOpError::invalid_param(
                "failed to enumerate named parameter keys",
            ));
        };
        if tc_scope.has_caught() {
            return Err(DbOpError::invalid_param(
                "named parameter enumeration threw an error",
            ));
        }
        let len = keys.length();
        let mut pairs = Vec::with_capacity(len as usize);
        for index in 0..len {
            let Some(key_val) = keys.get_index(tc_scope, index) else {
                return Err(DbOpError::invalid_param(
                    "failed to read named parameter key",
                ));
            };
            let Ok(key_str) = v8::Local::<v8::String>::try_from(key_val) else {
                return Err(DbOpError::invalid_param(
                    "named parameter key must be a string",
                ));
            };
            let name = copy_v8_string(
                tc_scope,
                key_str,
                DbErrorCode::InvalidParam,
                "named parameter key must be valid UTF-8",
            )?;
            let Some(prop_val) = object.get(tc_scope, key_val) else {
                return Err(DbOpError::invalid_param(
                    "failed to read named parameter value",
                ));
            };
            if tc_scope.has_caught() {
                return Err(DbOpError::invalid_param(
                    "named parameter getter threw an error",
                ));
            }
            let param = js_value_to_param(tc_scope, prop_val)?;
            pairs.push((name, param));
        }
        return Ok(Params::named(pairs));
    }
    Err(DbOpError::invalid_param(
        "parameters must be undefined, null, an array, or a plain object",
    ))
}

fn u64_to_v8<'s>(scope: &mut v8::PinScope<'s, '_>, value: u64) -> v8::Local<'s, v8::Value> {
    if value <= MAX_SAFE_INTEGER_I64 as u64 {
        v8::Number::new(scope, value as f64).into()
    } else {
        v8::BigInt::new_from_u64(scope, value).into()
    }
}

fn bytes_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    bytes: Vec<u8>,
) -> Option<v8::Local<'s, v8::Uint8Array>> {
    let length = bytes.len();
    if length == 0 {
        let buffer = v8::ArrayBuffer::new(scope, 0);
        return v8::Uint8Array::new(scope, buffer, 0, 0);
    }
    let store = v8::ArrayBuffer::new_backing_store_from_bytes(bytes.into_boxed_slice());
    let store = store.make_shared();
    let buffer = v8::ArrayBuffer::with_backing_store(scope, &store);
    v8::Uint8Array::new(scope, buffer, 0, length)
}

fn value_to_v8<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &Value,
) -> Option<v8::Local<'s, v8::Value>> {
    match value {
        Value::Null(_) => Some(v8::null(scope).into()),
        Value::Bool(b) => Some(v8::Boolean::new(scope, *b).into()),
        Value::I64(n) => {
            if (-MAX_SAFE_INTEGER_I64..=MAX_SAFE_INTEGER_I64).contains(n) {
                Some(v8::Number::new(scope, *n as f64).into())
            } else {
                Some(v8::BigInt::new_from_i64(scope, *n).into())
            }
        }
        Value::F64(n) => Some(v8::Number::new(scope, *n).into()),
        Value::Text(text) => v8::String::new(scope, text).map(Into::into),
        Value::Bytes(bytes) => bytes_to_v8(scope, bytes.clone()).map(Into::into),
    }
}

fn row_to_v8<'s>(scope: &mut v8::PinScope<'s, '_>, row: &Row) -> Option<v8::Local<'s, v8::Object>> {
    let object = v8::Object::new(scope);
    for (name, value) in row.columns() {
        let key = v8::String::new(scope, name)?;
        let js_value = value_to_v8(scope, value)?;
        object.create_data_property(scope, key.into(), js_value);
    }
    Some(object)
}

/// Normalizes SQLite `:memory:` config without an explicit `max_connections` to
/// `max_connections: 1` so pool queries and transactions share one in-memory DB.
fn normalize_config_json(raw: &str) -> String {
    let Ok(serde_json::Value::Object(mut map)) = serde_json::from_str::<serde_json::Value>(raw)
    else {
        return raw.to_owned();
    };
    let is_sqlite_memory = map.get("driver").and_then(serde_json::Value::as_str) == Some("sqlite")
        && map.get("database").and_then(serde_json::Value::as_str) == Some(":memory:");
    if is_sqlite_memory && !map.contains_key("max_connections") {
        map.insert("max_connections".to_owned(), serde_json::json!(1));
        return serde_json::Value::Object(map).to_string();
    }
    raw.to_owned()
}

fn get_db(state: &Rc<RefCell<OpState>>, rid: ResourceId) -> Result<Arc<Db>, DbOpError> {
    let state = state.borrow();
    let Ok(resource) = state.resource_table.get::<DbHandleResource>(rid) else {
        return Err(DbOpError::closed());
    };
    Ok(Arc::clone(&resource.db))
}

fn get_tx(
    state: &Rc<RefCell<OpState>>,
    tx_rid: ResourceId,
) -> Result<mpsc::Sender<TxCommand>, DbOpError> {
    let state = state.borrow();
    let Ok(resource) = state.resource_table.get::<TxHandleResource>(tx_rid) else {
        return Err(DbOpError::closed());
    };
    Ok(resource.sender.clone())
}

fn unpack_sql_and_params(
    sql: DbTextInput,
    params: DbParamsInput,
) -> Result<(String, Params), DbOpError> {
    let sql = sql.0?;
    let params = params.0?;
    Ok((sql, params))
}

async fn send_tx_request<T>(
    sender: mpsc::Sender<TxCommand>,
    make_command: impl FnOnce(oneshot::Sender<Result<T, DbError>>) -> TxCommand,
) -> Result<T, DbOpError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    if sender.send(make_command(reply_tx)).await.is_err() {
        return Err(DbOpError::closed());
    }
    match reply_rx.await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.into()),
        Err(_) => Err(DbOpError::closed()),
    }
}

#[op2]
async fn op_neo_db_open(
    state: Rc<RefCell<OpState>>,
    #[scoped] config_json: DbTextInput,
) -> DbOpOutcome {
    let json = match config_json.0 {
        Ok(json) => normalize_config_json(&json),
        Err(mut err) => {
            err.code = DbErrorCode::Config;
            err.message = "configuration must be valid UTF-8";
            return DbOpOutcome::Err(err);
        }
    };
    match Db::open_json(&json).await {
        Ok(db) => {
            let rid = state
                .borrow_mut()
                .resource_table
                .add(DbHandleResource { db: Arc::new(db) });
            DbOpOutcome::Id(rid)
        }
        Err(error) => DbOpOutcome::Err(error.into()),
    }
}

#[op2]
async fn op_neo_db_open_sqlite(
    state: Rc<RefCell<OpState>>,
    #[scoped] database: DbTextInput,
) -> DbOpOutcome {
    let database = match database.0 {
        Ok(db) => db,
        Err(mut err) => {
            err.code = DbErrorCode::Config;
            err.message = "database must be valid UTF-8";
            return DbOpOutcome::Err(err);
        }
    };
    let json = if database == ":memory:" {
        serde_json::json!({
            "driver": "sqlite",
            "database": database,
            "max_connections": 1,
        })
    } else {
        serde_json::json!({
            "driver": "sqlite",
            "database": database,
        })
    };
    match Db::open_json(&json.to_string()).await {
        Ok(db) => {
            let rid = state
                .borrow_mut()
                .resource_table
                .add(DbHandleResource { db: Arc::new(db) });
            DbOpOutcome::Id(rid)
        }
        Err(error) => DbOpOutcome::Err(error.into()),
    }
}

#[op2]
async fn op_neo_db_execute(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[scoped] sql: DbTextInput,
    #[scoped] params: DbParamsInput,
) -> DbOpOutcome {
    let (sql, params) = match unpack_sql_and_params(sql, params) {
        Ok(parts) => parts,
        Err(err) => return DbOpOutcome::Err(err),
    };
    let db = match get_db(&state, rid) {
        Ok(db) => db,
        Err(err) => return DbOpOutcome::Err(err),
    };
    match db.execute(&sql, &params).await {
        Ok(rows_affected) => DbOpOutcome::Execute { rows_affected },
        Err(error) => DbOpOutcome::Err(error.into()),
    }
}

#[op2]
async fn op_neo_db_query(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[scoped] sql: DbTextInput,
    #[scoped] params: DbParamsInput,
) -> DbOpOutcome {
    let (sql, params) = match unpack_sql_and_params(sql, params) {
        Ok(parts) => parts,
        Err(err) => return DbOpOutcome::Err(err),
    };
    let db = match get_db(&state, rid) {
        Ok(db) => db,
        Err(err) => return DbOpOutcome::Err(err),
    };
    match db.query(&sql, &params).await {
        Ok(rows) => DbOpOutcome::Rows(rows),
        Err(error) => DbOpOutcome::Err(error.into()),
    }
}

#[op2]
async fn op_neo_db_query_one(
    state: Rc<RefCell<OpState>>,
    #[smi] rid: ResourceId,
    #[scoped] sql: DbTextInput,
    #[scoped] params: DbParamsInput,
) -> DbOpOutcome {
    let (sql, params) = match unpack_sql_and_params(sql, params) {
        Ok(parts) => parts,
        Err(err) => return DbOpOutcome::Err(err),
    };
    let db = match get_db(&state, rid) {
        Ok(db) => db,
        Err(err) => return DbOpOutcome::Err(err),
    };
    match db.query_one(&sql, &params).await {
        Ok(row) => DbOpOutcome::Row(row),
        Err(error) => DbOpOutcome::Err(error.into()),
    }
}

#[op2]
fn op_neo_db_close(state: &mut OpState, #[smi] rid: ResourceId) -> DbOpOutcome {
    let _ = state.resource_table.take::<DbHandleResource>(rid);
    DbOpOutcome::Unit
}

#[op2]
async fn op_neo_db_tx_begin(state: Rc<RefCell<OpState>>, #[smi] rid: ResourceId) -> DbOpOutcome {
    let db = match get_db(&state, rid) {
        Ok(db) => db,
        Err(err) => return DbOpOutcome::Err(err),
    };
    if tokio::runtime::Handle::try_current().is_err() {
        return DbOpOutcome::Err(DbOpError::new(
            DbErrorCode::Database,
            "database transaction requires an active Tokio runtime",
        ));
    }
    let mut conn = match db.connection().await {
        Ok(conn) => conn,
        Err(error) => return DbOpOutcome::Err(error.into()),
    };
    let (sender, mut receiver) = mpsc::channel::<TxCommand>(8);
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), DbError>>();
    tokio::spawn(async move {
        let mut tx = match conn.begin().await {
            Ok(tx) => {
                let _ = ready_tx.send(Ok(()));
                tx
            }
            Err(error) => {
                let _ = ready_tx.send(Err(error));
                return;
            }
        };
        while let Some(command) = receiver.recv().await {
            match command {
                TxCommand::Execute { sql, params, reply } => {
                    let _ = reply.send(tx.execute(&sql, &params).await);
                }
                TxCommand::Query { sql, params, reply } => {
                    let _ = reply.send(tx.query(&sql, &params).await);
                }
                TxCommand::QueryOne { sql, params, reply } => {
                    let _ = reply.send(tx.query_one(&sql, &params).await);
                }
                TxCommand::Commit { reply } => {
                    let _ = reply.send(tx.commit().await);
                    return;
                }
                TxCommand::Rollback { reply } => {
                    let _ = reply.send(tx.rollback().await);
                    return;
                }
            }
        }
    });
    match ready_rx.await {
        Ok(Ok(())) => {
            let tx_rid = state
                .borrow_mut()
                .resource_table
                .add(TxHandleResource { sender });
            DbOpOutcome::Id(tx_rid)
        }
        Ok(Err(error)) => DbOpOutcome::Err(error.into()),
        Err(_) => DbOpOutcome::Err(DbOpError::new(
            DbErrorCode::Database,
            "database transaction task terminated unexpectedly",
        )),
    }
}

#[op2]
async fn op_neo_db_tx_execute(
    state: Rc<RefCell<OpState>>,
    #[smi] tx_rid: ResourceId,
    #[scoped] sql: DbTextInput,
    #[scoped] params: DbParamsInput,
) -> DbOpOutcome {
    let (sql, params) = match unpack_sql_and_params(sql, params) {
        Ok(parts) => parts,
        Err(err) => return DbOpOutcome::Err(err),
    };
    let sender = match get_tx(&state, tx_rid) {
        Ok(sender) => sender,
        Err(err) => return DbOpOutcome::Err(err),
    };
    match send_tx_request(sender, |reply| TxCommand::Execute { sql, params, reply }).await {
        Ok(rows_affected) => DbOpOutcome::Execute { rows_affected },
        Err(err) => DbOpOutcome::Err(err),
    }
}

#[op2]
async fn op_neo_db_tx_query(
    state: Rc<RefCell<OpState>>,
    #[smi] tx_rid: ResourceId,
    #[scoped] sql: DbTextInput,
    #[scoped] params: DbParamsInput,
) -> DbOpOutcome {
    let (sql, params) = match unpack_sql_and_params(sql, params) {
        Ok(parts) => parts,
        Err(err) => return DbOpOutcome::Err(err),
    };
    let sender = match get_tx(&state, tx_rid) {
        Ok(sender) => sender,
        Err(err) => return DbOpOutcome::Err(err),
    };
    match send_tx_request(sender, |reply| TxCommand::Query { sql, params, reply }).await {
        Ok(rows) => DbOpOutcome::Rows(rows),
        Err(err) => DbOpOutcome::Err(err),
    }
}

#[op2]
async fn op_neo_db_tx_query_one(
    state: Rc<RefCell<OpState>>,
    #[smi] tx_rid: ResourceId,
    #[scoped] sql: DbTextInput,
    #[scoped] params: DbParamsInput,
) -> DbOpOutcome {
    let (sql, params) = match unpack_sql_and_params(sql, params) {
        Ok(parts) => parts,
        Err(err) => return DbOpOutcome::Err(err),
    };
    let sender = match get_tx(&state, tx_rid) {
        Ok(sender) => sender,
        Err(err) => return DbOpOutcome::Err(err),
    };
    match send_tx_request(sender, |reply| TxCommand::QueryOne { sql, params, reply }).await {
        Ok(row) => DbOpOutcome::Row(row),
        Err(err) => DbOpOutcome::Err(err),
    }
}

#[op2]
async fn op_neo_db_tx_commit(
    state: Rc<RefCell<OpState>>,
    #[smi] tx_rid: ResourceId,
) -> DbOpOutcome {
    let Ok(resource) = state
        .borrow_mut()
        .resource_table
        .take::<TxHandleResource>(tx_rid)
    else {
        return DbOpOutcome::Err(DbOpError::closed());
    };
    match send_tx_request(resource.sender.clone(), |reply| TxCommand::Commit { reply }).await {
        Ok(()) => DbOpOutcome::Unit,
        Err(err) => DbOpOutcome::Err(err),
    }
}

#[op2]
async fn op_neo_db_tx_rollback(
    state: Rc<RefCell<OpState>>,
    #[smi] tx_rid: ResourceId,
) -> DbOpOutcome {
    let Ok(resource) = state
        .borrow_mut()
        .resource_table
        .take::<TxHandleResource>(tx_rid)
    else {
        return DbOpOutcome::Unit;
    };
    let _ = send_tx_request(resource.sender.clone(), |reply| TxCommand::Rollback {
        reply,
    })
    .await;
    DbOpOutcome::Unit
}

deno_core::extension!(
    neojs_db,
    ops = [
        op_neo_db_open,
        op_neo_db_open_sqlite,
        op_neo_db_execute,
        op_neo_db_query,
        op_neo_db_query_one,
        op_neo_db_close,
        op_neo_db_tx_begin,
        op_neo_db_tx_execute,
        op_neo_db_tx_query,
        op_neo_db_tx_query_one,
        op_neo_db_tx_commit,
        op_neo_db_tx_rollback,
    ],
    docs =
        "Async SQLite and driver-neutral database ops behind `Neo.getBuiltInModule(\"neo:db\")`."
);

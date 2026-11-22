#![cfg(feature = "sqlite")]

use neojs_dbapi::{Db, ErrorKind, Param, Params, ProviderErrorKind, RowLimit};
use tempfile::TempDir;

fn config_json(directory: &TempDir, name: &str) -> String {
    let path = directory.path().join(name);
    serde_json::json!({
        "driver": "sqlite",
        "database": path.to_string_lossy(),
        "busy_timeout_ms": 2_000,
        "max_connections": 2,
    })
    .to_string()
}

async fn open(directory: &TempDir, name: &str) -> Db {
    Db::open_json(&config_json(directory, name))
        .await
        .expect("open sqlite")
}

async fn create_users(db: &Db) {
    db.execute(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        &Params::none(),
    )
    .await
    .expect("create table");
}

async fn count_users(db: &Db) -> i64 {
    let row = db
        .query_one("SELECT COUNT(*) AS total FROM users", &Params::none())
        .await
        .expect("count")
        .expect("one row");
    row.read::<i64>("total").expect("count value")
}

#[tokio::test]
async fn named_parameters_round_trip_through_real_sqlite() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "named.db").await;
    create_users(&db).await;

    let insert = Params::named([
        ("id", Param::I64(7)),
        ("name", Param::Text("Ada".to_owned())),
    ]);
    let affected = db
        .execute("INSERT INTO users (id, name) VALUES (@id, @name)", &insert)
        .await
        .expect("insert");
    assert_eq!(affected, 1);

    let rows = db
        .query(
            "SELECT id, name FROM users WHERE name = @name",
            &Params::named([("name", Param::Text("Ada".to_owned()))]),
        )
        .await
        .expect("select");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].read::<i64>("id").expect("id"), 7);
    assert_eq!(rows[0].read::<String>("name").expect("name"), "Ada");
}

#[tokio::test]
async fn repeated_named_parameters_bind_each_occurrence_for_sqlite() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "repeated.db").await;
    create_users(&db).await;

    db.execute(
        "INSERT INTO users (id, name) VALUES (@id, @name)",
        &Params::named([
            ("id", Param::I64(1)),
            ("name", Param::Text("Ada".to_owned())),
        ]),
    )
    .await
    .expect("insert");

    let row = db
        .query_one(
            "SELECT id + id AS doubled FROM users WHERE id = @id OR id = @id",
            &Params::named([("id", Param::I64(1))]),
        )
        .await
        .expect("select")
        .expect("row");
    assert_eq!(row.read::<i64>("doubled").expect("doubled"), 2);
}

#[tokio::test]
async fn positional_parameters_round_trip_through_real_sqlite() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "positional.db").await;
    create_users(&db).await;

    db.execute(
        "INSERT INTO users (id, name) VALUES (?, ?)",
        &Params::positional([Param::I64(11), Param::Text("Grace".to_owned())]),
    )
    .await
    .expect("insert");

    let row = db
        .query_one(
            "SELECT name FROM users WHERE id = ?",
            &Params::positional([Param::I64(11)]),
        )
        .await
        .expect("select")
        .expect("row");
    assert_eq!(row.read::<String>("name").expect("name"), "Grace");
}

#[tokio::test]
async fn injection_shaped_text_stays_bound_and_cannot_change_the_statement() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "injection.db").await;
    create_users(&db).await;

    let payload = "'; DROP TABLE users; --";
    db.execute(
        "INSERT INTO users (id, name) VALUES (@id, @name)",
        &Params::named([
            ("id", Param::I64(1)),
            ("name", Param::Text(payload.to_owned())),
        ]),
    )
    .await
    .expect("insert");

    let row = db
        .query_one(
            "SELECT name FROM users WHERE id = @id",
            &Params::named([("id", Param::I64(1))]),
        )
        .await
        .expect("select")
        .expect("row");
    assert_eq!(row.read::<String>("name").expect("name"), payload);
    assert_eq!(count_users(&db).await, 1);
}

#[tokio::test]
async fn transaction_commit_is_visible_to_later_pool_queries() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "commit.db").await;
    create_users(&db).await;

    let mut connection = db.connection().await.expect("connection");
    let mut transaction = connection.begin().await.expect("begin");
    transaction
        .execute(
            "INSERT INTO users (id, name) VALUES (@id, @name)",
            &Params::named([
                ("id", Param::I64(3)),
                ("name", Param::Text("Linus".to_owned())),
            ]),
        )
        .await
        .expect("insert");
    transaction.commit().await.expect("commit");

    assert_eq!(count_users(&db).await, 1);
}

#[tokio::test]
async fn transaction_rollback_discards_the_change() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "rollback.db").await;
    create_users(&db).await;

    let mut connection = db.connection().await.expect("connection");
    let mut transaction = connection.begin().await.expect("begin");
    transaction
        .execute(
            "INSERT INTO users (id, name) VALUES (@id, @name)",
            &Params::named([
                ("id", Param::I64(4)),
                ("name", Param::Text("Barbara".to_owned())),
            ]),
        )
        .await
        .expect("insert");
    transaction.rollback().await.expect("rollback");

    assert_eq!(count_users(&db).await, 0);
}

#[tokio::test]
async fn failed_statement_does_not_hide_the_constraint_or_corrupt_the_pool() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "failure.db").await;
    create_users(&db).await;

    let mut connection = db.connection().await.expect("connection");
    let mut transaction = connection.begin().await.expect("begin");
    transaction
        .execute(
            "INSERT INTO users (id, name) VALUES (1, 'Ada')",
            &Params::none(),
        )
        .await
        .expect("first insert");
    let error = transaction
        .execute(
            "INSERT INTO users (id, name) VALUES (1, 'Grace')",
            &Params::none(),
        )
        .await
        .expect_err("duplicate key");
    assert_eq!(error.kind(), ErrorKind::Database);
    assert_eq!(error.database_kind(), Some(ProviderErrorKind::Constraint));
    drop(transaction);
    drop(connection);

    assert_eq!(count_users(&db).await, 0);
}

#[tokio::test]
async fn null_bool_float_and_bytes_round_trip() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "types.db").await;
    db.execute(
        "CREATE TABLE items (id INTEGER PRIMARY KEY, flag INTEGER, score REAL, body BLOB, note TEXT)",
        &Params::none(),
    )
    .await
    .expect("create table");

    let body = vec![0_u8, 1, 127, 255];
    db.execute(
        "INSERT INTO items (id, flag, score, body, note) VALUES (@id, @flag, @score, @body, @note)",
        &Params::named([
            ("id", Param::I64(1)),
            ("flag", Param::Bool(true)),
            ("score", Param::F64(1.5)),
            ("body", Param::Bytes(body.clone())),
            ("note", Param::Null),
        ]),
    )
    .await
    .expect("insert");

    let row = db
        .query_one(
            "SELECT flag, score, body, note FROM items WHERE id = @id",
            &Params::named([("id", Param::I64(1))]),
        )
        .await
        .expect("select")
        .expect("row");
    assert_eq!(row.read::<i64>("flag").expect("flag"), 1);
    assert_eq!(row.read::<f64>("score").expect("score"), 1.5);
    assert_eq!(row.read::<Vec<u8>>("body").expect("body"), body);
    assert_eq!(row.read::<Option<String>>("note").expect("note"), None);
}

#[tokio::test]
async fn separate_temp_databases_are_isolated() {
    let first_directory = TempDir::new().expect("first dir");
    let second_directory = TempDir::new().expect("second dir");
    let first = open(&first_directory, "first.db").await;
    let second = open(&second_directory, "second.db").await;
    create_users(&first).await;
    create_users(&second).await;

    first
        .execute(
            "INSERT INTO users (id, name) VALUES (1, 'Ada')",
            &Params::none(),
        )
        .await
        .expect("insert");
    assert_eq!(count_users(&first).await, 1);
    assert_eq!(count_users(&second).await, 0);
}

#[tokio::test]
async fn query_limit_fails_instead_of_truncating() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "limit.db").await;
    create_users(&db).await;
    for id in 1..=3 {
        db.execute(
            "INSERT INTO users (id, name) VALUES (@id, @name)",
            &Params::named([
                ("id", Param::I64(id)),
                ("name", Param::Text(format!("user-{id}"))),
            ]),
        )
        .await
        .expect("insert");
    }

    let limit = RowLimit::new(2, 4_096).expect("limit");
    let error = db
        .query_with_limit("SELECT id FROM users ORDER BY id", &Params::none(), limit)
        .await
        .expect_err("over limit");
    assert_eq!(error.kind(), ErrorKind::Database);
    assert_eq!(error.database_kind(), Some(ProviderErrorKind::Overflow));
}

#[tokio::test]
async fn query_one_rejects_more_than_one_row() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "one.db").await;
    create_users(&db).await;
    db.execute(
        "INSERT INTO users (id, name) VALUES (1, 'Ada'), (2, 'Grace')",
        &Params::none(),
    )
    .await
    .expect("insert");

    let error = db
        .query_one("SELECT id FROM users", &Params::none())
        .await
        .expect_err("two rows");
    assert_eq!(error.kind(), ErrorKind::Cardinality);
}

#[tokio::test]
async fn provider_error_text_does_not_leak_sql_or_paths() {
    let directory = TempDir::new().expect("temp dir");
    let db = open(&directory, "provider-error.db").await;
    let error = db
        .execute("SELEC definitely_not_valid", &Params::none())
        .await
        .expect_err("syntax error");
    assert_eq!(error.kind(), ErrorKind::Database);
    let text = format!("{error} {error:?}");
    assert!(!text.contains("SELEC"));
    assert!(!text.contains("provider-error.db"));
}

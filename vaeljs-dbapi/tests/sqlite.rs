#![cfg(feature = "sqlite")]

use tempfile::TempDir;
use vaeljs_dbapi::{Db, ErrorKind, Param, Params, ProviderErrorKind, RowLimit};

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

#[tokio::test]
async fn wal_concurrent_readers_writers_and_busy_contention() {
    use std::sync::Arc;
    use std::time::Duration;

    let directory = TempDir::new().expect("temp dir");
    let path = directory.path().join("wal-concurrency.db");
    let cfg = serde_json::json!({
        "driver": "sqlite",
        "database": path.to_string_lossy(),
        "busy_timeout_ms": 2_000,
        "max_connections": 4,
    })
    .to_string();
    let db = Arc::new(Db::open_json(&cfg).await.expect("open wal pool"));

    let mode = db
        .query_one("PRAGMA journal_mode = WAL", &Params::none())
        .await
        .expect("enable wal")
        .expect("wal row");
    assert_eq!(
        mode.read::<String>("journal_mode").expect("journal_mode"),
        "wal"
    );

    db.execute(
        "CREATE TABLE events (id INTEGER PRIMARY KEY, worker INTEGER NOT NULL, seq INTEGER NOT NULL)",
        &Params::none(),
    )
    .await
    .expect("create events");

    let mut handles = Vec::new();
    for worker in 0_i64..4 {
        let pool = Arc::clone(&db);
        handles.push(tokio::spawn(async move {
            for seq in 0_i64..25 {
                let id = worker * 100 + seq;
                pool.execute(
                    "INSERT INTO events (id, worker, seq) VALUES (@id, @worker, @seq)",
                    &Params::named([
                        ("id", Param::I64(id)),
                        ("worker", Param::I64(worker)),
                        ("seq", Param::I64(seq)),
                    ]),
                )
                .await
                .expect("concurrent wal insert");
            }
        }));
    }
    for _ in 0..4 {
        let pool = Arc::clone(&db);
        handles.push(tokio::spawn(async move {
            for _ in 0..20 {
                let row = pool
                    .query_one("SELECT COUNT(*) AS total FROM events", &Params::none())
                    .await
                    .expect("concurrent wal read")
                    .expect("count row");
                let total = row.read::<i64>("total").expect("total");
                assert!((0..=100).contains(&total));
                tokio::task::yield_now().await;
            }
        }));
    }
    for h in handles {
        h.await.expect("worker task");
    }

    let final_row = db
        .query_one("SELECT COUNT(*) AS total FROM events", &Params::none())
        .await
        .expect("final count")
        .expect("row");
    assert_eq!(final_row.read::<i64>("total").expect("total"), 100);

    // Verify SQLITE_BUSY handling and WAL snapshot reads under an active write lock.
    let short_cfg = serde_json::json!({
        "driver": "sqlite",
        "database": path.to_string_lossy(),
        "busy_timeout_ms": 25,
        "max_connections": 1,
    })
    .to_string();
    let db_short = Db::open_json(&short_cfg).await.expect("open short timeout");

    let mut conn1 = db.connection().await.expect("conn1");
    let mut tx1 = conn1.begin().await.expect("begin tx1");
    tx1.execute(
        "INSERT INTO events (id, worker, seq) VALUES (999, 9, 9)",
        &Params::none(),
    )
    .await
    .expect("tx1 insert");

    // WAL readers do not block on an active writer and see the committed snapshot.
    let snap = db_short
        .query_one("SELECT COUNT(*) AS total FROM events", &Params::none())
        .await
        .expect("wal snapshot read during active tx")
        .expect("row");
    assert_eq!(snap.read::<i64>("total").expect("total"), 100);

    // Concurrent writer hits SQLITE_BUSY after busy_timeout_ms expires.
    let busy_err = db_short
        .execute(
            "INSERT INTO events (id, worker, seq) VALUES (1000, 10, 10)",
            &Params::none(),
        )
        .await
        .expect_err("expected SQLITE_BUSY while tx1 holds write lock");
    assert_eq!(busy_err.kind(), ErrorKind::Database);
    assert_eq!(busy_err.database_kind(), Some(ProviderErrorKind::Busy));

    tx1.commit().await.expect("commit tx1");
    drop(conn1);

    // Pool recovers immediately once write lock is released.
    tokio::time::sleep(Duration::from_millis(5)).await;
    db_short
        .execute(
            "INSERT INTO events (id, worker, seq) VALUES (1000, 10, 10)",
            &Params::none(),
        )
        .await
        .expect("insert after lock release");
    let after_row = db
        .query_one("SELECT COUNT(*) AS total FROM events", &Params::none())
        .await
        .expect("after count")
        .expect("row");
    assert_eq!(after_row.read::<i64>("total").expect("total"), 102);
}

#[tokio::test]
async fn pool_saturation_contention_and_recovery() {
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::oneshot;

    let directory = TempDir::new().expect("temp dir");
    let db = Arc::new(open(&directory, "saturated.db").await);
    db.query_one("PRAGMA journal_mode = WAL", &Params::none())
        .await
        .expect("wal");
    create_users(&db).await;

    // `open()` sets max_connections: 2. Hold both connections simultaneously.
    let c1 = db.connection().await.expect("c1");
    let mut c2 = db.connection().await.expect("c2");

    let (tx, mut rx) = oneshot::channel();
    let pool_for_waiter = Arc::clone(&db);
    let waiter = tokio::spawn(async move {
        let mut conn = pool_for_waiter
            .connection()
            .await
            .expect("waiter acquires connection");
        conn.execute(
            "INSERT INTO users (id, name) VALUES (1, 'Unblocked')",
            &Params::none(),
        )
        .await
        .expect("waiter insert");
        let _ = tx.send(());
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        matches!(rx.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
        "pool waiter must stay blocked while max_connections=2 are held"
    );

    // Releasing c1 unblocks the waiter immediately.
    drop(c1);
    tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("waiter timed out after c1 release")
        .expect("waiter channel");
    waiter.await.expect("waiter task");

    // Trigger a constraint failure inside a transaction on c2 and drop both,
    // verifying the pool replaces the invalidated connection cleanly.
    {
        let mut tx2 = c2.begin().await.expect("begin tx2");
        let err = tx2
            .execute(
                "INSERT INTO users (id, name) VALUES (1, 'Duplicate')",
                &Params::none(),
            )
            .await
            .expect_err("duplicate primary key");
        assert_eq!(err.database_kind(), Some(ProviderErrorKind::Constraint));
    }
    drop(c2);

    // Full capacity (2 concurrent connections) is usable again.
    let mut r1 = db.connection().await.expect("recovered conn 1");
    let mut r2 = db.connection().await.expect("recovered conn 2");
    let n1 = r1
        .query_one("SELECT COUNT(*) AS total FROM users", &Params::none())
        .await
        .expect("r1 query")
        .expect("row")
        .read::<i64>("total")
        .expect("total");
    let n2 = r2
        .query_one("SELECT COUNT(*) AS total FROM users", &Params::none())
        .await
        .expect("r2 query")
        .expect("row")
        .read::<i64>("total")
        .expect("total");
    assert_eq!(n1, 1);
    assert_eq!(n2, 1);
}

#[tokio::test]
async fn abrupt_transaction_drop_rolls_back_and_preserves_committed_rows() {
    let directory = TempDir::new().expect("temp dir");
    {
        let db = open(&directory, "durability.db").await;
        db.query_one("PRAGMA journal_mode = WAL", &Params::none())
            .await
            .expect("wal");
        db.execute("PRAGMA synchronous = FULL", &Params::none())
            .await
            .expect("sync full");
        create_users(&db).await;

        // 1. Committed baseline row.
        let mut conn1 = db.connection().await.expect("conn1");
        let mut tx1 = conn1.begin().await.expect("begin tx1");
        tx1.execute(
            "INSERT INTO users (id, name) VALUES (1, 'Committed')",
            &Params::none(),
        )
        .await
        .expect("insert committed");
        tx1.commit().await.expect("commit tx1");
        drop(conn1);

        // 2. Uncommitted transaction dropped abruptly without commit or rollback.
        let mut conn2 = db.connection().await.expect("conn2");
        let mut tx2 = conn2.begin().await.expect("begin tx2");
        tx2.execute(
            "INSERT INTO users (id, name) VALUES (2, 'AbruptDrop')",
            &Params::none(),
        )
        .await
        .expect("insert abrupt");
        drop(tx2);
        drop(conn2);

        // 3. Explicit rollback.
        let mut conn3 = db.connection().await.expect("conn3");
        let mut tx3 = conn3.begin().await.expect("begin tx3");
        tx3.execute(
            "INSERT INTO users (id, name) VALUES (3, 'ExplicitRollback')",
            &Params::none(),
        )
        .await
        .expect("insert rollback");
        tx3.rollback().await.expect("rollback tx3");
        drop(conn3);
    }

    // Reopen fresh pool from disk and verify durability + integrity.
    let reopened = open(&directory, "durability.db").await;
    let check = reopened
        .query_one("PRAGMA integrity_check", &Params::none())
        .await
        .expect("integrity check")
        .expect("row");
    assert_eq!(
        check
            .read::<String>("integrity_check")
            .expect("integrity text"),
        "ok"
    );

    let rows = reopened
        .query(
            "SELECT id, name FROM users ORDER BY id ASC",
            &Params::none(),
        )
        .await
        .expect("select rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].read::<i64>("id").expect("id"), 1);
    assert_eq!(rows[0].read::<String>("name").expect("name"), "Committed");

    reopened
        .execute(
            "INSERT INTO users (id, name) VALUES (4, 'AfterRecovery')",
            &Params::none(),
        )
        .await
        .expect("insert after recovery");
    assert_eq!(count_users(&reopened).await, 2);
}

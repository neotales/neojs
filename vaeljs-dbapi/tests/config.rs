use vaeljs_dbapi::{
    Db, DbConfig, Dialect, Driver, ErrorKind, NetworkConfig, Param, Params, SqliteConfig, compile,
};

#[test]
fn sqlite_config_parses_and_reports_values() {
    let config = DbConfig::from_json(
        r#"{"driver":"sqlite","database":"app.sqlite","busy_timeout_ms":2500,"max_connections":3}"#,
    )
    .expect("valid sqlite config");
    assert_eq!(config.driver(), Driver::Sqlite);
    let sqlite = config.sqlite_options().expect("sqlite options");
    assert_eq!(sqlite.database().to_string_lossy(), "app.sqlite");
    assert_eq!(sqlite.busy_timeout().as_millis(), 2500);
    assert_eq!(sqlite.max_connections(), 3);
    assert!(!format!("{config:?} {sqlite:?}").contains("app.sqlite"));
}

#[test]
fn sqlite_config_defaults_are_applied() {
    let config =
        DbConfig::from_json(r#"{"driver":"sqlite","database":"app.sqlite"}"#).expect("config");
    let sqlite = config.sqlite_options().expect("sqlite options");
    assert_eq!(sqlite.busy_timeout().as_millis(), 5000);
    assert_eq!(sqlite.max_connections(), 5);
}

#[test]
fn unknown_config_field_is_rejected_without_echoing_the_secret() {
    let json = r#"{"driver":"sqlite","database":"/private/app.db","password":"s3cr3t"}"#;
    let error = DbConfig::from_json(json).expect_err("unknown field");
    assert_eq!(error.kind(), ErrorKind::Config);
    let text = format!("{error} {error:?}");
    assert!(!text.contains("s3cr3t"));
    assert!(!text.contains("/private/app.db"));
}

#[test]
fn malformed_config_does_not_echo_its_content() {
    let json = r#"{"driver":"sqlite","database":"/private/app.db","password":"s3cr3t""#;
    let error = DbConfig::from_json(json).expect_err("malformed");
    assert_eq!(error.kind(), ErrorKind::Config);
    let text = format!("{error} {error:?}");
    assert!(!text.contains("s3cr3t"));
    assert!(!text.contains("/private/app.db"));
}

#[test]
fn config_rejects_missing_or_invalid_values() {
    for json in [
        r#"{}"#,
        r#"[]"#,
        r#"{"driver":"sqlite"}"#,
        r#"{"driver":"sqlite","database":""}"#,
        r#"{"driver":"sqlite","database":7}"#,
        r#"{"driver":"sqlite","database":"x","busy_timeout_ms":0}"#,
        r#"{"driver":"sqlite","database":"x","busy_timeout_ms":1.5}"#,
        r#"{"driver":"sqlite","database":"x","busy_timeout_ms":2147483648}"#,
        r#"{"driver":"sqlite","database":"x","max_connections":0}"#,
        r#"{"driver":"sqlite","database":"x","max_connections":1025}"#,
        r#"{"driver":"firebird","database":"x"}"#,
        r#"{"driver":"sqlite","database":"x","port":5432}"#,
    ] {
        let error = DbConfig::from_json(json).expect_err(json);
        assert_eq!(error.kind(), ErrorKind::Config, "{json}");
    }
}

#[test]
fn manually_constructed_invalid_sqlite_config_is_rejected() {
    let config = DbConfig::sqlite(
        SqliteConfig::new("")
            .with_busy_timeout(std::time::Duration::ZERO)
            .with_max_connections(0),
    );
    let error = config.validate().expect_err("invalid values");
    assert_eq!(error.kind(), ErrorKind::Config);
}

#[test]
fn network_config_parses_but_password_is_redacted() {
    let config = DbConfig::from_json(
        r#"{"driver":"postgres","host":"db.internal","database":"app","user":"admin","password":"s3cr3t","tls":true}"#,
    )
    .expect("network config");
    assert_eq!(config.driver(), Driver::Postgres);
    let network = config.network_options().expect("network options");
    assert_eq!(network.port(), 5432);
    assert!(network.has_password());
    assert!(network.tls_enabled());
    let debug = format!("{config:?} {network:?}");
    assert!(!debug.contains("s3cr3t"));
    assert!(!debug.contains("db.internal"));
    assert!(!debug.contains("admin"));
}

#[test]
fn network_variant_supplies_the_driver() {
    let postgres = DbConfig::postgres(NetworkConfig::new("db.internal", 5432, "app", "admin"));
    assert_eq!(postgres.driver(), Driver::Postgres);
    assert_eq!(postgres.network_options().expect("network").port(), 5432);

    let mysql = DbConfig::mysql(NetworkConfig::new("db.internal", 3306, "app", "admin"));
    assert_eq!(mysql.driver(), Driver::MySql);

    let mssql = DbConfig::mssql(NetworkConfig::new("db.internal", 1433, "app", "admin"));
    assert_eq!(mssql.driver(), Driver::Mssql);

    let zero_port = DbConfig::postgres(NetworkConfig::new("db.internal", 0, "app", "admin"));
    assert_eq!(
        zero_port.validate().expect_err("zero port").kind(),
        ErrorKind::Config
    );
}

#[test]
fn network_driver_defaults_are_available() {
    assert_eq!(Driver::Postgres.default_port(), Some(5432));
    assert_eq!(Driver::MySql.default_port(), Some(3306));
    assert_eq!(Driver::Mssql.default_port(), Some(1433));
    assert_eq!(Driver::Sqlite.default_port(), None);
}

#[tokio::test]
async fn network_driver_is_reported_unavailable_without_echoing_config() {
    let config = DbConfig::from_json(
        r#"{"driver":"mysql","host":"db.internal","database":"app","user":"admin","password":"s3cr3t"}"#,
    )
    .expect("config");
    let error = Db::open(&config).await.expect_err("no provider");
    assert_eq!(error.kind(), ErrorKind::DriverUnavailable);
    let text = format!("{error} {error:?}");
    assert!(!text.contains("s3cr3t"));
    assert!(!text.contains("db.internal"));
    assert!(!text.contains("admin"));
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn sqlite_open_error_does_not_echo_the_path() {
    let path = "/definitely/not/a/real/directory/private.db";
    let config = DbConfig::from_json(&format!(r#"{{"driver":"sqlite","database":"{path}"}}"#))
        .expect("config");
    let error = Db::open(&config).await.expect_err("parent is absent");
    assert_eq!(error.kind(), ErrorKind::Database);
    let text = format!("{error} {error:?}");
    assert!(!text.contains("private.db"));
}

#[test]
fn default_build_manifest_has_no_node_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in ["deno_core", "vaeljs-core", "v8", "napi"] {
        assert!(
            !manifest.contains(forbidden),
            "default manifest mentions {forbidden}"
        );
    }
    assert!(manifest.contains("default = [\"sqlite\"]"));
}

#[test]
fn dialect_maps_from_driver() {
    for (driver, dialect) in [
        (Driver::Sqlite, Dialect::Sqlite),
        (Driver::Postgres, Dialect::Postgres),
        (Driver::MySql, Dialect::MySql),
        (Driver::Mssql, Dialect::Mssql),
    ] {
        assert_eq!(Dialect::from(driver), dialect);
        assert_eq!(dialect.driver(), driver);
    }
}

#[test]
fn parameter_placeholder_is_never_interpolated() {
    let payload = "'; DROP TABLE users; --";
    let compiled = compile(
        "SELECT @value",
        &Params::named([("value", Param::Text(payload.to_owned()))]),
        Dialect::Sqlite,
    )
    .expect("compiled");
    assert_eq!(compiled.sql(), "SELECT ?");
    assert_eq!(compiled.parameters().len(), 1);
}

#[test]
fn sqlite_config_rejects_uri_prefixes_and_query_flags() {
    for bad in [
        "file:app.sqlite",
        "FILE:app.sqlite",
        "file::memory:?cache=shared",
        "file:app.sqlite?mode=ro",
        "app.sqlite?mode=ro",
        "app.sqlite?vfs=memdb",
    ] {
        let config = DbConfig::sqlite(SqliteConfig::new(bad));
        let err = config.validate().expect_err(bad);
        assert_eq!(err.kind(), ErrorKind::Config, "{bad}");

        let json = format!(r#"{{"driver":"sqlite","database":"{bad}"}}"#);
        let json_err = DbConfig::from_json(&json).expect_err(&json);
        assert_eq!(json_err.kind(), ErrorKind::Config, "{json}");
    }

    for ok in [":memory:", "app.sqlite", "/tmp/dir/app.sqlite"] {
        let config = DbConfig::sqlite(SqliteConfig::new(ok));
        config.validate().expect(ok);
    }
}

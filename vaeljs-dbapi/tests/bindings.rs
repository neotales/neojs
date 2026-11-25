use vaeljs_dbapi::{DbType, Dialect, ErrorKind, Param, Params, compile};

fn named(pairs: &[(&str, Param)]) -> Params {
    Params::named(pairs.iter().map(|(name, value)| (*name, value.clone())))
}

fn id(value: i64) -> Params {
    named(&[("id", Param::I64(value))])
}

#[test]
fn sqlite_named_placeholders_become_question_marks() {
    let compiled = compile(
        "SELECT name FROM users WHERE id = @id AND active = @active",
        &named(&[("id", Param::I64(7)), ("active", Param::Bool(true))]),
        Dialect::Sqlite,
    )
    .expect("compiled");
    assert_eq!(
        compiled.sql(),
        "SELECT name FROM users WHERE id = ? AND active = ?"
    );
    assert_eq!(compiled.parameters().len(), 2);
}

#[test]
fn repeated_named_placeholders_repeat_values_for_sqlite_and_mysql() {
    for dialect in [Dialect::Sqlite, Dialect::MySql] {
        let compiled = compile("SELECT @id, @id", &id(7), dialect).expect("compiled");
        assert_eq!(compiled.sql(), "SELECT ?, ?");
        assert_eq!(compiled.parameters().len(), 2, "{dialect:?}");
    }
}

#[test]
fn repeated_named_placeholders_reuse_one_value_for_postgres_and_mssql() {
    let postgres = compile("SELECT @id, @id", &id(7), Dialect::Postgres).expect("compiled");
    assert_eq!(postgres.sql(), "SELECT $1, $1");
    assert_eq!(postgres.parameters().len(), 1);

    let mssql = compile("SELECT @id, @id", &id(7), Dialect::Mssql).expect("compiled");
    assert_eq!(mssql.sql(), "SELECT @p1, @p1");
    assert_eq!(mssql.parameters().len(), 1);
}

#[test]
fn positional_placeholders_become_driver_specific_forms() {
    let params = Params::positional([Param::I64(1), Param::Text("Ada".to_owned())]);

    let sqlite = compile("SELECT ?, ?", &params, Dialect::Sqlite).expect("compiled");
    assert_eq!(sqlite.sql(), "SELECT ?, ?");
    assert_eq!(sqlite.parameters().len(), 2);

    let mssql = compile("SELECT ?, ?", &params, Dialect::Mssql).expect("compiled");
    assert_eq!(mssql.sql(), "SELECT @p1, @p2");
    assert_eq!(mssql.parameters().len(), 2);
}

#[test]
fn quoted_text_comments_and_identifiers_hide_placeholders() {
    let sql = "SELECT '@id', \"@id\", `@id`, [@id], @id -- @id\nFROM t /* @id */";
    let compiled = compile(sql, &id(1), Dialect::Sqlite).expect("compiled");
    assert_eq!(
        compiled.sql(),
        "SELECT '@id', \"@id\", `@id`, [@id], ? -- @id\nFROM t /* @id */"
    );
    assert_eq!(compiled.parameters().len(), 1);
}

#[test]
fn unrecognized_quoting_is_rejected() {
    let backtick = compile("SELECT `x`", &Params::none(), Dialect::Postgres);
    assert_eq!(
        backtick.expect_err("postgres has no backticks").kind(),
        ErrorKind::Sql
    );

    let mysql_backtick = compile("SELECT `x`", &Params::none(), Dialect::MySql);
    assert!(mysql_backtick.is_ok());
}

#[test]
fn postgres_operators_containing_at_or_question_mark_are_preserved() {
    for sql in [
        "SELECT document @> '{}'::jsonb FROM t",
        "SELECT '{}'::jsonb <@ document FROM t",
        "SELECT to_tsvector('a') @@ to_tsquery('b')",
        "SELECT document ?| array['a'] FROM t",
        "SELECT document ?& array['a'] FROM t",
        "SELECT value @? '$.a' FROM t",
    ] {
        let compiled = compile(sql, &Params::none(), Dialect::Postgres).expect(sql);
        assert_eq!(compiled.sql(), sql);
        assert!(compiled.parameters().is_empty());
    }
}

#[test]
fn postgres_unicode_escape_quoted_identifiers_are_rejected() {
    let error = compile("SELECT U&\"d\\0061t\"", &Params::none(), Dialect::Postgres)
        .expect_err("U& quoted identifier");
    assert_eq!(error.kind(), ErrorKind::Sql);
}

#[test]
fn postgres_array_subscripts_keep_named_placeholders() {
    let compiled = compile(
        "SELECT values[@index], @value",
        &named(&[("index", Param::I64(1)), ("value", Param::I64(2))]),
        Dialect::Postgres,
    )
    .expect("compiled");
    assert_eq!(compiled.sql(), "SELECT values[$1], $2");
    assert_eq!(compiled.parameters().len(), 2);
}

#[test]
fn ambiguous_question_forms_are_rejected() {
    for sql in ["SELECT ??", "SELECT ?1", "SELECT ?| 1"] {
        let error = compile(sql, &Params::none(), Dialect::Sqlite).expect_err(sql);
        assert_eq!(error.kind(), ErrorKind::Sql, "{sql}");
    }
    let error = compile("SELECT ?| array['a']", &Params::none(), Dialect::Sqlite)
        .expect_err("operator needs postgres");
    assert_eq!(error.kind(), ErrorKind::Sql);

    let postgres = compile(
        "SELECT ?",
        &Params::positional([Param::I64(1)]),
        Dialect::Postgres,
    )
    .expect_err("postgres bare ? is ambiguous");
    assert_eq!(postgres.kind(), ErrorKind::Sql);
}

#[test]
fn unsupported_parameter_forms_are_rejected() {
    for (sql, dialect) in [
        ("SELECT :name", Dialect::Sqlite),
        ("SELECT $name", Dialect::Sqlite),
        ("SELECT $1", Dialect::Postgres),
        ("SELECT @", Dialect::Sqlite),
        ("SELECT @café", Dialect::Sqlite),
        ("SELECT @id", Dialect::Sqlite),
    ] {
        let params = if sql.contains("@id") {
            Params::positional([Param::I64(1)])
        } else {
            Params::none()
        };
        let error = compile(sql, &params, dialect).expect_err(sql);
        assert!(
            matches!(error.kind(), ErrorKind::Sql | ErrorKind::Parameter),
            "{sql}: {error:?}"
        );
    }
}

#[test]
fn postgres_dollar_quotes_are_literals() {
    let compiled =
        compile("SELECT $tag$@id$tag$, @id", &id(3), Dialect::Postgres).expect("compiled");
    assert_eq!(compiled.sql(), "SELECT $tag$@id$tag$, $1");
    assert_eq!(compiled.parameters().len(), 1);
}

#[test]
fn explicit_escape_strings_are_accepted() {
    let postgres =
        compile(r"SELECT E'it\'s @id', @id", &id(1), Dialect::Postgres).expect("compiled");
    assert_eq!(postgres.sql(), r"SELECT E'it\'s @id', $1");
    assert_eq!(postgres.parameters().len(), 1);

    let unicode = compile(r"SELECT U&'d\0061t', @id", &id(1), Dialect::Postgres).expect("compiled");
    assert_eq!(unicode.parameters().len(), 1);
}

#[test]
fn server_dependent_string_boundaries_are_rejected() {
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        let error = compile(r"SELECT 'it\'s @id', @id", &id(1), dialect)
            .expect_err("backslash-quote boundary depends on server mode");
        assert_eq!(error.kind(), ErrorKind::Sql, "{dialect:?}");
    }

    let error = compile(r"SELECT 'a\'", &Params::none(), Dialect::MySql)
        .expect_err("escaped closing quote");
    assert_eq!(error.kind(), ErrorKind::Sql);
}

#[test]
fn mode_independent_backslashes_are_accepted() {
    for dialect in [Dialect::Postgres, Dialect::MySql] {
        let compiled = compile(r"SELECT 'C:\temp', @id", &id(1), dialect).expect("compiled");
        assert_eq!(compiled.parameters().len(), 1, "{dialect:?}");

        let compiled = compile(r"SELECT 'a\\', @id", &id(1), dialect).expect("compiled");
        assert_eq!(compiled.parameters().len(), 1, "{dialect:?}");
    }
}

#[test]
fn server_dependent_text_as_a_parameter_stays_bound() {
    let payload = r"a\' OR 1=1 --";
    let compiled = compile(
        "SELECT @value",
        &named(&[("value", Param::Text(payload.to_owned()))]),
        Dialect::MySql,
    )
    .expect("compiled");
    assert_eq!(compiled.sql(), "SELECT ?");
    assert!(!compiled.sql().contains("OR 1=1"));
    assert_eq!(compiled.parameters().len(), 1);
}

#[test]
fn mysql_double_dash_is_a_comment_only_before_whitespace() {
    let compiled = compile("SELECT 1--2", &Params::none(), Dialect::MySql).expect("compiled");
    assert_eq!(compiled.sql(), "SELECT 1--2");

    let error = compile("SELECT 1--2; SELECT 3", &Params::none(), Dialect::MySql)
        .expect_err("the semicolon is not inside a comment");
    assert_eq!(error.kind(), ErrorKind::Sql);

    let unused =
        compile("SELECT 1-- @id", &id(1), Dialect::MySql).expect_err("a space makes it a comment");
    assert_eq!(unused.kind(), ErrorKind::Parameter);

    for sql in ["SELECT 1--\t2", "SELECT 1--\u{7f}2"] {
        let compiled = compile(sql, &Params::none(), Dialect::MySql).expect("a control character");
        assert_eq!(compiled.sql(), sql);
    }
}

#[test]
fn mysql_double_dash_without_space_keeps_placeholders() {
    let compiled = compile(
        "SELECT 1--? , ?",
        &Params::positional([Param::I64(1), Param::I64(2)]),
        Dialect::MySql,
    )
    .expect("compiled");
    assert_eq!(compiled.sql(), "SELECT 1--? , ?");
    assert_eq!(compiled.parameters().len(), 2);

    let compiled = compile("SELECT 1--@id", &id(1), Dialect::MySql).expect("compiled");
    assert_eq!(compiled.sql(), "SELECT 1--?");
    assert_eq!(compiled.parameters().len(), 1);
}

#[test]
fn mysql_and_mariadb_executable_comments_after_a_terminator_are_rejected() {
    for sql in [
        "SELECT 1; /*!50700 SELECT 2 */",
        "SELECT 1; /*M!100100 SELECT 2 */",
    ] {
        let error = compile(sql, &Params::none(), Dialect::MySql).expect_err(sql);
        assert_eq!(error.kind(), ErrorKind::Sql, "{sql}");
    }
}

#[test]
fn trailing_semicolons_and_comments_are_allowed() {
    let compiled = compile("SELECT @id; -- done\n;", &id(1), Dialect::Sqlite).expect("compiled");
    assert_eq!(compiled.sql(), "SELECT ?; -- done\n;");
}

#[test]
fn multiple_statements_are_rejected() {
    for sql in [
        "SELECT @id; DROP TABLE users",
        "CREATE TRIGGER t AFTER INSERT ON users BEGIN SELECT 1; END",
    ] {
        let error = compile(sql, &id(1), Dialect::Sqlite).expect_err(sql);
        assert_eq!(error.kind(), ErrorKind::Sql, "{sql}");
    }
}

#[test]
fn semicolons_inside_literals_and_comments_are_not_separators() {
    for sql in ["SELECT ';'", "SELECT 1 /* ; */", "SELECT 1 -- ;"] {
        compile(sql, &Params::none(), Dialect::Sqlite).expect(sql);
    }
}

#[test]
fn mssql_nested_block_comments_are_recognized() {
    let sql = "SELECT 1 /* outer /* @id */ still */";
    let compiled = compile(sql, &Params::none(), Dialect::Mssql).expect("compiled");
    assert_eq!(compiled.sql(), sql);
    assert!(compiled.parameters().is_empty());
}

#[test]
fn mysql_and_mariadb_executable_comments_are_rejected() {
    for sql in ["SELECT 1 /*!50700 , @id */", "SELECT 1 /*M!100100 , @id */"] {
        let error = compile(sql, &Params::none(), Dialect::MySql).expect_err(sql);
        assert_eq!(error.kind(), ErrorKind::Sql, "{sql}");
    }
}

#[test]
fn unterminated_text_is_rejected() {
    for sql in [
        "SELECT 'unterminated",
        "SELECT \"unterminated",
        "SELECT `unterminated",
        "SELECT [unterminated",
        "SELECT 1 /* unterminated",
        "SELECT $tag$unterminated",
    ] {
        let dialect = if sql.starts_with("SELECT `") {
            Dialect::Sqlite
        } else if sql.starts_with("SELECT $") {
            Dialect::Postgres
        } else {
            Dialect::Sqlite
        };
        let error = compile(sql, &Params::none(), dialect).expect_err(sql);
        assert_eq!(error.kind(), ErrorKind::Sql, "{sql}");
    }
}

#[test]
fn empty_or_null_containing_sql_is_rejected() {
    for sql in ["", "   ", "-- only a comment", ";"] {
        let error = compile(sql, &Params::none(), Dialect::Sqlite).expect_err(sql);
        assert_eq!(error.kind(), ErrorKind::Sql, "{sql:?}");
    }
    let error = compile("SELECT 1\0", &Params::none(), Dialect::Sqlite).expect_err("NUL");
    assert_eq!(error.kind(), ErrorKind::Sql);
}

#[test]
fn missing_and_unused_parameters_are_distinct_errors() {
    let missing = compile("SELECT @id", &Params::none(), Dialect::Sqlite).expect_err("missing");
    assert_eq!(missing.kind(), ErrorKind::Parameter);

    let unused = compile(
        "SELECT @id",
        &named(&[("id", Param::I64(1)), ("other", Param::I64(2))]),
        Dialect::Sqlite,
    )
    .expect_err("unused");
    assert_eq!(unused.kind(), ErrorKind::Parameter);

    let too_few = compile(
        "SELECT ?, ?",
        &Params::positional([Param::I64(1)]),
        Dialect::Sqlite,
    )
    .expect_err("too few");
    assert_eq!(too_few.kind(), ErrorKind::Parameter);

    let too_many = compile(
        "SELECT ?",
        &Params::positional([Param::I64(1), Param::I64(2)]),
        Dialect::Sqlite,
    )
    .expect_err("too many");
    assert_eq!(too_many.kind(), ErrorKind::Parameter);
}

#[test]
fn named_and_positional_forms_cannot_be_mixed() {
    let named_sql = compile(
        "SELECT @id, ?",
        &named(&[("id", Param::I64(1))]),
        Dialect::Sqlite,
    )
    .expect_err("mixed");
    assert_eq!(named_sql.kind(), ErrorKind::Parameter);

    let positional_sql = compile(
        "SELECT @id, ?",
        &Params::positional([Param::I64(1), Param::I64(2)]),
        Dialect::Sqlite,
    )
    .expect_err("mixed");
    assert_eq!(positional_sql.kind(), ErrorKind::Parameter);
}

#[test]
fn names_are_case_sensitive() {
    let error = compile(
        "SELECT @ID",
        &named(&[("id", Param::I64(1))]),
        Dialect::Sqlite,
    )
    .expect_err("case differs");
    assert_eq!(error.kind(), ErrorKind::Parameter);

    let compiled = compile(
        "SELECT @ID",
        &named(&[("ID", Param::I64(1))]),
        Dialect::Sqlite,
    )
    .expect("exact");
    assert_eq!(compiled.parameters().len(), 1);
}

#[test]
fn json_parameters_parse_as_positional_or_named() {
    let positional = Params::from_json(r#"[1, "Ada", null, true, 1.5]"#).expect("array");
    assert_eq!(positional.len(), 5);
    let compiled = compile("SELECT ?, ?, ?, ?, ?", &positional, Dialect::Sqlite).expect("compiled");
    assert_eq!(compiled.parameters().len(), 5);

    let named = Params::from_json(r#"{"id": 1, "name": "Ada"}"#).expect("object");
    let compiled = compile("SELECT @id, @name", &named, Dialect::Sqlite).expect("compiled");
    assert_eq!(compiled.parameters().len(), 2);
}

#[test]
fn unsupported_json_parameter_values_are_rejected() {
    for json in [r#"[[1]]"#, r#"[{"a":1}]"#, r#"{"id":[1]}"#, r#""scalar""#] {
        let error = Params::from_json(json).expect_err(json);
        assert_eq!(error.kind(), ErrorKind::Parameter, "{json}");
    }
    let error = Params::from_json(r#"[18446744073709551615]"#).expect_err("u64 range");
    assert_eq!(error.kind(), ErrorKind::Parameter);
}

#[test]
fn non_finite_float_parameter_is_rejected() {
    let error = compile(
        "SELECT ?",
        &Params::positional([Param::F64(f64::NAN)]),
        Dialect::Sqlite,
    )
    .expect_err("NaN");
    assert_eq!(error.kind(), ErrorKind::Parameter);
}

#[test]
fn typed_null_keeps_its_declared_type() {
    let compiled = compile(
        "SELECT ?",
        &Params::positional([Param::TypedNull(DbType::I64)]),
        Dialect::Sqlite,
    )
    .expect("compiled");
    assert_eq!(compiled.parameters().len(), 1);
}

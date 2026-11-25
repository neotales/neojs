use std::fs;
use std::path::Path;

use neojs_http_core::{
    DEFAULT_MAX_BODY_BYTES, DEFAULT_MAX_COOKIES, DEFAULT_MAX_HEADER_BYTES, HttpCoreError,
    HttpCoreErrorCode, RouteMatch, Router, SameSite, SetCookie, format_set_cookie,
    mime_type_for_path, parse_cookies, parse_cookies_with_limit, resolve_static_file,
    validate_header_name, validate_header_value,
};

#[test]
fn router_specificity_is_deterministic_regardless_of_registration_order() {
    let patterns = [
        ("GET /{catchall...}", 1),
        ("GET /files/{path...}", 2),
        ("GET /files/{name}", 3),
        ("GET /files/readme.md", 4),
        ("GET /users/{id}", 5),
        ("GET /users/me", 6),
        ("/users/me", 7),
        ("GET /", 8),
        ("/{a}/b", 9),
        ("/a/{b}", 10),
    ];

    let mut forward = Router::new();
    for &(pat, id) in &patterns {
        forward.add(pat, id).unwrap();
    }

    let mut reverse = Router::new();
    for &(pat, id) in patterns.iter().rev() {
        reverse.add(pat, id).unwrap();
    }

    assert_eq!(forward.len(), patterns.len());
    assert!(!forward.is_empty());

    let cases = [
        (
            "GET",
            "/",
            RouteMatch::Matched {
                handler_id: 8,
                params: vec![],
            },
        ),
        (
            "GET",
            "/files/readme.md",
            RouteMatch::Matched {
                handler_id: 4,
                params: vec![],
            },
        ),
        (
            "GET",
            "/files/notes.txt",
            RouteMatch::Matched {
                handler_id: 3,
                params: vec![("name".to_owned(), "notes.txt".to_owned())],
            },
        ),
        (
            "GET",
            "/files/sub/dir/notes.txt",
            RouteMatch::Matched {
                handler_id: 2,
                params: vec![("path".to_owned(), "sub/dir/notes.txt".to_owned())],
            },
        ),
        (
            "GET",
            "/other/deep/path",
            RouteMatch::Matched {
                handler_id: 1,
                params: vec![("catchall".to_owned(), "other/deep/path".to_owned())],
            },
        ),
        (
            "GET",
            "/users/me",
            RouteMatch::Matched {
                handler_id: 6,
                params: vec![],
            },
        ),
        (
            "POST",
            "/users/me",
            RouteMatch::Matched {
                handler_id: 7,
                params: vec![],
            },
        ),
        (
            "GET",
            "/users/42",
            RouteMatch::Matched {
                handler_id: 5,
                params: vec![("id".to_owned(), "42".to_owned())],
            },
        ),
        (
            "GET",
            "/a/b",
            RouteMatch::Matched {
                handler_id: 10,
                params: vec![("b".to_owned(), "b".to_owned())],
            },
        ),
    ];

    for (method, path, expected) in cases {
        assert_eq!(
            forward.match_route(method, path),
            expected,
            "{method} {path} forward"
        );
        assert_eq!(
            reverse.match_route(method, path),
            expected,
            "{method} {path} reverse"
        );
    }
}

#[test]
fn router_head_fallback_and_405_allow_header() {
    let mut router = Router::new();
    router.add("POST /items", 1).unwrap();
    router.add("GET /items", 2).unwrap();
    router.add("DELETE /items", 3).unwrap();

    // GET automatically allows HEAD when no explicit HEAD route is registered.
    assert_eq!(
        router.match_route("HEAD", "/items"),
        RouteMatch::Matched {
            handler_id: 2,
            params: vec![],
        }
    );

    // 405 Method Not Allowed includes sorted deterministic Allow list with HEAD.
    assert_eq!(
        router.match_route("PUT", "/items"),
        RouteMatch::MethodNotAllowed {
            allow: vec![
                "DELETE".to_owned(),
                "GET".to_owned(),
                "HEAD".to_owned(),
                "POST".to_owned(),
            ],
        }
    );

    // Registering an explicit HEAD route overrides the GET->HEAD fallback without duplicating HEAD.
    router.add("HEAD /items", 4).unwrap();
    assert_eq!(
        router.match_route("HEAD", "/items"),
        RouteMatch::Matched {
            handler_id: 4,
            params: vec![],
        }
    );
    assert_eq!(
        router.match_route("PATCH", "/items"),
        RouteMatch::MethodNotAllowed {
            allow: vec![
                "DELETE".to_owned(),
                "GET".to_owned(),
                "HEAD".to_owned(),
                "POST".to_owned(),
            ],
        }
    );
}

#[test]
fn router_head_beats_any_method_when_get_registered() {
    let mut router = Router::new();
    router.add("/items", 1).unwrap();
    router.add("GET /items", 2).unwrap();

    assert_eq!(
        router.match_route("HEAD", "/items"),
        RouteMatch::Matched {
            handler_id: 2,
            params: vec![],
        }
    );
    assert_eq!(
        router.match_route("PUT", "/items"),
        RouteMatch::Matched {
            handler_id: 1,
            params: vec![],
        }
    );
}

#[test]
fn router_percent_decodes_and_rejects_invalid_encodings() {
    let mut router = Router::new();
    router.add("GET /users/{id}", 1).unwrap();
    router.add("GET /files/{path...}", 2).unwrap();

    // Space, UTF-8, literal '+', and %2F within a single param segment.
    assert_eq!(
        router.match_route("GET", "/users/alice%20caf%C3%A9+1%2F2"),
        RouteMatch::Matched {
            handler_id: 1,
            params: vec![("id".to_owned(), "alice café+1/2".to_owned())],
        }
    );

    // Multi-segment wildcard percent-decoding and trailing slash empty wildcard.
    assert_eq!(
        router.match_route("GET", "/files/dir%20one/file%20two.txt"),
        RouteMatch::Matched {
            handler_id: 2,
            params: vec![("path".to_owned(), "dir one/file two.txt".to_owned())],
        }
    );
    assert_eq!(
        router.match_route("GET", "/files/"),
        RouteMatch::Matched {
            handler_id: 2,
            params: vec![("path".to_owned(), String::new())],
        }
    );
    assert_eq!(router.match_route("GET", "/files"), RouteMatch::NotFound);
    assert_eq!(router.match_route("GET", "/users/"), RouteMatch::NotFound);

    // Malformed percent escapes, NUL bytes, and invalid UTF-8 fail closed as NotFound.
    assert_eq!(
        router.match_route("GET", "/users/%ZZ"),
        RouteMatch::NotFound
    );
    assert_eq!(router.match_route("GET", "/users/%2"), RouteMatch::NotFound);
    assert_eq!(
        router.match_route("GET", "/users/%00"),
        RouteMatch::NotFound
    );
    assert_eq!(
        router.match_route("GET", "/users/a\0b"),
        RouteMatch::NotFound
    );
    assert_eq!(
        router.match_route("GET", "/users/%FF"),
        RouteMatch::NotFound
    );
    assert_eq!(router.match_route("GET", "relative"), RouteMatch::NotFound);
}

#[test]
fn router_rejects_invalid_and_conflicting_patterns() {
    let mut router = Router::new();

    let invalid_patterns = [
        "",
        "GET",
        "get /users",
        "Get /users",
        "GET users",
        "GET  /users",
        "/users//posts",
        "/files/{path...}/extra",
        "/users/{id}/posts/{id}",
        "/users/{}",
        "/users/{...}",
        "/users/pre{id}",
        "/users/{id}post",
        "/users/{{id}}",
        "/users/{id}?q=1",
        "/users/{id}#frag",
        "/users/\n",
    ];

    for pat in invalid_patterns {
        let err = router.add(pat, 1).unwrap_err();
        assert_eq!(
            err.code(),
            HttpCoreErrorCode::InvalidPattern,
            "expected InvalidPattern for {pat:?}, got {err:?}"
        );
    }

    router.add("GET /users/{id}", 1).unwrap();
    let conflict_same = router.add("GET /users/{id}", 2).unwrap_err();
    assert_eq!(conflict_same.code(), HttpCoreErrorCode::RouteConflict);

    let conflict_renamed = router.add("GET /users/{user_id}", 3).unwrap_err();
    assert_eq!(conflict_renamed.code(), HttpCoreErrorCode::RouteConflict);

    // Different method or any-method is distinct and allowed.
    router.add("POST /users/{id}", 4).unwrap();
    router.add("/users/{id}", 5).unwrap();

    router.add("GET /files/{path...}", 6).unwrap();
    let wildcard_conflict = router.add("GET /files/{rest...}", 7).unwrap_err();
    assert_eq!(wildcard_conflict.code(), HttpCoreErrorCode::RouteConflict);
}

#[test]
fn static_file_resolution_handles_index_mime_and_security_guards() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();

    fs::write(root.path().join("index.html"), b"<html>root</html>").unwrap();
    fs::create_dir(root.path().join("sub")).unwrap();
    fs::write(root.path().join("sub/index.html"), b"<html>sub</html>").unwrap();
    fs::write(root.path().join("sub/hello world.txt"), b"hello").unwrap();
    fs::write(outside.path().join("secret.txt"), b"top-secret").unwrap();

    // Root index resolution.
    let root_index = resolve_static_file(root.path(), "/", Some("index.html")).unwrap();
    assert_eq!(root_index.mime_type, "text/html; charset=utf-8");
    assert_eq!(root_index.size, 17);
    assert!(root_index.path.ends_with("index.html"));

    // Subdirectory index resolution.
    let sub_index = resolve_static_file(root.path(), "/sub", Some("index.html")).unwrap();
    assert_eq!(sub_index.size, 16);

    // Directory without index_file yields NotFound.
    let err_no_index = resolve_static_file(root.path(), "/sub", None).unwrap_err();
    assert_eq!(err_no_index.code(), HttpCoreErrorCode::NotFound);

    // Missing file yields NotFound.
    let err_missing = resolve_static_file(root.path(), "/missing.txt", None).unwrap_err();
    assert_eq!(err_missing.code(), HttpCoreErrorCode::NotFound);

    // Percent-encoded filename and relative path (from wildcard capture).
    let space_file = resolve_static_file(root.path(), "sub/hello%20world.txt", None).unwrap();
    assert_eq!(space_file.mime_type, "text/plain; charset=utf-8");
    assert_eq!(space_file.size, 5);

    // Traversal, NUL, backslash, and encoded slash rejections.
    let bad_paths = [
        "/../secret.txt",
        "/sub/../../secret.txt",
        "/%2e%2e/secret.txt",
        "/sub/%2e%2e/sub/index.html",
        "/index.html\0.png",
        "/index%00.html",
        "/sub\\index.html",
        "/sub%5cindex.html",
        "/sub%2findex.html",
        "/%FF",
        "/%2",
    ];
    for bad in bad_paths {
        let err = resolve_static_file(root.path(), bad, Some("index.html")).unwrap_err();
        assert_eq!(
            err.code(),
            HttpCoreErrorCode::InvalidPath,
            "expected InvalidPath for {bad:?}, got {err:?}"
        );
    }

    // Invalid index_file values.
    for bad_idx in ["", ".", "..", "sub/index.html", "a\\b", "idx\0.html"] {
        let err = resolve_static_file(root.path(), "/", Some(bad_idx)).unwrap_err();
        assert_eq!(err.code(), HttpCoreErrorCode::InvalidPath);
    }

    // Symlink tests on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        // Symlink escaping root_dir is rejected.
        let escape_link = root.path().join("escape.txt");
        symlink(outside.path().join("secret.txt"), &escape_link).unwrap();
        let err = resolve_static_file(root.path(), "/escape.txt", None).unwrap_err();
        assert_eq!(err.code(), HttpCoreErrorCode::InvalidPath);
        assert!(
            !err.to_string().contains("secret.txt"),
            "error must not echo internal path"
        );

        // Directory symlink escaping root_dir is rejected.
        let dir_escape = root.path().join("outside_dir");
        symlink(outside.path(), &dir_escape).unwrap();
        let err = resolve_static_file(root.path(), "/outside_dir/secret.txt", None).unwrap_err();
        assert_eq!(err.code(), HttpCoreErrorCode::InvalidPath);

        // Index file symlink escaping root_dir is rejected.
        fs::create_dir(root.path().join("tricky")).unwrap();
        symlink(
            outside.path().join("secret.txt"),
            root.path().join("tricky/index.html"),
        )
        .unwrap();
        let err = resolve_static_file(root.path(), "/tricky", Some("index.html")).unwrap_err();
        assert_eq!(err.code(), HttpCoreErrorCode::InvalidPath);

        // Symlink staying inside root_dir is permitted.
        symlink(
            root.path().join("sub/hello world.txt"),
            root.path().join("alias.txt"),
        )
        .unwrap();
        let resolved = resolve_static_file(root.path(), "/alias.txt", None).unwrap();
        assert_eq!(resolved.size, 5);
    }
}

#[test]
fn mime_type_table_covers_all_required_extensions() {
    let cases = [
        ("a.html", "text/html; charset=utf-8"),
        ("a.HTM", "text/html; charset=utf-8"),
        ("a.css", "text/css; charset=utf-8"),
        ("a.js", "text/javascript; charset=utf-8"),
        ("a.mjs", "text/javascript; charset=utf-8"),
        ("a.json", "application/json"),
        ("a.svg", "image/svg+xml"),
        ("a.png", "image/png"),
        ("a.jpg", "image/jpeg"),
        ("a.jpeg", "image/jpeg"),
        ("a.gif", "image/gif"),
        ("a.webp", "image/webp"),
        ("a.ico", "image/x-icon"),
        ("a.txt", "text/plain; charset=utf-8"),
        ("a.xml", "application/xml"),
        ("a.wasm", "application/wasm"),
        ("a.woff", "font/woff"),
        ("a.woff2", "font/woff2"),
        ("a.pdf", "application/pdf"),
        ("a.unknown", "application/octet-stream"),
        ("README", "application/octet-stream"),
    ];

    for (name, expected) in cases {
        assert_eq!(mime_type_for_path(Path::new(name)), expected, "{name}");
    }
}

#[test]
fn cookies_parse_format_and_reject_injection() {
    assert_eq!(parse_cookies("").unwrap(), Vec::<(String, String)>::new());
    assert_eq!(
        parse_cookies("a=1; b=\"two\"; empty=; ").unwrap(),
        vec![
            ("a".to_owned(), "1".to_owned()),
            ("b".to_owned(), "two".to_owned()),
            ("empty".to_owned(), String::new()),
        ]
    );

    // Cookie count bound enforcement.
    let at_limit = (0..DEFAULT_MAX_COOKIES)
        .map(|i| format!("c{i}=v{i}"))
        .collect::<Vec<_>>()
        .join("; ");
    assert_eq!(parse_cookies(&at_limit).unwrap().len(), DEFAULT_MAX_COOKIES);

    let over_limit = format!("{at_limit}; extra=1");
    let err = parse_cookies(&over_limit).unwrap_err();
    assert_eq!(err.code(), HttpCoreErrorCode::InvalidCookie);

    assert_eq!(parse_cookies_with_limit("a=1; b=2", 2).unwrap().len(), 2);
    assert!(parse_cookies_with_limit("a=1; b=2; c=3", 2).is_err());

    // Malformed / injected Cookie headers.
    for bad in [
        "a=1\r\nEvil: 1",
        "a=1\nEvil: 1",
        "a=1\0",
        "bad name=1",
        "=noname",
        "noequals",
        "a=\"unclosed",
    ] {
        assert_eq!(
            parse_cookies(bad).unwrap_err().code(),
            HttpCoreErrorCode::InvalidCookie,
            "expected InvalidCookie for {bad:?}"
        );
    }

    // Set-Cookie formatting with all attributes.
    let full = SetCookie::new("session", "abc_123")
        .path("/app")
        .domain("example.com")
        .max_age(0)
        .http_only(true)
        .secure(true)
        .same_site(SameSite::None);
    assert_eq!(
        full.format().unwrap(),
        "session=abc_123; Path=/app; Domain=example.com; Max-Age=0; HttpOnly; Secure; SameSite=None"
    );

    assert_eq!(
        format_set_cookie(&SetCookie::new("s", "1").same_site(SameSite::Lax)).unwrap(),
        "s=1; SameSite=Lax"
    );
    assert_eq!(
        format_set_cookie(&SetCookie::new("s", "1").same_site(SameSite::Strict)).unwrap(),
        "s=1; SameSite=Strict"
    );

    // Rejection of CR/LF/NUL/attribute-injection in Set-Cookie fields.
    let invalid_cookies = [
        SetCookie::new("", "val"),
        SetCookie::new("bad name", "val"),
        SetCookie::new("bad\r\n", "val"),
        SetCookie::new("ok", "val\r\nSet-Cookie: admin=1"),
        SetCookie::new("ok", "val\n"),
        SetCookie::new("ok", "val\0"),
        SetCookie::new("ok", "val; HttpOnly"),
        SetCookie::new("ok", "val with space"),
        SetCookie::new("ok", "val\"quote"),
        SetCookie::new("ok", "val,comma"),
        SetCookie::new("ok", "val\\slash"),
        SetCookie::new("ok", "1").path(""),
        SetCookie::new("ok", "1").path("/; HttpOnly"),
        SetCookie::new("ok", "1").path("/\r\n"),
        SetCookie::new("ok", "1").domain(""),
        SetCookie::new("ok", "1").domain("a.com; HttpOnly"),
        SetCookie::new("ok", "1").domain(".."),
    ];

    for cookie in invalid_cookies {
        let err = format_set_cookie(&cookie).unwrap_err();
        assert_eq!(err.code(), HttpCoreErrorCode::InvalidCookie, "{cookie:?}");
        // Ensure error strings never leak secret cookie values.
        assert!(!err.to_string().contains("admin=1"));
    }
}

#[test]
fn policy_constants_and_header_validation() {
    assert_eq!(DEFAULT_MAX_BODY_BYTES, 10 * 1024 * 1024);
    assert_eq!(DEFAULT_MAX_HEADER_BYTES, 64 * 1024);

    assert!(validate_header_name("X-Custom-Header_1").is_ok());
    for bad_name in ["", "Bad Name", "Bad:Name", "Bad\r\n", "Bad\0", "Café"] {
        let err = validate_header_name(bad_name).unwrap_err();
        assert!(matches!(err, HttpCoreError::InvalidHeader { .. }));
        assert_eq!(err.code(), HttpCoreErrorCode::InvalidHeader);
    }

    assert!(validate_header_value("").is_ok());
    assert!(validate_header_value("text/plain;\tcharset=utf-8").is_ok());
    for bad_val in ["a\r\nb", "a\nb", "a\rb", "a\0b", "a\x07b", "a\x7Fb"] {
        let err = validate_header_value(bad_val).unwrap_err();
        assert_eq!(err.code(), HttpCoreErrorCode::InvalidHeader);
    }
}

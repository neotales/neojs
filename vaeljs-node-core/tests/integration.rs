use std::path::Path;
use std::sync::Mutex;

use vaeljs_node_core::buffer::{Encoding, MAX_DECODE_INPUT_BYTES, decode, encode};
use vaeljs_node_core::canonical::{
    SUPPORTED_NODE_MODULES, is_known_bare_node_module, is_supported_node_module,
};
use vaeljs_node_core::child_process::{SpawnOptions, spawn_sync};
use vaeljs_node_core::error::NodeCoreErrorCode;
use vaeljs_node_core::os::{
    arch, cpus_count, endianness, eol, homedir, hostname, machine, os_type, platform, tmpdir,
};
use vaeljs_node_core::path::{
    MAX_AGGREGATE_PATH_BYTES, MAX_SINGLE_PATH_BYTES, ParsedPath, Style, basename, delimiter,
    dirname, extname, format, is_absolute, join, normalize, parse, relative, resolve, sep,
};
use vaeljs_node_core::process::{chdir, cwd, env_entries, env_get, hrtime_nanos, pid};
use vaeljs_node_core::url::{file_url_to_path, path_to_file_url};

static CWD_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn path_posix_and_win32_comprehensive() {
    assert_eq!(sep(Style::Posix), "/");
    assert_eq!(sep(Style::Win32), "\\");
    assert_eq!(delimiter(Style::Posix), ":");
    assert_eq!(delimiter(Style::Win32), ";");

    // POSIX join / resolve / normalize / relative
    assert_eq!(
        join(Style::Posix, &["/a", "b", "c", "..", "d"]).unwrap(),
        "/a/b/d"
    );
    assert_eq!(join(Style::Posix, &["", "."]).unwrap(), ".");
    assert_eq!(
        resolve(Style::Posix, "/base/dir", &["sub", "../file.txt"]).unwrap(),
        "/base/dir/file.txt"
    );
    assert_eq!(
        resolve(Style::Posix, "/base/dir", &["sub", "/override", "x"]).unwrap(),
        "/override/x"
    );
    assert_eq!(
        resolve(Style::Posix, "not-abs", &["/abs", "x"]).unwrap(),
        "/abs/x"
    );
    assert_eq!(
        resolve(Style::Posix, "not-abs", &["rel"])
            .unwrap_err()
            .code(),
        NodeCoreErrorCode::InvalidPath
    );
    assert_eq!(
        normalize(Style::Posix, "/a/./b/../c//d/").unwrap(),
        "/a/c/d/"
    );
    assert_eq!(
        relative(Style::Posix, "/a/b/c", "/a/d/e").unwrap(),
        "../../d/e"
    );

    // Win32 join / resolve / normalize / relative / parse / format
    assert_eq!(
        join(Style::Win32, &["C:\\a", "b", "..\\c"]).unwrap(),
        "C:\\a\\c"
    );
    assert_eq!(
        resolve(Style::Win32, "C:\\base", &["D:\\other", "sub", "..\\file"]).unwrap(),
        "D:\\other\\file"
    );
    assert_eq!(
        resolve(Style::Win32, "C:\\base\\dir", &["C:rel"]).unwrap(),
        "C:\\base\\dir\\rel"
    );
    assert_eq!(
        normalize(Style::Win32, "C:/a/b/../c\\").unwrap(),
        "C:\\a\\c\\"
    );
    assert_eq!(
        relative(Style::Win32, "C:\\a\\b", "C:\\a\\c\\d").unwrap(),
        "..\\c\\d"
    );
    assert_eq!(relative(Style::Win32, "C:\\a", "D:\\b").unwrap(), "D:\\b");

    let parsed = parse(Style::Win32, "C:\\dir\\sub\\archive.tar.gz").unwrap();
    assert_eq!(
        parsed,
        ParsedPath {
            root: "C:\\".to_owned(),
            dir: "C:\\dir\\sub".to_owned(),
            base: "archive.tar.gz".to_owned(),
            ext: ".gz".to_owned(),
            name: "archive.tar".to_owned(),
        }
    );
    assert_eq!(
        format(Style::Win32, &parsed).unwrap(),
        "C:\\dir\\sub\\archive.tar.gz"
    );

    let unc = parse(Style::Win32, "\\\\server\\share\\folder\\file.txt").unwrap();
    assert_eq!(unc.root, "\\\\server\\share\\");
    assert_eq!(unc.dir, "\\\\server\\share\\folder");
    assert_eq!(unc.base, "file.txt");
    assert_eq!(
        dirname(Style::Win32, "\\\\server\\share").unwrap(),
        "\\\\server\\share"
    );
    assert_eq!(
        basename(Style::Win32, "\\\\server\\share\\", None).unwrap(),
        ""
    );
    assert_eq!(extname(Style::Win32, "\\\\server\\share").unwrap(), "");
    assert!(is_absolute(Style::Win32, "\\\\server\\share\\x").unwrap());
    assert!(!is_absolute(Style::Win32, "C:relative").unwrap());

    // Bounds
    const {
        assert!(MAX_SINGLE_PATH_BYTES < MAX_AGGREGATE_PATH_BYTES);
    }
    let oversized = "x".repeat(MAX_SINGLE_PATH_BYTES + 1);
    assert_eq!(
        dirname(Style::Posix, &oversized).unwrap_err().code(),
        NodeCoreErrorCode::InputTooLarge
    );
}

#[test]
fn os_values_follow_node_conventions() {
    let p = platform();
    assert!(
        [
            "linux", "darwin", "win32", "freebsd", "openbsd", "netbsd", "android", "sunos", "aix"
        ]
        .contains(&p),
        "unexpected platform: {p}"
    );
    let a = arch();
    assert!(
        [
            "x64", "arm64", "ia32", "arm", "loong64", "mips", "mipsel", "ppc", "ppc64", "riscv64",
            "s390x"
        ]
        .contains(&a),
        "unexpected arch: {a}"
    );
    if cfg!(windows) {
        assert_eq!(eol(), "\r\n");
        assert_eq!(os_type(), "Windows_NT");
    } else if cfg!(target_os = "macos") {
        assert_eq!(eol(), "\n");
        assert_eq!(os_type(), "Darwin");
    } else if cfg!(target_os = "linux") {
        assert_eq!(eol(), "\n");
        assert_eq!(os_type(), "Linux");
    }
    assert!(cpus_count() >= 1);
    assert!(matches!(endianness(), "LE" | "BE"));
    assert!(!machine().is_empty());
    assert!(tmpdir().is_absolute());
    assert!(!hostname().unwrap().is_empty());
    if std::env::var_os("HOME").is_some_and(|v| !v.is_empty())
        || std::env::var_os("USERPROFILE").is_some_and(|v| !v.is_empty())
    {
        assert!(!homedir().unwrap().as_os_str().is_empty());
    }
}

#[test]
fn process_cwd_chdir_env_pid_and_hrtime() {
    let _guard = CWD_LOCK.lock().unwrap();
    let original = cwd().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    chdir(tmp.path()).unwrap();
    let now_cwd = cwd().unwrap();
    assert_eq!(
        now_cwd.canonicalize().unwrap(),
        tmp.path().canonicalize().unwrap()
    );
    chdir(&original).unwrap();

    assert!(pid() > 0);
    let t1 = hrtime_nanos();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let t2 = hrtime_nanos();
    assert!(t2 > t1);

    let entries = env_entries();
    assert!(entries.windows(2).all(|w| w[0].0 <= w[1].0));
    if let Some((k, v)) = entries.first() {
        assert_eq!(env_get(k).unwrap().as_deref(), Some(v.as_str()));
    }
    assert_eq!(
        env_get("VAELJS_NODE_CORE_MISSING_ENV_KEY_987654").unwrap(),
        None
    );
}

#[test]
fn url_file_round_trip_and_security_guards() {
    if cfg!(windows) {
        let path = Path::new("C:\\Users\\neo dev\\file.txt");
        let url = path_to_file_url(path).unwrap();
        assert_eq!(url, "file:///C:/Users/neo%20dev/file.txt");
        assert_eq!(file_url_to_path(&url).unwrap(), path);
    } else {
        let path = Path::new("/tmp/neo dev/file.txt");
        let url = path_to_file_url(path).unwrap();
        assert_eq!(url, "file:///tmp/neo%20dev/file.txt");
        assert_eq!(file_url_to_path(&url).unwrap(), path);
        assert_eq!(
            file_url_to_path("file://localhost/tmp/neo%20dev/file.txt").unwrap(),
            path
        );
        assert_eq!(
            file_url_to_path("file://evil.example/etc/passwd")
                .unwrap_err()
                .code(),
            NodeCoreErrorCode::InvalidUrl
        );
    }

    for bad in [
        "",
        "http://example.com/a",
        "https://localhost/tmp/a",
        "file:///tmp/a%2fb",
        "file:///tmp/a%2Fb",
        "file:///tmp/a%5cb",
        "file:///tmp/a%5Cb",
        "file:///tmp/a%00b",
        "file://user@localhost/tmp/a",
    ] {
        assert_eq!(
            file_url_to_path(bad).unwrap_err().code(),
            NodeCoreErrorCode::InvalidUrl,
            "expected InvalidUrl for {bad:?}"
        );
    }

    assert_eq!(
        path_to_file_url("relative/path.txt").unwrap_err().code(),
        NodeCoreErrorCode::InvalidPath
    );
}

#[test]
fn buffer_encodings_and_bounds() {
    let sample = b"\x00\x01\x7f\x80\xffhello vaeljs!";
    for enc in [
        Encoding::Hex,
        Encoding::Base64,
        Encoding::Base64Url,
        Encoding::Latin1,
    ] {
        let encoded = encode(sample, enc);
        assert_eq!(decode(&encoded, enc).unwrap(), sample);
    }

    assert_eq!(encode(b"hi", Encoding::Utf8), "hi");
    assert_eq!(decode("hi", Encoding::Utf8).unwrap(), b"hi");
    assert_eq!(encode(b"\x80A", Encoding::Ascii), "\x00A");
    assert_eq!(decode("ABC", Encoding::Ascii).unwrap(), b"ABC");
    assert_eq!(encode(&[0xff, 0xfe], Encoding::Utf8), "\u{fffd}\u{fffd}");

    let too_big = "a".repeat(MAX_DECODE_INPUT_BYTES + 1);
    assert_eq!(
        decode(&too_big, Encoding::Utf8).unwrap_err().code(),
        NodeCoreErrorCode::InputTooLarge
    );
}

#[test]
fn canonical_supported_and_bare_node_modules() {
    let expected = [
        "node:path",
        "node:path/posix",
        "node:path/win32",
        "node:os",
        "node:fs",
        "node:fs/promises",
        "node:process",
        "node:child_process",
        "node:url",
        "node:buffer",
        "node:crypto",
    ];
    for spec in expected {
        assert!(SUPPORTED_NODE_MODULES.contains(&spec));
        assert!(is_supported_node_module(spec));
        assert!(!is_known_bare_node_module(spec));
        let bare = spec.strip_prefix("node:").unwrap();
        assert!(is_known_bare_node_module(bare));
    }

    assert!(is_known_bare_node_module("http"));
    assert!(is_known_bare_node_module("zlib"));
    assert!(!is_known_bare_node_module("vael:fs"));
    assert!(!is_known_bare_node_module("./fs"));
    assert!(!is_known_bare_node_module("lodash"));
}

#[test]
fn fs_and_crypto_reexports_work() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("note.txt");
    vaeljs_node_core::fs::write_file_atomic(
        &file,
        b"node:fs reuse",
        vaeljs_node_core::fs::AtomicWriteOptions::default(),
    )
    .unwrap();
    let read = vaeljs_node_core::fs::read_file(&file, vaeljs_node_core::fs::ReadOptions::default())
        .unwrap();
    assert_eq!(read, b"node:fs reuse");

    let digest = vaeljs_node_core::crypto::hash::digest(
        vaeljs_node_core::crypto::hash::HashAlgorithm::Sha256,
        b"abc",
        vaeljs_node_core::crypto::hash::OutputEncoding::Hex,
    )
    .unwrap();
    assert_eq!(
        digest,
        b"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn child_process_rejects_bad_inputs_without_spawning() {
    assert_eq!(
        spawn_sync("", &[], &SpawnOptions::default())
            .unwrap_err()
            .code(),
        NodeCoreErrorCode::InvalidInput
    );
    assert_eq!(
        spawn_sync(
            "definitely_missing_vaeljs_binary_xyz",
            &[],
            &SpawnOptions::default()
        )
        .unwrap_err()
        .code(),
        NodeCoreErrorCode::NotFound
    );
}

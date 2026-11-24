use std::collections::HashSet;
use std::error::Error as _;
use std::path::PathBuf;

use neojs_fs_core::{
    AtomicWriteOptions, CopyOptions, EXPORTS, FsError, FsErrorCode, MkdirOptions, ReadOptions,
    RemoveOptions, SPECIFIER, WriteOptions, copy_file, exists, lstat, mkdir, read_dir, read_file,
    read_text_file, realpath, remove, rename, stat, write_file, write_file_atomic, write_text_file,
};

#[cfg(unix)]
fn create_symlink(original: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(original, link)
}

#[cfg(windows)]
fn create_symlink(original: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    if original.is_dir() {
        std::os::windows::fs::symlink_dir(original, link)
    } else {
        std::os::windows::fs::symlink_file(original, link)
    }
}

fn assert_code<T: std::fmt::Debug>(res: Result<T, FsError>, expected: FsErrorCode) -> FsError {
    let err = res.expect_err("expected error");
    assert_eq!(err.code(), expected, "unexpected error: {err}");
    assert!(err.code().as_str().starts_with("NEO_FS_"));
    err
}

#[test]
fn module_metadata_matches_surface() {
    assert_eq!(SPECIFIER, "neo:fs");
    assert_eq!(
        EXPORTS,
        &[
            "readFile",
            "readTextFile",
            "writeFile",
            "writeTextFile",
            "writeFileAtomic",
            "exists",
            "stat",
            "lstat",
            "readDir",
            "mkdir",
            "remove",
            "rename",
            "copyFile",
            "realpath",
        ]
    );
    let unique: HashSet<&str> = EXPORTS.iter().copied().collect();
    assert_eq!(unique.len(), EXPORTS.len());
}

#[test]
fn rejects_empty_and_nul_paths_across_all_operations() {
    let dir = tempfile::tempdir().unwrap();
    let valid = dir.path().join("valid.txt");
    write_file(&valid, b"ok", WriteOptions::default()).unwrap();

    let nul_path = "bad\0path";

    for bad in ["", nul_path] {
        assert_code(
            read_file(bad, ReadOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(
            read_text_file(bad, ReadOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(
            write_file(bad, b"x", WriteOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(
            write_text_file(bad, "x", WriteOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(
            write_file_atomic(bad, b"x", AtomicWriteOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(exists(bad), FsErrorCode::InvalidPath);
        assert_code(stat(bad), FsErrorCode::InvalidPath);
        assert_code(lstat(bad), FsErrorCode::InvalidPath);
        assert_code(read_dir(bad), FsErrorCode::InvalidPath);
        assert_code(
            mkdir(bad, MkdirOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(
            remove(bad, RemoveOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(rename(bad, &valid), FsErrorCode::InvalidPath);
        assert_code(rename(&valid, bad), FsErrorCode::InvalidPath);
        assert_code(
            copy_file(bad, &valid, CopyOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(
            copy_file(&valid, bad, CopyOptions::default()),
            FsErrorCode::InvalidPath,
        );
        assert_code(realpath(bad), FsErrorCode::InvalidPath);
    }
}

#[test]
fn error_messages_do_not_echo_caller_paths() {
    let dir = tempfile::tempdir().unwrap();
    let secret_segment = "secret-internal-workspace-segment-42";
    let missing = dir.path().join(secret_segment).join("missing.txt");
    let err = assert_code(
        read_file(&missing, ReadOptions::default()),
        FsErrorCode::NotFound,
    );
    assert!(!err.to_string().contains(secret_segment));
    assert!(err.source().is_some());
}

#[test]
fn read_and_write_files_with_bounds_and_utf8_checks() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bytes.bin");
    let txt = dir.path().join("note.txt");

    // Missing file fails with NotFound.
    assert_code(
        read_file(&bin, ReadOptions::default()),
        FsErrorCode::NotFound,
    );
    assert_code(
        read_text_file(&txt, ReadOptions::default()),
        FsErrorCode::NotFound,
    );

    // Binary write & bounded read (exact boundary succeeds, +1 fails).
    let payload = b"0123456789";
    write_file(&bin, payload, WriteOptions::default()).unwrap();
    assert_eq!(
        read_file(&bin, ReadOptions::default()).unwrap(),
        payload.to_vec()
    );
    assert_eq!(
        read_file(
            &bin,
            ReadOptions {
                max_bytes: Some(10)
            }
        )
        .unwrap(),
        payload.to_vec()
    );
    assert_code(
        read_file(&bin, ReadOptions { max_bytes: Some(9) }),
        FsErrorCode::TooLarge,
    );
    assert_code(
        read_file(&bin, ReadOptions { max_bytes: Some(0) }),
        FsErrorCode::TooLarge,
    );

    // Empty file with max_bytes: Some(0) succeeds.
    let empty = dir.path().join("empty.bin");
    write_file(&empty, b"", WriteOptions::default()).unwrap();
    assert_eq!(
        read_file(&empty, ReadOptions { max_bytes: Some(0) }).unwrap(),
        Vec::<u8>::new()
    );

    // Text write, append, create: false, create_new: true, and invalid UTF-8.
    write_text_file(&txt, "hello", WriteOptions::default()).unwrap();
    write_text_file(
        &txt,
        " neo",
        WriteOptions {
            append: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read_text_file(&txt, ReadOptions::default()).unwrap(),
        "hello neo"
    );

    // Truncate by default when overwriting.
    write_text_file(&txt, "reset", WriteOptions::default()).unwrap();
    assert_eq!(
        read_text_file(&txt, ReadOptions::default()).unwrap(),
        "reset"
    );

    // create: false fails with NotFound on missing path, succeeds on existing path.
    let missing = dir.path().join("no-create.txt");
    assert_code(
        write_file(
            &missing,
            b"x",
            WriteOptions {
                create: false,
                ..Default::default()
            },
        ),
        FsErrorCode::NotFound,
    );
    write_file(
        &txt,
        b"updated",
        WriteOptions {
            create: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        read_text_file(&txt, ReadOptions::default()).unwrap(),
        "updated"
    );

    // create_new: true fails if file already exists.
    assert_code(
        write_file(
            &txt,
            b"dup",
            WriteOptions {
                create_new: true,
                ..Default::default()
            },
        ),
        FsErrorCode::AlreadyExists,
    );

    // Conflicting create_new + append fails with InvalidArgument.
    assert_code(
        write_file(
            &txt,
            b"conflict",
            WriteOptions {
                create: true,
                create_new: true,
                append: true,
            },
        ),
        FsErrorCode::InvalidArgument,
    );
    assert_code(
        write_text_file(
            &txt,
            "conflict",
            WriteOptions {
                create: true,
                create_new: true,
                append: true,
            },
        ),
        FsErrorCode::InvalidArgument,
    );

    // Invalid UTF-8 bytes fail closed with InvalidUtf8.
    let bad_utf8 = dir.path().join("bad.utf8");
    write_file(&bad_utf8, [0xff, 0xfe, 0xfd], WriteOptions::default()).unwrap();
    assert_code(
        read_text_file(&bad_utf8, ReadOptions::default()),
        FsErrorCode::InvalidUtf8,
    );
    assert_code(
        read_text_file(&bad_utf8, ReadOptions { max_bytes: Some(2) }),
        FsErrorCode::TooLarge,
    );

    // Reading or writing a directory returns IsADirectory.
    assert_code(
        read_file(dir.path(), ReadOptions::default()),
        FsErrorCode::IsADirectory,
    );
    assert_code(
        read_file(dir.path(), ReadOptions { max_bytes: Some(1) }),
        FsErrorCode::IsADirectory,
    );
    assert_code(
        write_file(dir.path(), b"x", WriteOptions::default()),
        FsErrorCode::IsADirectory,
    );
}

#[test]
fn write_file_atomic_overwrite_and_temp_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("atomic.txt");

    // Create via atomic write with sync: true.
    write_file_atomic(&target, b"first", AtomicWriteOptions::default()).unwrap();
    assert_eq!(
        read_text_file(&target, ReadOptions::default()).unwrap(),
        "first"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    // Overwrite: false fails with AlreadyExists without clobbering.
    assert_code(
        write_file_atomic(
            &target,
            b"second",
            AtomicWriteOptions {
                sync: false,
                overwrite: false,
            },
        ),
        FsErrorCode::AlreadyExists,
    );
    assert_eq!(
        read_text_file(&target, ReadOptions::default()).unwrap(),
        "first"
    );

    // Overwrite: true replaces contents cleanly.
    write_file_atomic(
        &target,
        b"second",
        AtomicWriteOptions {
            sync: false,
            overwrite: true,
        },
    )
    .unwrap();
    assert_eq!(
        read_text_file(&target, ReadOptions::default()).unwrap(),
        "second"
    );

    // overwrite: false works when target does not yet exist.
    let fresh = dir.path().join("fresh.txt");
    write_file_atomic(
        &fresh,
        b"brand-new",
        AtomicWriteOptions {
            sync: true,
            overwrite: false,
        },
    )
    .unwrap();
    assert_eq!(
        read_text_file(&fresh, ReadOptions::default()).unwrap(),
        "brand-new"
    );

    // Failure when target is an existing directory cleans up the sibling temp file.
    let subdir = dir.path().join("subdir");
    mkdir(&subdir, MkdirOptions::default()).unwrap();
    assert_code(
        write_file_atomic(
            &subdir,
            b"cannot-replace-dir",
            AtomicWriteOptions::default(),
        ),
        FsErrorCode::IsADirectory,
    );

    // Trailing slash or root-only paths are rejected as InvalidPath.
    assert_code(
        write_file_atomic(
            format!("{}/", dir.path().display()),
            b"x",
            AtomicWriteOptions::default(),
        ),
        FsErrorCode::InvalidPath,
    );
    assert_code(
        write_file_atomic("/", b"x", AtomicWriteOptions::default()),
        FsErrorCode::InvalidPath,
    );

    // Missing parent directory returns NotFound.
    assert_code(
        write_file_atomic(
            dir.path().join("missing-parent/file.txt"),
            b"x",
            AtomicWriteOptions::default(),
        ),
        FsErrorCode::NotFound,
    );

    // Verify no leftover `.neojs-tmp-*` files exist in dir.
    let entries = read_dir(dir.path()).unwrap();
    assert!(
        entries.iter().all(|e| !e.name.starts_with(".neojs-tmp-")),
        "found leftover temp file: {entries:?}"
    );
}

#[test]
fn exists_stat_lstat_and_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("real.txt");
    write_file(&file, b"abcdef", WriteOptions::default()).unwrap();

    assert!(exists(&file).unwrap());
    assert!(!exists(dir.path().join("missing.txt")).unwrap());

    // Traversing through a regular file surfaces NotADirectory instead of returning Ok(false).
    assert_code(exists(file.join("child")), FsErrorCode::NotADirectory);

    let file_stat = stat(&file).unwrap();
    assert!(file_stat.is_file);
    assert!(!file_stat.is_directory);
    assert!(!file_stat.is_symlink);
    assert_eq!(file_stat.size, 6);
    assert!(file_stat.modified_ms.is_some());
    assert!(!file_stat.readonly);

    let dir_stat = stat(dir.path()).unwrap();
    assert!(dir_stat.is_directory);
    assert!(!dir_stat.is_file);
    assert!(!dir_stat.is_symlink);

    // Symlink to file and dangling symlink.
    let link = dir.path().join("link.txt");
    create_symlink(&file, &link).unwrap();

    let s = stat(&link).unwrap();
    assert!(s.is_file);
    assert!(!s.is_symlink);
    assert_eq!(s.size, 6);

    let ls = lstat(&link).unwrap();
    assert!(ls.is_symlink);
    assert!(!ls.is_file);

    assert!(exists(&link).unwrap());

    let dangling = dir.path().join("dangling.txt");
    create_symlink(&dir.path().join("does-not-exist"), &dangling).unwrap();
    assert!(!exists(&dangling).unwrap());
    assert_code(stat(&dangling), FsErrorCode::NotFound);
    let dls = lstat(&dangling).unwrap();
    assert!(dls.is_symlink);
    assert!(!dls.is_file);

    // Readonly metadata check.
    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&file, perms).unwrap();
    assert!(stat(&file).unwrap().readonly);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    #[cfg(not(unix))]
    {
        let mut writable = std::fs::metadata(&file).unwrap().permissions();
        writable.set_readonly(false);
        std::fs::set_permissions(&file, writable).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn exists_surfaces_permission_denied() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    mkdir(&locked, MkdirOptions::default()).unwrap();
    let child = locked.join("secret.txt");
    write_file(&child, b"secret", WriteOptions::default()).unwrap();

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let res = exists(&child);
    // Restore permissions before asserting so tempdir cleanup always succeeds.
    let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));

    // Root bypasses DAC 0o000; only assert PermissionDenied when non-root.
    if std::fs::read_dir("/root").is_err() {
        assert_code(res, FsErrorCode::PermissionDenied);
    }
}

#[test]
fn read_dir_is_sorted_and_handles_errors() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path().join("zeta.txt"), b"z", WriteOptions::default()).unwrap();
    write_file(dir.path().join("alpha.txt"), b"a", WriteOptions::default()).unwrap();
    mkdir(dir.path().join("mid-dir"), MkdirOptions::default()).unwrap();
    create_symlink(&dir.path().join("alpha.txt"), &dir.path().join("beta-link")).unwrap();

    let entries = read_dir(dir.path()).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["alpha.txt", "beta-link", "mid-dir", "zeta.txt"]);

    assert!(entries[0].is_file && !entries[0].is_directory && !entries[0].is_symlink);
    assert!(entries[1].is_symlink && !entries[1].is_directory);
    assert!(entries[2].is_directory && !entries[2].is_file && !entries[2].is_symlink);
    assert!(entries[3].is_file);

    assert_code(
        read_dir(dir.path().join("alpha.txt")),
        FsErrorCode::NotADirectory,
    );
    assert_code(read_dir(dir.path().join("nope")), FsErrorCode::NotFound);
}

#[cfg(unix)]
#[test]
fn read_dir_rejects_non_utf8_entry_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = tempfile::tempdir().unwrap();
    let bad_name = OsStr::from_bytes(b"bad-\xff-name");
    std::fs::write(dir.path().join(bad_name), b"x").unwrap();

    assert_code(read_dir(dir.path()), FsErrorCode::InvalidUtf8);
}

#[test]
fn mkdir_and_remove_recursive_and_symlink_safety() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("one/two/three");

    // Non-recursive fails when parent does not exist.
    assert_code(
        mkdir(&nested, MkdirOptions::default()),
        FsErrorCode::NotFound,
    );

    // Recursive creates full hierarchy and is idempotent on existing directory.
    mkdir(&nested, MkdirOptions { recursive: true }).unwrap();
    mkdir(&nested, MkdirOptions { recursive: true }).unwrap();
    assert!(stat(&nested).unwrap().is_directory);

    // Non-recursive mkdir on existing directory fails with AlreadyExists.
    assert_code(
        mkdir(&nested, MkdirOptions::default()),
        FsErrorCode::AlreadyExists,
    );

    // Populate directory tree and add a symlink pointing to an external directory.
    let outside = dir.path().join("outside");
    mkdir(&outside, MkdirOptions::default()).unwrap();
    let keep_file = outside.join("keep.txt");
    write_text_file(&keep_file, "must survive", WriteOptions::default()).unwrap();

    let tree = dir.path().join("tree");
    mkdir(tree.join("sub"), MkdirOptions { recursive: true }).unwrap();
    write_file(tree.join("sub/file.txt"), b"hi", WriteOptions::default()).unwrap();
    create_symlink(&outside, &tree.join("link-to-outside")).unwrap();

    // Non-recursive remove fails on non-empty directory.
    assert_code(
        remove(&tree, RemoveOptions::default()),
        FsErrorCode::DirectoryNotEmpty,
    );

    // Removing a symlink directly (even with recursive: true) unlinks only the symlink.
    let direct_link = dir.path().join("direct-link");
    create_symlink(&outside, &direct_link).unwrap();
    remove(&direct_link, RemoveOptions { recursive: true }).unwrap();
    assert!(!exists(&direct_link).unwrap());
    assert!(exists(&keep_file).unwrap());

    // Recursive remove deletes tree and symlink inside it, without touching outside target.
    remove(&tree, RemoveOptions { recursive: true }).unwrap();
    assert!(!exists(&tree).unwrap());
    assert!(exists(&keep_file).unwrap());
    assert_eq!(
        read_text_file(&keep_file, ReadOptions::default()).unwrap(),
        "must survive"
    );

    // Non-recursive remove succeeds on empty dir and regular file, fails on missing.
    let empty_dir = dir.path().join("empty");
    mkdir(&empty_dir, MkdirOptions::default()).unwrap();
    remove(&empty_dir, RemoveOptions::default()).unwrap();
    assert!(!exists(&empty_dir).unwrap());

    let single = dir.path().join("single.txt");
    write_file(&single, b"bye", WriteOptions::default()).unwrap();
    remove(&single, RemoveOptions::default()).unwrap();
    assert!(!exists(&single).unwrap());

    assert_code(
        remove(&single, RemoveOptions::default()),
        FsErrorCode::NotFound,
    );
}

#[test]
fn rename_copy_file_and_realpath() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.txt");
    let dst = dir.path().join("dst.txt");
    write_text_file(&src, "alpha", WriteOptions::default()).unwrap();

    // copy_file with overwrite: false succeeds on new destination.
    copy_file(&src, &dst, CopyOptions::default()).unwrap();
    assert_eq!(
        read_text_file(&dst, ReadOptions::default()).unwrap(),
        "alpha"
    );

    // copy_file with overwrite: false fails with AlreadyExists without clobbering.
    write_text_file(&src, "beta", WriteOptions::default()).unwrap();
    assert_code(
        copy_file(&src, &dst, CopyOptions::default()),
        FsErrorCode::AlreadyExists,
    );
    assert_eq!(
        read_text_file(&dst, ReadOptions::default()).unwrap(),
        "alpha"
    );

    // copy_file with overwrite: false rejects existing symlink (even dangling) without clobbering.
    let link_dst = dir.path().join("link-dst.txt");
    create_symlink(&dir.path().join("nowhere"), &link_dst).unwrap();
    assert_code(
        copy_file(&src, &link_dst, CopyOptions::default()),
        FsErrorCode::AlreadyExists,
    );
    assert!(lstat(&link_dst).unwrap().is_symlink);

    // copy_file with overwrite: true replaces destination.
    copy_file(&src, &dst, CopyOptions { overwrite: true }).unwrap();
    assert_eq!(
        read_text_file(&dst, ReadOptions::default()).unwrap(),
        "beta"
    );

    // copy_file from missing source does not create destination.
    let missing_src = dir.path().join("missing.txt");
    let uncreated_dst = dir.path().join("uncreated.txt");
    assert_code(
        copy_file(&missing_src, &uncreated_dst, CopyOptions::default()),
        FsErrorCode::NotFound,
    );
    assert!(!exists(&uncreated_dst).unwrap());

    // copy_file from directory returns IsADirectory.
    assert_code(
        copy_file(dir.path(), &uncreated_dst, CopyOptions::default()),
        FsErrorCode::IsADirectory,
    );

    // rename moves file and updates existence.
    let moved = dir.path().join("moved.txt");
    rename(&dst, &moved).unwrap();
    assert!(!exists(&dst).unwrap());
    assert_eq!(
        read_text_file(&moved, ReadOptions::default()).unwrap(),
        "beta"
    );
    assert_code(rename(&dst, &moved), FsErrorCode::NotFound);

    // realpath resolves `.` / `..` and symlinks to the canonical path.
    let sub = dir.path().join("sub");
    mkdir(&sub, MkdirOptions::default()).unwrap();
    let link = dir.path().join("moved-link.txt");
    create_symlink(&moved, &link).unwrap();

    let canonical_moved: PathBuf = realpath(&moved).unwrap();
    assert_eq!(realpath(sub.join("../moved.txt")).unwrap(), canonical_moved);
    assert_eq!(realpath(&link).unwrap(), canonical_moved);
    assert_code(realpath(dir.path().join("missing")), FsErrorCode::NotFound);
}

#[test]
fn copy_file_overwrite_self_copy_is_safe_noop() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.txt");
    write_text_file(&file, "keep-me", WriteOptions::default()).unwrap();

    // Identical path.
    copy_file(&file, &file, CopyOptions { overwrite: true }).unwrap();
    assert_eq!(
        read_text_file(&file, ReadOptions::default()).unwrap(),
        "keep-me"
    );

    // `./a` vs `a` and `sub/../a` vs `a`.
    let dot_file = dir.path().join("./a.txt");
    copy_file(&dot_file, &file, CopyOptions { overwrite: true }).unwrap();
    assert_eq!(
        read_text_file(&file, ReadOptions::default()).unwrap(),
        "keep-me"
    );

    let sub = dir.path().join("sub");
    mkdir(&sub, MkdirOptions::default()).unwrap();
    let parent_dot_file = sub.join("../a.txt");
    copy_file(&file, &parent_dot_file, CopyOptions { overwrite: true }).unwrap();
    assert_eq!(
        read_text_file(&file, ReadOptions::default()).unwrap(),
        "keep-me"
    );

    // Symlink alias in both directions.
    let link = dir.path().join("link.txt");
    create_symlink(&file, &link).unwrap();
    copy_file(&file, &link, CopyOptions { overwrite: true }).unwrap();
    assert_eq!(
        read_text_file(&file, ReadOptions::default()).unwrap(),
        "keep-me"
    );
    copy_file(&link, &file, CopyOptions { overwrite: true }).unwrap();
    assert_eq!(
        read_text_file(&file, ReadOptions::default()).unwrap(),
        "keep-me"
    );

    // Hard link alias.
    let hard = dir.path().join("hard.txt");
    std::fs::hard_link(&file, &hard).unwrap();
    copy_file(&file, &hard, CopyOptions { overwrite: true }).unwrap();
    assert_eq!(
        read_text_file(&file, ReadOptions::default()).unwrap(),
        "keep-me"
    );
}

#[test]
fn remove_and_lstat_do_not_follow_symlink_trailing_separators_or_dot() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside");
    mkdir(&outside, MkdirOptions::default()).unwrap();
    let keep = outside.join("keep.txt");
    write_text_file(&keep, "intact", WriteOptions::default()).unwrap();

    let link = dir.path().join("dir-link");
    create_symlink(&outside, &link).unwrap();

    let link_slash = format!("{}/", link.display());
    let link_multi_slash = format!("{}///", link.display());
    let link_dot = format!("{}/.", link.display());
    let link_dot_slash = format!("{}/./", link.display());

    // `lstat` with trailing `/` inspects the symlink itself rather than resolving through it.
    let info = lstat(&link_slash).unwrap();
    assert!(info.is_symlink);
    let info_multi = lstat(&link_multi_slash).unwrap();
    assert!(info_multi.is_symlink);

    // Trailing `/.` and `/./` are rejected with InvalidPath in both `lstat` and `remove`.
    assert_code(lstat(&link_dot), FsErrorCode::InvalidPath);
    assert_code(lstat(&link_dot_slash), FsErrorCode::InvalidPath);
    assert_code(
        remove(&link_dot, RemoveOptions { recursive: true }),
        FsErrorCode::InvalidPath,
    );
    assert_code(
        remove(&link_dot_slash, RemoveOptions { recursive: true }),
        FsErrorCode::InvalidPath,
    );
    assert!(exists(&keep).unwrap());

    // `remove("link/", recursive: true)` unlinks only the symlink and leaves target intact.
    remove(&link_slash, RemoveOptions { recursive: true }).unwrap();
    assert!(
        lstat(&link).is_err(),
        "symlink itself should have been removed"
    );
    assert_eq!(
        read_text_file(&keep, ReadOptions::default()).unwrap(),
        "intact"
    );

    // `write_file_atomic` rejects `.` and trailing `/.`.
    assert_code(
        write_file_atomic(".", b"x", AtomicWriteOptions::default()),
        FsErrorCode::InvalidPath,
    );
    assert_code(
        write_file_atomic(dir.path().join("."), b"x", AtomicWriteOptions::default()),
        FsErrorCode::InvalidPath,
    );
}

#[cfg(unix)]
#[test]
fn copy_file_preserves_private_and_readonly_modes() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let priv_src = dir.path().join("private.txt");
    write_text_file(&priv_src, "secret", WriteOptions::default()).unwrap();
    std::fs::set_permissions(&priv_src, std::fs::Permissions::from_mode(0o600)).unwrap();

    let priv_dst = dir.path().join("private-copy.txt");
    copy_file(&priv_src, &priv_dst, CopyOptions::default()).unwrap();
    let priv_mode = std::fs::metadata(&priv_dst).unwrap().permissions().mode() & 0o777;
    assert_eq!(priv_mode, 0o600);

    let ro_src = dir.path().join("ro.txt");
    write_text_file(&ro_src, "readonly", WriteOptions::default()).unwrap();
    std::fs::set_permissions(&ro_src, std::fs::Permissions::from_mode(0o400)).unwrap();

    let ro_dst = dir.path().join("ro-copy.txt");
    copy_file(&ro_src, &ro_dst, CopyOptions::default()).unwrap();
    let ro_mode = std::fs::metadata(&ro_dst).unwrap().permissions().mode() & 0o777;
    assert_eq!(ro_mode, 0o400);
    assert_eq!(
        read_text_file(&ro_dst, ReadOptions::default()).unwrap(),
        "readonly"
    );
}

#[test]
fn read_file_max_bytes_u64_max_and_special_files() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("small.txt");
    write_text_file(&file, "tiny", WriteOptions::default()).unwrap();

    let bytes = read_file(
        &file,
        ReadOptions {
            max_bytes: Some(u64::MAX),
        },
    )
    .unwrap();
    assert_eq!(bytes, b"tiny");

    #[cfg(target_os = "linux")]
    {
        // `/proc/version` reports metadata length 0 on Linux procfs; bounded `take` still enforces `max_bytes`.
        let proc_path = std::path::Path::new("/proc/version");
        if proc_path.exists() {
            let full = read_file(proc_path, ReadOptions::default()).unwrap();
            assert!(!full.is_empty());

            assert_code(
                read_file(proc_path, ReadOptions { max_bytes: Some(1) }),
                FsErrorCode::TooLarge,
            );

            let bounded = read_file(
                proc_path,
                ReadOptions {
                    max_bytes: Some(1024 * 1024),
                },
            )
            .unwrap();
            assert_eq!(bounded, full);
        }
    }
}

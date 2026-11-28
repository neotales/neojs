use std::ffi::OsStr;
use std::fs::{self, File};
use std::time::Duration;

use vaeljs_exec_core::{
    CommandOutput, CommandSpec, ExecError, ExecErrorCode, FileLock, LockMode, LockOptions,
    MAX_OUTPUT_BYTES, Signal, StdioMode, which, which_all,
};

#[test]
fn command_spec_validates_inputs() {
    assert_eq!(
        CommandSpec::new("").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        CommandSpec::new("bad\0prog").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );

    let base = CommandSpec::new("echo").unwrap();
    assert_eq!(
        base.clone().arg("bad\0arg").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().cwd("").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().cwd("dir\0sub").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().env("", "val").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().env("A=B", "val").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().env("A\0B", "val").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().env("KEY", "v\0al").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().env_remove("BAD=KEY").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone()
            .stdout(StdioMode::Bytes(vec![1]))
            .unwrap_err()
            .code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone()
            .stderr(StdioMode::Bytes(vec![1]))
            .unwrap_err()
            .code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().timeout(Duration::ZERO).unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.clone().timeout(Duration::MAX).unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        base.max_output_bytes(MAX_OUTPUT_BYTES + 1)
            .unwrap_err()
            .code(),
        ExecErrorCode::InvalidArgument
    );
}

#[test]
fn signal_parse_and_display_round_trip() {
    let cases = [
        ("sigint", Signal::SigInt, "SIGINT"),
        ("INT", Signal::SigInt, "SIGINT"),
        ("SIGTERM", Signal::SigTerm, "SIGTERM"),
        ("term", Signal::SigTerm, "SIGTERM"),
        ("SIGKILL", Signal::SigKill, "SIGKILL"),
        ("SIGHUP", Signal::SigHup, "SIGHUP"),
        ("SIGQUIT", Signal::SigQuit, "SIGQUIT"),
        ("SIGUSR1", Signal::SigUsr1, "SIGUSR1"),
        ("usr2", Signal::SigUsr2, "SIGUSR2"),
    ];
    for (input, expected, canonical) in cases {
        let parsed = Signal::parse(input).unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(parsed.as_str(), canonical);
        assert_eq!(parsed.to_string(), canonical);
    }

    assert_eq!(
        Signal::parse("").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        Signal::parse("SIGUNKNOWN").unwrap_err().code(),
        ExecErrorCode::InvalidArgument
    );
}

#[test]
fn command_output_text_and_lines_helpers() {
    let out = CommandOutput {
        code: 0,
        signal: None,
        success: true,
        stdout: b"alpha\nbeta\r\ngamma\n".to_vec(),
        stderr: b"err1\nerr2".to_vec(),
        timed_out: false,
        truncated: false,
    };
    assert_eq!(out.text(), "alpha\nbeta\r\ngamma\n");
    assert_eq!(out.lines(), vec!["alpha", "beta", "gamma"]);
    assert_eq!(out.error_text(), "err1\nerr2");
    assert_eq!(out.error_lines(), vec!["err1", "err2"]);
}

#[test]
fn which_and_which_all_lookup_behavior() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_a = tmp.path().join("bin_a");
    let dir_b = tmp.path().join("bin_b");
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();

    // Directory with the target name must NOT be matched as an executable.
    let sub_dir = dir_a.join("my_vael_tool");
    fs::create_dir_all(&sub_dir).unwrap();
    assert_eq!(
        which(sub_dir.to_str().unwrap(), None).unwrap(),
        None,
        "directories must not be matched by which"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        // Non-executable regular file in dir_a should be skipped.
        let non_exec = dir_a.join("tool_x");
        fs::write(&non_exec, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&non_exec, fs::Permissions::from_mode(0o644)).unwrap();

        let exec_b = dir_b.join("tool_x");
        fs::write(&exec_b, b"#!/bin/sh\n").unwrap();
        fs::set_permissions(&exec_b, fs::Permissions::from_mode(0o755)).unwrap();

        let prepend = format!("{}:{}", dir_a.display(), dir_b.display());
        assert_eq!(
            which("tool_x", Some(OsStr::new(&prepend))).unwrap(),
            Some(exec_b.clone())
        );

        // Explicit executable path works directly without PATH.
        assert_eq!(
            which(exec_b.to_str().unwrap(), None).unwrap(),
            Some(exec_b.clone())
        );
        assert_eq!(which(non_exec.to_str().unwrap(), None).unwrap(), None);

        // Make dir_a/tool_x executable too and verify which_all returns both in order.
        fs::set_permissions(&non_exec, fs::Permissions::from_mode(0o755)).unwrap();
        let all = which_all("tool_x", Some(OsStr::new(&prepend))).unwrap();
        assert_eq!(all, vec![non_exec, exec_b]);
    }

    #[cfg(windows)]
    {
        let exe_a = dir_a.join("tool_x.exe");
        let exe_b = dir_b.join("tool_x.exe");
        fs::write(&exe_a, b"stub").unwrap();
        fs::write(&exe_b, b"stub").unwrap();
        let prepend = format!("{};{}", dir_a.display(), dir_b.display());
        assert_eq!(
            which("tool_x", Some(OsStr::new(&prepend))).unwrap(),
            Some(exe_a.clone())
        );
        let all = which_all("tool_x", Some(OsStr::new(&prepend))).unwrap();
        assert_eq!(all, vec![exe_a, exe_b]);
    }

    assert_eq!(
        which("definitely_nonexistent_vaeljs_binary_xyz_99999", None).unwrap(),
        None
    );
    assert!(
        which_all("definitely_nonexistent_vaeljs_binary_xyz_99999", None)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn file_lock_exclusive_shared_nonblocking_timeout_and_symlink_safety() {
    let tmp = tempfile::tempdir().unwrap();
    let lock_path = tmp.path().join("resource.lock");

    // 1. Exclusive lock blocks a second exclusive lock.
    let guard1 = FileLock::acquire(&lock_path, &LockOptions::default()).unwrap();
    assert_eq!(guard1.mode(), LockMode::Exclusive);

    let err_nb = FileLock::acquire(
        &lock_path,
        &LockOptions::new(LockMode::Exclusive).non_blocking(true),
    )
    .unwrap_err();
    assert!(matches!(err_nb, ExecError::LockWouldBlock));
    assert_eq!(err_nb.code(), ExecErrorCode::LockWouldBlock);

    let err_to = FileLock::acquire(
        &lock_path,
        &LockOptions::new(LockMode::Exclusive).timeout(Duration::from_millis(40)),
    )
    .unwrap_err();
    assert!(matches!(err_to, ExecError::LockTimeout));
    assert_eq!(err_to.code(), ExecErrorCode::LockTimeout);

    // 2. Dropping lock allows immediate re-acquisition.
    drop(guard1);
    let guard2 = FileLock::acquire(
        &lock_path,
        &LockOptions::new(LockMode::Exclusive).non_blocking(true),
    )
    .unwrap();
    guard2.unlock().unwrap();

    // 3. Shared locks allow concurrent shared locks, while exclusive fails.
    let s1 = FileLock::acquire(
        &lock_path,
        &LockOptions::new(LockMode::Shared).non_blocking(true),
    )
    .unwrap();
    let s2 = FileLock::acquire(
        &lock_path,
        &LockOptions::new(LockMode::Shared).non_blocking(true),
    )
    .unwrap();
    let ex_while_shared = FileLock::acquire(
        &lock_path,
        &LockOptions::new(LockMode::Exclusive).non_blocking(true),
    )
    .unwrap_err();
    assert_eq!(ex_while_shared.code(), ExecErrorCode::LockWouldBlock);

    drop(s1);
    // Still held by s2.
    assert_eq!(
        FileLock::acquire(
            &lock_path,
            &LockOptions::new(LockMode::Exclusive).non_blocking(true),
        )
        .unwrap_err()
        .code(),
        ExecErrorCode::LockWouldBlock
    );
    drop(s2);

    // Now exclusive succeeds again.
    let guard3 = FileLock::acquire(
        &lock_path,
        &LockOptions::new(LockMode::Exclusive).non_blocking(true),
    )
    .unwrap();
    drop(guard3);

    // 4. Symlink lock path must be rejected.
    let real_target = tmp.path().join("real.lock");
    File::create(&real_target).unwrap();
    let symlink_path = tmp.path().join("link.lock");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_target, &symlink_path).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&real_target, &symlink_path).unwrap();

    let sym_err = FileLock::acquire(&symlink_path, &LockOptions::default()).unwrap_err();
    assert_eq!(sym_err.code(), ExecErrorCode::InvalidArgument);

    // 5. Validation errors on lock paths and options.
    assert_eq!(
        FileLock::acquire(std::path::Path::new(""), &LockOptions::default())
            .unwrap_err()
            .code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        FileLock::acquire(std::path::Path::new("bad\0.lock"), &LockOptions::default())
            .unwrap_err()
            .code(),
        ExecErrorCode::InvalidArgument
    );
    assert_eq!(
        FileLock::acquire(
            &lock_path,
            &LockOptions::new(LockMode::Exclusive)
                .non_blocking(true)
                .timeout(Duration::from_millis(10)),
        )
        .unwrap_err()
        .code(),
        ExecErrorCode::InvalidArgument
    );
}
